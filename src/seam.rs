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
use crate::log_detail::{attach, LogDetail};
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
///
/// The one funnel for this endpoint's refusals, and therefore where the log
/// learns about them — the same arrangement `/oidc/token`'s `bad_request` has,
/// for the same reason.
fn bad_request(error: &str, description: String) -> Response {
    let response = (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": error, "error_description": description.clone() })),
    )
        .into_response();
    attach(
        response,
        LogDetail {
            error: Some(error.to_string()),
            error_description: Some(description),
            ..LogDetail::default()
        },
    )
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

    // Absent behaves as a `client_id` nobody scoped to — the unscoped set — for
    // the reason the visibility rule gives: a request with no `client_id` sees
    // exactly what an unrecognised one sees.
    let people = state.personas.resolve();
    let persona = match query.get("persona") {
        Some(id) => match people.get(id, query.get("client_id").map(String::as_str)) {
            Some(p) => Some(&p.persona),
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
        state.clock,
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
            // The seam mints, so it is logged like every other thing that
            // mints — the flaw it was asked for and the header and payload that
            // actually came out. `--alg-none`'s claims are perfect; only the
            // emitted header says what happened.
            attach(
                Json(response).into_response(),
                LogDetail {
                    flaw: flaw.map(|flaw| flaw.as_str().to_string()),
                    issued: crate::log_detail::decoded_token(&issued.token).map(|decoded| {
                        [("access_token".to_string(), decoded)]
                            .into_iter()
                            .collect()
                    }),
                    request: Some(query_map(&query)),
                    ..LogDetail::default()
                },
            )
        }
        Err(message) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "issuance_failed", "error_description": message })),
        )
            .into_response(),
    }
}

/// **The query string is how they are minted**, so it is what the event's
/// `request` carries. The body is the claims, and those come back decoded in
/// `issued` rather than echoed twice.
fn query_map(query: &HashMap<String, String>) -> Map<String, Value> {
    let pairs: Vec<(String, String)> = query.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    crate::log_detail::request_map(&pairs)
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

/// The JSON seam the picker reads, and the surface a test asserts the
/// visibility rule on without scraping HTML.
///
/// **With `client_id`, the visible set; without it, everything** — every
/// persona from every source, each row carrying the `client:` that actually
/// applies to it and the file it came from. The unfiltered form is the
/// debugging view, and it is the one a developer reaches for when the filtered
/// one surprised them.
async fn personas(
    State(state): State<SharedState>,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let people = state.personas.resolve();
    let rows: Vec<Value> = match query.get("client_id") {
        Some(client_id) => people
            .visible_to(Some(client_id))
            .into_iter()
            .map(row)
            .collect(),
        None => people.all().iter().map(row).collect(),
    };
    Json(json!({ "personas": rows, "warnings": people.warnings }))
}

/// A persona, plus the two things it does not carry itself: the **effective**
/// `client:` and the source it came from.
fn row(sourced: &crate::registry::Sourced) -> Value {
    let mut value = serde_json::to_value(&sourced.persona).unwrap_or(Value::Null);
    if let Some(object) = value.as_object_mut() {
        match &sourced.client {
            Some(client) => object.insert("client".into(), Value::from(client.clone())),
            None => object.remove("client"),
        };
        object.insert("source".into(), Value::from(sourced.source()));
    }
    value
}
