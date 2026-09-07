//! `GET /_/health` — **liveness, and only liveness.**
//!
//! The whole point of a health endpoint is `depends_on: condition:
//! service_healthy`, and an application that waits for lanyard must start even
//! when the issuer is misconfigured — because a misconfigured issuer is exactly
//! the state the developer is trying to debug. A health check that went red on
//! a *diagnosis* would turn "your token will be rejected" into "your stack will
//! not come up", which is strictly less debuggable.
//!
//! **Diagnosis lives in [`crate::doctor`]; liveness lives here.** So this is
//! `200` whenever the process is serving, whatever `doctor` thinks.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

use crate::app::SharedState;

pub fn routes() -> Router<SharedState> {
    Router::new().route("/health", get(health))
}

/// **Always `ok` while the process serves.** The four facts below are the ones
/// `doctor` needs in order to compare two sides of a misconfiguration without
/// either of them consulting a third party.
async fn health(State(state): State<SharedState>) -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        // Byte-equal to the banner's `Issuer →` line, because that is the
        // string a discovery document is compared against.
        "issuer": state.config.issuer,
        // Read off the loaded key, not off the file again: a health endpoint
        // that could disagree with `/oidc/jwks` is worse than one that reports
        // nothing at all.
        "kid": state.key.kid(),
        // **Unix milliseconds, and the only reason this returns a body.** It is
        // what makes skew measurable between two machines. On the process's own
        // clock, so a skewed lanyard is caught by the comparison rather than
        // hiding behind it.
        "now": state.clock.now_millis(),
        // And it says which way it is lying, so `doctor` can report the skew as
        // a fact rather than infer it from a difference.
        "skew": state.clock.skew_seconds(),
        // The third surface the `Host` mismatch reaches, and the one that lets
        // `doctor` report it from **outside** the process.
        "hosts_seen": state.hosts.seen(),
    }))
}
