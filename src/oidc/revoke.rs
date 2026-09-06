//! `POST /oidc/revoke` — RFC 7009.
//!
//! **Always `200` with an empty body.** A token that was never issued, a string
//! that is not a JWT, one signed by somebody else, an absent `token` — all of
//! them get the same answer a real revocation gets, because RFC 7009 §2.2
//! requires it: a client must not be able to use this endpoint to find out
//! whether a token exists.
//!
//! Client authentication is accepted in any form and checked in none, as on
//! `/oidc/token` and `/oidc/introspect`.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::app::SharedState;
use crate::log_detail::{attach, LogDetail};
use crate::oidc::revocation::{self, Presented};
use crate::oidc::token::Form;
use serde_json::Value;

pub fn route() -> axum::routing::MethodRouter<SharedState> {
    axum::routing::post(revoke)
}

async fn revoke(State(state): State<SharedState>, body: Bytes) -> Response {
    let form = Form::parse(&body);

    // `token_type_hint` is read as a hint and acted on by nothing:
    // [`revocation::resolve`] identifies a string by what it is.
    //
    // Resolved *before* revoking, because revoking is what makes the record
    // unavailable — and the log's whole job here is to say which application's
    // token just died.
    let mut detail = LogDetail::default();
    if let Some(token) = form.get("token") {
        let (revoked, client_id) = match revocation::resolve(&state, token) {
            Presented::Access(claims) => (
                true,
                claims
                    .get("client_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            ),
            Presented::Refresh(record) => (true, Some(record.client_id.clone())),
            Presented::Unrecognized => (false, None),
        };
        revocation::revoke(&state, token);
        detail.client_id = client_id;
        detail.detail = Some(
            [("revoked".to_string(), Value::Bool(revoked))]
                .into_iter()
                .collect(),
        );
    }

    let response = (
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-store")],
        // Empty on purpose. RFC 7009 §2.2 defines the successful response as a
        // `200` with no body, and a `{"revoked": true}` here would be lanyard
        // telling a caller something the RFC says it must not. The **log** may
        // say it, because the log is not a response to the caller.
        (),
    )
        .into_response();
    attach(response, detail)
}
