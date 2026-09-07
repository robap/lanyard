//! **The inbound `Host` against the issuer's authority**, and the sticky set of
//! the ones that did not match.
//!
//! This one check covers two of CONCEPT §8's gotchas. `-p 9500:8080` is not a
//! separate detector: the container listens on 8080, the issuer says 8080, the
//! browser arrives with `Host: localhost:9500`, and the mismatch *is* the port.
//!
//! **Never the other way round.** Nothing here derives the issuer from a
//! request; CONCEPT §8 is explicit, and this module exists precisely so that
//! nobody is tempted to paper the mismatch over.

use std::sync::Mutex;

/// How `lanyard doctor` names itself, so the server can tell lanyard's own
/// diagnostics apart from an application arriving by a name.
///
/// **`doctor` must not change what it observes.** It dials the address it was
/// pointed at *on purpose*, and when that address is not the issuer's it says
/// so — to a person, with the fix. Counting the same fact a second time in the
/// server's sighting set would mean the container image's `HEALTHCHECK`, which
/// is `lanyard doctor --quiet` every ten seconds, left a permanent warning
/// about itself on `/_/` and in `/_/health`.
///
/// Spoofable, and that is fine: this is a loopback development tool, and the
/// worst a forged header buys is a warning nobody sees.
pub const DOCTOR_USER_AGENT: &str = concat!("lanyard-doctor/", env!("CARGO_PKG_VERSION"));

/// How many distinct mismatching hosts are worth remembering.
///
/// The set is fed by a request header, so it is capped rather than trusted to
/// stay small on its own. In practice a machine is reached by two or three
/// names; a process that has seen thirty-two has already said everything useful
/// it has to say.
const MAX_SIGHTINGS: usize = 32;

/// The authority — host and port — of a URL, without parsing it as one.
///
/// `url::Url` is already a dependency and would do this properly, but every
/// caller here has a string that came out of [`crate::config::Config`] and is
/// known to be `scheme://authority/path`. Returning `None` on anything else is
/// what makes an unparseable issuer a *skipped* check rather than a panic.
pub fn authority_of(url: &str) -> Option<&str> {
    let after_scheme = url.split_once("://")?.1;
    Some(match after_scheme.find('/') {
        Some(slash) => &after_scheme[..slash],
        None => after_scheme,
    })
}

/// Whether a request that arrived as `host` agrees with `issuer`.
///
/// **One rule, no severity tiers.** `localhost:9500` against an issuer of
/// `127.0.0.1:9500` is a mismatch and gets the warning, because a conforming
/// relying party compares `iss` as a string and that one genuinely breaks. The
/// alternative — a loopback-equivalence exemption — is a special case that has
/// to be explained in the README, and it hides the mismatch that produces
/// `IDX10205` in .NET, which is the single most common way this fails.
///
/// An issuer whose authority cannot be read matches everything: a check that
/// cannot be made is not a check that failed.
pub fn matches(issuer: &str, host: &str) -> bool {
    match authority_of(issuer) {
        Some(authority) => authority.eq_ignore_ascii_case(host),
        None => true,
    }
}

/// The sentence a mismatch earns: both authorities, the `iss` that will
/// actually be minted, and the one line that fixes it.
///
/// The fix is built by swapping the authority in the configured issuer, so it
/// keeps the scheme and the `/oidc` path the developer already has rather than
/// inventing a URL they then have to correct.
pub fn warning(issuer: &str, host: &str) -> String {
    let fix = match authority_of(issuer) {
        Some(authority) => issuer.replacen(authority, host, 1),
        None => issuer.to_string(),
    };
    format!(
        "reached as \"{host}\" but the issuer is \"{issuer}\" — a token minted here says \
         iss={issuer} and a relying party that dialled {host} will reject it. \
         Fix with: LANYARD_ISSUER={fix} lanyard serve"
    )
}

/// Every distinct mismatching `Host` this process has been reached by, in the
/// order it first saw them.
///
/// **The second piece of interior mutability on `AppState`**, after
/// [`crate::store::Stores`] — and like those, it dies with the process.
#[derive(Default)]
pub struct HostSightings {
    seen: Mutex<Vec<String>>,
}

impl HostSightings {
    /// Record a mismatching `host`, and say whether this is the **first**
    /// sighting of it.
    ///
    /// Once per distinct host for the life of the process, not once per
    /// request: discovery and JWKS are polled, a health probe runs every few
    /// seconds, and a warning that repeats is a warning that gets filtered out.
    pub fn sight(&self, host: &str) -> bool {
        let mut seen = self.seen.lock().expect("host sightings poisoned");
        if seen.iter().any(|known| known == host) {
            return false;
        }
        if seen.len() >= MAX_SIGHTINGS {
            return false;
        }
        seen.push(host.to_string());
        true
    }

    pub fn seen(&self) -> Vec<String> {
        self.seen.lock().expect("host sightings poisoned").clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISSUER: &str = "http://127.0.0.1:9500/oidc";

    #[test]
    fn an_authority_is_read_out_of_a_url_without_parsing_it() {
        assert_eq!(authority_of(ISSUER), Some("127.0.0.1:9500"));
        assert_eq!(authority_of("http://lanyard:9500"), Some("lanyard:9500"));
        assert_eq!(authority_of("http://lanyard"), Some("lanyard"));
        assert_eq!(authority_of("not a url"), None);
    }

    #[test]
    fn the_issuers_own_authority_matches_and_nothing_else_does() {
        assert!(matches(ISSUER, "127.0.0.1:9500"));
        assert!(!matches(ISSUER, "lanyard:9500"));
    }

    /// **The port is the mismatch**, which is how one check covers CONCEPT §8's
    /// `-p 9500:8080` gotcha without a second detector.
    #[test]
    fn the_same_host_on_another_port_is_a_mismatch() {
        assert!(!matches("http://localhost:8080/oidc", "localhost:9500"));
    }

    /// One rule, no severity tiers: a conforming relying party compares `iss`
    /// as a string, and this one genuinely breaks.
    #[test]
    fn localhost_and_the_loopback_address_are_not_the_same_name() {
        assert!(!matches(ISSUER, "localhost:9500"));
    }

    /// A check that cannot be made is not a check that failed.
    #[test]
    fn an_unreadable_issuer_matches_everything_rather_than_warning_about_it() {
        assert!(matches("nonsense", "lanyard:9500"));
    }

    /// Named once, used on both sides — a second spelling is a header that
    /// silences nothing.
    #[test]
    fn doctor_names_itself_with_its_version() {
        assert!(DOCTOR_USER_AGENT.starts_with("lanyard-doctor/"));
        assert!(DOCTOR_USER_AGENT.ends_with(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn the_warning_names_both_authorities_the_iss_and_the_fix() {
        let w = warning(ISSUER, "lanyard:9500");
        assert!(w.contains("lanyard:9500"), "{w}");
        assert!(w.contains(ISSUER), "{w}");
        assert!(w.contains("will reject it"), "{w}");
        assert!(
            w.contains("LANYARD_ISSUER=http://lanyard:9500/oidc"),
            "the fix keeps the scheme and the /oidc path: {w}"
        );
    }

    /// Once per distinct host for the life of the process. A warning that
    /// repeats is a warning that gets filtered out.
    #[test]
    fn a_host_is_sighted_once_however_often_it_arrives() {
        let hosts = HostSightings::default();
        assert!(hosts.sight("lanyard:9500"));
        for _ in 0..10 {
            assert!(!hosts.sight("lanyard:9500"));
        }
        assert!(
            hosts.sight("other:9500"),
            "a second host is a second warning"
        );
        assert_eq!(hosts.seen(), vec!["lanyard:9500", "other:9500"]);
    }

    /// The set is fed by a request header, so it is capped rather than trusted.
    #[test]
    fn the_set_stops_growing_once_it_has_said_everything_useful() {
        let hosts = HostSightings::default();
        for i in 0..MAX_SIGHTINGS + 10 {
            hosts.sight(&format!("host-{i}:9500"));
        }
        assert_eq!(hosts.seen().len(), MAX_SIGHTINGS);
    }
}
