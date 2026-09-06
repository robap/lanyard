//! The one emit point.
//!
//! An axum middleware wraps the whole router: it times the request, reads the
//! final status, folds in whatever the handler attached to the response
//! extensions (see [`crate::log_detail`]), publishes **exactly one**
//! [`crate::events::Event`], and prints one line.
//!
//! **Two places that could build an event is the drift north star 3 exists to
//! prevent** — applied to observation rather than to issuance. If stdout and
//! the SSE stream could disagree about what happened, the structure is wrong.
//! So the middleware knows the envelope (method, path, status, duration) and
//! the handler knows the diagnosis (`client_id`, `grant_type`, the decoded
//! parameters, the claims), and they meet here.

use std::time::Instant;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::app::SharedState;
use crate::events::{pretty, EventDraft};
use crate::log_detail::LogDetail;

/// Which paths produce an event.
///
/// **The exclusions are load-bearing, not cosmetic.** `GET /_/api/events` is
/// held open by every connected client; logging the connection would publish an
/// event that every one of them receives, and a second client's connect would
/// feed the first. Discovery and JWKS are excluded for the spec's stated
/// reason: an SDK's poll loop on them would drown everything else.
///
/// Under `/_/` the rule is the other way round — that surface is pages, and a
/// page is not a decision. Exactly two routes there are decisions: `POST
/// /_/pick` turns "the picker appeared" into "you are Ada", and the test seam's
/// `POST /_/api/token` mints. Everything else — `/_/` itself, `/_/log`,
/// `/_/assets/…`, `/_/.zero/…`, `/_/api/events`, the session controls — emits
/// nothing.
pub fn emits(path: &str) -> bool {
    match path {
        "/oidc/.well-known/openid-configuration" | "/oidc/jwks" => false,
        "/_/pick" | "/_/api/token" => true,
        _ => path.starts_with("/oidc/"),
    }
}

pub async fn layer(State(state): State<SharedState>, request: Request, next: Next) -> Response {
    if !emits(request.uri().path()) {
        return next.run(request).await;
    }

    let method = request.method().to_string();
    let endpoint = request.uri().path().to_owned();
    let started = Instant::now();

    let mut response = next.run(request).await;

    // Popped rather than read: the detail is a courier between the handler and
    // this line, and leaving it on the response would put decoded claims in an
    // extension a later layer could serialize.
    let detail = response
        .extensions_mut()
        .remove::<LogDetail>()
        .unwrap_or_default();

    let event = state.events.publish(EventDraft {
        client_id: detail.client_id,
        endpoint,
        method,
        status: response.status().as_u16(),
        duration_ms: started.elapsed().as_millis() as u64,
        grant_type: detail.grant_type,
        error: detail.error,
        error_description: detail.error_description,
        request: detail.request,
        detail: detail.detail,
        issued: detail.issued,
        flaw: detail.flaw,
        warnings: detail.warnings,
    });

    // stdout is the durable surface: `lanyard serve > lanyard.log` is the whole
    // persistence story, and it is always on with no flag.
    println!("{}", pretty(&event));

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_protocol_surface_emits_except_the_two_static_documents() {
        assert!(emits("/oidc/token"));
        assert!(emits("/oidc/authorize"));
        assert!(emits("/oidc/userinfo"));
        assert!(emits("/oidc/introspect"));
        assert!(emits("/oidc/revoke"));
        assert!(emits("/oidc/end_session"));
        assert!(!emits("/oidc/jwks"));
        assert!(!emits("/oidc/.well-known/openid-configuration"));
    }

    /// The event stream must not log itself, or one connected client feeds the
    /// next and the log becomes its own traffic.
    #[test]
    fn the_event_stream_and_the_log_page_emit_nothing() {
        assert!(!emits("/_/api/events"));
        assert!(!emits("/_/api/events/clear"));
        assert!(!emits("/_/log"));
        assert!(!emits("/_/assets/app.5dd6bdc5.css"));
        assert!(!emits("/_/.zero/fonts/Geist-VariableFont_wght.woff2"));
        assert!(!emits("/_/"));
    }

    #[test]
    fn the_two_underscore_routes_that_decide_something_emit() {
        assert!(emits("/_/pick"));
        assert!(emits("/_/api/token"));
        // …and no other `/_/` route does.
        assert!(!emits("/_/session"));
        assert!(!emits("/_/logout"));
        assert!(!emits("/_/forget"));
        assert!(!emits("/_/expire"));
        assert!(!emits("/_/api/personas"));
    }
}
