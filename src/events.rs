//! The in-process live-request event bus.
//!
//! One [`Event`] is recorded per request by [`crate::log_layer`]. The bus is a
//! `tokio::sync::broadcast` channel plus a 1,000-event ring buffer: new
//! subscribers replay the ring (optionally from a `Last-Event-ID`) and then
//! stream live, while a consumer that lags the channel is told how many it
//! missed rather than stalling the server. Nothing is persisted — the log
//! resets on restart, which is correct for a dev tool.
//!
//! The same event feeds three surfaces: stdout (see [`pretty`]), the SSE
//! stream and its ndjson twin.
//!
//! **This file is cubby's `src/events.rs` wearing lanyard's domain.** Every
//! name that can match cubby's does — `Event`, `EventDraft`, `BusSignal`,
//! `EventBus`, `subscribe(after)`, `pretty` — because the post-v1 plan is to
//! extract one crate shared by both, and a rename is a merge conflict for no
//! reason. What diverges is the payload: cubby captures no parameters, and
//! lanyard's parameters *are* the diagnosis.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::broadcast;

/// One captured request: what was asked, what was decided, what came out.
///
/// `request`, `detail` and `issued` are the decoded payloads a handler
/// attached (see [`crate::log_detail::LogDetail`]); everything else the
/// middleware knows on its own. Absent fields are skipped in the JSON so an
/// event carries only what actually happened.
///
/// `Deserialize` as well as `Serialize` because `lanyard logs` reads this back
/// off the wire and renders it with [`pretty`] — **the same function `lanyard
/// serve` prints with**, which is what makes criterion 4's "the same lines"
/// true by construction rather than by two formatters agreeing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub id: u64,
    /// Unix milliseconds when the event was recorded.
    pub ts: i64,
    pub client_id: Option<String>,
    pub endpoint: String,
    pub method: String,
    pub status: u16,
    pub duration_ms: u64,
    pub grant_type: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
    /// The decoded request parameters — `scope` as a list, `code_challenge`
    /// beside its method, and on a failure the values that were compared.
    pub request: Option<Map<String, Value>>,
    /// The endpoint-specific payload. `detail.pkce`'s three values are the
    /// whole of "which parameter mismatched".
    pub detail: Option<Map<String, Value>>,
    /// The decoded header and payload of every token minted.
    pub issued: Option<Map<String, Value>>,
    pub flaw: Option<String>,
}

/// What travels on the broadcast channel: a newly recorded [`Event`], or a
/// `Clear` directive (the ring was drained, so live consumers empty their view
/// too — see [`EventBus::clear`]).
///
/// The event is behind an `Arc` rather than inline — cubby's shape — because a
/// broadcast channel clones the signal once **per subscriber**, and lanyard's
/// event carries decoded parameters and claims where cubby's carries a bucket
/// and a key. Two open tabs and a `lanyard logs` should cost three refcount
/// bumps, not three deep copies of every token payload.
#[derive(Clone, Debug)]
pub enum BusSignal {
    Event(Arc<Event>),
    Clear,
}

/// Everything known before the bus stamps an `id` and a `ts`.
#[derive(Clone, Debug, Default)]
pub struct EventDraft {
    pub client_id: Option<String>,
    pub endpoint: String,
    pub method: String,
    pub status: u16,
    pub duration_ms: u64,
    pub grant_type: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
    pub request: Option<Map<String, Value>>,
    pub detail: Option<Map<String, Value>>,
    pub issued: Option<Map<String, Value>>,
    pub flaw: Option<String>,
}

/// Events retained for replay.
const RING_CAPACITY: usize = 1000;
/// How far a live consumer may lag before it is told events were dropped.
const CHANNEL_CAPACITY: usize = 1024;

struct Inner {
    tx: broadcast::Sender<BusSignal>,
    ring: Mutex<VecDeque<Event>>,
    next_id: AtomicU64,
    capacity: usize,
}

/// Cheap-to-clone handle to the shared bus.
#[derive(Clone)]
pub struct EventBus {
    inner: Arc<Inner>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::with_capacity(RING_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                tx,
                ring: Mutex::new(VecDeque::with_capacity(capacity.min(64))),
                next_id: AtomicU64::new(1),
                capacity,
            }),
        }
    }

    /// Stamp `draft` with a monotonic id and the current time, retain it in the
    /// ring, broadcast it, and return the finished event so the caller can also
    /// print a stdout line. Broadcasting under the ring lock keeps id order and
    /// makes [`EventBus::subscribe`] atomic: no event slips between a
    /// subscriber's backlog snapshot and its live feed.
    pub fn publish(&self, draft: EventDraft) -> Event {
        let event = Event {
            id: self.inner.next_id.fetch_add(1, Ordering::Relaxed),
            ts: now_millis(),
            client_id: draft.client_id,
            endpoint: draft.endpoint,
            method: draft.method,
            status: draft.status,
            duration_ms: draft.duration_ms,
            grant_type: draft.grant_type,
            error: draft.error,
            error_description: draft.error_description,
            request: draft.request,
            detail: draft.detail,
            issued: draft.issued,
            flaw: draft.flaw,
        };
        let mut ring = self.inner.ring.lock().expect("event ring poisoned");
        if ring.len() == self.inner.capacity {
            ring.pop_front();
        }
        ring.push_back(event.clone());
        // "No live subscribers" is not an error — the ring still retains it.
        let _ = self
            .inner
            .tx
            .send(BusSignal::Event(Arc::new(event.clone())));
        event
    }

    /// Drain the ring and tell live subscribers to clear. Held under the ring
    /// lock, as [`EventBus::publish`] is, so nothing slips between the drain
    /// and the signal: a later `subscribe` sees an empty backlog, and every
    /// connected stream empties its own view.
    pub fn clear(&self) {
        let mut ring = self.inner.ring.lock().expect("event ring poisoned");
        ring.clear();
        let _ = self.inner.tx.send(BusSignal::Clear);
    }

    /// Subscribe for live signals, returning the replay backlog first. `after`
    /// filters the backlog to events with `id > after` — the `Last-Event-ID`
    /// resume point; `None` replays the whole ring.
    pub fn subscribe(&self, after: Option<u64>) -> (Vec<Event>, broadcast::Receiver<BusSignal>) {
        let ring = self.inner.ring.lock().expect("event ring poisoned");
        let rx = self.inner.tx.subscribe();
        let backlog: Vec<Event> = ring
            .iter()
            .filter(|e| after.is_none_or(|a| e.id > a))
            .cloned()
            .collect();
        (backlog, rx)
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The aligned stdout line. **A developer who never opens the UI still gets the
/// sentence**, so a failure puts the whole `error_description` on the line and
/// truncates nothing.
pub fn pretty(e: &Event) -> String {
    format!(
        "{}  {:<12}  {:<4} {:<22} {:>3} {:>6}  {}",
        clock(e.ts),
        e.client_id.as_deref().unwrap_or("-"),
        e.method,
        e.endpoint,
        e.status,
        format!("{}ms", e.duration_ms),
        tail(e),
    )
    .trim_end()
    .to_owned()
}

/// `HH:MM:SS`, **UTC**. `std::time` offers no local offset and the three
/// dependencies this phase adds were budgeted; a timezone crate is not one of
/// them. The README says which clock this is rather than leaving a developer to
/// wonder why the log is six hours out.
fn clock(ts_millis: i64) -> String {
    let secs = ts_millis.div_euclid(1000).rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Everything after the status: the diagnosis, in the order a reader wants it.
/// A failure is its error and the **whole** sentence and nothing else; a success
/// names the grant, the person, what was minted and any flaw asked for.
fn tail(e: &Event) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(error) = &e.error {
        parts.push(error.clone());
        if let Some(description) = &e.error_description {
            parts.push(description.clone());
        }
    } else {
        if let Some(grant_type) = &e.grant_type {
            parts.push(grant_type.clone());
        }
        if let Some(persona) = e
            .detail
            .as_ref()
            .and_then(|d| d.get("persona"))
            .and_then(Value::as_str)
        {
            parts.push(persona.to_owned());
        }
        parts.extend(minted(e));
        if let Some(scope) = granted_scope(e) {
            parts.push(format!("scope={scope}"));
        }
        if let Some(method) = e
            .request
            .as_ref()
            .and_then(|r| r.get("code_challenge_method"))
            .and_then(Value::as_str)
        {
            parts.push(format!("pkce={method}"));
        }
    }

    if let Some(flaw) = &e.flaw {
        parts.push(format!("flaw={flaw}"));
    }
    parts.join("  ")
}

/// The scopes this request **granted**, comma-joined.
///
/// Read from the decoded parameters when the request named them, and otherwise
/// off the access token that came out. That second source is what makes the
/// authorization-code line name its scopes: an RP exchanging a code sends
/// `code` and a verifier and no `scope` at all — the grant is on the token, not
/// in the form, and a log that only read the form would go quiet on exactly the
/// line criterion 1 asks about.
fn granted_scope(e: &Event) -> Option<String> {
    if let Some(scope) = e
        .request
        .as_ref()
        .and_then(|r| r.get("scope"))
        .and_then(Value::as_array)
    {
        let names: Vec<&str> = scope.iter().filter_map(Value::as_str).collect();
        if !names.is_empty() {
            return Some(names.join(","));
        }
    }
    let claimed = e
        .issued
        .as_ref()
        .and_then(|i| i.get("access_token"))
        .and_then(|t| t.get("payload"))
        .and_then(|p| p.get("scope"))
        .and_then(Value::as_str)?;
    let names: Vec<&str> = claimed.split_whitespace().collect();
    (!names.is_empty()).then(|| names.join(","))
}

/// `sub=…`, `aud=…` and the token's **actual** remaining life, read off the
/// access token that was minted. `exp` is rendered relative to the `iat` of the
/// same token, so `--expired` reads as negative rather than as a number that
/// needs a calculator.
fn minted(e: &Event) -> Vec<String> {
    let Some(payload) = e
        .issued
        .as_ref()
        .and_then(|i| i.get("access_token"))
        .and_then(|t| t.get("payload"))
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(sub) = payload.get("sub").and_then(Value::as_str) {
        out.push(format!("sub={sub}"));
    }
    match payload.get("aud") {
        Some(Value::String(aud)) => out.push(format!("aud={aud}")),
        Some(Value::Array(auds)) => {
            let names: Vec<&str> = auds.iter().filter_map(Value::as_str).collect();
            if !names.is_empty() {
                out.push(format!("aud={}", names.join(",")));
            }
        }
        _ => {}
    }
    if let (Some(iat), Some(exp)) = (
        payload.get("iat").and_then(Value::as_i64),
        payload.get("exp").and_then(Value::as_i64),
    ) {
        out.push(format!("exp={:+}s", exp - iat));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn draft(endpoint: &str) -> EventDraft {
        EventDraft {
            client_id: Some("billing-web".to_owned()),
            endpoint: endpoint.to_owned(),
            method: "POST".to_owned(),
            status: 200,
            duration_ms: 3,
            ..EventDraft::default()
        }
    }

    fn obj(v: Value) -> Option<Map<String, Value>> {
        match v {
            Value::Object(map) => Some(map),
            _ => panic!("not an object"),
        }
    }

    #[test]
    fn ids_are_monotonic_from_one() {
        let bus = EventBus::new();
        let a = bus.publish(draft("/oidc/token"));
        let b = bus.publish(draft("/oidc/token"));
        assert_eq!(a.id, 1);
        assert_eq!(b.id, 2);
        assert!(b.ts >= a.ts);
    }

    #[test]
    fn the_ring_caps_at_capacity_and_drops_the_oldest() {
        let bus = EventBus::with_capacity(3);
        for _ in 0..5 {
            bus.publish(draft("/oidc/token"));
        }
        let (backlog, _rx) = bus.subscribe(None);
        assert_eq!(backlog.len(), 3);
        assert_eq!(backlog.first().unwrap().id, 3);
        assert_eq!(backlog.last().unwrap().id, 5);
    }

    #[test]
    fn subscribe_replays_strictly_after_the_given_id() {
        let bus = EventBus::new();
        for _ in 0..4 {
            bus.publish(draft("/oidc/token"));
        }
        let (backlog, _rx) = bus.subscribe(Some(2));
        let ids: Vec<u64> = backlog.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![3, 4]);
    }

    #[tokio::test]
    async fn a_live_subscriber_receives_what_is_published_after_it_subscribed() {
        let bus = EventBus::new();
        let (_backlog, mut rx) = bus.subscribe(None);
        let published = bus.publish(draft("/oidc/authorize"));
        match rx.recv().await.unwrap() {
            BusSignal::Event(received) => {
                assert_eq!(received.id, published.id);
                assert_eq!(received.endpoint, "/oidc/authorize");
            }
            BusSignal::Clear => panic!("expected an Event, got Clear"),
        }
    }

    #[tokio::test]
    async fn clear_empties_the_ring_and_tells_live_subscribers() {
        let bus = EventBus::new();
        bus.publish(draft("/oidc/token"));
        let (_backlog, mut rx) = bus.subscribe(None);

        bus.clear();

        let (backlog, _rx2) = bus.subscribe(None);
        assert!(backlog.is_empty(), "the ring is empty after clear");
        match rx.recv().await.unwrap() {
            BusSignal::Clear => {}
            BusSignal::Event(e) => panic!("expected Clear, got Event {}", e.id),
        }
    }

    /// The point of the stdout surface: the sentence Phases 4 and 5 wrote is
    /// on the line, whole, for a developer who never opens the UI.
    #[test]
    fn pretty_puts_the_whole_error_description_on_the_line() {
        let description = "the code_verifier does not match the S256 code_challenge \
                           this code was issued against";
        let bus = EventBus::new();
        let e = bus.publish(EventDraft {
            endpoint: "/oidc/token".to_owned(),
            status: 400,
            error: Some("invalid_grant".to_owned()),
            error_description: Some(description.to_owned()),
            grant_type: Some("authorization_code".to_owned()),
            ..draft("/oidc/token")
        });
        let line = pretty(&e);
        assert!(line.contains("billing-web"), "{line}");
        assert!(line.contains("POST"), "{line}");
        assert!(line.contains("/oidc/token"), "{line}");
        assert!(line.contains("400"), "{line}");
        assert!(line.contains("3ms"), "how long it took: {line}");
        assert!(line.contains("invalid_grant"), "{line}");
        assert!(line.contains(description), "the whole sentence: {line}");
    }

    #[test]
    fn pretty_names_the_grant_and_what_it_minted() {
        let bus = EventBus::new();
        let e = bus.publish(EventDraft {
            client_id: Some("lanyard-cli".to_owned()),
            grant_type: Some("client_credentials".to_owned()),
            issued: obj(json!({
                "access_token": {
                    "header": {"alg": "RS256"},
                    "payload": {"sub": "ada", "aud": "billing-api", "iat": 100, "exp": 160}
                }
            })),
            ..draft("/oidc/token")
        });
        let line = pretty(&e);
        assert!(line.contains("client_credentials"), "{line}");
        assert!(line.contains("sub=ada"), "{line}");
        assert!(line.contains("aud=billing-api"), "{line}");
        assert!(line.contains("exp=+60s"), "{line}");
    }

    #[test]
    fn pretty_names_the_persona_the_scopes_and_the_pkce_method_on_an_authorize() {
        let bus = EventBus::new();
        let e = bus.publish(EventDraft {
            method: "GET".to_owned(),
            status: 302,
            request: obj(json!({
                "scope": ["openid", "email", "profile"],
                "code_challenge_method": "S256",
            })),
            detail: obj(json!({ "persona": "ada" })),
            ..draft("/oidc/authorize")
        });
        let line = pretty(&e);
        assert!(line.contains(" ada"), "{line}");
        assert!(line.contains("scope=openid,email,profile"), "{line}");
        assert!(line.contains("pkce=S256"), "{line}");
    }

    /// Criterion 1's token line. The exchange form carries `code` and a
    /// verifier and no `scope`; the grant is on the token that came out.
    #[test]
    fn pretty_names_the_granted_scopes_even_when_the_request_did_not() {
        let bus = EventBus::new();
        let e = bus.publish(EventDraft {
            grant_type: Some("authorization_code".to_owned()),
            request: obj(json!({ "code": "8Xk2", "code_verifier": "dBjftJeZ" })),
            issued: obj(json!({
                "access_token": {
                    "header": {"alg": "RS256"},
                    "payload": {"sub": "ada", "scope": "openid email profile"}
                }
            })),
            ..draft("/oidc/token")
        });
        let line = pretty(&e);
        assert!(line.contains("scope=openid,email,profile"), "{line}");
    }

    /// The form wins when it has one: it is what was *asked for*, and a
    /// narrowed grant should read as the narrowing.
    #[test]
    fn a_requested_scope_is_preferred_over_the_tokens_own() {
        let bus = EventBus::new();
        let e = bus.publish(EventDraft {
            request: obj(json!({ "scope": ["openid"] })),
            issued: obj(json!({
                "access_token": {"header": {}, "payload": {"scope": "openid email profile"}}
            })),
            ..draft("/oidc/token")
        });
        assert!(pretty(&e).contains("scope=openid"), "{}", pretty(&e));
        assert!(!pretty(&e).contains("scope=openid,email"), "{}", pretty(&e));
    }

    #[test]
    fn pretty_names_the_flaw_when_one_was_asked_for() {
        let bus = EventBus::new();
        let e = bus.publish(EventDraft {
            flaw: Some("alg-none".to_owned()),
            ..draft("/oidc/token")
        });
        assert!(pretty(&e).contains("flaw=alg-none"));
    }

    /// **One shape, whatever produced it.** The spec's worked example carries
    /// `"issued": null` and `"flaw": null` rather than omitting them, so a
    /// consumer reads a field that is always there and sometimes null — which
    /// is the difference between "nothing was minted" and "this build is older
    /// than that field".
    #[test]
    fn every_event_carries_the_whole_shape_with_nulls_for_what_did_not_happen() {
        let bus = EventBus::new();
        let e = bus.publish(draft("/oidc/token"));
        let json: Value = serde_json::to_value(&e).unwrap();
        for field in [
            "id",
            "ts",
            "client_id",
            "endpoint",
            "method",
            "status",
            "duration_ms",
            "grant_type",
            "error",
            "error_description",
            "request",
            "detail",
            "issued",
            "flaw",
        ] {
            assert!(json.get(field).is_some(), "{field} is missing from {json}");
        }
        assert_eq!(json["endpoint"], "/oidc/token");
        assert!(json["issued"].is_null());
        assert!(json["error_description"].is_null());
    }
}
