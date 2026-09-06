//! `GET /_/api/events` — the live request log as Server-Sent Events, and its
//! `?format=ndjson` twin for `jq` and test harnesses.
//!
//! **One handler, one stream of already-encoded frames, two content types.**
//! Two code paths would be two things that could disagree about what an event
//! looks like, which is the drift north star 3 forbids — the same argument that
//! keeps issuance in one function.
//!
//! Both replay the ring buffer first, honouring `Last-Event-ID`, then stream
//! live. A consumer that lags the broadcast channel receives a synthetic
//! `dropped` marker naming how many it missed, rather than stalling the server
//! or losing them silently.

use std::collections::HashMap;
use std::convert::Infallible;

use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::sync::broadcast::error::RecvError;

use crate::app::SharedState;
use crate::events::{BusSignal, Event};

/// `POST /_/api/events/clear` — drain the server-side ring and tell live
/// streams to empty. `204`.
///
/// Draining the ring rather than just the client's own list is what makes the
/// clear **survive an `EventSource` reconnect replay** and stay consistent
/// across open tabs: a reconnect after a clear replays nothing, and every other
/// tab empties itself when the `Clear` signal arrives.
pub async fn clear(State(state): State<SharedState>) -> Response {
    state.events.clear();
    StatusCode::NO_CONTENT.into_response()
}

/// Stream the event log. SSE by default; newline-delimited JSON when
/// `?format=ndjson` is set.
pub async fn stream(
    State(state): State<SharedState>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let ndjson = query.get("format").map(String::as_str) == Some("ndjson");

    // Reconnect replay. A `Last-Event-ID` that is not a number is no resume
    // point at all rather than an error: the browser sends this header on its
    // own, and refusing a stream because of it would be lanyard breaking a
    // reconnect it did not ask for.
    let after = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.trim().parse::<u64>().ok());

    let (backlog, mut rx) = state.events.subscribe(after);

    let frames = async_stream::stream! {
        for event in backlog {
            yield Ok::<_, Infallible>(frame_bytes(&event, ndjson));
        }
        loop {
            match rx.recv().await {
                Ok(BusSignal::Event(event)) => yield Ok(frame_bytes(&event, ndjson)),
                Ok(BusSignal::Clear) => yield Ok(clear_bytes(ndjson)),
                // **A log with an invisible hole is worse than no log**, because
                // the developer concludes the request never happened. The
                // fan-out is lossy at the subscriber and never at the source:
                // the ring and the stdout line were written regardless.
                Err(RecvError::Lagged(n)) => yield Ok(dropped_bytes(n, ndjson)),
                Err(RecvError::Closed) => break,
            }
        }
    };

    let content_type = if ndjson {
        "application/x-ndjson; charset=utf-8"
    } else {
        "text/event-stream; charset=utf-8"
    };

    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
            // Defeat proxy buffering so frames arrive as they happen.
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        Body::from_stream(frames),
    )
        .into_response()
}

/// One event, as an SSE frame (`id:` + `data:`) or as one ndjson line.
fn frame_bytes(event: &Event, ndjson: bool) -> Bytes {
    let json = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_owned());
    if ndjson {
        Bytes::from(format!("{json}\n"))
    } else {
        Bytes::from(format!("id: {}\ndata: {json}\n\n", event.id))
    }
}

/// The "empty your view" directive. It rides the **default** `data:` channel
/// carrying `"clear":true` instead of an `id`, so the client's one `onmessage`
/// handler sees it without a second listener.
fn clear_bytes(ndjson: bool) -> Bytes {
    if ndjson {
        Bytes::from_static(b"{\"clear\":true}\n")
    } else {
        Bytes::from_static(b"data: {\"clear\":true}\n\n")
    }
}

/// "You missed N." Named `dropped` on its own SSE event type, because a reader
/// that treated it as an ordinary event would render a row for a hole.
fn dropped_bytes(n: u64, ndjson: bool) -> Bytes {
    if ndjson {
        Bytes::from(format!("{{\"dropped\":{n}}}\n"))
    } else {
        Bytes::from(format!("event: dropped\ndata: {{\"dropped\":{n}}}\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventBus, EventDraft};

    fn event() -> Event {
        EventBus::new().publish(EventDraft {
            client_id: Some("billing-web".to_owned()),
            endpoint: "/oidc/token".to_owned(),
            method: "POST".to_owned(),
            status: 200,
            duration_ms: 3,
            ..EventDraft::default()
        })
    }

    #[test]
    fn an_sse_frame_carries_the_id_and_the_data() {
        let bytes = frame_bytes(&event(), false);
        let frame = std::str::from_utf8(&bytes).unwrap();
        assert!(frame.starts_with("id: 1\ndata: {"), "{frame}");
        assert!(frame.ends_with("}\n\n"), "{frame}");
        assert!(frame.contains("\"endpoint\":\"/oidc/token\""), "{frame}");
    }

    /// One JSON object per line, and no SSE framing to strip: `jq` reads this
    /// directly, which is what criterion 2 does.
    #[test]
    fn an_ndjson_frame_is_one_line_with_no_framing() {
        let bytes = frame_bytes(&event(), true);
        let frame = std::str::from_utf8(&bytes).unwrap();
        assert!(!frame.contains("data:"), "{frame}");
        assert!(frame.ends_with("}\n"), "{frame}");
        assert_eq!(frame.matches('\n').count(), 1, "{frame}");
    }

    #[test]
    fn the_clear_directive_rides_the_default_data_channel() {
        assert_eq!(
            std::str::from_utf8(&clear_bytes(false)).unwrap(),
            "data: {\"clear\":true}\n\n"
        );
        assert_eq!(
            std::str::from_utf8(&clear_bytes(true)).unwrap(),
            "{\"clear\":true}\n"
        );
    }

    /// Named, so a client can tell a hole from an event rather than rendering
    /// a row for one.
    #[test]
    fn the_dropped_marker_names_the_count_and_its_own_event_type() {
        assert_eq!(
            std::str::from_utf8(&dropped_bytes(37, false)).unwrap(),
            "event: dropped\ndata: {\"dropped\":37}\n\n"
        );
        assert_eq!(
            std::str::from_utf8(&dropped_bytes(37, true)).unwrap(),
            "{\"dropped\":37}\n"
        );
    }
}
