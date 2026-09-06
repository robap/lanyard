//! `POST /oidc/introspect` — RFC 7662.
//!
//! **Always `200`, and everything unrecognized is exactly `{"active": false}`.**
//! RFC 7662 §2.2 is explicit that an inactive response reveals no other fields,
//! so a token signed by another issuer, a string that is not a JWT, an empty
//! `token`, a revoked token and an expired one all produce the same two words.
//! That is not an error: "I do not recognise this" *is* the answer this endpoint
//! exists to give.
//!
//! **Client authentication is accepted in any form and checked in none**, which
//! RFC 7662 §2.1 says MUST NOT be the case. It is a deliberate divergence, it is
//! the same one `/oidc/token` already ships, and the mitigation is the same:
//! loopback by default, and a README that says so. An introspection endpoint
//! demanding a credential lanyard does not check would be theatre with extra
//! steps.
//!
//! **No claims are assembled here.** The fields come off the verified token, or
//! off the refresh record, and nothing is invented.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Map, Value};

use crate::app::SharedState;
use crate::log_detail::{attach, LogDetail};
use crate::oidc::revocation::{self, Presented};
use crate::oidc::token::Form;

pub fn route() -> axum::routing::MethodRouter<SharedState> {
    axum::routing::post(introspect)
}

async fn introspect(State(state): State<SharedState>, body: Bytes) -> Response {
    let form = Form::parse(&body);

    // `token_type_hint` is read as a hint, which here means read and not acted
    // on: [`revocation::resolve`] identifies a string by what it is rather than
    // by what it was announced as, and RFC 7662 §2.1 says a server MUST NOT
    // reject a request because the hint was wrong.
    let Some(token) = form.get("token") else {
        return inactive();
    };

    match revocation::resolve(&state, token) {
        Presented::Access(claims) => active(&claims),
        Presented::Refresh(record) => {
            let mut body = Map::new();
            body.insert("active".into(), Value::Bool(true));
            body.insert("token_type".into(), Value::from("Bearer"));
            body.insert("client_id".into(), Value::from(record.client_id.clone()));
            body.insert("sub".into(), Value::from(record.persona.id.clone()));
            body.insert("iss".into(), Value::from(state.config.issuer.clone()));
            if let Some(scope) = &record.scope_raw {
                body.insert("scope".into(), Value::from(scope.clone()));
            }
            if let Some(audience) = &record.audience {
                body.insert("aud".into(), Value::from(audience.clone()));
            }
            // No `exp`, `iat` or `jti`: a refresh token is an id into a record,
            // not a document with claims, and inventing three would be lanyard
            // saying something it does not know.
            attach(
                json_response(Value::Object(body)),
                outcome(true, Some(&record.client_id)),
            )
        }
        Presented::Unrecognized => inactive(),
    }
}

/// The active response, assembled out of the token's own claims. Every field is
/// present only if the token carried it — an access token minted with no `aud`
/// is legitimate, and reporting one it does not have would be a lie about the
/// token rather than a description of it.
fn active(claims: &Map<String, Value>) -> Response {
    let mut body = Map::new();
    body.insert("active".into(), Value::Bool(true));
    body.insert("token_type".into(), Value::from("Bearer"));
    for name in [
        "scope",
        "client_id",
        "sub",
        "aud",
        "iss",
        "exp",
        "iat",
        "jti",
    ] {
        if let Some(value) = claims.get(name) {
            body.insert(name.to_string(), value.clone());
        }
    }
    let client_id = claims
        .get("client_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    attach(
        json_response(Value::Object(body)),
        outcome(true, client_id.as_deref()),
    )
}

/// **Exactly two words, and nothing more** (RFC 7662 §2.2).
///
/// The *log* says more, and has to: "lanyard said no" and "lanyard was never
/// called" look identical from the client side, and telling them apart is what
/// the log is for. Nothing about the response body changes.
fn inactive() -> Response {
    attach(
        json_response(json!({ "active": false })),
        outcome(false, None),
    )
}

/// What this endpoint decided, for the log: whether the token was recognized
/// and which application it belonged to.
fn outcome(active: bool, client_id: Option<&str>) -> LogDetail {
    LogDetail {
        client_id: client_id.map(str::to_string),
        detail: Some(
            [("active".to_string(), Value::Bool(active))]
                .into_iter()
                .collect(),
        ),
        ..LogDetail::default()
    }
}

fn json_response(body: Value) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}
