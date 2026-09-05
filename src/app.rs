use std::sync::Arc;

use axum::Router;

use crate::config::Config;
use crate::keys::SigningKey;
use crate::persona::Personas;

/// Everything resolved once at startup and shared, read-only, by every handler.
/// Nothing here is derived per-request — least of all the issuer.
pub struct AppState {
    pub config: Config,
    pub key: SigningKey,
    pub personas: Personas,
}

pub type SharedState = Arc<AppState>;

/// Builds the whole HTTP surface. Deliberately does not own the listener: the
/// binary binds the one fixed port, tests bind port 0.
pub fn router(state: SharedState) -> Router {
    Router::new()
        // Mounted from day one: /oidc for the protocol, /_/ for the UI and its
        // JSON seam, and the root left free. Costs nothing, avoids a breaking
        // URL change later (CONCEPT §3).
        .nest("/oidc", crate::oidc::routes::routes())
        .nest("/_", crate::seam::routes())
        .with_state(state)
}
