//! The HTTP surface, exercised over a real socket on port 0 so these can run in
//! parallel. Port 9500 is global to the machine; only the port-conflict
//! acceptance check is allowed to take it.

use std::sync::Arc;

use lanyard_cli::app::{self, AppState};
use lanyard_cli::config::Config;
use lanyard_cli::keys::{SigningKey, DEFAULT_DEV_KEY_PEM};
use lanyard_cli::persona::Personas;
use lanyard_cli::store::Stores;

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
        stores: Stores::default(),
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

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

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
    assert!(doc["subject_types_supported"].is_array());

    // The document stays honest in both directions: it advertises the token
    // endpoint in the phase that implements it.
    assert_eq!(doc["token_endpoint"], format!("{ISSUER}/token"));
    assert_eq!(
        doc["grant_types_supported"],
        serde_json::json!(["client_credentials", "authorization_code", "refresh_token"])
    );

    // Phase 4 implemented these two, so the document says so.
    assert_eq!(doc["authorization_endpoint"], format!("{ISSUER}/authorize"));
    assert_eq!(doc["userinfo_endpoint"], format!("{ISSUER}/userinfo"));
    assert_eq!(
        doc["code_challenge_methods_supported"],
        serde_json::json!(["S256", "plain"])
    );
    assert_eq!(
        doc["response_modes_supported"],
        serde_json::json!(["query", "form_post"])
    );
    assert_eq!(
        doc["scopes_supported"],
        serde_json::json!(["openid", "email", "profile", "offline_access"])
    );

    // **Exactly `["code"]`.** Phase 1 wrote `["id_token", "code"]` when nothing
    // implemented either; advertising `id_token` now would tell a .NET app that
    // its default setting is supported and then reject it at request time.
    assert_eq!(
        doc["response_types_supported"],
        serde_json::json!(["code"]),
        "advertising a response type /authorize refuses is the fail-later failure \
         this rule exists to prevent"
    );
    // All three are true, because none of them are checked.
    assert_eq!(
        doc["token_endpoint_auth_methods_supported"],
        serde_json::json!(["none", "client_secret_basic", "client_secret_post"])
    );

    // **The phase where "advertise only what exists" cuts the other way.** Until
    // Phase 5 this loop asserted these three were *absent*; now they exist, and
    // .NET reads `end_session_endpoint` out of this document to build its logout
    // redirect — so an endpoint that exists and is not advertised fails exactly
    // as confusingly as one advertised and missing. This asserts both halves:
    // the URL is there, and it answers.
    for (advertised, path) in [
        ("end_session_endpoint", "end_session"),
        ("introspection_endpoint", "introspect"),
        ("revocation_endpoint", "revoke"),
    ] {
        assert_eq!(
            doc[advertised],
            format!("{ISSUER}/{path}"),
            "{advertised} has to be in the document in the phase that implements it"
        );
        let res = reqwest::Client::new()
            .post(format!("{base}/oidc/{path}"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body("")
            .send()
            .await
            .unwrap();
        assert!(
            res.status().as_u16() < 400,
            "{advertised} is advertised and does not answer: {}",
            res.status()
        );
    }

    // All three are true here for the same reason they are true at the token
    // endpoint: none of them are checked.
    for methods in [
        "introspection_endpoint_auth_methods_supported",
        "revocation_endpoint_auth_methods_supported",
    ] {
        assert_eq!(
            doc[methods],
            serde_json::json!(["none", "client_secret_basic", "client_secret_post"]),
            "{methods}"
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

// ------------------------------------------------------ the grant's `flaw` --

/// `expires_in` is the token's actual remaining life, floored at zero, so
/// `flaw=expired` reports `0` rather than claiming 60 seconds it does not have.
/// No new field: an SDK reading this response should see nothing unusual,
/// because the whole point is that the token looks ordinary until it is
/// validated.
#[tokio::test]
async fn an_expired_grant_reports_no_remaining_life_and_no_flaw() {
    let base = spawn().await;
    let (status, _, body) = post_grant(
        &base,
        &[
            ("grant_type", "client_credentials"),
            ("persona", "ada"),
            ("audience", "billing-api"),
            ("flaw", "expired"),
        ],
    )
    .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["expires_in"], 0, "it has no life left to report");

    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["access_token", "expires_in", "token_type"],
        "the OAuth envelope stays exactly the shape an SDK expects"
    );

    let claims = payload_of(body["access_token"].as_str().unwrap());
    assert!(claims["exp"].as_u64().unwrap() < now(), "{claims}");
    assert_eq!(claims["aud"], "billing-api", "only the clock is wrong");
}

/// The three that are not claims reach the grant too — the CLI's
/// `--bad-signature` is this request, not a local string edit.
#[tokio::test]
async fn the_header_level_flaws_reach_the_grant() {
    let base = spawn().await;

    let (_, _, unsigned) = post_grant(
        &base,
        &[
            ("grant_type", "client_credentials"),
            ("persona", "ada"),
            ("flaw", "alg-none"),
        ],
    )
    .await;
    let token = unsigned["access_token"].as_str().unwrap();
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[2], "");
    assert_eq!(
        String::from_utf8(lanyard_cli::b64::decode(parts[0]).unwrap()).unwrap(),
        r#"{"alg":"none","typ":"JWT"}"#
    );

    let (_, _, wrong_key) = post_grant(
        &base,
        &[
            ("grant_type", "client_credentials"),
            ("persona", "ada"),
            ("flaw", "unknown-kid"),
        ],
    )
    .await;
    let header: serde_json::Value = serde_json::from_slice(
        &lanyard_cli::b64::decode(
            wrong_key["access_token"]
                .as_str()
                .unwrap()
                .split('.')
                .next()
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(header["kid"], "lanyard-unknown-kid");
    assert_eq!(header["alg"], "RS256");
}

#[tokio::test]
async fn an_unknown_flaw_on_the_grant_is_a_400_naming_it_and_all_six() {
    let base = spawn().await;
    let (status, _, body) = post_grant(
        &base,
        &[
            ("grant_type", "client_credentials"),
            ("persona", "ada"),
            ("flaw", "expried"),
        ],
    )
    .await;

    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], "invalid_request");
    let description = body["error_description"].as_str().unwrap();
    assert!(description.contains("expried"), "{description}");
    for name in [
        "expired",
        "wrong-aud",
        "wrong-iss",
        "bad-signature",
        "alg-none",
        "unknown-kid",
    ] {
        assert!(description.contains(name), "{name} missing: {description}");
    }
    assert!(
        body.as_object().unwrap().get("access_token").is_none(),
        "a typo must not mint anything"
    );

    // Still serving.
    let (status, _, _) = post_grant(
        &base,
        &[("grant_type", "client_credentials"), ("persona", "ada")],
    )
    .await;
    assert_eq!(status, 200);
}

/// `-d flaw=` is the shell saying nothing, exactly as `-d scope=` is.
#[tokio::test]
async fn an_empty_flaw_reads_as_absent_on_the_grant() {
    let base = spawn().await;
    let (status, _, body) = post_grant(
        &base,
        &[
            ("grant_type", "client_credentials"),
            ("persona", "ada"),
            ("flaw", ""),
        ],
    )
    .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["expires_in"], 60);
}

// ------------------------------------------------------- the seam's `flaw` --

/// The seam is a fixture, so it echoes what it was asked for. For the three
/// header-level flaws that echo is the only way a fixture can tell it got the
/// token it asked for, because their claims are perfect.
#[tokio::test]
async fn the_seam_applies_a_flaw_and_echoes_it() {
    let base = spawn().await;
    let (status, body) = post_token(
        &base,
        "?persona=ada&flaw=wrong-aud",
        r#"{"aud":"billing-api"}"#,
    )
    .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["flaw"], "wrong-aud");
    assert_eq!(body["claims"]["aud"], "wrong-billing-api");
    assert_eq!(
        payload_of(body["token"].as_str().unwrap()),
        body["claims"],
        "the seam's contract is that `claims` is what was signed"
    );
}

/// `claims` shows the damage for the claim-level flaws, and is untouched for the
/// header-level ones — exactly true in both cases.
#[tokio::test]
async fn the_seam_reports_the_damage_it_did_and_no_more() {
    let base = spawn().await;

    let (_, expired) = post_token(&base, "?persona=ada&flaw=expired", "").await;
    let claims = &expired["claims"];
    assert_eq!(
        claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap(),
        60,
        "the lifetime survives the shift"
    );
    assert!(claims["exp"].as_u64().unwrap() < now(), "{claims}");

    let (_, unsigned) = post_token(&base, "?persona=ada&flaw=alg-none", "").await;
    assert_eq!(unsigned["flaw"], "alg-none");
    assert_eq!(
        unsigned["claims"]["iss"], ISSUER,
        "a header-level flaw leaves the claims correct"
    );
}

#[tokio::test]
async fn no_flaw_means_no_flaw_field_at_all() {
    let base = spawn().await;
    let (_, body) = post_token(&base, "?persona=ada", "").await;
    assert!(
        body.as_object().unwrap().get("flaw").is_none(),
        "an unflawed response must look exactly as it did in Phase 2: {body}"
    );
}

/// A known parameter with an unrecognized value is a refusal, not an ignored
/// parameter: a typo that quietly mints a good token is a negative test suite
/// reporting six passes and proving nothing.
#[tokio::test]
async fn an_unknown_flaw_on_the_seam_is_a_400_naming_it_and_all_six() {
    let base = spawn().await;
    let (status, body) = post_token(&base, "?persona=ada&flaw=expried", "").await;

    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], "invalid_request");
    let description = body["error_description"].as_str().unwrap();
    assert!(description.contains("expried"), "{description}");
    for name in [
        "expired",
        "wrong-aud",
        "wrong-iss",
        "bad-signature",
        "alg-none",
        "unknown-kid",
    ] {
        assert!(description.contains(name), "{name} missing: {description}");
    }
    assert!(body.as_object().unwrap().get("token").is_none());

    // Still serving.
    let (status, _) = post_token(&base, "?persona=ada", "").await;
    assert_eq!(status, 200);
}

/// `?flaw=` is the shell saying nothing, and the grant's `Form::get` already
/// reads an empty value as absent. The two surfaces have to agree, or the same
/// spelling is a `200` on one and a `400` on the other.
#[tokio::test]
async fn an_empty_flaw_reads_as_absent_on_the_seam() {
    let base = spawn().await;
    let (status, body) = post_token(&base, "?persona=ada&flaw=", "").await;

    assert_eq!(status, 200, "{body}");
    assert!(body.as_object().unwrap().get("flaw").is_none());
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

/// **North star 3 with the flaws included.** A flaw that is applied in
/// `oidc::token` rather than in the one function would show up here as a diff:
/// two callers, one claim set.
#[tokio::test]
async fn the_two_surfaces_agree_under_a_flaw_too() {
    let base = spawn().await;

    for flaw in ["wrong-iss", "wrong-aud"] {
        let (_, seam) = post_token(
            &base,
            &format!("?persona=ada&flaw={flaw}"),
            r#"{"aud":"billing-api"}"#,
        )
        .await;
        let (_, _, grant) = post_grant(
            &base,
            &[
                ("grant_type", "client_credentials"),
                ("persona", "ada"),
                ("audience", "billing-api"),
                ("flaw", flaw),
            ],
        )
        .await;

        let mut through_the_seam = seam["claims"].as_object().unwrap().clone();
        let mut through_the_grant = payload_of(grant["access_token"].as_str().unwrap())
            .as_object()
            .unwrap()
            .clone();
        for per_request in ["iat", "nbf", "exp", "jti", "client_id"] {
            through_the_seam.remove(per_request);
            through_the_grant.remove(per_request);
        }

        assert_eq!(through_the_seam, through_the_grant, "for flaw={flaw}");
        // Guard against the test passing because the flaw did nothing.
        match flaw {
            "wrong-iss" => {
                assert_eq!(
                    through_the_grant["iss"],
                    "https://wrong-issuer.example.test"
                )
            }
            _ => assert_eq!(through_the_grant["aud"], "wrong-billing-api"),
        }
    }
}

/// Flaws mint; they do not mutate. Nothing about producing a broken token
/// touches server state — least of all the key an RP has already cached.
#[tokio::test]
async fn the_jwks_is_unchanged_after_minting_every_flaw() {
    let base = spawn().await;

    let before: serde_json::Value = reqwest::get(format!("{base}/oidc/jwks"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    for flaw in [
        "expired",
        "wrong-aud",
        "wrong-iss",
        "bad-signature",
        "alg-none",
        "unknown-kid",
    ] {
        let (status, _, _) = post_grant(
            &base,
            &[
                ("grant_type", "client_credentials"),
                ("persona", "ada"),
                ("audience", "billing-api"),
                ("flaw", flaw),
            ],
        )
        .await;
        assert_eq!(status, 200, "{flaw} did not mint");
        let (status, _) = post_token(&base, &format!("?persona=ada&flaw={flaw}"), "").await;
        assert_eq!(status, 200, "{flaw} did not mint on the seam");
    }

    let after: serde_json::Value = reqwest::get(format!("{base}/oidc/jwks"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(after, before);
    assert_eq!(after["keys"].as_array().unwrap().len(), 1);
    assert_eq!(
        after["keys"][0]["kid"], "TXntCt2biz2Bj578hZocZOb2A2nQV9JfrBvFN55QWpU",
        "the unknown kid must never reach the JWKS"
    );
    assert!(!serde_json::to_string(&after)
        .unwrap()
        .contains("lanyard-unknown-kid"));
}

// ------------------------------------------------------------ introspect --
//
// RFC 7662. Always `200`, and everything unrecognized is exactly
// `{"active": false}` — §2.2 says an inactive response reveals no other fields.

/// Form-encoded, as RFC 7662 §2.1 requires, and with whatever client
/// authentication the caller felt like sending.
async fn post_form(base: &str, path: &str, body: &str) -> (u16, String) {
    let res = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    (res.status().as_u16(), res.text().await.unwrap())
}

async fn introspect(base: &str, body: &str) -> serde_json::Value {
    let (status, text) = post_form(base, "/oidc/introspect", body).await;
    assert_eq!(status, 200, "introspection always answers 200: {text}");
    serde_json::from_str(&text).expect("introspection is JSON")
}

/// An access token minted through the seam, which signs with the same key
/// `/oidc/token` does — so introspection cannot tell them apart, which is the
/// point.
async fn an_access_token(base: &str, query: &str) -> String {
    let (_, body) = post_token(
        base,
        query,
        r#"{"client_id":"billing-web","scope":"openid email"}"#,
    )
    .await;
    body["token"].as_str().unwrap().to_string()
}

/// A real RS256 token, signed by a key that is not lanyard's. It is
/// well-formed, unexpired, and names lanyard as its issuer — everything except
/// the signature is right, so this is the assertion that the signature is
/// actually checked.
fn a_foreign_token() -> String {
    let key = lanyard_cli::keys::SigningKey::from_pem(include_str!("data/other-key.pem")).unwrap();
    let claims: serde_json::Map<String, serde_json::Value> = serde_json::from_value(
        serde_json::json!({ "sub": "ada", "iss": ISSUER, "exp": now() + 300, "jti": "foreign" }),
    )
    .unwrap();
    lanyard_cli::oidc::jws::sign(&key, &claims, None).unwrap()
}

#[tokio::test]
async fn introspecting_a_live_access_token_describes_it() {
    let base = spawn().await;
    let token = an_access_token(&base, "?persona=ada").await;

    let out = introspect(&base, &format!("token={token}")).await;
    assert_eq!(out["active"], true);
    assert_eq!(out["token_type"], "Bearer");
    assert_eq!(out["sub"], "ada");
    assert_eq!(out["client_id"], "billing-web");
    assert_eq!(out["scope"], "openid email");
    assert_eq!(out["iss"], ISSUER);
    assert!(out["exp"].is_number() && out["iat"].is_number());
    assert!(out["jti"].is_string(), "the jti is what a revocation names");
}

/// Only what the token carried. An access token with no `aud` is legitimate —
/// it is what an API that forgets to validate audience will happily accept —
/// and reporting one it does not have would be a lie about the token.
#[tokio::test]
async fn introspection_reports_no_audience_when_the_token_has_none() {
    let base = spawn().await;
    let token = an_access_token(&base, "?persona=ada").await;
    let out = introspect(&base, &format!("token={token}")).await;
    assert!(out.get("aud").is_none(), "{out}");
}

/// **Exactly two words** for every kind of nothing. RFC 7662 §2.2: an inactive
/// response reveals no other fields, so expired, foreign, malformed, absent and
/// never-issued are one answer.
#[tokio::test]
async fn everything_unrecognized_is_exactly_active_false() {
    let base = spawn().await;
    let expired = {
        let (_, body) = post_token(&base, "?persona=ada&flaw=expired", "{}").await;
        body["token"].as_str().unwrap().to_string()
    };

    for body in [
        String::new(),
        "token=".to_string(),
        "token=not-a-jwt".to_string(),
        "token=a.b.c".to_string(),
        format!("token={expired}"),
        format!("token={}", a_foreign_token()),
        "token=some-refresh-token-nobody-issued".to_string(),
    ] {
        let out = introspect(&base, &body).await;
        assert_eq!(
            out,
            serde_json::json!({ "active": false }),
            "{body:?} must produce exactly two words and nothing more"
        );
    }
}

/// North star 1 arriving at a third endpoint. RFC 7662 §2.1 says the client
/// MUST be authenticated here; lanyard accepts every form of it and checks
/// none, exactly as `/oidc/token` already does.
#[tokio::test]
async fn client_authentication_is_accepted_in_any_form_and_checked_in_none() {
    let base = spawn().await;
    let token = an_access_token(&base, "?persona=ada").await;

    // Form parameters.
    let out = introspect(
        &base,
        &format!("token={token}&client_id=whoever&client_secret=nonsense"),
    )
    .await;
    assert_eq!(out["active"], true);

    // HTTP Basic, and none at all.
    let res = reqwest::Client::new()
        .post(format!("{base}/oidc/introspect"))
        .header("authorization", "Basic bm90OmNoZWNrZWQ=")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("token={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    assert_eq!(
        res.json::<serde_json::Value>().await.unwrap()["active"],
        true
    );
}

/// RFC 7662 §2.1: a server MUST NOT refuse a request because the hint was
/// wrong. The string is identified by what it is, not by what it was announced
/// as.
#[tokio::test]
async fn a_wrong_token_type_hint_is_read_as_a_hint_and_nothing_more() {
    let base = spawn().await;
    let token = an_access_token(&base, "?persona=ada").await;
    let out = introspect(
        &base,
        &format!("token={token}&token_type_hint=refresh_token"),
    )
    .await;
    assert_eq!(out["active"], true, "the hint was wrong and did not matter");
}

// ---------------------------------------------------------------- revoke --
//
// RFC 7009. **Always `200` with an empty body**, including for a token that was
// never issued or is not a JWT — §2.2 requires exactly that, so a client cannot
// use this endpoint to find out whether a token exists.

async fn revoke(base: &str, body: &str) {
    let (status, text) = post_form(base, "/oidc/revoke", body).await;
    assert_eq!(status, 200, "revocation always answers 200: {text}");
    assert_eq!(text, "", "and an empty body");
}

async fn userinfo_status(base: &str, token: &str) -> (u16, Option<String>) {
    let res = reqwest::Client::new()
        .get(format!("{base}/oidc/userinfo"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    let challenge = res
        .headers()
        .get("www-authenticate")
        .map(|v| v.to_str().unwrap().to_string());
    (res.status().as_u16(), challenge)
}

/// Criterion 16, at the HTTP level: introspection flips, and `/userinfo` agrees.
/// `/introspect` saying `active: false` while `/userinfo` hands over claims
/// would be lanyard disagreeing with itself.
#[tokio::test]
async fn revoking_an_access_token_flips_introspection_and_closes_userinfo() {
    let base = spawn().await;
    let token = an_access_token(&base, "?persona=ada").await;

    assert_eq!(
        introspect(&base, &format!("token={token}")).await["active"],
        true
    );
    assert_eq!(userinfo_status(&base, &token).await.0, 200);

    revoke(&base, &format!("token={token}")).await;

    assert_eq!(
        introspect(&base, &format!("token={token}")).await,
        serde_json::json!({ "active": false }),
        "exactly two words after revocation too"
    );
    let (status, challenge) = userinfo_status(&base, &token).await;
    assert_eq!(status, 401);
    let challenge = challenge.expect("RFC 6750 §3's header");
    assert!(challenge.contains("invalid_token"), "{challenge}");
}

/// Criterion 17. RFC 7009 §2.2: a client must not be able to probe existence
/// here, so a token nobody issued and a string that is not a JWT get the same
/// `200` a real one does.
#[tokio::test]
async fn revoking_something_that_was_never_issued_is_still_two_hundred_and_empty() {
    let base = spawn().await;
    for body in [
        String::new(),
        "token=".to_string(),
        "token=not-a-jwt".to_string(),
        "token=some-refresh-token-nobody-issued".to_string(),
        format!("token={}", a_foreign_token()),
        format!("token={}&token_type_hint=refresh_token", a_foreign_token()),
    ] {
        revoke(&base, &body).await;
    }
    // And a foreign token still introspects as inactive rather than as an
    // error, before and after.
    assert_eq!(
        introspect(&base, &format!("token={}", a_foreign_token())).await,
        serde_json::json!({ "active": false })
    );
}

/// Revoking somebody else's token must not revoke ours: the set is keyed on
/// the `jti`, and a foreign token's `jti` never enters it because the token
/// never verified.
#[tokio::test]
async fn revoking_a_foreign_token_does_not_touch_a_real_one() {
    let base = spawn().await;
    let mine = an_access_token(&base, "?persona=ada").await;
    revoke(&base, &format!("token={}", a_foreign_token())).await;
    assert_eq!(
        introspect(&base, &format!("token={mine}")).await["active"],
        true
    );
}

/// Client authentication is accepted in any form and checked in none, here too.
#[tokio::test]
async fn revocation_accepts_any_client_authentication() {
    let base = spawn().await;
    let token = an_access_token(&base, "?persona=ada").await;
    let res = reqwest::Client::new()
        .post(format!("{base}/oidc/revoke"))
        .header("authorization", "Basic bm90OmNoZWNrZWQ=")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("token={token}&client_id=whoever"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    assert_eq!(
        introspect(&base, &format!("token={token}")).await["active"],
        false
    );
}

/// A revoked `jti` is forgotten at the moment its token would have expired
/// anyway, which is what bounds the set. Proved through the front door: two
/// tokens revoked, and the set holds only the one still alive.
#[tokio::test]
async fn a_revocation_is_not_recorded_for_a_token_that_has_already_expired() {
    let base = spawn().await;
    let (_, body) = post_token(&base, "?persona=ada&flaw=expired", "{}").await;
    let expired = body["token"].as_str().unwrap().to_string();

    revoke(&base, &format!("token={expired}")).await;
    // It was already inactive, and it still is — nothing was remembered about
    // a token there was nothing left to revoke.
    assert_eq!(
        introspect(&base, &format!("token={expired}")).await,
        serde_json::json!({ "active": false })
    );
}
