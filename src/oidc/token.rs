//! `POST /oidc/token` — the OAuth 2.0 token endpoint.
//!
//! **This file builds no claims.** It parses a form, maps a handful of
//! parameters into an overrides map, and calls [`crate::oidc::issue`] — the same
//! function, with the same persona→claims table, that the test seam calls. Any
//! claim shaping that appears here is the drift north star 3 exists to prevent.
//!
//! The form is parsed by hand rather than through axum's `Form` extractor for
//! the same reason [`crate::seam::parse_claims`] hand-rolls its JSON: every 4xx
//! on this endpoint has to be ours and carry the OAuth
//! `{"error", "error_description"}` shape, not axum's 415 or 422.
//!
//! Dispatch on `grant_type` has exactly one arm today. Phase 4 adds
//! `authorization_code` as a second arm rather than a second handler.
//!
//! Phase 3's `flaw` parameter is parsed here and applied nowhere: it is turned
//! into a [`Flaw`] and handed to the same one function. The response says
//! nothing about it — an SDK reading this body should see nothing unusual,
//! because the whole point is that the token looks ordinary until it is
//! validated.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine as _;
use serde_json::{json, Map, Value};

use crate::app::SharedState;
use crate::oidc::flaw::Flaw;
use crate::oidc::issue::{self, DEFAULT_TTL};

pub fn route() -> axum::routing::MethodRouter<SharedState> {
    // `post` alone: axum answers `GET /oidc/token` with 405 and an `Allow`
    // header, which is the correct answer and not one we have to write.
    axum::routing::post(token)
}

async fn token(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Response {
    let form = Form::parse(&body);

    // An empty `grant_type=` is the same mistake as no `grant_type` at all, and
    // both are `invalid_request` rather than `unsupported_grant_type`: nothing
    // was named, so nothing can be unsupported.
    match form.get("grant_type") {
        None => bad_request(
            "invalid_request",
            "grant_type is required; this build supports client_credentials".to_string(),
        ),
        Some("client_credentials") => client_credentials(&state, &headers, &form),
        Some(other) => bad_request(
            "unsupported_grant_type",
            format!(
                "grant_type {other:?} is not supported; this build supports client_credentials"
            ),
        ),
    }
}

/// Client authentication is accepted in **any** form — HTTP Basic, form
/// parameters, or entirely absent — and checked in none. That is north star 1
/// arriving at the endpoint where every other IdP would put a client registry,
/// and it is what produces the multi-project property.
fn client_credentials(state: &SharedState, headers: &HeaderMap, form: &Form) -> Response {
    let persona = match form.get("persona") {
        Some(id) => match state.personas.get(id) {
            Some(persona) => Some(persona),
            None => {
                return bad_request(
                    "invalid_request",
                    format!("no persona with id {id:?} is loaded"),
                )
            }
        },
        None => None,
    };

    // A known parameter with an unrecognized value is a refusal, unlike an
    // unknown parameter, which OAuth says to ignore and this endpoint does:
    // `flaw=expried` quietly minting a good token is a negative test suite
    // reporting a pass and proving nothing.
    let flaw = match form.get("flaw") {
        Some(raw) => match Flaw::parse(raw) {
            Ok(flaw) => Some(flaw),
            Err(message) => return bad_request("invalid_request", message),
        },
        None => None,
    };

    let mut overrides = Map::new();
    // `audience` is the Auth0 convention most developers have seen; `resource`
    // is the standards-track one (RFC 8707). Accepting both means a real SDK
    // works whichever it sends. Only when asked for: a token with no `aud` is
    // legitimate and useful — it is what an API that forgets to validate
    // audience will happily accept.
    if let Some(audience) = form.get("audience").or_else(|| form.get("resource")) {
        overrides.insert("aud".into(), Value::from(audience));
    }
    let scope = form.get("scope");
    if let Some(scope) = scope {
        overrides.insert("scope".into(), Value::from(scope));
    }
    // Emitted only when the request supplied one, matching what a real
    // `client_credentials` token carries and giving Phase 6's log and Phase 7's
    // namespacing something honest to key on.
    if let Some(client_id) = form
        .get("client_id")
        .map(str::to_string)
        .or_else(|| basic_client_id(headers))
    {
        overrides.insert("client_id".into(), Value::from(client_id));
    }

    match issue::issue(
        &state.key,
        &state.config.issuer,
        persona,
        &overrides,
        DEFAULT_TTL,
        flaw,
    ) {
        Ok(issued) => {
            // The token's actual remaining life, floored at zero, rather than
            // the TTL it was asked for: `flaw=expired` reporting 60 seconds it
            // does not have is the one place this envelope could lie. Computed
            // from the moment `issue` used, because reading the clock a second
            // time here makes the ordinary case report 59 or 61 at random.
            let expires_in = issued
                .claims
                .get("exp")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .saturating_sub(issued.issued_at);
            let mut body = json!({
                "access_token": issued.token,
                "token_type": "Bearer",
                "expires_in": expires_in,
            });
            // Present only when one was asked for. No `id_token`: client
            // credentials has no user authentication event, so returning one
            // would be a lie an SDK might believe. No `refresh_token` either.
            if let Some(scope) = scope {
                body["scope"] = Value::from(scope);
            }
            ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
        }
        Err(message) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "issuance_failed", "error_description": message })),
        )
            .into_response(),
    }
}

/// The username half of `Authorization: Basic`, which RFC 6749 §2.3.1 defines as
/// the client id. Nothing here validates: a header that will not decode simply
/// yields no client id, because a malformed credential is still an accepted one.
fn basic_client_id(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let encoded = raw
        .strip_prefix("Basic ")
        .or_else(|| raw.strip_prefix("basic "))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let credentials = String::from_utf8(decoded).ok()?;
    let username = credentials.split(':').next()?;
    // §2.3.1 says both halves are form-urlencoded before being base64'd, so a
    // client id with a space arrives as `%20` and must be undone.
    Some(form_decode(username))
}

fn form_decode(raw: &str) -> String {
    serde_urlencoded::from_str::<Vec<(String, String)>>(&format!("v={raw}"))
        .ok()
        .and_then(|pairs| pairs.into_iter().next())
        .map(|(_, value)| value)
        .unwrap_or_else(|| raw.to_string())
}

/// `400` in the OAuth error shape the seam already uses.
fn bad_request(error: &str, description: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response()
}

/// The decoded form, kept as ordered pairs so a repeated parameter resolves to
/// its first occurrence rather than to whichever one a map happened to keep.
struct Form(Vec<(String, String)>);

impl Form {
    /// Infallible, and that is an observed fact rather than an assumption:
    /// `serde_urlencoded` decoding into pairs is lossy on invalid UTF-8 and
    /// lenient about stray `%` escapes, so no byte string fails. A body that is
    /// not a form at all therefore arrives as unknown parameters and falls out
    /// as `invalid_request` for the missing `grant_type` — still our shape,
    /// never axum's 415 or 422.
    ///
    /// The content-type header is deliberately not consulted. Rejecting a
    /// well-formed body over its label would be the one registration-shaped
    /// "no" on an endpoint whose whole point is that it has none.
    fn parse(body: &Bytes) -> Self {
        Form(serde_urlencoded::from_bytes::<Vec<(String, String)>>(body).unwrap_or_default())
    }

    /// An empty value is the same as an absent one. `-d scope=` should not echo
    /// an empty `scope`, and `-d grant_type=` should not name an unsupported
    /// grant of `""`.
    fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, value)| key == name && !value.is_empty())
            .map(|(_, value)| value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic(value: &str) -> Option<String> {
        let mut headers = HeaderMap::new();
        headers.insert(axum::http::header::AUTHORIZATION, value.parse().unwrap());
        basic_client_id(&headers)
    }

    fn encode(credentials: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(credentials)
    }

    #[test]
    fn the_basic_username_is_the_client_id() {
        assert_eq!(
            basic(&format!("Basic {}", encode("billing-web:unchecked"))),
            Some("billing-web".to_string())
        );
    }

    #[test]
    fn a_percent_encoded_basic_username_is_decoded() {
        // RFC 6749 §2.3.1 form-urlencodes each half before base64.
        assert_eq!(
            basic(&format!("Basic {}", encode("billing%20web:s%3Acret"))),
            Some("billing web".to_string())
        );
    }

    #[test]
    fn a_secretless_basic_credential_still_yields_a_client_id() {
        assert_eq!(
            basic(&format!("Basic {}", encode("billing-web"))),
            Some("billing-web".to_string())
        );
    }

    /// A credential we cannot read is still an accepted one — it yields no
    /// client id rather than a 401. Nothing on this endpoint rejects.
    #[test]
    fn an_unreadable_credential_yields_no_client_id_rather_than_an_error() {
        assert_eq!(basic("Bearer something"), None);
        assert_eq!(basic("Basic !!!not-base64!!!"), None);
        assert_eq!(basic(&format!("Basic {}", encode(""))), Some(String::new()));
    }

    /// An empty value is an absent one, so `-d scope=` echoes nothing and
    /// `-d grant_type=` names no grant.
    #[test]
    fn an_empty_form_value_reads_as_absent() {
        let form = Form::parse(&Bytes::from_static(b"grant_type=&scope=orders%3Aread"));
        assert_eq!(form.get("grant_type"), None);
        assert_eq!(form.get("scope"), Some("orders:read"));
    }

    /// Ordered pairs, not a map: a repeated parameter resolves to its first
    /// occurrence rather than to whichever one a map happened to keep.
    #[test]
    fn a_repeated_parameter_resolves_to_the_first() {
        let form = Form::parse(&Bytes::from_static(b"persona=ada&persona=mira"));
        assert_eq!(form.get("persona"), Some("ada"));
    }
}
