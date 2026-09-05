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
use lanyard_cli::keys::{SigningKey, DEFAULT_DEV_KEY_PEM};
use lanyard_cli::persona::Personas;
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

pub async fn spawn_configured(personas: Personas, stores: Stores) -> String {
    let config = Config::resolve(|key| match key {
        "HOME" => Some("/nonexistent".to_string()),
        _ => None,
    })
    .unwrap();
    let key = SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap();
    let state = Arc::new(AppState {
        config,
        key,
        personas,
        stores,
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app::router(state)).await.unwrap();
    });
    format!("http://{addr}")
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
