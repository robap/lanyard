//! **The `Host` mismatch, once per distinct host**, on three surfaces from one
//! source: the event stream, the `/_/` band, and `/_/health`'s `hosts_seen`.

mod support;

use serde_json::Value;

use lanyard_cli::events::Event;

async fn get_as(base: &str, path: &str, host: &str) -> reqwest::StatusCode {
    reqwest::Client::new()
        .get(format!("{base}{path}"))
        .header("host", host)
        .send()
        .await
        .unwrap()
        .status()
}

fn host_warnings(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| e.warnings.as_ref())
        .flatten()
        .filter(|w| w.contains("reached as"))
        .cloned()
        .collect()
}

const DISCOVERY: &str = "/oidc/.well-known/openid-configuration";

/// The harness binds an ephemeral port while the issuer stays the canonical
/// `:9500`, so **every** request has to name the issuer's authority explicitly
/// or it is itself a mismatch — which is the check working, not a nuisance.
const AS_THE_ISSUER: &str = "127.0.0.1:9500";

async fn body_as(base: &str, path: &str, host: &str) -> String {
    reqwest::Client::new()
        .get(format!("{base}{path}"))
        .header("host", host)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
}

async fn health_as(base: &str, host: &str) -> Value {
    serde_json::from_str(&body_as(base, "/_/health", host).await).unwrap()
}

/// Criterion 10. Ten identical curls, one warning — and it names both
/// authorities and the line that fixes it.
#[tokio::test]
async fn a_mismatching_host_warns_exactly_once_and_names_the_fix() {
    let (base, bus) = support::spawn_logged().await;

    for _ in 0..10 {
        assert_eq!(get_as(&base, DISCOVERY, "lanyard:9500").await, 200);
    }

    let (events, _rx) = bus.subscribe(None);
    let warnings = host_warnings(&events);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let w = &warnings[0];
    assert!(w.contains("lanyard:9500"), "{w}");
    assert!(w.contains(support::ISSUER), "{w}");
    assert!(
        w.contains("LANYARD_ISSUER=http://lanyard:9500/oidc"),
        "the fix, verbatim: {w}"
    );

    // A second distinct host is a second warning; the first stays silent.
    assert_eq!(get_as(&base, DISCOVERY, "other:9500").await, 200);
    assert_eq!(get_as(&base, DISCOVERY, "lanyard:9500").await, 200);
    let (events, _rx) = bus.subscribe(None);
    let warnings = host_warnings(&events);
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[1].contains("other:9500"), "{warnings:?}");
}

/// **Discovery is the first request of every flow**, and the one where the
/// mismatch is cheapest to catch — so the first sighting publishes an event
/// even though a discovery request normally publishes none. The once-per-host
/// rule is what makes that safe.
#[tokio::test]
async fn the_warning_fires_on_discovery_which_otherwise_emits_nothing() {
    let (base, bus) = support::spawn_logged().await;

    assert_eq!(get_as(&base, DISCOVERY, AS_THE_ISSUER).await, 200);
    assert_eq!(get_as(&base, "/oidc/jwks", AS_THE_ISSUER).await, 200);
    let (quiet, _rx) = bus.subscribe(None);
    assert!(
        quiet.is_empty(),
        "a matching host on an excluded path is still silent: {quiet:?}"
    );

    assert_eq!(get_as(&base, "/oidc/jwks", "lanyard:9500").await, 200);
    let (events, _rx) = bus.subscribe(None);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].endpoint, "/oidc/jwks");
    assert_eq!(host_warnings(&events).len(), 1);
}

/// A health probe is excluded from the log for good reason, but the *first*
/// sighting of a name is worth one line whatever asked for it.
#[tokio::test]
async fn health_reports_every_host_seen_so_doctor_can_read_it_from_outside() {
    let (base, _bus) = support::spawn_logged().await;

    assert_eq!(get_as(&base, DISCOVERY, "lanyard:9500").await, 200);
    assert_eq!(get_as(&base, DISCOVERY, "other:9500").await, 200);

    let body = health_as(&base, AS_THE_ISSUER).await;
    let seen: Vec<&str> = body["hosts_seen"]
        .as_array()
        .expect("hosts_seen")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(seen, ["lanyard:9500", "other:9500"]);
}

/// The third surface. `/_/` is where a developer looks when something is odd,
/// and Phase 7's band is the shape this reuses rather than inventing one.
#[tokio::test]
async fn the_picker_shows_a_band_naming_every_mismatching_host() {
    let (base, _bus) = support::spawn_logged().await;

    assert_eq!(get_as(&base, DISCOVERY, "lanyard:9500").await, 200);
    assert_eq!(get_as(&base, DISCOVERY, "other:9500").await, 200);

    let page = body_as(&base, "/_/", AS_THE_ISSUER).await;
    assert!(page.contains("lanyard:9500"), "{page}");
    assert!(page.contains("other:9500"), "{page}");
    assert!(
        page.contains("LANYARD_ISSUER=http://lanyard:9500/oidc"),
        "the band carries the fix, not just the complaint"
    );
}

/// Nothing anywhere changes for a request that arrives at the name the issuer
/// names.
#[tokio::test]
async fn a_matching_host_produces_no_warning_on_any_surface() {
    let (base, bus) = support::spawn_logged().await;

    assert_eq!(get_as(&base, "/oidc/userinfo", AS_THE_ISSUER).await, 401);

    let (events, _rx) = bus.subscribe(None);
    assert_eq!(host_warnings(&events).len(), 0, "{events:?}");

    let body = health_as(&base, AS_THE_ISSUER).await;
    assert_eq!(body["hosts_seen"], serde_json::json!([]));

    let page = body_as(&base, "/_/", AS_THE_ISSUER).await;
    assert!(!page.contains("reached as"), "{page}");
}

/// **The stream must not publish an event because a client connected to the
/// stream.** One connected client would feed the next, which is the whole
/// reason `log_layer::emits` excludes it — and a host warning is an event like
/// any other. So the event stream is skipped entirely: not sighted, not warned
/// about, which keeps the contract that every host in `hosts_seen` produced
/// exactly one warning.
#[tokio::test]
async fn connecting_to_the_event_stream_by_an_odd_name_feeds_it_nothing() {
    let (base, bus) = support::spawn_logged().await;

    let stream = reqwest::Client::new()
        .get(format!("{base}/_/api/events?format=ndjson"))
        .header("host", "lanyard:9500")
        .timeout(std::time::Duration::from_millis(300))
        .send()
        .await;
    drop(stream);

    let (events, _rx) = bus.subscribe(None);
    assert!(events.is_empty(), "{events:?}");

    let body = health_as(&base, AS_THE_ISSUER).await;
    assert_eq!(
        body["hosts_seen"],
        serde_json::json!([]),
        "not sighted either, so the next real request still earns the warning"
    );

    // …and it does.
    assert_eq!(get_as(&base, DISCOVERY, "lanyard:9500").await, 200);
    let (events, _rx) = bus.subscribe(None);
    assert_eq!(host_warnings(&events).len(), 1, "{events:?}");
}

/// **The image's `HEALTHCHECK` is `lanyard doctor --quiet`, which dials
/// `127.0.0.1:PORT` from inside the container.** With an issuer naming the
/// container network that is a mismatch on every probe — so a containerised
/// lanyard would report a permanent warning about itself, concerning a token no
/// liveness check can mint.
#[tokio::test]
async fn a_health_probe_by_an_odd_name_is_not_a_name_anyone_arrived_by() {
    let (base, bus) = support::spawn_logged().await;

    for _ in 0..5 {
        assert_eq!(get_as(&base, "/_/health", "lanyard:9500").await, 200);
    }

    let (events, _rx) = bus.subscribe(None);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(
        health_as(&base, AS_THE_ISSUER).await["hosts_seen"],
        serde_json::json!([])
    );

    // …and the first real request by that name still earns the warning.
    assert_eq!(get_as(&base, DISCOVERY, "lanyard:9500").await, 200);
    let (events, _rx) = bus.subscribe(None);
    assert_eq!(host_warnings(&events).len(), 1, "{events:?}");
}

/// **A diagnostic must not change what it observes.** `lanyard doctor` dials
/// whatever address it was pointed at, on purpose, and reports the mismatch
/// itself — and the container image runs it as a `HEALTHCHECK` every ten
/// seconds, so counting it would leave a permanent warning about the probe.
#[tokio::test]
async fn doctor_probing_by_an_odd_name_is_not_an_application_arriving() {
    let (base, bus) = support::spawn_logged().await;

    for _ in 0..3 {
        let status = reqwest::Client::new()
            .get(format!("{base}{DISCOVERY}"))
            .header("host", "lanyard:9500")
            .header("user-agent", lanyard_cli::hosts::DOCTOR_USER_AGENT)
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, 200);
    }

    let (events, _rx) = bus.subscribe(None);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(
        health_as(&base, AS_THE_ISSUER).await["hosts_seen"],
        serde_json::json!([])
    );

    // A browser or an SDK by the same name still earns it.
    assert_eq!(get_as(&base, DISCOVERY, "lanyard:9500").await, 200);
    let (events, _rx) = bus.subscribe(None);
    assert_eq!(host_warnings(&events).len(), 1, "{events:?}");
}
