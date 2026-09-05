//! The one cookie.
//!
//! ```text
//! lanyard_session=<opaque>; Path=/; HttpOnly; SameSite=Lax
//! ```
//!
//! **No `Secure`, ever, while lanyard serves HTTP.** That is Phase 0's
//! obligation #1, and its failure mode is not a security hole — it is "it asks
//! me to pick a persona every single time", because a browser silently drops a
//! `Secure` cookie sent over `http://` and nothing anywhere says why.
//!
//! **No `Max-Age` and no `Expires` either.** It is a browser-session cookie, so
//! closing the browser is a working reset and no persona selection outlives the
//! day by weeks. The other reset is restarting lanyard, because the server side
//! of this is in memory (spec, open question 9).
//!
//! `SameSite=Lax` is also why cross-site posts to `/_/pick` are not worth a CSRF
//! token: the cookie does not travel on a cross-site POST in the first place,
//! and the request id is unguessable and single-use.

use axum::http::HeaderMap;

pub const COOKIE: &str = "lanyard_session";

/// The session id the browser sent, if it sent one.
pub fn from_headers(headers: &HeaderMap) -> Option<String> {
    // Every `Cookie` header, not just the first: HTTP/2 clients are allowed to
    // split them.
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|header| header.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| name.trim() == COOKIE)
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The `Set-Cookie` value, written out rather than built, so the attributes are
/// readable in one line and a reviewer can see that `Secure` is not among them.
pub fn set_cookie(session_id: &str) -> String {
    format!("{COOKIE}={session_id}; Path=/; HttpOnly; SameSite=Lax")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for pair in pairs {
            headers.append(
                axum::http::header::COOKIE,
                axum::http::HeaderValue::from_str(pair).unwrap(),
            );
        }
        headers
    }

    #[test]
    fn the_session_id_is_read_out_of_a_cookie_header() {
        assert_eq!(
            from_headers(&headers(&["lanyard_session=abc123"])),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn it_is_found_among_other_peoples_cookies() {
        assert_eq!(
            from_headers(&headers(&[
                ".AspNetCore.Correlation.x=y; lanyard_session=abc123; PHPSESSID=z"
            ])),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn it_is_found_across_split_cookie_headers() {
        assert_eq!(
            from_headers(&headers(&["PHPSESSID=z", "lanyard_session=abc123"])),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn no_cookie_and_an_empty_one_both_read_as_absent() {
        assert_eq!(from_headers(&HeaderMap::new()), None);
        assert_eq!(from_headers(&headers(&["lanyard_session="])), None);
        assert_eq!(from_headers(&headers(&["other=1"])), None);
        // A prefix match would find this one, and it is a different cookie.
        assert_eq!(from_headers(&headers(&["lanyard_session_x=1"])), None);
    }

    /// Phase 0's obligation #1, as an assertion rather than a comment. This test
    /// exists to fail the day somebody "hardens" the cookie.
    #[test]
    fn the_cookie_is_never_secure_and_never_expires() {
        let value = set_cookie("abc123");
        assert_eq!(
            value,
            "lanyard_session=abc123; Path=/; HttpOnly; SameSite=Lax"
        );
        assert!(
            !value.contains("Secure"),
            "a Secure cookie is silently dropped over http:// and the symptom is \
             'it asks me to pick a persona every single time'"
        );
        assert!(
            !value.contains("Max-Age"),
            "closing the browser is the reset"
        );
        assert!(!value.contains("Expires"));
    }
}
