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
use crate::oidc::revocation;
use crate::oidc::token::Form;

pub fn route() -> axum::routing::MethodRouter<SharedState> {
    axum::routing::post(revoke)
}

async fn revoke(State(state): State<SharedState>, body: Bytes) -> Response {
    let form = Form::parse(&body);

    // `token_type_hint` is read as a hint and acted on by nothing:
    // [`revocation::resolve`] identifies a string by what it is.
    if let Some(token) = form.get("token") {
        revocation::revoke(&state, token);
    }

    (
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-store")],
        // Empty on purpose. RFC 7009 §2.2 defines the successful response as a
        // `200` with no body, and a `{"revoked": true}` here would be lanyard
        // telling a caller something the RFC says it must not.
        (),
    )
        .into_response()
}
