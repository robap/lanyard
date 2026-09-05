//! The `/oidc` endpoints that exist. Everything else — `/authorize`,
//! `/userinfo`, `/end_session` — appears in the phase that implements it, and
//! the discovery document does not advertise it before then.

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
        .route("/token", crate::oidc::token::route())
}

/// The issuer comes from startup config, never from the request. CONCEPT §8:
/// deriving it from `Host` works beautifully until one flow crosses the
/// boundary — which is every browser login — and then it fails confusingly.
async fn discovery(State(state): State<SharedState>) -> impl IntoResponse {
    Json(json!({
        "issuer": state.config.issuer,
        "jwks_uri": state.config.jwks_uri(),
        "token_endpoint": state.config.token_endpoint(),
        "grant_types_supported": ["client_credentials"],
        // All three are true, because none of them are checked: HTTP Basic,
        // form parameters, or no client authentication at all all mint.
        "token_endpoint_auth_methods_supported": [
            "none", "client_secret_basic", "client_secret_post"
        ],
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
