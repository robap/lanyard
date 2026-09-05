//! The one rejection. lanyard has no client registry, no redirect allowlist and
//! no consent screen; this file is the entire security boundary, and it is one
//! sentence long: **the host has to be loopback.**
//!
//! It is a *literal* check. No DNS lookup happens here or anywhere else, so
//! `web.localtest.me` is rejected although it resolves to 127.0.0.1. A rule
//! enforced by resolution depends on the network, is cacheable, is different
//! inside a container than outside one, and — worst — stops being explainable in
//! one line, which is the property CONCEPT §3 is actually protecting.
//!
//! Parsing is `url`'s rather than ours precisely *because* this is the boundary:
//! it is the one place in the project where a parser bug would cost something,
//! and its IDNA normalization is what turns a Cyrillic homograph of `localhost`
//! into `xn--…` so the literal comparison fails.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

/// The rule, in the words the rendered `400` uses.
///
/// It is told which parameter it is guarding because Phase 5 pointed the same
/// check at a second one, `post_logout_redirect_uri`, and a rule that names the
/// wrong parameter is a rule the developer reading it has to translate.
pub fn rule(parameter: &str) -> String {
    format!(
        "lanyard only accepts a {parameter} on loopback: \
         scheme http or https, and a host of localhost, any *.localhost name, \
         any 127.0.0.0/8 address, or [::1]. The host is compared literally — \
         no DNS lookup happens — so a name like web.localtest.me is rejected \
         even though it resolves to 127.0.0.1."
    )
}

/// Parse `raw` and accept it only if its host is loopback. Any port, any path,
/// any query.
///
/// **One function, two parameters.** `/oidc/authorize` guards `redirect_uri`
/// and `/oidc/end_session` guards `post_logout_redirect_uri` with this exact
/// call: extending the boundary to a second parameter must not mean a second
/// implementation of it (north star 1).
pub fn check(parameter: &str, raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|e| format!("{parameter} {raw:?} is not a URL: {e}"))?;

    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "{parameter} {raw:?} has scheme {:?}, which is not http or https",
            url.scheme()
        ));
    }

    match url.host() {
        Some(Host::Domain(name)) if is_loopback_name(name) => Ok(url),
        Some(Host::Ipv4(addr)) if is_loopback_v4(addr) => Ok(url),
        Some(Host::Ipv6(addr)) if addr == Ipv6Addr::LOCALHOST => Ok(url),
        Some(host) => Err(format!(
            "{parameter} {raw:?} has host {host}, which is not loopback"
        )),
        None => Err(format!("{parameter} {raw:?} has no host")),
    }
}

/// `localhost` itself, or anything under it — RFC 6761 §6.3 reserves the whole
/// name. `url` has already lowercased and punycoded whatever arrived, so this
/// comparison sees `xn--lcalhost-90i`, not a Cyrillic lookalike.
fn is_loopback_name(name: &str) -> bool {
    name == "localhost" || name.ends_with(".localhost")
}

/// The whole of `127.0.0.0/8`, not just `127.0.0.1`. Browsers and SDKs both use
/// the rest of it, and `Ipv4Addr::is_loopback` is exactly that predicate.
fn is_loopback_v4(addr: Ipv4Addr) -> bool {
    IpAddr::V4(addr).is_loopback()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accepted(raw: &str) -> Url {
        check("redirect_uri", raw).unwrap_or_else(|e| panic!("{raw} should be accepted: {e}"))
    }

    fn rejected(raw: &str) -> String {
        check("redirect_uri", raw).expect_err(&format!("{raw} should be rejected"))
    }

    /// The same check, told it is guarding the logout parameter: same verdict,
    /// and a message that names what the developer actually sent.
    #[test]
    fn the_same_check_names_whichever_parameter_it_is_guarding() {
        assert!(check("post_logout_redirect_uri", "http://localhost:5000/").is_ok());
        let message = check("post_logout_redirect_uri", "https://evil.example.com/").unwrap_err();
        assert!(message.contains("post_logout_redirect_uri"), "{message}");
        assert!(message.contains("not loopback"), "{message}");
        assert!(rule("post_logout_redirect_uri").contains("post_logout_redirect_uri"));
    }

    #[test]
    fn loopback_in_all_three_spellings_is_accepted_on_any_port_and_path() {
        assert_eq!(accepted("http://localhost:5000/cb").port(), Some(5000));
        assert_eq!(accepted("http://127.0.0.9:1/x?y=z").path(), "/x");
        assert_eq!(accepted("http://[::1]:5173/").port(), Some(5173));
        // Any port includes none, and https is as loopback as http.
        accepted("https://localhost/signin-oidc");
        // RFC 6761 reserves the whole name, not just the label.
        accepted("http://app.dev.localhost:8080/cb");
    }

    #[test]
    fn a_public_host_is_rejected_and_the_message_names_it() {
        let message = rejected("https://evil.example.com/cb");
        assert!(
            message.contains("https://evil.example.com/cb"),
            "the developer reading this needs to see what was rejected: {message}"
        );
    }

    /// The decision that costs something: this name really does resolve to
    /// 127.0.0.1, and is rejected anyway, because no DNS lookup happens.
    #[test]
    fn a_name_that_resolves_to_loopback_is_still_rejected() {
        let message = rejected("http://web.localtest.me:5000/cb");
        assert!(message.contains("web.localtest.me"), "{message}");
        assert!(message.contains("not loopback"), "{message}");
    }

    /// Native-app shapes are out of scope entirely; they fail on the scheme
    /// rather than the host, and the message says which.
    #[test]
    fn a_non_http_scheme_is_rejected_naming_the_scheme() {
        for raw in ["com.example.app:/cb", "urn:ietf:wg:oauth:2.0:oob"] {
            let message = rejected(raw);
            assert!(message.contains("not http or https"), "{raw}: {message}");
        }
    }

    /// The reason a real URL parser earns its place: `lоcalhost` here carries a
    /// Cyrillic `о`, IDNA turns it into `xn--…`, and the literal comparison
    /// against `localhost` fails. A hand-rolled `host == "localhost"` on the raw
    /// bytes would too, but only by accident — this is the check being right on
    /// purpose.
    #[test]
    fn a_homograph_of_localhost_punycodes_and_fails_the_literal_comparison() {
        let message = rejected("http://l\u{043e}calhost:5000/cb");
        assert!(message.contains("xn--"), "IDNA should have run: {message}");
        assert!(message.contains("not loopback"), "{message}");
    }

    #[test]
    fn something_that_is_not_a_url_at_all_is_rejected_rather_than_panicking() {
        assert!(rejected("not a url").contains("not a URL"));
        assert!(rejected("").contains("not a URL"));
    }

    /// `http:///cb` parses with an empty host. Not a crash, and not an accept.
    #[test]
    fn an_empty_host_is_rejected() {
        let message = rejected("http:///cb");
        assert!(
            message.contains("no host") || message.contains("not loopback"),
            "{message}"
        );
    }
}
