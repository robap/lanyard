//! `GET`/`POST /oidc/userinfo` — the persona's claims, scope-filtered, for a
//! caller holding one of lanyard's own access tokens.
//!
//! **The second place lanyard says no**, and for PKCE's reason: an app that
//! reads `/userinfo` with an expired token has a bug, and a mock that answers
//! anyway hides it. `jumbojett`'s `requestUserInfo()` is the client that proves
//! the endpoint works; `lanyard token --as ada --expired` is the one that proves
//! it refuses.
//!
//! **No claims are assembled here.** The scope-filtered persona claims come out
//! of [`crate::oidc::issue::persona_claims`] — the same table the ID token was
//! built from, which is the whole reason that function was extracted.

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Map, Value};

use crate::app::SharedState;
use crate::log_detail::{attach, LogDetail};
use crate::oidc::issue::persona_claims;
use crate::oidc::jws;
use crate::oidc::scope::{ClaimFilter, Scopes};

pub fn route() -> axum::routing::MethodRouter<SharedState> {
    // Both verbs: OIDC Core §5.3.1 allows either, and `jumbojett` uses `GET`
    // while some SDKs POST.
    axum::routing::get(userinfo).post(userinfo)
}

/// Claims that belong to the token rather than to the person. Everything else
/// in an access token got there through the persona→claims table, so removing
/// exactly these leaves exactly the persona.
///
/// This list is not a second claim table — it is the *complement* of one, and
/// it is only consulted for an identity that is no longer in the persona list.
const PROTOCOL_CLAIMS: [&str; 12] = [
    "iss",
    "aud",
    "exp",
    "iat",
    "nbf",
    "jti",
    "scope",
    "client_id",
    "nonce",
    "at_hash",
    "c_hash",
    "auth_time",
];

async fn userinfo(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let Some(token) = bearer(&headers) else {
        return unauthorized("no bearer token was presented");
    };

    let claims = match jws::verify(&state.key, &state.config.issuer, &token) {
        Ok(claims) => claims,
        Err(message) => return unauthorized(&message),
    };

    // **The same revocation set `/introspect` reads.** `/introspect` answering
    // `active: false` while this endpoint handed over claims would be lanyard
    // disagreeing with itself, and it is the same argument that made an expired
    // token a `401`: an app reading UserInfo with a dead token has a bug.
    let jti = claims
        .get("jti")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if crate::oidc::revocation::is_revoked(&state, jti) {
        return unauthorized("that token has been revoked");
    }

    let Some(sub) = claims.get("sub").and_then(Value::as_str) else {
        return unauthorized("the token has no sub, so it identifies nobody");
    };

    // The access token is not scope-filtered, but UserInfo is — OIDC Core §5.4
    // specifies claims-per-scope here and says nothing about an access token.
    // The scope to filter by is the one the token was granted.
    let filter = ClaimFilter::ByScope(Scopes::parse(claims.get("scope").and_then(Value::as_str)));

    // **Scoped like the mint was.** The `client_id` claim on the presented token
    // is the application this request is for, so the `sub`→persona lookup uses
    // it — looking `sub` up against the unscoped set would answer with somebody
    // the token was never minted for.
    let token_client = claims.get("client_id").and_then(Value::as_str);
    let people = state.personas.resolve();
    let body = match people.get(sub, token_client) {
        // The ordinary case: the same table, read again with a filter.
        Some(sourced) => persona_claims(&sourced.persona, &filter),
        // An identity minted from the picker's "mint one now" panel is in no
        // file, so there is nothing to look up. Its claims are already in the
        // token — put there by `persona_claims` with no filter — so filtering
        // what is left after the protocol claims are removed gives exactly what
        // looking the persona up would have given.
        None => filtered_from_token(&claims, &filter),
    };

    // Who asked, and who they got. `client_id` comes off the token because that
    // is the only thing this request carries about the application — a
    // `/userinfo` call names itself nowhere else.
    attach(
        (
            [(header::CACHE_CONTROL, "no-store")],
            Json(Value::Object(body)),
        )
            .into_response(),
        LogDetail {
            client_id: token_client.map(str::to_string),
            detail: Some(
                [("sub".to_string(), Value::from(sub))]
                    .into_iter()
                    .collect(),
            ),
            ..LogDetail::default()
        },
    )
}

fn filtered_from_token(claims: &Map<String, Value>, filter: &ClaimFilter) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in claims {
        if PROTOCOL_CLAIMS.contains(&key.as_str()) {
            continue;
        }
        // `sub` is always present, which the filter already agrees with.
        if filter.allows(key) {
            out.insert(key.clone(), value.clone());
        }
    }
    out
}

/// `Authorization: Bearer <token>`, case-insensitively, because RFC 7235 says
/// the scheme is case-insensitive and some clients send `bearer`.
fn bearer(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = raw.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_string())
}

/// RFC 6750 §3's shape. The `WWW-Authenticate` header is what tells a client
/// library this was an authentication failure rather than a server that fell
/// over.
fn unauthorized(description: &str) -> Response {
    let challenge = format!(
        "Bearer error=\"invalid_token\", error_description=\"{}\"",
        // The description is ours, but it can quote a token's `iss`, and a
        // quote inside a quoted-string would produce a header no client parses.
        description.replace('"', "'")
    );
    let response = (
        StatusCode::UNAUTHORIZED,
        [
            (header::WWW_AUTHENTICATE, challenge.as_str()),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Json(json!({ "error": "invalid_token", "error_description": description })),
    )
        .into_response();
    attach(
        response,
        LogDetail {
            error: Some("invalid_token".to_string()),
            error_description: Some(description.to_string()),
            ..LogDetail::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, value.parse().unwrap());
        headers
    }

    #[test]
    fn the_bearer_scheme_is_case_insensitive_and_the_token_is_trimmed() {
        assert_eq!(bearer(&headers("Bearer eyJ.a.b")), Some("eyJ.a.b".into()));
        assert_eq!(bearer(&headers("bearer  eyJ.a.b ")), Some("eyJ.a.b".into()));
    }

    #[test]
    fn anything_that_is_not_a_bearer_token_reads_as_absent() {
        assert_eq!(bearer(&HeaderMap::new()), None);
        assert_eq!(bearer(&headers("Basic abc")), None);
        assert_eq!(bearer(&headers("Bearer ")), None);
        assert_eq!(bearer(&headers("eyJ.a.b")), None);
    }

    /// A quote inside a quoted-string produces a header no client parses, and
    /// the description quotes values that came off the wire.
    #[test]
    fn the_challenge_header_cannot_be_broken_by_a_quote_in_the_description() {
        let response = unauthorized(r#"issued by "someone else""#);
        let challenge = response.headers()[header::WWW_AUTHENTICATE]
            .to_str()
            .unwrap();
        assert_eq!(challenge.matches('"').count(), 4, "{challenge}");
        assert!(challenge.starts_with("Bearer error=\"invalid_token\""));
    }

    /// The complement list is only reached for an identity no longer in the
    /// persona list, and it has to leave the person and remove the protocol.
    #[test]
    fn filtering_a_tokens_own_claims_drops_the_protocol_and_keeps_the_person() {
        let claims: Map<String, Value> = serde_json::from_value(json!({
            "sub": "zed", "email": "zed@example.test", "email_verified": true,
            "name": "Zed", "preferred_username": "zed", "department": "ops",
            "iss": "http://x/oidc", "aud": "api", "exp": 1, "iat": 1, "nbf": 1,
            "jti": "j", "scope": "openid email", "client_id": "c"
        }))
        .unwrap();

        let out = filtered_from_token(
            &claims,
            &ClaimFilter::ByScope(Scopes::parse(Some("openid email"))),
        );
        let mut keys: Vec<&str> = out.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["department", "email", "email_verified", "sub"]);
    }
}
