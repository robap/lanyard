//! `lanyard doctor` — six checks, one line each, an exit-code contract, and a
//! consequence sentence for anything that is not OK.
//!
//! **The consequence sentence is the feature.** A check that says "port
//! mismatch" and stops has moved the developer one step; a check that says what
//! will be rejected, and prints the line that fixes it, has finished the job.
//!
//! The comparisons here are pure functions of what was fetched, so the
//! arithmetic is unit-testable without a server — which matters because the
//! interesting cases are the ones a working setup cannot produce.

use std::fmt;
use std::time::Duration;

use serde::Deserialize;

use crate::config::Config;
use crate::runtime::Runtime;

/// What a check decided. The **exit-code contract** hangs off this: a container
/// health probe runs `doctor --quiet`, so only a `Fail` may be non-zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

impl fmt::Display for Level {
    /// `f.pad`, not `write_str`: the report is a column of levels, and
    /// `write_str` bypasses the width the format string asks for.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            Level::Ok => "OK",
            Level::Warn => "WARN",
            Level::Fail => "FAIL",
        })
    }
}

/// One line of the report, plus the sentence a reader needs when it is not OK.
pub struct Check {
    pub name: &'static str,
    pub level: Level,
    pub detail: String,
    /// The consequence, and where there is one, the command that fixes it.
    /// One entry per line, indented under the detail.
    pub consequence: Vec<String>,
}

impl Check {
    pub fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Check {
            name,
            level: Level::Ok,
            detail: detail.into(),
            consequence: Vec::new(),
        }
    }

    pub fn warn(name: &'static str, detail: impl Into<String>, consequence: Vec<String>) -> Self {
        Check {
            name,
            level: Level::Warn,
            detail: detail.into(),
            consequence,
        }
    }

    pub fn fail(name: &'static str, detail: impl Into<String>, consequence: Vec<String>) -> Self {
        Check {
            name,
            level: Level::Fail,
            detail: detail.into(),
            consequence,
        }
    }
}

/// Two spaces, so the report sits under its own heading the way the startup
/// banner's lines sit under `lanyard 0.1.0`.
const INDENT: &str = "  ";
/// Wide enough for `Signing key`, the longest name.
const NAME_WIDTH: usize = 12;
/// Wide enough for `FAIL` plus the gap before the detail.
const LEVEL_WIDTH: usize = 5;
/// Where the detail starts, and therefore where a consequence hangs.
const DETAIL_COLUMN: usize = INDENT.len() + NAME_WIDTH + 1 + LEVEL_WIDTH + 1;

/// Every check that ran, in the order a request travels.
#[derive(Default)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    pub fn push(&mut self, check: Check) {
        self.checks.push(check);
    }

    /// **Only a `FAIL` exits non-zero.**
    ///
    /// A warning may be deliberate: `LANYARD_ISSUER=http://lanyard:9500/oidc`
    /// looks like a mismatch from the host shell and is exactly right for the
    /// container network. The image's `HEALTHCHECK` is this command, so a
    /// container that never went healthy over an unusual issuer would block
    /// every service waiting on it — which is why the default contract has to
    /// be liveness-shaped. `--strict` is the CI lever that says otherwise.
    pub fn failed(&self, strict: bool) -> bool {
        self.checks.iter().any(|check| match check.level {
            Level::Fail => true,
            Level::Warn => strict,
            Level::Ok => false,
        })
    }

    /// The block a developer pastes into an issue.
    pub fn render(&self) -> String {
        let mut out = String::from("lanyard doctor\n");
        for check in &self.checks {
            out.push_str(&format!(
                "{INDENT}{:<NAME_WIDTH$} {:<LEVEL_WIDTH$} {}\n",
                check.name, check.level, check.detail
            ));
            for line in &check.consequence {
                // Aligned under the detail it explains, so the report reads as
                // one column of findings rather than as prose with a margin.
                out.push_str(&format!("{:width$}{line}\n", "", width = DETAIL_COLUMN));
            }
        }
        out
    }
}

/// What `/_/health` said, which is the whole of what `doctor` knows about the
/// other side.
#[derive(Debug, Clone, Deserialize)]
pub struct Health {
    pub status: String,
    pub version: String,
    pub issuer: String,
    pub kid: String,
    /// Unix milliseconds **on the server's clock**, which is the comparison the
    /// Clock check exists to make.
    pub now: i64,
    #[serde(default)]
    pub skew: i64,
    #[serde(default)]
    pub hosts_seen: Vec<String>,
}

/// The first check, and the only one that needs nothing but the environment.
///
/// Resolves it **exactly as `serve` does** — the same function, not a second
/// reading — so a `FAIL` here is a promise that `lanyard serve` would refuse to
/// start, rather than a guess about it.
pub fn config_check(resolved: &Result<Config, String>) -> Check {
    match resolved {
        Ok(config) => Check::ok(
            "Config",
            format!("issuer {}, bind {}", config.issuer, config.listen_addr()),
        ),
        Err(message) => Check::fail(
            "Config",
            message.clone(),
            vec!["lanyard serve would refuse to start in this environment.".into()],
        ),
    }
}

/// The second check. **Everything below it needs a running server**, so a
/// `FAIL` here skips the rest rather than guessing at them — and it names the
/// exact address it tried, because "connection refused" without an address is
/// the failure this whole phase exists to delete.
pub fn reachable_check(url: &str, reached: &Result<(Health, Duration), String>) -> Check {
    match reached {
        Ok((health, took)) => Check::ok(
            "Reachable",
            format!(
                "{url} answered in {}ms (lanyard {})",
                took.as_millis(),
                health.version
            ),
        ),
        Err(message) => Check::fail(
            "Reachable",
            format!("{url} — {message}"),
            vec![
                format!("Nothing answered at {url}, so every check below it was skipped."),
                "Start it with `lanyard serve`, or point doctor elsewhere with LANYARD_URL.".into(),
            ],
        ),
    }
}

/// `GET {url}/_/health`, timed.
///
/// **The round trip is measured, not just the answer**, because half of it is
/// what the Clock check subtracts before comparing two clocks. Errors come back
/// as prose rather than as a `reqwest::Error` so the report reads as sentences.
pub async fn fetch_health(url: &str) -> Result<(Health, Duration), String> {
    let started = std::time::Instant::now();
    let response = client()
        .get(format!("{url}/_/health"))
        // Long enough for a container that is still starting, short enough that
        // a `HEALTHCHECK` interval is not spent waiting on it.
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(short)?;
    let took = started.elapsed();

    if !response.status().is_success() {
        return Err(format!("answered {}", response.status()));
    }
    let health = response.json::<Health>().await.map_err(short)?;
    Ok((health, took))
}

/// A `reqwest::Error`'s own `Display` is a sentence about a URL the report has
/// already printed, wrapped around the thing a reader actually wants.
fn short(error: reqwest::Error) -> String {
    let mut source: &dyn std::error::Error = &error;
    while let Some(inner) = source.source() {
        source = inner;
    }
    source.to_string()
}

/// The whole report: the checks that need nothing, then the ones that need a
/// server, in the order a request travels.
///
/// `config` is passed in rather than resolved here so the caller can hand over
/// the same `Result` `serve` would have got.
pub async fn run(url: &str, config: &Result<Config, String>) -> Report {
    let mut report = Report::default();
    report.push(config_check(config));

    // **`doctor` reads the true clock, always**, and both ends of the round
    // trip come off it — so the two numbers that get subtracted have one
    // source.
    let before = crate::clock::Clock::real().now_millis();
    let reached = fetch_health(url).await;
    let after = crate::clock::Clock::real().now_millis();
    report.push(reachable_check(url, &reached));

    // The file, and nothing else — so this reports whether or not anything is
    // listening. Criterion 6 asks for exactly that.
    let key_facts = config
        .as_ref()
        .ok()
        .map(|config| inspect_key(&config.data_dir));

    // **Nothing below reports a value it could not have measured.** With
    // nothing listening there is no discovery document to compare and no key to
    // fetch, and a check that guessed would be worse than a check that is
    // absent.
    let Ok((health, _took)) = reached else {
        if let Some(facts) = &key_facts {
            report.push(key_check(facts, None));
        }
        return report;
    };

    let discovery =
        fetch_json::<Discovery>(&format!("{url}/oidc/.well-known/openid-configuration")).await;
    report.push(issuer_check(url, &discovery, &health.hosts_seen));

    if let Ok(discovery) = &discovery {
        // **As advertised, not as configured.** Dialling the configured
        // address would test nothing: the interesting failure is that the name
        // in the issuer does not resolve from where a relying party stands.
        let jwks = fetch_json::<Jwks>(&discovery.jwks_uri).await;
        report.push(jwks_check(
            &discovery.jwks_uri,
            &jwks,
            Runtime::detect(),
            &health.kid,
        ));
    }

    // The server read its clock somewhere in the middle of the round trip, so
    // half of it comes back off. No NTP and no third party: the only comparison
    // that needs no network beyond the one already in use.
    let midpoint = before + (after - before) / 2;
    report.push(clock_check(
        health.now - midpoint,
        health.skew,
        std::env::var(crate::clock::SKEW_VAR).ok().as_deref(),
    ));

    if let Some(facts) = &key_facts {
        report.push(key_check(facts, Some(&health.kid)));
    }

    report
}

/// **Named, so the server does not mistake a diagnostic for an application.**
/// See [`crate::hosts::DOCTOR_USER_AGENT`].
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(crate::hosts::DOCTOR_USER_AGENT)
        .build()
        // A client with no TLS backend and a static user agent has nothing to
        // fail on, but a `doctor` that panicked on its own construction would
        // be the worst possible bug in a diagnostic.
        .unwrap_or_default()
}

async fn fetch_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let response = client()
        .get(url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(short)?;
    if !response.status().is_success() {
        return Err(format!("answered {}", response.status()));
    }
    response.json::<T>().await.map_err(short)
}

/// The two fields of the discovery document `doctor` compares. Deliberately
/// not the whole document: this check is about the issuer and the key location,
/// and a struct that mirrored every field would fail to parse the day one is
/// added.
#[derive(Debug, Clone, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub jwks_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Jwks {
    pub keys: Vec<Jwk>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Jwk {
    pub kid: String,
}

/// The third check: **does the running server agree with the address I
/// dialled?**
///
/// This is one half of CONCEPT §8's headline gotcha reported to a person rather
/// than to a log — the other half is [`crate::hosts`] reporting it at request
/// time. `-p 9500:8080` is not a separate detector: `doctor` dialled `:9500`,
/// the discovery document says `:8080`, and that is the same fact.
///
/// A mismatch is a `WARN`, never a `FAIL`: `LANYARD_ISSUER=http://lanyard:9500/oidc`
/// looks wrong from the host shell and is exactly right for the container
/// network.
pub fn issuer_check(
    dialled: &str,
    discovery: &Result<Discovery, String>,
    hosts_seen: &[String],
) -> Check {
    let discovery = match discovery {
        Ok(discovery) => discovery,
        Err(message) => {
            return Check::warn(
                "Issuer",
                format!("could not read the discovery document — {message}"),
                vec!["Nothing that reads it can configure itself against this lanyard.".into()],
            )
        }
    };

    // **The same fact from the other side**, so it belongs on the same line:
    // `doctor` dialled one address; the server has been dialled by others.
    // Reported even when `doctor`'s own address agrees, because a name that
    // reached the server and is not the issuer's is still a token that will be
    // rejected — just not one of `doctor`'s.
    let also_reached = |mut consequence: Vec<String>| {
        if !hosts_seen.is_empty() {
            consequence.push(format!(
                "That server has also been reached as {} — tokens minted for those \
                 callers still say iss={}.",
                hosts_seen.join(", "),
                discovery.issuer,
            ));
        }
        consequence
    };

    // **A note, and never a level.** See [`silent_renew_note`]: `doctor` cannot
    // know what origin a browser app is served from, so appending it must not
    // turn the shipped default into a `WARN` at every developer who is not
    // building a SPA.
    let note = silent_renew_note(&discovery.issuer);

    let here = crate::hosts::authority_of(dialled).unwrap_or(dialled);
    let there = crate::hosts::authority_of(&discovery.issuer).unwrap_or(&discovery.issuer);
    if here.eq_ignore_ascii_case(there) {
        let agreed = "the running server agrees with the address I dialled";
        let consequence = also_reached(Vec::new());
        let mut check = if consequence.is_empty() {
            Check::ok("Issuer", agreed)
        } else {
            Check::warn(
                "Issuer",
                format!("{agreed}, but others do not"),
                consequence,
            )
        };
        check.consequence.extend(note);
        return check;
    }

    let mut consequence = vec![format!(
        "A token minted here says iss={}, and a relying party that dialled {here} \
         compares that string and rejects it.",
        discovery.issuer
    )];
    // Named separately, because a one-character difference in a `-p` flag is
    // the hardest of these to see.
    if port_of(here) != port_of(there) {
        consequence.push(format!(
            "The port is inside the issuer: published as {}, but the issuer says {}.",
            port_of(here).unwrap_or("(none)"),
            port_of(there).unwrap_or("(none)"),
        ));
    }
    consequence.push(format!(
        "If {here} is the name everything should use: LANYARD_ISSUER={} lanyard serve",
        discovery.issuer.replacen(there, here, 1)
    ));

    let mut consequence = also_reached(consequence);
    consequence.extend(note);
    Check::warn(
        "Issuer",
        format!("the server says {there}, I dialled {here}"),
        consequence,
    )
}

/// **What a loopback-IP issuer costs a browser app, said once, as a note.**
///
/// `SameSite` compares scheme and host and **ignores the port**, so an app
/// served from `http://localhost:5173` is *same-site* to an issuer on
/// `http://localhost:9500` and *cross-site* to one on `http://127.0.0.1:9500`
/// — and lanyard's `Lax` session cookie does not ride a cross-site iframe
/// navigation. A hidden-iframe silent renew (`prompt=none`) therefore gets
/// `login_required` on the shipped default, which is the row measured in
/// `docs/decisions/silent-renew-over-http.md`.
///
/// **A note rather than a check**, because `doctor` knows the issuer and cannot
/// know the app's origin: an app also served from `127.0.0.1` is same-site to
/// this issuer and perfectly fine, and a project with no browser app at all is
/// unaffected. It never changes the level for the same reason. Phase 8
/// established that `doctor` is where this class of naming problem gets
/// explained; this is the same fact one layer out.
fn silent_renew_note(issuer: &str) -> Option<String> {
    let url = url::Url::parse(issuer).ok()?;
    // The *literal* loopback addresses only. A `Domain` host is `localhost`, or
    // a container service name, or something else nobody here can reason about
    // — and `localhost` is the row that works.
    let host = match url.host()? {
        url::Host::Ipv4(v4) if v4.is_loopback() => v4.to_string(),
        url::Host::Ipv6(v6) if v6.is_loopback() => v6.to_string(),
        _ => return None,
    };
    let mut fixed = url.clone();
    fixed.set_host(Some("localhost")).ok()?;
    Some(format!(
        "The issuer host is {host}; a browser app served from http://localhost is \
         cross-site to it — SameSite compares the host and ignores the port — so a \
         hidden-iframe silent renew (prompt=none) will not be sent lanyard's session \
         cookie. If that app is yours: LANYARD_ISSUER={fixed} lanyard serve",
        fixed = fixed.as_str().trim_end_matches('/'),
    ))
}

fn port_of(authority: &str) -> Option<&str> {
    authority.rsplit_once(':').map(|(_, port)| port)
}

/// The fourth check: fetch `jwks_uri` **as advertised**, not as configured.
///
/// Fetching the configured address would test nothing: the interesting failure
/// is precisely that the name in the issuer does not resolve from where the
/// relying party is standing, and the only way to find that out is to dial the
/// string the document actually hands out.
pub fn jwks_check(
    jwks_uri: &str,
    fetched: &Result<Jwks, String>,
    runtime: Runtime,
    live_kid: &str,
) -> Check {
    match fetched {
        Ok(jwks) => {
            let count = jwks.keys.len();
            let kids: Vec<&str> = jwks.keys.iter().map(|k| k.kid.as_str()).collect();
            let detail = format!(
                "{jwks_uri}, {count} key{}, kid {}",
                if count == 1 { "" } else { "s" },
                kids.first().map(|k| abbreviate(k)).unwrap_or_default(),
            );
            if !kids.contains(&live_kid) {
                return Check::warn(
                    "JWKS",
                    detail,
                    vec![format!(
                        "This is not the key the server at the address I dialled is signing \
                         with ({}). Two lanyards are answering on one name.",
                        abbreviate(live_kid)
                    )],
                );
            }
            Check::ok("JWKS", detail)
        }
        Err(message) if unresolvable(message) => {
            let host = crate::hosts::authority_of(jwks_uri)
                .and_then(|a| a.split(':').next())
                .unwrap_or("the issuer's host");
            Check::warn(
                "JWKS",
                format!("{jwks_uri} — {message}"),
                vec![
                    format!(
                        "\"{host}\" does not resolve from here, so nothing that reads this \
                         discovery document can fetch the keys."
                    ),
                    format!("Make the one name resolve on both sides:  echo '127.0.0.1 {host}' | sudo tee -a /etc/hosts"),
                    format!("From inside a container, this machine is {}.", runtime.host_alias()),
                ],
            )
        }
        Err(message) => Check::warn(
            "JWKS",
            format!("{jwks_uri} — {message}"),
            vec!["A relying party fetches this address, not the one you configured.".into()],
        ),
    }
}

/// Whether a fetch failed because the **name** did not resolve, as opposed to
/// nothing listening at an address that did.
///
/// Matched on the message rather than on an error kind because `reqwest`'s DNS
/// failure arrives as an opaque boxed source; the strings below are what hyper
/// and the resolver actually produce.
fn unresolvable(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "dns",
        "resolve",
        "lookup",
        "name or service not known",
        "nodename",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// A `kid` is a 43-character base64url thumbprint, and a report is meant to be
/// read. Enough to tell two apart, in the same shape the spec's example uses.
fn abbreviate(kid: &str) -> String {
    match kid.char_indices().nth(8) {
        Some((cut, _)) => format!("{}…", &kid[..cut]),
        None => kid.to_string(),
    }
}

/// Above this, the clocks are reported as disagreeing. Two seconds is chosen
/// against a 60-second token: below it nothing breaks, and above it the
/// arithmetic in the consequence sentence starts to matter.
pub const CLOCK_TOLERANCE_MS: i64 = 2_000;

/// The fifth check: **two lanyard clocks, and no third party.**
///
/// `offset_ms` is the server's `now` minus this process's, already corrected
/// for half the round trip. `doctor_skew_var` is `LANYARD_CLOCK_SKEW` as it
/// appears in *`doctor`'s own* environment — which `doctor` deliberately does
/// not apply. A developer with it exported, running a skewed `serve` and a
/// skewed `doctor`, would otherwise be told the clocks agree, which is the one
/// lie this whole phase exists to prevent.
pub fn clock_check(offset_ms: i64, server_skew: i64, doctor_skew_var: Option<&str>) -> Check {
    let mut consequence = Vec::new();
    if server_skew != 0 {
        consequence.push(format!(
            "That server is running with {}={} on purpose.",
            crate::clock::SKEW_VAR,
            crate::clock::format_offset(server_skew)
        ));
    }
    if let Some(raw) = doctor_skew_var {
        consequence.push(format!(
            "{}={raw} is set here too; doctor ignores it and reads the true clock.",
            crate::clock::SKEW_VAR
        ));
    }

    if offset_ms.abs() <= CLOCK_TOLERANCE_MS {
        // Sub-second, because "0s apart" and "0.9s apart" are different
        // answers to "is my clock the problem".
        let mut check = Check::ok("Clock", format!("{} apart", precise(offset_ms)));
        check.consequence = consequence;
        return check;
    }

    // Rounded, not truncated: five minutes of skew measured over a round trip
    // is 299,9xx ms, and reporting it as `4m59s` next to a
    // `LANYARD_CLOCK_SKEW=-5m0s` the same report prints is a contradiction a
    // reader has to stop and resolve.
    let seconds = (offset_ms.unsigned_abs() + 500) / 1_000;
    let direction = if offset_ms < 0 { "behind" } else { "ahead of" };
    // **The consequence, not the number.** An access token lives 60 seconds, so
    // any disagreement larger than that makes every token dead on arrival.
    consequence.insert(
        0,
        if seconds >= crate::oidc::issue::DEFAULT_TTL {
            format!(
                "An access token lives {}s, so one minted there is already {} expired \
                 by this clock — every call fails with a 401 that looks like anything else.",
                crate::oidc::issue::DEFAULT_TTL,
                crate::clock::format_duration(seconds - crate::oidc::issue::DEFAULT_TTL)
            )
        } else {
            format!(
                "An access token lives {}s, so this eats {} of it.",
                crate::oidc::issue::DEFAULT_TTL,
                crate::clock::format_duration(seconds)
            )
        },
    );

    Check::warn(
        "Clock",
        format!(
            "that server's clock is {} {direction} this one",
            crate::clock::format_duration(seconds)
        ),
        consequence,
    )
}

/// Milliseconds as seconds, to three places — the precision the spec's example
/// report prints, and the precision "is my clock the problem" needs.
fn precise(ms: i64) -> String {
    format!("{:.3}s", ms as f64 / 1_000.0)
}

/// What can be learned about the signing key **from the file**, with no server
/// involved — which is why this check still reports when nothing is listening.
#[derive(Debug, Clone)]
pub struct KeyFacts {
    pub path: std::path::PathBuf,
    /// `None` when the file is not there yet, which is the ordinary state of a
    /// machine that has never run `lanyard serve`.
    pub mode: Option<u32>,
    pub kid: Option<String>,
    pub is_default: bool,
    pub error: Option<String>,
}

/// Read the key file without creating it. **`doctor` never writes**: a
/// diagnostic that bootstrapped a data directory would change the thing it was
/// asked to describe.
pub fn inspect_key(data_dir: &std::path::Path) -> KeyFacts {
    use std::os::unix::fs::PermissionsExt as _;

    let path = crate::keys::key_path(data_dir);
    let Ok(pem) = std::fs::read_to_string(&path) else {
        return KeyFacts {
            path,
            mode: None,
            kid: None,
            // Not yet written, so the key that *will* be used is the built-in
            // one — which is the answer to "will a fresh container serve the
            // same kid".
            is_default: true,
            error: None,
        };
    };
    let mode = std::fs::metadata(&path)
        .ok()
        .map(|m| m.permissions().mode() & 0o777);
    match crate::keys::SigningKey::from_pem(&pem) {
        Ok(key) => {
            let default = crate::keys::SigningKey::from_pem(crate::keys::DEFAULT_DEV_KEY_PEM)
                .map(|d| d.kid().to_string())
                .ok();
            KeyFacts {
                path,
                mode,
                is_default: default.as_deref() == Some(key.kid()),
                kid: Some(key.kid().to_string()),
                error: None,
            }
        }
        Err(message) => KeyFacts {
            path,
            mode,
            kid: None,
            is_default: false,
            error: Some(message),
        },
    }
}

/// The sixth check: the file, its mode, whether it is the built-in default, and
/// whether its `kid` is the one being served.
///
/// `live_kid` is `None` when nothing answered — the comparison is skipped
/// rather than guessed at.
pub fn key_check(facts: &KeyFacts, live_kid: Option<&str>) -> Check {
    let path = facts.path.display();

    if let Some(message) = &facts.error {
        return Check::fail(
            "Signing key",
            format!("{path} — {message}"),
            vec!["lanyard serve refuses to start on a key file it cannot read.".into()],
        );
    }

    let Some(kid) = &facts.kid else {
        return Check::ok(
            "Signing key",
            format!(
                "{path} — not written yet; the built-in default key will be, \
                 and it is the same key on every machine"
            ),
        );
    };

    let mode = facts
        .mode
        .map(|m| format!("{m:04o}"))
        .unwrap_or_else(|| "unknown mode".into());
    let which = if facts.is_default {
        "the built-in default key — stable across a wiped data dir"
    } else {
        "a key of your own"
    };
    let detail = format!("{path}, {mode}, {which}");

    let mut consequence = Vec::new();
    if facts.mode.is_some_and(|m| m != 0o600) {
        consequence.push(format!(
            "A private key at {mode} is readable by more than you; lanyard writes 0600."
        ));
    }
    // **The volume rule, stated where it is actionable.** No volume is needed
    // for the default key; one is needed for a key you replaced.
    if !facts.is_default {
        consequence.push(
            "A container with no volume will not have this key — it will serve the \
             built-in default instead."
                .into(),
        );
    }
    if let Some(live) = live_kid {
        if live != kid {
            consequence.push(format!(
                "The server I dialled is signing with {}, not this file's {} — \
                 either it is another machine, or it was started with another data dir.",
                abbreviate(live),
                abbreviate(kid),
            ));
        }
    }

    if consequence.is_empty() {
        Check::ok("Signing key", detail)
    } else {
        Check::warn("Signing key", detail, consequence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(checks: Vec<Check>) -> Report {
        Report { checks }
    }

    /// The shape a developer pastes into an issue: one line per check, the
    /// level in its own column, the detail after it.
    #[test]
    fn a_report_is_one_aligned_line_per_check() {
        let out = report(vec![
            Check::ok(
                "Config",
                "issuer http://127.0.0.1:9500/oidc, bind 127.0.0.1:9500",
            ),
            Check::ok(
                "Signing key",
                "/home/you/.local/share/lanyard/signing-key.pem, 0600",
            ),
        ])
        .render();

        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "lanyard doctor");
        assert!(
            lines[1].starts_with("  Config       OK    issuer"),
            "{:?}",
            lines[1]
        );
        // The longest name still leaves the level column where it was.
        let column_of = |line: &str, needle: &str| line.find(needle).unwrap();
        assert_eq!(column_of(lines[1], "OK"), column_of(lines[2], "OK"));
    }

    /// **The pairing is the feature.** Anything not OK is followed by an
    /// indented consequence, aligned under the detail it explains.
    #[test]
    fn a_warning_carries_its_consequence_indented_under_the_detail() {
        let out = report(vec![Check::warn(
            "Issuer",
            "the running server says :8080, I dialled :9500",
            vec!["a relying party dialling :9500 will reject it".into()],
        )])
        .render();

        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[1].contains("WARN"), "{:?}", lines[1]);
        let detail_column = lines[1].find("the running server").unwrap();
        let consequence = lines[2];
        assert_eq!(
            consequence.find("a relying party").unwrap(),
            detail_column,
            "aligned under the detail: {consequence:?}"
        );
    }

    /// A warning may be deliberate — `LANYARD_ISSUER=http://lanyard:9500/oidc`
    /// looks like a mismatch from the host shell and is exactly right for the
    /// container network — so it must not stop a container going healthy.
    #[test]
    fn only_a_fail_exits_non_zero() {
        assert!(!report(vec![Check::ok("Config", "fine")]).failed(false));
        assert!(!report(vec![Check::warn("Issuer", "odd", vec![])]).failed(false));
        assert!(report(vec![Check::fail("Reachable", "nothing there", vec![])]).failed(false));
    }

    fn health() -> Health {
        Health {
            status: "ok".into(),
            version: "0.1.0".into(),
            issuer: "http://127.0.0.1:9500/oidc".into(),
            kid: "TXntCt2biz2Bj578hZocZOb2A2nQV9JfrBvFN55QWpU".into(),
            now: 1_700_000_000_000,
            skew: 0,
            hosts_seen: Vec::new(),
        }
    }

    fn config() -> Config {
        Config::resolve(|key| match key {
            "HOME" => Some("/home/dev".to_string()),
            _ => None,
        })
        .unwrap()
    }

    #[test]
    fn config_names_the_issuer_and_the_bind_address() {
        let check = config_check(&Ok(config()));
        assert_eq!(check.level, Level::Ok);
        assert!(
            check.detail.contains("http://127.0.0.1:9500/oidc"),
            "{}",
            check.detail
        );
        assert!(check.detail.contains("127.0.0.1:9500"), "{}", check.detail);
    }

    /// A `FAIL` here is a promise about `serve`, because it is the same
    /// resolution `serve` performs.
    #[test]
    fn a_config_serve_would_refuse_is_a_failure_that_quotes_it() {
        let check = config_check(&Err("LANYARD_PORT is not a port number: nine".into()));
        assert_eq!(check.level, Level::Fail);
        assert!(check.detail.contains("LANYARD_PORT"), "{}", check.detail);
        assert!(
            check
                .consequence
                .iter()
                .any(|c| c.contains("refuse to start")),
            "{:?}",
            check.consequence
        );
    }

    #[test]
    fn reachable_names_the_address_the_time_and_the_version() {
        let check = reachable_check(
            "http://127.0.0.1:9500",
            &Ok((health(), Duration::from_millis(2))),
        );
        assert_eq!(check.level, Level::Ok);
        assert!(
            check.detail.contains("http://127.0.0.1:9500"),
            "{}",
            check.detail
        );
        assert!(check.detail.contains("2ms"), "{}", check.detail);
        assert!(check.detail.contains("lanyard 0.1.0"), "{}", check.detail);
    }

    /// Criterion 6. "Connection refused" without an address is the failure this
    /// whole phase exists to delete.
    #[test]
    fn nothing_listening_names_the_exact_address_it_tried() {
        let check = reachable_check("http://localhost:9500", &Err("connection refused".into()));
        assert_eq!(check.level, Level::Fail);
        assert!(
            check.detail.contains("http://localhost:9500"),
            "{}",
            check.detail
        );
        assert!(
            check
                .consequence
                .iter()
                .any(|c| c.contains("http://localhost:9500")),
            "the consequence names it too: {:?}",
            check.consequence
        );
        assert!(
            check.consequence.iter().any(|c| c.contains("LANYARD_URL")),
            "{:?}",
            check.consequence
        );
    }

    fn discovery(issuer: &str) -> Discovery {
        Discovery {
            issuer: issuer.to_string(),
            jwks_uri: format!("{issuer}/jwks"),
        }
    }

    const LIVE_KID: &str = "TXntCt2biz2Bj578hZocZOb2A2nQV9JfrBvFN55QWpU";

    fn jwks(kid: &str) -> Jwks {
        Jwks {
            keys: vec![Jwk {
                kid: kid.to_string(),
            }],
        }
    }

    #[test]
    fn an_issuer_that_agrees_with_the_address_dialled_is_ok() {
        let check = issuer_check(
            "http://localhost:9500",
            &Ok(discovery("http://localhost:9500/oidc")),
            &[],
        );
        assert_eq!(check.level, Level::Ok);
        assert!(check.consequence.is_empty());
    }

    /// **Phase 9's note.** `doctor` knows the issuer and cannot know what origin
    /// a browser app is served from, so this states a consequence rather than
    /// reaching a verdict — and it must not change the level, or the shipped
    /// default would print a `WARN` at every developer for a configuration that
    /// is only wrong if they happen to be building a SPA.
    ///
    /// Measured before it was written down:
    /// `docs/decisions/silent-renew-over-http.md`.
    #[test]
    fn a_loopback_ip_issuer_notes_what_it_costs_a_silent_renew() {
        let check = issuer_check(
            "http://127.0.0.1:9500",
            &Ok(discovery("http://127.0.0.1:9500/oidc")),
            &[],
        );
        assert_eq!(check.level, Level::Ok, "a note is not a verdict");

        let said = check.consequence.join(" ");
        assert!(said.contains("127.0.0.1"), "{said}");
        assert!(said.contains("localhost"), "{said}");
        assert!(said.contains("cross-site"), "{said}");
        assert!(said.contains("prompt=none"), "{said}");
        assert!(
            said.contains("LANYARD_ISSUER=http://localhost:9500/oidc"),
            "the fix, with this issuer's own port: {said}"
        );
    }

    /// And the row that works says nothing, because there is nothing to say.
    #[test]
    fn a_localhost_issuer_gets_no_silent_renew_note() {
        for issuer in ["http://localhost:9500/oidc", "http://lanyard:9500/oidc"] {
            let check = issuer_check("http://localhost:9500", &Ok(discovery(issuer)), &[]);
            assert!(
                !check.consequence.join(" ").contains("prompt=none"),
                "{issuer}: {:?}",
                check.consequence
            );
        }
    }

    /// The note rides on a mismatch too — a developer whose ports disagree is
    /// no less likely to be running a SPA.
    #[test]
    fn the_silent_renew_note_survives_an_issuer_mismatch() {
        let check = issuer_check(
            "http://localhost:9500",
            &Ok(discovery("http://127.0.0.1:8080/oidc")),
            &[],
        );
        assert_eq!(check.level, Level::Warn);
        let said = check.consequence.join(" ");
        assert!(
            said.contains("rejects it"),
            "the mismatch is still first: {said}"
        );
        assert!(
            said.contains("prompt=none"),
            "and the note is still there: {said}"
        );
    }

    /// Criterion 8. `-p 9500:8080` is not a separate detector: `doctor` dialled
    /// `:9500` and the document says `:8080`, and the consequence carries the
    /// `iss` that will actually be minted.
    #[test]
    fn a_published_port_that_is_not_the_issuers_port_warns_and_names_both() {
        let check = issuer_check(
            "http://localhost:9500",
            &Ok(discovery("http://127.0.0.1:8080/oidc")),
            &[],
        );

        assert_eq!(
            check.level,
            Level::Warn,
            "deliberate is possible, so never FAIL"
        );
        assert!(check.detail.contains("8080"), "{}", check.detail);
        assert!(check.detail.contains("9500"), "{}", check.detail);

        let said = check.consequence.join(" ");
        assert!(
            said.contains("iss=http://127.0.0.1:8080/oidc"),
            "the value that will be minted: {said}"
        );
        assert!(said.contains("rejects it"), "{said}");
        assert!(
            said.contains("published as 9500, but the issuer says 8080"),
            "both ports, named: {said}"
        );
        assert!(
            said.contains("LANYARD_ISSUER=http://localhost:9500/oidc"),
            "the fix keeps the path: {said}"
        );
    }

    /// The host case. One rule, and the fix keeps the scheme and the path.
    #[test]
    fn a_host_that_is_not_the_issuers_host_warns() {
        let check = issuer_check(
            "http://127.0.0.1:9500",
            &Ok(discovery("http://lanyard:9500/oidc")),
            &[],
        );
        assert_eq!(check.level, Level::Warn);
        let said = check.consequence.join(" ");
        assert!(
            !said.contains("The port is inside the issuer"),
            "the ports agree; do not say they do not: {said}"
        );
        assert!(
            said.contains("LANYARD_ISSUER=http://127.0.0.1:9500/oidc"),
            "{said}"
        );
    }

    #[test]
    fn a_jwks_that_serves_the_live_key_is_ok_and_names_the_address_it_used() {
        let check = jwks_check(
            "http://127.0.0.1:9500/oidc/jwks",
            &Ok(jwks(LIVE_KID)),
            Runtime::Host,
            LIVE_KID,
        );
        assert_eq!(check.level, Level::Ok);
        assert!(
            check.detail.contains("http://127.0.0.1:9500/oidc/jwks"),
            "{}",
            check.detail
        );
        assert!(check.detail.contains("1 key"), "{}", check.detail);
        assert!(check.detail.contains("TXntCt2b…"), "{}", check.detail);
    }

    /// Criterion 9. The name in the issuer does not resolve from here — which
    /// is only findable by dialling the address the document hands out.
    #[test]
    fn an_unresolvable_issuer_host_prints_the_etc_hosts_line_verbatim() {
        let check = jwks_check(
            "http://lanyard:9500/oidc/jwks",
            &Err("failed to lookup address information: Name or service not known".into()),
            Runtime::Host,
            LIVE_KID,
        );

        assert_eq!(check.level, Level::Warn);
        let said = check.consequence.join("\n");
        assert!(
            said.contains("echo '127.0.0.1 lanyard' | sudo tee -a /etc/hosts"),
            "verbatim, to paste: {said}"
        );
        assert!(said.contains("does not resolve from here"), "{said}");
    }

    /// Podman's name for the host is not Docker's, and CONCEPT §8 names only
    /// the Docker spelling. Inside a runtime, `doctor` prints that runtime's.
    #[test]
    fn the_host_alias_matches_the_runtime_detected() {
        let podman = jwks_check(
            "http://lanyard:9500/oidc/jwks",
            &Err("dns error".into()),
            Runtime::Podman,
            LIVE_KID,
        );
        assert!(podman
            .consequence
            .join(" ")
            .contains("host.containers.internal"));
        assert!(!podman
            .consequence
            .join(" ")
            .contains("host.docker.internal"));

        let docker = jwks_check(
            "http://lanyard:9500/oidc/jwks",
            &Err("dns error".into()),
            Runtime::Docker,
            LIVE_KID,
        );
        assert!(docker
            .consequence
            .join(" ")
            .contains("host.docker.internal"));

        // On the host, neither is a fact about this machine — so both are
        // offered as suggestions about the other side.
        let host = jwks_check(
            "http://lanyard:9500/oidc/jwks",
            &Err("dns error".into()),
            Runtime::Host,
            LIVE_KID,
        );
        assert!(host
            .consequence
            .join(" ")
            .contains("host.containers.internal"));
        assert!(host.consequence.join(" ").contains("host.docker.internal"));
    }

    /// **Two lanyards answering on one name** is the failure a `kid` comparison
    /// exists to catch, and it is invisible any other way.
    #[test]
    fn a_jwks_serving_someone_elses_key_warns_and_says_which() {
        let check = jwks_check(
            "http://lanyard:9500/oidc/jwks",
            &Ok(jwks("SOMEONEELSEabcdefghijklmnopqrstuvwxyz0123")),
            Runtime::Host,
            LIVE_KID,
        );
        assert_eq!(check.level, Level::Warn);
        assert!(
            check.consequence.join(" ").contains("TXntCt2b…"),
            "{:?}",
            check.consequence
        );
    }

    /// A name that resolves to a machine with nothing on it is a different bug
    /// from a name that does not resolve, and gets a different sentence.
    #[test]
    fn a_reachable_but_dead_jwks_does_not_suggest_etc_hosts() {
        let check = jwks_check(
            "http://127.0.0.1:9500/oidc/jwks",
            &Err("Connection refused (os error 111)".into()),
            Runtime::Host,
            LIVE_KID,
        );
        assert_eq!(check.level, Level::Warn);
        assert!(
            !check.consequence.join(" ").contains("/etc/hosts"),
            "{:?}",
            check.consequence
        );
    }

    // ------------------------------------------------------- the clock --

    #[test]
    fn clocks_that_agree_report_the_difference_to_the_millisecond() {
        let check = clock_check(1, 0, None);
        assert_eq!(check.level, Level::Ok);
        assert_eq!(check.detail, "0.001s apart");
        assert!(check.consequence.is_empty());
    }

    #[test]
    fn a_second_of_difference_is_still_ok() {
        assert_eq!(clock_check(1_500, 0, None).level, Level::Ok);
        assert_eq!(clock_check(-1_500, 0, None).level, Level::Ok);
    }

    /// Criterion 13. **The consequence, spelled out**: a 60-second token minted
    /// five minutes behind is already four minutes dead here, and the `401` it
    /// earns looks like anything else.
    #[test]
    fn five_minutes_of_skew_warns_and_does_the_sixty_second_arithmetic() {
        let check = clock_check(-300_000, 0, None);
        assert_eq!(check.level, Level::Warn);
        assert!(check.detail.contains("5m0s"), "{}", check.detail);
        assert!(check.detail.contains("behind"), "{}", check.detail);
        let said = check.consequence.join(" ");
        assert!(said.contains("60s"), "{said}");
        assert!(said.contains("4m0s expired"), "{said}");
    }

    /// A disagreement smaller than the token's life eats into it rather than
    /// killing it, and says so.
    /// `4m59s` next to a `LANYARD_CLOCK_SKEW=-5m0s` the same report prints is a
    /// contradiction a reader has to stop and resolve.
    #[test]
    fn a_round_trip_worth_of_milliseconds_does_not_round_five_minutes_down() {
        let check = clock_check(-299_940, -300, None);
        assert!(check.detail.contains("5m0s"), "{}", check.detail);
        assert!(
            check.consequence.join(" ").contains("4m0s expired"),
            "{:?}",
            check.consequence
        );
    }

    #[test]
    fn a_disagreement_shorter_than_the_token_says_what_it_costs() {
        let said = clock_check(30_000, 0, None).consequence.join(" ");
        assert!(said.contains("eats 30s"), "{said}");
    }

    /// **The one lie this phase exists to prevent.** A developer with the
    /// variable exported runs a skewed `serve` and would otherwise run a skewed
    /// `doctor` and be told the clocks agree.
    #[test]
    fn doctor_says_it_is_ignoring_the_skew_variable_in_its_own_environment() {
        let check = clock_check(1, 0, Some("-5m"));
        assert_eq!(check.level, Level::Ok, "ignoring it means the clocks agree");
        let said = check.consequence.join(" ");
        assert!(
            said.contains("LANYARD_CLOCK_SKEW=-5m is set here too"),
            "{said}"
        );
        assert!(said.contains("reads the true clock"), "{said}");
    }

    /// And it names the *server's* skew as a fact, because `/_/health` said so
    /// rather than because the difference implied it.
    #[test]
    fn a_server_running_skewed_is_reported_as_deliberate() {
        let said = clock_check(-300_000, -300, None).consequence.join(" ");
        assert!(
            said.contains("LANYARD_CLOCK_SKEW=-5m0s on purpose"),
            "{said}"
        );
    }

    // -------------------------------------------------- the signing key --

    fn facts(kid: Option<&str>, mode: Option<u32>, is_default: bool) -> KeyFacts {
        KeyFacts {
            path: "/home/you/.local/share/lanyard/signing-key.pem".into(),
            mode,
            kid: kid.map(str::to_string),
            is_default,
            error: None,
        }
    }

    #[test]
    fn the_default_key_at_0600_serving_the_live_kid_is_ok() {
        let check = key_check(&facts(Some(LIVE_KID), Some(0o600), true), Some(LIVE_KID));
        assert_eq!(check.level, Level::Ok);
        assert!(check.detail.contains("signing-key.pem"), "{}", check.detail);
        assert!(check.detail.contains("0600"), "{}", check.detail);
        assert!(
            check.detail.contains("stable across a wiped data dir"),
            "the answer to the ephemeral-keys gotcha: {}",
            check.detail
        );
    }

    /// **The volume rule, stated where it is actionable.** No volume is needed
    /// for the default key; one is needed for a key you replaced.
    #[test]
    fn a_key_of_your_own_warns_that_a_container_will_not_have_it() {
        let check = key_check(&facts(Some("MINEabcdefgh"), Some(0o600), false), None);
        assert_eq!(check.level, Level::Warn);
        assert!(
            check
                .consequence
                .join(" ")
                .contains("no volume will not have this key"),
            "{:?}",
            check.consequence
        );
    }

    #[test]
    fn a_world_readable_private_key_warns_about_its_mode() {
        let check = key_check(&facts(Some(LIVE_KID), Some(0o644), true), Some(LIVE_KID));
        assert_eq!(check.level, Level::Warn);
        assert!(
            check.consequence.join(" ").contains("0644"),
            "{:?}",
            check.consequence
        );
    }

    /// Two lanyards, one name — invisible any other way.
    #[test]
    fn a_file_whose_kid_is_not_the_one_being_served_warns_and_names_both() {
        let check = key_check(
            &facts(Some(LIVE_KID), Some(0o600), true),
            Some("SOMEONEELSEabcdef"),
        );
        assert_eq!(check.level, Level::Warn);
        let said = check.consequence.join(" ");
        assert!(said.contains("SOMEONEE…"), "{said}");
        assert!(said.contains("TXntCt2b…"), "{said}");
    }

    /// Criterion 6's shape: with nothing listening there is no live `kid`, so
    /// the comparison is skipped rather than guessed at.
    #[test]
    fn with_no_server_the_key_still_reports_and_compares_nothing() {
        let check = key_check(&facts(Some(LIVE_KID), Some(0o600), true), None);
        assert_eq!(check.level, Level::Ok);
        assert!(
            !check.consequence.join(" ").contains("signing with"),
            "{:?}",
            check.consequence
        );
    }

    /// A machine that has never run `lanyard serve` has no file, and the answer
    /// to "which key will it use" is still known.
    #[test]
    fn a_data_dir_that_does_not_exist_yet_names_the_key_that_will_be_written() {
        let dir = tempfile::tempdir().unwrap();
        let inspected = inspect_key(&dir.path().join("never-run"));
        assert!(inspected.kid.is_none());
        assert!(inspected.is_default);

        let check = key_check(&inspected, None);
        assert_eq!(check.level, Level::Ok);
        assert!(check.detail.contains("not written yet"), "{}", check.detail);
        assert!(
            !dir.path().join("never-run").exists(),
            "doctor never writes: a diagnostic that bootstrapped the thing it \
             was asked to describe would change the answer"
        );
    }

    /// The real file, read the way `serve` reads it.
    #[test]
    fn inspecting_a_real_data_dir_reports_the_mode_and_the_default_kid() {
        let dir = tempfile::tempdir().unwrap();
        let key = crate::keys::load_or_create(dir.path()).unwrap();

        let inspected = inspect_key(dir.path());
        assert_eq!(inspected.kid.as_deref(), Some(key.kid()));
        assert_eq!(inspected.mode, Some(0o600));
        assert!(inspected.is_default);
        assert!(inspected.error.is_none());
    }

    #[test]
    fn a_key_file_that_will_not_parse_fails_rather_than_warns() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(crate::keys::key_path(dir.path()), "not a key\n").unwrap();

        let check = key_check(&inspect_key(dir.path()), None);
        assert_eq!(
            check.level,
            Level::Fail,
            "lanyard serve refuses to start on this, so doctor may not call it a warning"
        );
    }

    // ------------------------------------------------- the hosts seen --

    /// Criterion 10's last clause. The address `doctor` used agrees, and the
    /// server has still been reached by names that do not.
    #[test]
    fn hosts_the_server_has_seen_are_reported_even_when_doctors_address_agrees() {
        let check = issuer_check(
            "http://127.0.0.1:9500",
            &Ok(discovery("http://127.0.0.1:9500/oidc")),
            &["lanyard:9500".to_string(), "other:9500".to_string()],
        );
        assert_eq!(check.level, Level::Warn);
        let said = check.consequence.join(" ");
        assert!(said.contains("lanyard:9500, other:9500"), "{said}");
        assert!(said.contains("iss=http://127.0.0.1:9500/oidc"), "{said}");
    }

    /// `--strict` is for CI, where "may be deliberate" is not good enough.
    #[test]
    fn strict_promotes_every_warning_to_a_failure() {
        assert!(report(vec![Check::warn("Issuer", "odd", vec![])]).failed(true));
        assert!(!report(vec![Check::ok("Config", "fine")]).failed(true));
    }
}
