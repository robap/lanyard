use std::sync::Arc;

use axum::Router;

use crate::config::Config;
use crate::keys::SigningKey;
use crate::persona::Personas;
use crate::store::Stores;

/// Everything resolved once at startup and shared, read-only, by every handler.
/// Nothing here is derived per-request — least of all the issuer.
pub struct AppState {
    pub config: Config,
    pub key: SigningKey,
    pub personas: Personas,
    /// The only mutable state in the process, and all of it dies with the
    /// process: pending authorization requests, authorization codes, and
    /// sessions. Nothing here is persisted, which is why restarting lanyard
    /// logs everybody out.
    pub stores: Stores,
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
        // The UI and the JSON seam share the `/_` nest: one is for a human and
        // one is for a test, and both are lanyard talking about itself rather
        // than speaking the protocol.
        .nest("/_", crate::seam::routes().merge(crate::ui::routes()))
        // **`/_/` is routed twice on purpose.** `nest("/_")` answers `/_` and
        // `/_/pick`, but not `/_/` — axum treats the nest root's trailing slash
        // as a different path. `/_/` is the URL the banner prints and the URL
        // `/authorize` redirects to, so it is routed rather than moved.
        .route("/_/", axum::routing::get(crate::ui::picker))
        .with_state(state)
}
