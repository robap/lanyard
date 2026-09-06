//! lanyard talking about itself, as JSON.
//!
//! `/_/api/token` — the Phase 1 test seam — lives in [`crate::seam`]; this
//! module is the live request log's two endpoints. They sit under the same
//! `/_/api/` prefix so lanyard's own namespace stays one namespace, and it is
//! the same path cubby serves them on, which is what makes the post-v1
//! extraction a merge rather than a translation.

pub mod events;

use axum::routing::{get, post};
use axum::Router;

use crate::app::SharedState;

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route("/api/events", get(events::stream))
        .route("/api/events/clear", post(events::clear))
}
