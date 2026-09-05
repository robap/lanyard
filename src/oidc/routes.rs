//! The `/oidc` endpoints that exist. Each appears in the discovery document in
//! the phase that implements it — and, since Phase 5, **in the same commit as
//! the endpoint**, because .NET reads `end_session_endpoint` out of the document
//! and a URL that is missing there is a logout button that appears to do
//! nothing.

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
        .route("/authorize", crate::oidc::authorize::route())
        .route("/userinfo", crate::oidc::userinfo::route())
        .route("/end_session", crate::oidc::end_session::route())
        .route("/introspect", crate::oidc::introspect::route())
        .route("/revoke", crate::oidc::revoke::route())
        // The protocol surface, and only the protocol surface. `/_/` is for a
        // human and a test on this machine; nothing there answers another
        // origin.
        .layer(axum::middleware::from_fn(crate::oidc::cors::layer))
}

/// The issuer comes from startup config, never from the request. CONCEPT §8:
/// deriving it from `Host` works beautifully until one flow crosses the
/// boundary — which is every browser login — and then it fails confusingly.
async fn discovery(State(state): State<SharedState>) -> impl IntoResponse {
    Json(json!({
        "issuer": state.config.issuer,
        "jwks_uri": state.config.jwks_uri(),
        "token_endpoint": state.config.token_endpoint(),
        "authorization_endpoint": state.config.authorization_endpoint(),
        "userinfo_endpoint": state.config.userinfo_endpoint(),
        // **This is the phase where "advertise only what exists" cuts the other
        // way.** `SignOutAsync` reads `end_session_endpoint` from this document
        // and simply does not build a redirect when it is absent, so the
        // endpoint and its advertisement had to ship in the same commit — an
        // endpoint that exists and is not advertised fails exactly as
        // confusingly as one advertised and missing.
        "end_session_endpoint": state.config.end_session_endpoint(),
        "introspection_endpoint": state.config.introspection_endpoint(),
        "revocation_endpoint": state.config.revocation_endpoint(),
        "grant_types_supported": [
            "client_credentials", "authorization_code", "refresh_token"
        ],
        // All three are true, because none of them are checked: HTTP Basic,
        // form parameters, or no client authentication at all all mint.
        "token_endpoint_auth_methods_supported": [
            "none", "client_secret_basic", "client_secret_post"
        ],
        // The same three, for the same reason. RFC 7662 §2.1 requires client
        // authentication on introspection; lanyard accepts every form of it and
        // checks none, which the README says in the section that already
        // explains that anyone who can reach the port can mint anything.
        "introspection_endpoint_auth_methods_supported": [
            "none", "client_secret_basic", "client_secret_post"
        ],
        "revocation_endpoint_auth_methods_supported": [
            "none", "client_secret_basic", "client_secret_post"
        ],
        // **Exactly `["code"]`, corrected in Phase 4.** Phase 1 wrote
        // `["id_token", "code"]` when nothing implemented either. Now that
        // `/authorize` exists, advertising `id_token` tells a .NET app that its
        // default `ResponseType` is supported and then rejects it at request
        // time — the exact fail-later-and-further-away failure that the
        // advertise-only-what-exists rule was written to prevent.
        "response_types_supported": ["code"],
        "response_modes_supported": ["query", "form_post"],
        "code_challenge_methods_supported": ["S256", "plain"],
        // The four that change what a token carries or what comes back with it.
        // Any other scope is accepted and echoed; these are the ones with a
        // documented effect. `offline_access` is not a claim filter and adds
        // nothing to any token — it decides whether there is a `refresh_token`.
        "scopes_supported": ["openid", "email", "profile", "offline_access"],
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
