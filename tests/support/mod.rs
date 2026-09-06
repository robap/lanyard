//! Shared harness for the HTTP-level tests: a server on an ephemeral port with
//! the issuer still the canonical fixed string, and a client that never follows
//! a redirect, because most of these assertions are *about* the redirect.
//!
//! Each integration test binary compiles this module separately and uses a
//! different subset of it, so anything one binary does not call is dead code in
//! that binary. That is the shared-harness tax, not an unused helper.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use lanyard_cli::app::{self, AppState};
use lanyard_cli::config::Config;
use lanyard_cli::events::EventBus;
use lanyard_cli::keys::{SigningKey, DEFAULT_DEV_KEY_PEM};
use lanyard_cli::persona::Personas;
use lanyard_cli::registry::Registry;
use lanyard_cli::store::Stores;

pub const ISSUER: &str = "http://127.0.0.1:9500/oidc";

/// The .NET spike's callback, url-encoded, which is what every test here uses
/// as a redirect target.
pub const ADA_CB: &str = "http%3A%2F%2Flocalhost%3A5000%2Fsignin-oidc";

pub async fn spawn() -> String {
    spawn_with(Personas::builtin()).await
}

/// A server whose codes die almost immediately, so the expired-code refusal is
/// testable without a 70-second sleep.
pub async fn spawn_with_short_lived_codes() -> String {
    spawn_configured(
        Personas::builtin(),
        Stores::with_ttls(
            std::time::Duration::from_secs(300),
            std::time::Duration::ZERO,
            lanyard_cli::store::REFRESH_TTL,
        ),
    )
    .await
}

pub async fn spawn_with(personas: Personas) -> String {
    spawn_configured(personas, Stores::default()).await
}

/// A server over a real [`Registry`] — the sources on disk, re-read as they
/// change. Everything else in this harness hands over one already-parsed list,
/// which is the right shape for a test about a grant and the wrong one for a
/// test about where personas come from.
pub async fn spawn_with_registry(registry: Registry) -> String {
    spawn_observed_registry(registry, Stores::default()).await.0
}

pub async fn spawn_configured(personas: Personas, stores: Stores) -> String {
    spawn_observed(personas, stores).await.0
}

/// The server **and its event bus**, for the tests that assert on what was
/// logged rather than on what was answered.
pub async fn spawn_observed(personas: Personas, stores: Stores) -> (String, EventBus) {
    spawn_observed_registry(Registry::fixed(personas), stores).await
}

pub async fn spawn_observed_registry(personas: Registry, stores: Stores) -> (String, EventBus) {
    let config = Config::resolve(|key| match key {
        "HOME" => Some("/nonexistent".to_string()),
        _ => None,
    })
    .unwrap();
    let key = SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap();
    let events = EventBus::new();
    let state = Arc::new(AppState {
        config,
        key,
        personas,
        stores,
        events: events.clone(),
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app::router(state)).await.unwrap();
    });
    (format!("http://{addr}"), events)
}

/// The event bus alongside the URL, with the built-in personas.
pub async fn spawn_logged() -> (String, EventBus) {
    spawn_observed(Personas::builtin(), Stores::default()).await
}

/// A whole browser login, ending at the authorization code. The picker is
/// driven exactly as a human drives it, so the code that comes back is a real
/// one with a real record behind it.
pub async fn login(base: &str, persona: &str, client_id: &str, scope: &str) -> String {
    login_as(&client(), base, persona, client_id, scope).await
}

/// The same login driven by a caller's client, so the session cookie the pick
/// sets stays with the browser the rest of the test is using.
pub async fn login_as(
    client: &reqwest::Client,
    base: &str,
    persona: &str,
    client_id: &str,
    scope: &str,
) -> String {
    let res = client
        .get(format!(
            "{base}/oidc/authorize?client_id={client_id}&redirect_uri={ADA_CB}\
             &response_type=code&scope={}",
            scope.replace(' ', "%20")
        ))
        .send()
        .await
        .unwrap();
    let location = res.headers()["location"].to_str().unwrap().to_owned();

    // A browser that already chose somebody for this `client_id` never sees the
    // picker: `/authorize` completes on the remembered selection and redirects
    // straight back to the RP. That is Phase 4's behaviour, not a special case
    // — so a second login through this helper has to take it.
    let location = match location.split("req=").nth(1) {
        Some(req) => {
            let res = client
                .post(format!("{base}/_/pick"))
                .form(&[("req", req), ("persona", persona)])
                .send()
                .await
                .unwrap();
            res.headers()["location"].to_str().unwrap().to_owned()
        }
        None => location,
    };
    url::Url::parse(&location)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .expect("an authorization code")
}

/// Read from a held-open stream until `until` bytes' worth of frames have
/// arrived or the deadline passes, then give back what was read.
///
/// The endpoint never ends its response, so a plain `.text()` would hang for
/// ever: these assertions are about what arrives, not about a body.
pub async fn read_stream(res: reqwest::Response, frames: usize) -> String {
    read_stream_for(res, frames, std::time::Duration::from_secs(5)).await
}

/// The same, with the deadline named — for the assertions that expect **no**
/// frames, where waiting out a generous timeout is the whole cost of the test.
pub async fn read_stream_for(
    res: reqwest::Response,
    frames: usize,
    within: std::time::Duration,
) -> String {
    use futures_util::StreamExt as _;

    let mut body = res.bytes_stream();
    let mut out = String::new();
    let deadline = tokio::time::Instant::now() + within;
    while complete_records(&out) < frames {
        let chunk = tokio::time::timeout_at(deadline, body.next()).await;
        match chunk {
            Ok(Some(Ok(bytes))) => out.push_str(&String::from_utf8_lossy(&bytes)),
            Ok(Some(Err(e))) => panic!("stream error: {e}"),
            Ok(None) => break,
            Err(_) => break,
        }
    }
    out
}

/// How many whole records have arrived, counting either encoding: an SSE frame
/// ends with a blank line, an ndjson record with one.
fn complete_records(out: &str) -> usize {
    out.matches("\n\n").count().max(out.matches("}\n").count())
}

/// Everything the bus has retained, in order.
pub fn logged(events: &EventBus) -> Vec<lanyard_cli::events::Event> {
    events.subscribe(None).0
}

/// The real binary, run the way a shell runs it, with a clean environment plus
/// whatever the caller names.
///
/// `env_clear` and a `HOME` that does not exist, so nothing here can read or
/// write a developer's real config directory — the registry these tests write
/// is always one `LANYARD_LINKS` points at.
pub fn run(env: &[(&str, &str)], args: &[&str]) -> std::process::Output {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_lanyard"));
    command
        .env_clear()
        .env("HOME", "/nonexistent")
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .args(args);
    for (key, value) in env {
        command.env(key, value);
    }
    // `cargo llvm-cov` tells an instrumented child where to write its profile;
    // a child that loses it drops a stray `default_*.profraw` in the repository
    // root and reports 0% for `main.rs`.
    if let Ok(profile) = std::env::var("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command.output().unwrap()
}

pub fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// A client that stops at the first response. Following a `302` to
/// `http://localhost:5000` in a test would try to reach a .NET app that is not
/// running, and the interesting thing was the header anyway.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .cookie_store(true)
        .build()
        .unwrap()
}

pub async fn authorize(base: &str, query: &str) -> reqwest::Response {
    client()
        .get(format!("{base}/oidc/authorize?{query}"))
        .send()
        .await
        .unwrap()
}

/// The decoded query parameters of a response's `Location`, which is where an
/// authorization error actually lands.
pub fn query_of(res: &reqwest::Response) -> HashMap<String, String> {
    let location = res.headers()["location"].to_str().unwrap();
    let url = url::Url::options()
        .base_url(Some(&url::Url::parse("http://127.0.0.1/").unwrap()))
        .parse(location)
        .unwrap();
    url.query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}
