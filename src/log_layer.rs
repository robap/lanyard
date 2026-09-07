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
use axum::http::header::{HOST, USER_AGENT};
use axum::middleware::Next;
use axum::response::Response;

use crate::app::SharedState;
use crate::events::{pretty, EventDraft};
use crate::hosts;
use crate::log_detail::LogDetail;

/// Which paths produce an event.
///
/// **The exclusions are load-bearing, not cosmetic.** `GET /_/health` is
/// probed every few seconds by a container runtime, and `GET /_/api/events` is
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
    // **Read before anything else, and on every path.** Discovery is the first
    // request of every flow and the one where a `Host` mismatch is cheapest to
    // catch, so the first sighting of a name publishes an event even though a
    // discovery request normally publishes none. The once-per-host rule is what
    // makes that safe: a warning that repeated on a polled endpoint is a
    // warning that gets filtered out.
    let host_warning = first_sighting(&state, &request);

    let method = request.method().to_string();
    let endpoint = request.uri().path().to_owned();
    let started = Instant::now();

    if !emits(&endpoint) {
        let response = next.run(request).await;
        // One line, once, for a name nobody has mentioned before — and nothing
        // at all on the ordinary run, which is what keeps the health probe and
        // the JWKS poll out of the log.
        if let Some(warning) = host_warning {
            let event = state.events.publish(EventDraft {
                endpoint,
                method,
                status: response.status().as_u16(),
                duration_ms: started.elapsed().as_millis() as u64,
                warnings: Some(vec![warning]),
                ..EventDraft::default()
            });
            println!("{}", pretty(&event));
        }
        return response;
    }

    let mut response = next.run(request).await;

    // Popped rather than read: the detail is a courier between the handler and
    // this line, and leaving it on the response would put decoded claims in an
    // extension a later layer could serialize.
    let detail = response
        .extensions_mut()
        .remove::<LogDetail>()
        .unwrap_or_default();

    // The host warning joins whatever the handler had to say about the persona
    // sources, rather than displacing it: both are "something about this
    // process is not what you think", and a reader wants both.
    let warnings = match (detail.warnings, host_warning) {
        (Some(mut warnings), Some(host)) => {
            warnings.push(host);
            Some(warnings)
        }
        (Some(warnings), None) => Some(warnings),
        (None, Some(host)) => Some(vec![host]),
        (None, None) => None,
    };

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
        warnings,
    });

    // stdout is the durable surface: `lanyard serve > lanyard.log` is the whole
    // persistence story, and it is always on with no flag.
    println!("{}", pretty(&event));

    response
}

/// **A machine checking on lanyard, rather than an application arriving by a
/// name.** Neither is sighted and neither is warned about.
///
/// The event stream, because publishing there would put an event on the stream
/// *because a client connected to the stream* — the exact feedback `emits`
/// excludes it to prevent.
///
/// The health probe, because the container image's `HEALTHCHECK` is
/// `lanyard doctor --quiet`, which dials `127.0.0.1:PORT` from inside. With an
/// issuer of `http://lanyard:9500/oidc` that is a mismatch, so **every
/// containerised lanyard would permanently warn about its own health probe** —
/// a warning about a token that cannot exist, because nothing mints from a
/// liveness check.
///
/// Skipping rather than sighting-without-warning is deliberate: it keeps the
/// contract that **every host in `hosts_seen` produced exactly one warning**,
/// and it leaves the warning for the first real request to earn.
fn is_probe(path: &str) -> bool {
    path == "/_/health" || path.starts_with("/_/api/events")
}

/// The warning a request earns **the first time** a mismatching name is seen,
/// and `None` every other time — including for a request with no `Host` at all,
/// which is not a name anyone can act on.
fn first_sighting(state: &SharedState, request: &Request) -> Option<String> {
    if is_probe(request.uri().path()) {
        return None;
    }

    // **`doctor` must not change what it observes.** It dials whatever address
    // it was pointed at and reports the mismatch itself, to a person, with the
    // fix — and the image's `HEALTHCHECK` runs it every ten seconds.
    if request
        .headers()
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|agent| agent == hosts::DOCTOR_USER_AGENT)
    {
        return None;
    }

    let host = request.headers().get(HOST)?.to_str().ok()?;
    if hosts::matches(&state.config.issuer, host) {
        return None;
    }
    state
        .hosts
        .sight(host)
        .then(|| hosts::warning(&state.config.issuer, host))
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

    /// A probe every five seconds would drown the live request log — the one
    /// surface Phase 6 exists to keep readable. The rule that excludes it is
    /// the general `/_/` one; this pins it, because the endpoint's whole
    /// contract depends on it.
    #[test]
    fn the_health_probe_emits_nothing() {
        assert!(!emits("/_/health"));
    }

    /// The image's `HEALTHCHECK` dials `127.0.0.1:PORT` from inside the
    /// container, which is a mismatch whenever the issuer names the container
    /// network — so a containerised lanyard would warn about itself, for ever,
    /// about a token no health probe can mint.
    #[test]
    fn the_probes_are_not_names_an_application_arrived_by() {
        assert!(is_probe("/_/health"));
        assert!(is_probe("/_/api/events"));
        assert!(is_probe("/_/api/events/clear"));

        // Everything an application actually uses still counts.
        assert!(!is_probe("/oidc/.well-known/openid-configuration"));
        assert!(!is_probe("/oidc/jwks"));
        assert!(!is_probe("/oidc/token"));
        assert!(!is_probe("/_/"));
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
