//! The minimum CORS a browser-based public client needs, and nothing more.
//!
//! **This is Phase 5's scope arriving one phase early, on purpose.** Phase 4's
//! spec lists CORS as out of scope and says `/oidc/token` "still answers a
//! same-origin request only" — and then asks, in acceptance criterion 2, for
//! `oidc-client-ts` to complete `signinRedirect()` from a page on
//! `http://localhost:5173`. Those two cannot both be true: a browser will not
//! let a SPA read the discovery document or POST to the token endpoint on
//! another origin without these headers. The criterion is the stronger
//! commitment — it is what "done" means for this phase, and a `node-spa` spike
//! that cannot log in is a broken example — so the headers are here.
//!
//! What is *not* here is the rest of Phase 5: no `refresh_token`, no
//! `/end_session`, no `/introspect`, no `/revoke`.
//!
//! **No `Access-Control-Allow-Credentials`, deliberately.** A public client
//! authenticates with a bearer token, not with lanyard's cookie, and allowing
//! credentialed cross-origin requests would let any page a developer visits
//! drive their session. The one rejection stays the only thing lanyard guards,
//! and this does not widen it.

use axum::extract::Request;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// The headers a client may send: a bearer token, and a form or JSON body's
/// content type.
const ALLOWED_HEADERS: &str = "authorization, content-type";
const ALLOWED_METHODS: &str = "GET, POST, OPTIONS";

/// Echo the request's `Origin` rather than answering `*`.
///
/// `*` would be simpler and is nearly equivalent here, but echoing keeps `Vary:
/// Origin` honest and means the response says which origin it was actually for
/// — which is the thing a developer reading their network tab is trying to
/// find out.
pub async fn layer(request: Request, next: Next) -> Response {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    // A preflight is answered here rather than by a router that would otherwise
    // call it a 405. It never reaches a handler, so no endpoint has to know
    // about it.
    let mut response = if request.method() == Method::OPTIONS && origin.is_some() {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(request).await
    };

    // `Vary` unconditionally: a cache that stored the no-Origin answer and
    // replayed it to a cross-origin fetch would produce the confusing
    // intermittent CORS failure this header exists to prevent.
    response
        .headers_mut()
        .append(header::VARY, HeaderValue::from_static("Origin"));

    if let Some(origin) = origin {
        if let Ok(value) = HeaderValue::from_str(&origin) {
            let headers = response.headers_mut();
            headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static(ALLOWED_METHODS),
            );
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static(ALLOWED_HEADERS),
            );
            headers.insert(
                header::ACCESS_CONTROL_MAX_AGE,
                HeaderValue::from_static("600"),
            );
        }
    }

    response
}
