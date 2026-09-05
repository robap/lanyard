//! The test seam at `/_/api/…`. CONCEPT §6 says build it in week one, not as a
//! later addition, precisely because it forces the shape: it is the first caller
//! of [`crate::oidc::issue`], so Phase 4's browser flow arrives as a *second*
//! caller rather than as the original home of the claim logic.
//!
//! **The body is the claims. The query string is how they are minted.** Phase 3
//! added `flaw=alg-none` and friends to the query — including the ones that are
//! not expressible as claims — with no change to the body contract.
//!
//! This endpoint mints a token for any claims anyone asks for, which is exactly
//! why the default bind is loopback.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Map, Value};

use crate::app::SharedState;
use crate::oidc::flaw::Flaw;
use crate::oidc::issue::{self, DEFAULT_TTL};
use crate::oidc::scope::ClaimFilter;

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route("/api/token", post(token))
        .route("/api/personas", get(personas))
}

/// `400` with `{"error", "error_description"}` — the shape an OAuth client
/// already knows how to read.
fn bad_request(error: &str, description: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response()
}

async fn token(
    State(state): State<SharedState>,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    let ttl = match query.get("ttl") {
        Some(raw) => match raw.parse::<u64>() {
            Ok(seconds) => seconds,
            Err(_) => {
                return bad_request(
                    "invalid_request",
                    format!("ttl must be a whole number of seconds, got {raw:?}"),
                )
            }
        },
        None => DEFAULT_TTL,
    };

    let persona = match query.get("persona") {
        Some(id) => match state.personas.get(id) {
            Some(p) => Some(p),
            None => {
                return bad_request(
                    "unknown_persona",
                    format!("no persona with id {id:?} is loaded"),
                )
            }
        },
        None => None,
    };

    // An empty `?flaw=` is the shell saying nothing, exactly as `-d flaw=` is on
    // the grant. The two surfaces have to agree here, or the same spelling is a
    // `200` on one and a `400` on the other.
    let flaw = match query.get("flaw").filter(|raw| !raw.is_empty()) {
        Some(raw) => match Flaw::parse(raw) {
            Ok(flaw) => Some(flaw),
            Err(message) => return bad_request("invalid_request", message),
        },
        None => None,
    };

    let overrides = match parse_claims(&body) {
        Ok(claims) => claims,
        Err(message) => return bad_request("invalid_request", message),
    };

    match issue::issue(
        &state.key,
        &state.config.issuer,
        persona,
        &overrides,
        ttl,
        flaw,
        // The seam's contract is "the body is the claims": filtering it would
        // make a posted claim vanish for a reason the body never mentioned.
        &ClaimFilter::Unfiltered,
    ) {
        Ok(issued) => {
            let mut response = json!({ "token": issued.token, "claims": issued.claims });
            // Echoed only when one was applied, so an unflawed response is the
            // Phase 2 shape byte for byte. For the three header-level flaws this
            // is the only way a fixture can confirm it got what it asked for:
            // their claims are perfect.
            if let Some(flaw) = flaw {
                response["flaw"] = Value::from(flaw.as_str());
            }
            Json(response).into_response()
        }
        Err(message) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "issuance_failed", "error_description": message })),
        )
            .into_response(),
    }
}

/// The body may be absent or empty; anything else must be a JSON object.
/// Parsed by hand rather than with the `Json` extractor so the 400 is ours and
/// carries the documented shape.
fn parse_claims(body: &Bytes) -> Result<Map<String, Value>, String> {
    if body.is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(other) => Err(format!(
            "body must be a JSON object of claims, got {}",
            kind_of(&other)
        )),
        Err(e) => Err(format!("body is not valid JSON: {e}")),
    }
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// The JSON seam the Phase 4 picker will read, and the only way to observe that
/// persona loading works at all in this phase. Both `client:` levels are echoed
/// untouched; nothing filters on them until Phase 7.
async fn personas(State(state): State<SharedState>) -> impl IntoResponse {
    Json(json!({
        "client": state.personas.client,
        "personas": state.personas.list,
    }))
}
