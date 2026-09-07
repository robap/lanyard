//! **The only place in this crate that reads wall time.**
//!
//! Five copies of `unix_now()` and a sixth read in `events.rs` meant the process
//! had six clocks that happened to agree. `LANYARD_CLOCK_SKEW` is the moment
//! they stop agreeing — and a log whose timestamps contradict the tokens it is
//! describing is worse than no log. So the clock is a value, it lives on
//! [`crate::app::AppState`] beside the key and the config, and it is **injected
//! rather than global**: a `OnceLock` could not let one test binary spawn a
//! skewed server and an unskewed one.

/// The few seconds lanyard allows itself in both directions when verifying one
/// of its own tokens, and the amount `nbf` is backdated at issuance.
///
/// Small on purpose: 60 seconds of life plus 5 of leeway is 65, and Phase 2's
/// "the same call 90 seconds later returns 401" is still comfortably true.
pub const LEEWAY: u64 = 5;

/// The environment variable that makes lanyard lie about the time.
pub const SKEW_VAR: &str = "LANYARD_CLOCK_SKEW";

/// A clock, possibly a deliberately wrong one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    skew: i64,
}

impl Clock {
    /// The machine's clock, unmodified.
    pub const fn real() -> Self {
        Clock { skew: 0 }
    }

    /// A clock `skew` seconds away from the machine's.
    pub const fn skewed(skew: i64) -> Self {
        Clock { skew }
    }

    /// How far this clock is from the machine's, in seconds. Signed, and `0`
    /// for the ordinary case.
    pub const fn skew_seconds(&self) -> i64 {
        self.skew
    }

    pub const fn is_skewed(&self) -> bool {
        self.skew != 0
    }

    /// Unix seconds, which is the clock a token's `exp` is measured on.
    ///
    /// **The one `SystemTime::now()` in the crate**, together with
    /// [`Clock::now_millis`]. Saturating rather than wrapping: a clock skewed
    /// further into the past than the epoch is nonsense, and `0` is the same
    /// answer the five hand-rolled `unix_now()`s gave for a broken clock.
    pub fn now(&self) -> u64 {
        let real = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        real.saturating_add_signed(self.skew)
    }

    /// Unix milliseconds, which is what an event is timestamped with and what
    /// `/_/health` reports so two processes can compare clocks.
    pub fn now_millis(&self) -> i64 {
        let real = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        real.saturating_add(self.skew.saturating_mul(1_000))
    }
}

/// Parse `LANYARD_CLOCK_SKEW`: `-5m`, `+90s`, `300`, `2h`. Fatal on anything
/// else, naming the variable the way `LANYARD_PORT` does.
pub fn parse_skew(raw: &str) -> Result<i64, String> {
    let bad = || format!("{SKEW_VAR} is not an offset: {raw:?} — try -5m, +90s, 300 or 2h");
    let (digits, unit) = match raw.chars().last() {
        Some('s') => (&raw[..raw.len() - 1], 1),
        Some('m') => (&raw[..raw.len() - 1], 60),
        Some('h') => (&raw[..raw.len() - 1], 3600),
        Some(c) if c.is_ascii_digit() => (raw, 1),
        _ => return Err(bad()),
    };
    // `i64::from_str` accepts a leading `+` and `-` and nothing else — no
    // whitespace, no underscores, no second sign — which is exactly the
    // grammar this wants, so the digits are not re-validated by hand.
    let value: i64 = digits.parse().map_err(|_| bad())?;
    value.checked_mul(unit).ok_or_else(bad)
}

/// A signed offset in the words the banner and `doctor` print: `-5m0s`,
/// `+1m30s`, `+45s`.
pub fn format_offset(seconds: i64) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    format!("{sign}{}", format_duration(seconds.unsigned_abs()))
}

/// A span in the words a refusal prints: `4m12s`, `5m0s`, `45s`.
///
/// **The one duration formatter**, so a banner that says the clock is `-5m0s`
/// out and a `401` that says the token expired cannot disagree about how long
/// five minutes is. Leading zero units are dropped and trailing ones are not:
/// `5m0s` says "five minutes exactly" where `5m` would leave a reader wondering
/// whether the seconds were rounded away.
pub fn format_duration(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if h > 0 {
        format!("{h}h{m}m{s}s")
    } else if m > 0 {
        format!("{m}m{s}s")
    } else {
        format!("{s}s")
    }
}

/// Round a span to the precision the word "about" promises: to the nearest
/// minute once it is a minute and a half or more, exact below that.
///
/// A clock five minutes ahead is measured a few seconds after the token was
/// minted, so the raw figure is `4m57s` — technically exact and, prefixed with
/// "about", faintly absurd. The developer is going to compare it against a
/// `ClockSkew` setting or a VM's drift, and neither is measured in seconds.
pub fn round_about(seconds: u64) -> u64 {
    if seconds < 90 {
        return seconds;
    }
    seconds.div_ceil(30) / 2 * 60
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unskewed_clock_reads_the_machines_time() {
        let real = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let now = Clock::real().now();
        assert!(now.abs_diff(real) <= 1, "{now} vs {real}");
        assert!(!Clock::real().is_skewed());
    }

    #[test]
    fn a_skewed_clock_moves_seconds_and_millis_together() {
        let real = Clock::real();
        let back = Clock::skewed(-300);
        assert!(
            real.now().saturating_sub(back.now()).abs_diff(300) <= 1,
            "five minutes behind"
        );
        assert!(
            (real.now_millis() - back.now_millis()).abs_diff(300_000) <= 1_000,
            "and the same five minutes in millis"
        );
        assert_eq!(back.skew_seconds(), -300);
        assert!(back.is_skewed());
    }

    #[test]
    fn skew_parses_suffixed_and_bare_values() {
        assert_eq!(parse_skew("-5m"), Ok(-300));
        assert_eq!(parse_skew("+90s"), Ok(90));
        assert_eq!(parse_skew("300"), Ok(300));
        assert_eq!(parse_skew("-45"), Ok(-45));
        assert_eq!(parse_skew("2h"), Ok(7200));
        assert_eq!(parse_skew("0"), Ok(0));
    }

    #[test]
    fn garbage_skew_is_an_error_naming_the_variable() {
        for garbage in ["", "soon", "5x", "m5", "5m5", "--5m", "1.5m", " 5m"] {
            let err = parse_skew(garbage).unwrap_err();
            assert!(err.contains(SKEW_VAR), "{garbage:?} → {err}");
        }
    }

    #[test]
    fn a_duration_reads_the_way_a_refusal_prints_it() {
        assert_eq!(format_duration(252), "4m12s");
        assert_eq!(format_duration(300), "5m0s");
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(7200), "2h0m0s");
    }

    /// "About 4m57s" is a sentence that undoes its own hedge. Five minutes of
    /// skew measured three seconds late is five minutes.
    #[test]
    fn about_rounds_to_the_minute_once_there_is_a_minute_to_round() {
        assert_eq!(format_duration(round_about(297)), "5m0s");
        assert_eq!(format_duration(round_about(303)), "5m0s");
        assert_eq!(format_duration(round_about(252)), "4m0s");
        // Below a minute and a half, the seconds are the whole answer.
        assert_eq!(format_duration(round_about(45)), "45s");
        assert_eq!(format_duration(round_about(7)), "7s");
    }

    #[test]
    fn an_offset_reads_the_way_the_banner_prints_it() {
        assert_eq!(format_offset(-300), "-5m0s");
        assert_eq!(format_offset(90), "+1m30s");
        assert_eq!(format_offset(45), "+45s");
        assert_eq!(format_offset(-7200), "-2h0m0s");
        assert_eq!(format_offset(0), "+0s");
    }
}
