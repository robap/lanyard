//! `GET /_/health` — liveness, the facts `doctor` needs to compare two sides,
//! and **no event**.

mod support;

use serde_json::Value;

use lanyard_cli::clock::Clock;

async fn health(base: &str) -> (reqwest::StatusCode, String, Value) {
    // `support::client()` names the issuer's authority, so these probes are not
    // themselves a `Host` mismatch — see its comment.
    let res = support::client()
        .get(format!("{base}/_/health"))
        .send()
        .await
        .unwrap();
    let status = res.status();
    let content_type = res
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    (status, content_type, res.json().await.unwrap())
}

/// Criterion 1. Every field is here so that two lanyards on two machines can be
/// compared without either consulting a third.
#[tokio::test]
async fn health_reports_the_facts_doctor_compares() {
    let (base, _bus) = support::spawn_logged().await;
    let before = Clock::real().now_millis();

    let (status, content_type, body) = health(&base).await;

    assert_eq!(status, 200);
    assert!(
        content_type.starts_with("application/json"),
        "{content_type}"
    );
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        body["issuer"],
        support::ISSUER,
        "byte-equal to the banner's Issuer line, because discovery is compared against it"
    );

    // The `kid` is the one key in `/oidc/jwks`, not a second reading of the
    // file: a health endpoint that could disagree with the JWKS is worse than
    // one that reports nothing.
    let jwks: Value = support::client()
        .get(format!("{base}/oidc/jwks"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let keys = jwks["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(body["kid"], keys[0]["kid"]);

    // **`now` is the only reason this returns a body at all**: it is what makes
    // skew measurable between two machines.
    let now = body["now"].as_i64().expect("now is unix milliseconds");
    let after = Clock::real().now_millis();
    assert!(
        now >= before - 2_000 && now <= after + 2_000,
        "{now} is not within 2s of [{before}, {after}]"
    );

    assert_eq!(body["skew"], 0, "an ordinary run reports no skew");
}

/// **A dev tool may lie about the time; it may not do so quietly.** The
/// endpoint `doctor` measures skew against is the one that has to admit it is
/// the skewed party.
#[tokio::test]
async fn a_skewed_server_reports_its_skew_and_a_skewed_now() {
    let (base, _bus) = support::spawn_skewed(-300).await;
    let real = Clock::real().now_millis();

    let (status, _, body) = health(&base).await;

    assert_eq!(status, 200, "a wrong clock is a diagnosis, not a death");
    assert_eq!(body["skew"], -300);
    let now = body["now"].as_i64().unwrap();
    assert!(
        (real - now).abs_diff(300_000) <= 2_000,
        "now {now} should be five minutes behind the real {real}"
    );
}

/// Criterion 2. A probe every five seconds would drown the live request log —
/// the one surface Phase 6 exists to keep readable — and the events from a real
/// login must survive twenty of them, in order.
#[tokio::test]
async fn twenty_probes_leave_the_live_log_untouched() {
    let (base, bus) = support::spawn_logged().await;

    let code = support::login(&base, "ada", "billing-web", "openid%20profile").await;
    assert!(!code.is_empty());
    let (before, _rx) = bus.subscribe(None);
    assert!(!before.is_empty(), "the login was logged");

    for _ in 0..20 {
        assert_eq!(health(&base).await.0, 200);
    }

    let (after, _rx) = bus.subscribe(None);
    assert_eq!(
        after.len(),
        before.len(),
        "not one event for twenty probes: {:?}",
        after.iter().map(|e| e.endpoint.clone()).collect::<Vec<_>>()
    );
    let endpoints: Vec<&str> = after.iter().map(|e| e.endpoint.as_str()).collect();
    assert!(!endpoints.contains(&"/_/health"), "{endpoints:?}");
    assert_eq!(
        before.iter().map(|e| e.id).collect::<Vec<_>>(),
        after.iter().map(|e| e.id).collect::<Vec<_>>(),
        "and the login's events are still there in order"
    );
}
