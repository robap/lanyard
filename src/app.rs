use std::sync::Arc;

use axum::Router;

use crate::clock::Clock;
use crate::config::Config;
use crate::events::EventBus;
use crate::hosts::HostSightings;
use crate::keys::SigningKey;
use crate::registry::Registry;
use crate::store::Stores;

/// Everything resolved once at startup and shared, read-only, by every handler.
/// Nothing here is derived per-request — least of all the issuer.
pub struct AppState {
    pub config: Config,
    pub key: SigningKey,
    /// **The process's one clock**, and the reason `src/clock.rs` holds the only
    /// `SystemTime::now()` in the crate. Injected rather than global so a test
    /// binary can spawn a skewed server and an unskewed one at the same time —
    /// which a `OnceLock` could not.
    pub clock: Clock,
    /// **Not a list — a merged view of many sources, re-resolved on demand.**
    /// Phase 7 replaced an immutable field with this so that editing a linked
    /// project's `lanyard.yaml` needs no restart; every handler that needs
    /// people calls `resolve()` once and reads the snapshot it gets back.
    pub personas: Registry,
    /// The only mutable state in the process, and all of it dies with the
    /// process: pending authorization requests, authorization codes, and
    /// sessions. Nothing here is persisted, which is why restarting lanyard
    /// logs everybody out.
    pub stores: Stores,
    /// The live request log's bus. Not "mutable state" in the sense above — a
    /// ring of what already happened, which nothing reads back to make a
    /// decision.
    pub events: EventBus,
    /// Every distinct `Host` that did not match the issuer's authority, in the
    /// order it was first seen. The second piece of interior mutability here
    /// after `stores`, and like those it dies with the process.
    pub hosts: HostSightings,
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
        .nest(
            "/_",
            crate::seam::routes()
                .merge(crate::api::routes())
                .merge(crate::health::routes())
                .merge(crate::embed::routes())
                .merge(crate::ui::routes()),
        )
        // **`/_/` is routed twice on purpose.** `nest("/_")` answers `/_` and
        // `/_/pick`, but not `/_/` — axum treats the nest root's trailing slash
        // as a different path. `/_/` is the URL the banner prints and the URL
        // `/authorize` redirects to, so it is routed rather than moved.
        .route("/_/", axum::routing::get(crate::ui::picker))
        // **Outside everything**, so one request is one event whatever answered
        // it — a handler, a 404, or axum's own 405. What it does *not* log is
        // in `log_layer::emits`, and the event stream is on that list for a
        // reason.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::log_layer::layer,
        ))
        .with_state(state)
}
