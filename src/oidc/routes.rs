//! The two `/oidc` endpoints Phase 1 implements. Everything else — `/authorize`,
//! `/token`, `/userinfo` — appears in the phase that implements it, and the
//! discovery document does not advertise it before then.

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

use crate::app::SharedState;

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
}

/// The issuer comes from startup config, never from the request. CONCEPT §8:
/// deriving it from `Host` works beautifully until one flow crosses the
/// boundary — which is every browser login — and then it fails confusingly.
async fn discovery(State(state): State<SharedState>) -> impl IntoResponse {
    Json(json!({
        "issuer": state.config.issuer,
        "jwks_uri": state.config.jwks_uri(),
        "response_types_supported": ["id_token", "code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
    }))
}

async fn jwks(State(state): State<SharedState>) -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "keys": [state.key.public_jwk()] })),
    )
}
