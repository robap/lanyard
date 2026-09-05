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

    // Honest, not aspirational: an advertised endpoint that 404s sends a client
    // down a path that fails later and further away.
    for unimplemented in [
        "authorization_endpoint",
        "token_endpoint",
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
