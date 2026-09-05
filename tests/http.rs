//! The HTTP surface, exercised over a real socket on port 0 so these can run in
//! parallel. Port 9500 is global to the machine; only the port-conflict
//! acceptance check is allowed to take it.

use std::sync::Arc;

use lanyard_cli::app::{self, AppState};
use lanyard_cli::config::Config;
use lanyard_cli::keys::{SigningKey, DEFAULT_DEV_KEY_PEM};
use lanyard_cli::persona::Personas;

/// A server on an ephemeral port, with the issuer still the canonical fixed
/// string — the issuer never follows the address lanyard happens to be reached
/// at, and these tests would hide that if they moved it.
async fn spawn() -> String {
    spawn_with(Personas::builtin()).await
}

async fn spawn_with(personas: Personas) -> String {
    let config = Config::resolve(|key| match key {
        "HOME" => Some("/nonexistent".to_string()),
        _ => None,
    })
    .unwrap();
    let key = SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap();
    let state = Arc::new(AppState {
        config,
        key,
        personas,
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app::router(state)).await.unwrap();
    });
    format!("http://{addr}")
}

/// Decode a token's payload without verifying it. Verification against the live
/// JWKS is `jose`'s job in the acceptance run — north star 4 — not something
/// this crate should do to its own output.
fn payload_of(token: &str) -> serde_json::Value {
    let segment = token.split('.').nth(1).expect("compact JWS");
    serde_json::from_slice(&lanyard_cli::b64::decode(segment).unwrap()).unwrap()
}

async fn post_token(base: &str, query: &str, body: &str) -> (u16, serde_json::Value) {
    let res = reqwest::Client::new()
        .post(format!("{base}/_/api/token{query}"))
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap())
}

const ISSUER: &str = "http://127.0.0.1:9500/oidc";

#[tokio::test]
async fn root_is_left_free() {
    let base = spawn().await;
    let res = reqwest::get(format!("{base}/")).await.unwrap();
    assert_eq!(res.status().as_u16(), 404, "root must stay unrouted");
}

#[tokio::test]
async fn jwks_serves_one_public_key_and_nothing_private() {
    let base = spawn().await;
    let res = reqwest::get(format!("{base}/oidc/jwks")).await.unwrap();

    assert_eq!(res.status().as_u16(), 200);
    assert_eq!(
        res.headers().get("cache-control").unwrap(),
        "no-store",
        "a cached JWKS makes Phase 3's --unknown-kid and any rotation confusing"
    );

    let body: serde_json::Value = res.json().await.unwrap();
    let keys = body["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["kty"], "RSA");
    assert_eq!(keys[0]["alg"], "RS256");
    assert_eq!(keys[0]["use"], "sig");
    assert!(keys[0]["kid"].as_str().unwrap().len() > 8);

    let raw = serde_json::to_string(&body).unwrap();
    for private in [r#""d":"#, r#""p":"#, r#""q":"#, r#""dp":"#, r#""qi":"#] {
        assert!(
            !raw.contains(private),
            "private material in JWKS: {private}"
        );
    }
}

#[tokio::test]
async fn discovery_advertises_only_what_exists() {
    let base = spawn().await;
    let doc: serde_json::Value =
        reqwest::get(format!("{base}/oidc/.well-known/openid-configuration"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

    assert_eq!(doc["issuer"], ISSUER);
    assert_eq!(doc["jwks_uri"], format!("{ISSUER}/jwks"));
    assert_eq!(doc["id_token_signing_alg_values_supported"][0], "RS256");
    assert!(doc["response_types_supported"].is_array());
    assert!(doc["subject_types_supported"].is_array());

    // The document stays honest in both directions: it advertises the token
    // endpoint in the phase that implements it.
    assert_eq!(doc["token_endpoint"], format!("{ISSUER}/token"));
    assert_eq!(
        doc["grant_types_supported"],
        serde_json::json!(["client_credentials"])
    );
    // All three are true, because none of them are checked.
    assert_eq!(
        doc["token_endpoint_auth_methods_supported"],
        serde_json::json!(["none", "client_secret_basic", "client_secret_post"])
    );

    // Honest, not aspirational: an advertised endpoint that 404s sends a client
    // down a path that fails later and further away.
    for unimplemented in [
        "authorization_endpoint",
        "userinfo_endpoint",
        "end_session_endpoint",
        "introspection_endpoint",
        "revocation_endpoint",
    ] {
        assert!(
            doc.get(unimplemented).is_none(),
            "{unimplemented} is advertised but not implemented until a later phase"
        );
    }
}

#[tokio::test]
async fn the_issuer_does_not_follow_the_host_header() {
    let base = spawn().await;
    let doc: serde_json::Value = reqwest::Client::new()
        .get(format!("{base}/oidc/.well-known/openid-configuration"))
        .header("host", "lanyard:9500")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // CONCEPT §8. The document is being fetched over an ephemeral port under a
    // different Host and must still name the one configured issuer.
    assert_eq!(doc["issuer"], ISSUER);
    assert_eq!(doc["jwks_uri"], format!("{ISSUER}/jwks"));
    assert_eq!(doc["token_endpoint"], format!("{ISSUER}/token"));
}

// ---------------------------------------------------------------- the seam --
//
// The body is the claims. The query string is how they are minted. That split
// is what keeps Phase 3's `flaw=` additive.

#[tokio::test]
async fn the_seam_signs_the_posted_claims_and_echoes_them_back() {
    let base = spawn().await;
    let (status, body) = post_token(&base, "", r#"{"sub":"ada","aud":"billing-api"}"#).await;

    assert_eq!(status, 200);
    let token = body["token"].as_str().unwrap();
    assert_eq!(payload_of(token), body["claims"]);
    assert_eq!(body["claims"]["sub"], "ada");
    assert_eq!(body["claims"]["aud"], "billing-api");
    assert_eq!(body["claims"]["iss"], ISSUER);
}

#[tokio::test]
async fn an_absent_or_empty_body_is_fine() {
    let base = spawn().await;
    for body in ["", "{}"] {
        let (status, out) = post_token(&base, "?persona=ada", body).await;
        assert_eq!(status, 200, "body {body:?} was rejected: {out}");
        assert_eq!(out["claims"]["sub"], "ada");
    }
}

#[tokio::test]
async fn ttl_defaults_to_sixty_seconds_and_the_query_moves_it() {
    let base = spawn().await;

    let (_, body) = post_token(&base, "", "{}").await;
    let c = &body["claims"];
    assert_eq!(c["exp"].as_u64().unwrap() - c["iat"].as_u64().unwrap(), 60);

    let (_, body) = post_token(&base, "?ttl=300", "{}").await;
    let c = &body["claims"];
    assert_eq!(c["exp"].as_u64().unwrap() - c["iat"].as_u64().unwrap(), 300);
}

#[tokio::test]
async fn a_persona_query_mints_the_personas_claims() {
    let base = spawn().await;

    let (_, body) = post_token(&base, "?persona=ada", "{}").await;
    let c = &body["claims"];
    assert_eq!(c["sub"], "ada");
    assert_eq!(c["email"], "ada@example.test");
    assert_eq!(c["email_verified"], true);
    assert_eq!(c["roles"][0], "admin");

    let (_, body) = post_token(&base, "?persona=nobody", "{}").await;
    let c = body["claims"].as_object().unwrap();
    assert_eq!(c["sub"], "nobody");
    for absent in ["email", "name", "roles"] {
        assert!(!c.contains_key(absent), "nobody must not carry {absent}");
    }
}

#[tokio::test]
async fn body_claims_override_persona_claims_over_http() {
    let base = spawn().await;
    let (_, body) = post_token(&base, "?persona=ada", r#"{"email":"other@example.test"}"#).await;
    assert_eq!(body["claims"]["email"], "other@example.test");
}

#[tokio::test]
async fn an_unknown_persona_is_a_400_naming_the_id() {
    let base = spawn().await;
    let (status, body) = post_token(&base, "?persona=nope", "{}").await;

    assert_eq!(status, 400);
    assert!(body["error"].is_string());
    let described = body["error_description"].as_str().unwrap();
    assert!(
        described.contains("nope"),
        "must quote the id back: {described}"
    );
}

#[tokio::test]
async fn a_non_json_body_is_a_400_and_the_server_survives() {
    let base = spawn().await;
    let (status, body) = post_token(&base, "", "not json").await;

    assert_eq!(status, 400);
    assert!(body["error"].is_string());
    assert!(body["error_description"].is_string());

    // Still serving.
    let (status, _) = post_token(&base, "", r#"{"sub":"ada"}"#).await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn a_body_that_is_not_an_object_is_a_400() {
    let base = spawn().await;
    for body in ["[1,2,3]", "\"ada\"", "42"] {
        let (status, out) = post_token(&base, "", body).await;
        assert_eq!(status, 400, "body {body:?} should be rejected: {out}");
    }
}

#[tokio::test]
async fn a_non_numeric_ttl_is_a_400() {
    let base = spawn().await;
    let (status, body) = post_token(&base, "?ttl=soon", "{}").await;
    assert_eq!(status, 400);
    assert!(body["error_description"].as_str().unwrap().contains("soon"));
}

// ------------------------------------------------------------- the personas --

#[tokio::test]
async fn personas_lists_the_built_in_defaults() {
    let base = spawn().await;
    let body: serde_json::Value = reqwest::get(format!("{base}/_/api/personas"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let list = body["personas"].as_array().unwrap();
    let ids: Vec<&str> = list.iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["ada", "mira", "nobody"]);

    let nobody = list.iter().find(|p| p["id"] == "nobody").unwrap();
    assert!(nobody.get("email").is_none());
    assert_eq!(nobody["roles"], serde_json::json!([]));

    let ada = list.iter().find(|p| p["id"] == "ada").unwrap();
    assert_eq!(ada["name"], "Ada Bell");
    assert_eq!(ada["email"], "ada@example.test");
    assert_eq!(ada["roles"], serde_json::json!(["admin", "user"]));
}

/// Both `client:` levels are echoed back so that whichever shape Phase 7 wants
/// is already there. Nothing filters on it — every persona is still listed.
#[tokio::test]
async fn personas_echoes_client_at_both_levels() {
    let personas = Personas::parse(
        "client: billing-web\npersonas:\n  - id: ada\n  - id: ops\n    client: ops-console\n",
        std::path::Path::new("/tmp/users.yaml"),
    )
    .unwrap();
    let base = spawn_with(personas).await;

    let body: serde_json::Value = reqwest::get(format!("{base}/_/api/personas"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["client"], "billing-web");
    let list = body["personas"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert!(list[0].get("client").is_none());
    assert_eq!(list[1]["client"], "ops-console");
}

// ----------------------------------------------------------- /oidc/token --
//
// The second caller of `oidc::issue`, and the first one a real SDK would use.
// The handler builds no claims: it maps form parameters to an overrides map and
// calls the one function (north star 3).

/// Post a `client_credentials`-shaped form the way `curl -d` does, and hand back
/// enough of the response to assert on the OAuth envelope as well as the body.
async fn post_grant(
    base: &str,
    form: &[(&str, &str)],
) -> (u16, reqwest::header::HeaderMap, serde_json::Value) {
    let res = reqwest::Client::new()
        .post(format!("{base}/oidc/token"))
        .form(form)
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    let headers = res.headers().clone();
    (status, headers, res.json().await.unwrap())
}

#[tokio::test]
async fn the_grant_mints_a_persona_token() {
    let base = spawn().await;
    let (status, _, body) = post_grant(
        &base,
        &[("grant_type", "client_credentials"), ("persona", "ada")],
    )
    .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 60);

    let token = body["access_token"].as_str().expect("access_token");
    let claims = payload_of(token);
    assert_eq!(claims["sub"], "ada");
    assert_eq!(claims["email"], "ada@example.test");
    assert_eq!(claims["iss"], ISSUER);
    assert_eq!(
        claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap(),
        60,
        "the grant uses the default TTL, not one of its own"
    );
}

/// North star 1, arriving at the endpoint where every other IdP would put a
/// client registry. All four shapes mint.
#[tokio::test]
async fn any_client_authentication_mints_and_so_does_none() {
    let base = spawn().await;
    let client = reqwest::Client::new();
    let grant = [("grant_type", "client_credentials"), ("persona", "ada")];

    let none = client.post(format!("{base}/oidc/token")).form(&grant);
    let basic = client
        .post(format!("{base}/oidc/token"))
        .basic_auth("anything", Some("whatever"))
        .form(&grant);
    let post = client.post(format!("{base}/oidc/token")).form(&[
        ("grant_type", "client_credentials"),
        ("persona", "ada"),
        ("client_id", "x"),
        ("client_secret", "y"),
    ]);
    let id_only = client.post(format!("{base}/oidc/token")).form(&[
        ("grant_type", "client_credentials"),
        ("persona", "ada"),
        ("client_id", "x"),
    ]);

    for (label, request) in [
        ("no client auth", none),
        ("basic", basic),
        ("client_secret_post", post),
        ("client_id alone", id_only),
    ] {
        let res = request.send().await.unwrap();
        assert_eq!(res.status().as_u16(), 200, "{label} was rejected");
    }
}

#[tokio::test]
async fn client_id_becomes_a_claim_when_the_request_supplies_one() {
    let base = spawn().await;

    // Absent when nothing was sent — a token that claims a client it was never
    // given would be a lie Phase 6's log and Phase 7's namespacing would key on.
    let (_, _, body) = post_grant(
        &base,
        &[("grant_type", "client_credentials"), ("persona", "ada")],
    )
    .await;
    let claims = payload_of(body["access_token"].as_str().unwrap());
    assert!(
        claims.as_object().unwrap().get("client_id").is_none(),
        "no client_id was sent, so none belongs in the token: {claims}"
    );

    // From the form.
    let (_, _, body) = post_grant(
        &base,
        &[
            ("grant_type", "client_credentials"),
            ("persona", "ada"),
            ("client_id", "billing-web"),
        ],
    )
    .await;
    assert_eq!(
        payload_of(body["access_token"].as_str().unwrap())["client_id"],
        "billing-web"
    );

    // From HTTP Basic, whose username is the client id per RFC 6749 §2.3.1.
    let body: serde_json::Value = reqwest::Client::new()
        .post(format!("{base}/oidc/token"))
        .basic_auth("ops-console", Some("unchecked"))
        .form(&[("grant_type", "client_credentials"), ("persona", "ada")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        payload_of(body["access_token"].as_str().unwrap())["client_id"],
        "ops-console"
    );
}

#[tokio::test]
async fn audience_becomes_aud_and_resource_is_a_synonym() {
    let base = spawn().await;

    let (_, _, body) = post_grant(
        &base,
        &[
            ("grant_type", "client_credentials"),
            ("persona", "ada"),
            ("audience", "billing-api"),
        ],
    )
    .await;
    assert_eq!(
        payload_of(body["access_token"].as_str().unwrap())["aud"],
        "billing-api"
    );

    // RFC 8707 spells it `resource`; Auth0 taught everyone `audience`. Both
    // work, so a real SDK doing client credentials mints whichever it sends.
    let (_, _, body) = post_grant(
        &base,
        &[
            ("grant_type", "client_credentials"),
            ("persona", "ada"),
            ("resource", "billing-api"),
        ],
    )
    .await;
    assert_eq!(
        payload_of(body["access_token"].as_str().unwrap())["aud"],
        "billing-api"
    );

    // `audience` wins if both are sent.
    let (_, _, body) = post_grant(
        &base,
        &[
            ("grant_type", "client_credentials"),
            ("persona", "ada"),
            ("resource", "from-resource"),
            ("audience", "from-audience"),
        ],
    )
    .await;
    assert_eq!(
        payload_of(body["access_token"].as_str().unwrap())["aud"],
        "from-audience"
    );
}

/// A token with no `aud` is legitimate and useful: it is what an API that
/// forgets to validate audience will happily accept.
#[tokio::test]
async fn no_audience_asked_for_means_no_aud_key_at_all() {
    let base = spawn().await;
    let (_, _, body) = post_grant(
        &base,
        &[("grant_type", "client_credentials"), ("persona", "ada")],
    )
    .await;
    let claims = payload_of(body["access_token"].as_str().unwrap());
    assert!(
        claims.as_object().unwrap().get("aud").is_none(),
        "an empty or null aud is not the same as no aud: {claims}"
    );
}

#[tokio::test]
async fn scope_becomes_a_claim_and_is_echoed_in_the_response() {
    let base = spawn().await;
    let (_, _, body) = post_grant(
        &base,
        &[
            ("grant_type", "client_credentials"),
            ("persona", "ada"),
            ("scope", "orders:read orders:write"),
        ],
    )
    .await;

    assert_eq!(body["scope"], "orders:read orders:write");
    assert_eq!(
        payload_of(body["access_token"].as_str().unwrap())["scope"],
        "orders:read orders:write"
    );
}

#[tokio::test]
async fn the_response_envelope_is_no_store_and_carries_nothing_extra() {
    let base = spawn().await;
    let (status, headers, body) = post_grant(
        &base,
        &[("grant_type", "client_credentials"), ("persona", "ada")],
    )
    .await;

    assert_eq!(status, 200);
    assert_eq!(headers.get("cache-control").unwrap(), "no-store");

    let object = body.as_object().unwrap();
    // No `scope` unless one was requested; never an `id_token`, because client
    // credentials has no user authentication event to attest to; never a
    // `refresh_token`, because there is nothing to refresh.
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["access_token", "expires_in", "token_type"]);
}

// The endpoint's 4xx shapes. Every one of them is ours and carries the OAuth
// `{"error","error_description"}` envelope — never axum's 415 or 422.

async fn post_raw(base: &str, content_type: Option<&str>, body: &str) -> (u16, serde_json::Value) {
    let mut request = reqwest::Client::new()
        .post(format!("{base}/oidc/token"))
        .body(body.to_string());
    if let Some(content_type) = content_type {
        request = request.header("content-type", content_type);
    }
    let res = request.send().await.unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap())
}

#[tokio::test]
async fn an_unimplemented_grant_is_unsupported_and_a_missing_one_is_invalid() {
    let base = spawn().await;

    let (status, _, body) = post_grant(&base, &[("grant_type", "password")]).await;
    assert_eq!(status, 400);
    assert_eq!(body["error"], "unsupported_grant_type");
    assert!(body["error_description"]
        .as_str()
        .unwrap()
        .contains("password"));

    // Nothing was named, so nothing can be unsupported.
    for form in [vec![("persona", "ada")], vec![("grant_type", "")]] {
        let (status, _, body) = post_grant(&base, &form).await;
        assert_eq!(status, 400, "{form:?} should be invalid_request");
        assert_eq!(body["error"], "invalid_request", "for {form:?}");
    }
}

#[tokio::test]
async fn an_unknown_persona_on_the_grant_is_a_400_naming_the_id() {
    let base = spawn().await;
    let (status, _, body) = post_grant(
        &base,
        &[("grant_type", "client_credentials"), ("persona", "nope")],
    )
    .await;

    assert_eq!(status, 400);
    assert_eq!(body["error"], "invalid_request");
    assert!(
        body["error_description"].as_str().unwrap().contains("nope"),
        "the CLI surfaces this description verbatim, so it must name the id"
    );

    // Still serving.
    let (status, _, _) = post_grant(
        &base,
        &[("grant_type", "client_credentials"), ("persona", "ada")],
    )
    .await;
    assert_eq!(status, 200);
}

/// Unknown parameters are ignored, per OAuth's own rule. This is the extension
/// point: Phase 3's `flaw=alg-none` and Phase 4's `code`/`code_verifier` land
/// here without touching the contract.
#[tokio::test]
async fn unknown_parameters_are_ignored() {
    let base = spawn().await;
    let (status, _, body) = post_grant(
        &base,
        &[
            ("grant_type", "client_credentials"),
            ("persona", "ada"),
            ("flaw", "alg-none"),
            ("code_verifier", "not-a-thing-yet"),
        ],
    )
    .await;
    assert_eq!(status, 200);
    let claims = payload_of(body["access_token"].as_str().unwrap());
    for ignored in ["flaw", "code_verifier"] {
        assert!(
            claims.as_object().unwrap().get(ignored).is_none(),
            "{ignored} leaked into the claims"
        );
    }
}

/// The content-type header is not consulted at all: a well-formed body mints
/// whatever curl did or did not label it, and an unreadable one is our 400
/// rather than axum's 415 or 422.
#[tokio::test]
async fn the_body_is_read_without_consulting_the_content_type() {
    let base = spawn().await;

    for content_type in [
        None,
        Some("application/x-www-form-urlencoded"),
        Some("text/plain"),
    ] {
        let (status, body) = post_raw(
            &base,
            content_type,
            "grant_type=client_credentials&persona=ada",
        )
        .await;
        assert_eq!(status, 200, "content-type {content_type:?}: {body}");
    }

    // A body that is not a form at all is not a 415 or a 422 — it is unknown
    // parameters, and therefore the ordinary "you named no grant" 400.
    for garbage in [
        r#"{"grant_type":"client_credentials"}"#,
        "not a form at all",
    ] {
        let (status, body) = post_raw(&base, None, garbage).await;
        assert_eq!(status, 400, "{garbage:?}");
        assert_eq!(body["error"], "invalid_request", "{garbage:?}");
        assert!(body["error_description"].is_string(), "{garbage:?}");
    }
}

#[tokio::test]
async fn the_token_endpoint_is_post_only() {
    let base = spawn().await;
    let res = reqwest::get(format!("{base}/oidc/token")).await.unwrap();
    assert_eq!(res.status().as_u16(), 405);
}

// ------------------------------------------------ one function, observable --

/// **North star 3, finally falsifiable.** Phase 1 shipped `issue()` with a
/// single caller, which made "issuance is one function" unfalsifiable. Two
/// callers — the seam and the grant — must produce the same claim set for the
/// same request. If claim shaping ever appears in `oidc::token`, this is what
/// fails.
#[tokio::test]
async fn the_seam_and_the_grant_produce_the_same_claims() {
    let base = spawn().await;

    let (_, seam) = post_token(&base, "?persona=ada", r#"{"aud":"billing-api"}"#).await;
    let (_, _, grant) = post_grant(
        &base,
        &[
            ("grant_type", "client_credentials"),
            ("persona", "ada"),
            ("audience", "billing-api"),
        ],
    )
    .await;

    let mut through_the_seam = seam["claims"].as_object().unwrap().clone();
    let mut through_the_grant = payload_of(grant["access_token"].as_str().unwrap())
        .as_object()
        .unwrap()
        .clone();

    // The five that are per-request by construction: three timestamps, a fresh
    // id, and `client_id`, which the grant carries only because the request
    // supplied one and the seam has nowhere to supply it from.
    for per_request in ["iat", "nbf", "exp", "jti", "client_id"] {
        through_the_seam.remove(per_request);
        through_the_grant.remove(per_request);
    }

    assert_eq!(through_the_seam, through_the_grant);
    // Guard against the test passing because both sides are empty.
    assert_eq!(through_the_grant["sub"], "ada");
    assert_eq!(through_the_grant["aud"], "billing-api");
    assert_eq!(through_the_grant["email"], "ada@example.test");
}
