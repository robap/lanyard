//! **One clock, and a deliberately wrong one.**
//!
//! `src/clock.rs` is the only place in the crate that reads wall time, and the
//! clock is a value on `AppState`. This binary is what makes that structural
//! claim observable: it spawns a skewed server and an unskewed one *in the same
//! process*, which is precisely what a global clock could not support.

mod support;

use serde_json::Value;

use lanyard_cli::clock::{parse_skew, Clock};

/// Five minutes, the skew the spec's worked examples use.
const FIVE_MINUTES: i64 = 300;

async fn mint(base: &str) -> Value {
    let res = reqwest::Client::new()
        .post(format!("{base}/_/api/token?persona=ada"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    res.json::<Value>().await.unwrap()
}

/// The whole point of the knob: a token minted by a process five minutes behind
/// carries an `iat` five minutes in the past, and is therefore already dead for
/// anything on a correct clock.
#[tokio::test]
async fn a_skewed_server_mints_a_token_dated_by_its_own_clock() {
    let (base, _bus) = support::spawn_skewed(-FIVE_MINUTES).await;
    let real = Clock::real().now();

    let minted = mint(&base).await;
    let iat = minted["claims"]["iat"].as_u64().unwrap();

    assert!(
        real.saturating_sub(iat).abs_diff(FIVE_MINUTES as u64) <= 2,
        "iat {iat} should be ~5 minutes before the real {real}"
    );
    // `exp` moves with `iat`: the TTL is 60 seconds whatever the clock says,
    // and it is the *whole token* that is displaced, not its lifetime.
    let exp = minted["claims"]["exp"].as_u64().unwrap();
    assert_eq!(exp, iat + 60);
}

/// **The reason `events.rs` had to stop reading the clock itself.** A skewed
/// process whose log timestamps contradicted the tokens it was describing would
/// be worse than no log at all.
#[tokio::test]
async fn the_log_is_stamped_with_the_same_clock_the_token_was_minted_on() {
    let (base, bus) = support::spawn_skewed(-FIVE_MINUTES).await;

    let minted = mint(&base).await;
    let iat = minted["claims"]["iat"].as_u64().unwrap();

    let (events, _rx) = bus.subscribe(None);
    let event = events.last().expect("the mint was logged");
    assert_eq!(event.endpoint, "/_/api/token");
    assert!(
        (event.ts / 1000).unsigned_abs().abs_diff(iat) <= 2,
        "event ts {} and iat {iat} must be on the same clock",
        event.ts
    );
}

/// A skewed server and an unskewed one, alive at once, disagreeing about the
/// time by exactly the offset. Two processes could show this; one process
/// showing it is what proves the clock is injected rather than global.
#[tokio::test]
async fn a_skewed_server_and_an_unskewed_one_coexist_and_disagree() {
    let (skewed, _a) = support::spawn_skewed(-FIVE_MINUTES).await;
    let (straight, _b) = support::spawn_logged().await;

    let behind = mint(&skewed).await["claims"]["iat"].as_u64().unwrap();
    let now = mint(&straight).await["claims"]["iat"].as_u64().unwrap();

    assert!(
        now.saturating_sub(behind).abs_diff(FIVE_MINUTES as u64) <= 2,
        "{now} and {behind} are five minutes apart"
    );
}

/// Unset, nothing anywhere changes — the property that makes the knob safe to
/// ship at all.
#[tokio::test]
async fn an_unskewed_server_mints_on_the_machines_clock() {
    let (base, _bus) = support::spawn_logged().await;
    let real = Clock::real().now();

    let iat = mint(&base).await["claims"]["iat"].as_u64().unwrap();
    assert!(iat.abs_diff(real) <= 2, "{iat} vs {real}");
}

/// The variable is parsed exactly as `LANYARD_PORT` is: fatal on garbage,
/// naming itself. Asserted here as well as in the unit tests because the
/// spellings in the README are these.
#[test]
fn the_documented_spellings_all_parse() {
    assert_eq!(parse_skew("-5m"), Ok(-300));
    assert_eq!(parse_skew("+90s"), Ok(90));
    assert_eq!(parse_skew("300"), Ok(300));
}
