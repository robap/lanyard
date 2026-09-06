//! HTML by hand: an escaping helper and a document shell.
//!
//! No template engine and no bundler, which is Phase 4's criterion 29 and this
//! phase's criterion 20: on a machine with no `node`, no `npm` and no `zero` on
//! `PATH`, `cargo build --release` produces a binary that serves a styled
//! picker.
//!
//! **Phase 6 reverses one Phase 4 decision here.** The CSS used to be inlined
//! into every page on two grounds — one round trip, and no cache to be stale.
//! zero's design system is 36 KB, too much to repeat in every render, and its
//! output is *fingerprinted*, so "no cache to be stale" is now satisfied by the
//! filename. The round trip is on loopback. So the page gains a `<link>` whose
//! href comes out of the embedded `manifest.json` — no hash is ever written by
//! hand — and `src/ui/lanyard.css` is gone. What does not change: nothing is
//! fetched from a CDN, and the binary is still the whole website.
//!
//! What also does not change is the **script** rule. These pages are in the
//! login path — `/oidc/authorize` redirects a real browser to `/_/` — and that
//! path is committed to working with JavaScript disabled. `/_/log` is the one
//! page that ships a script, and it is not rendered here.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

/// The five characters that matter, and no others.
///
/// A persona file is the developer's own, but its `name` can come out of a
/// fixture generator, and a picker that executes its own persona list is a bad
/// look for a tool whose pitch is "it catches your bugs" (criterion 27).
/// Everything user-supplied goes through here on its way into a page — there is
/// no second path.
pub fn escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// A whole document. `title` is escaped; `body` is markup this crate built.
pub fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n\
         <html lang=\"en\">\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"color-scheme\" content=\"light dark\">\n\
         <title>{}</title>\n\
         <link rel=\"stylesheet\" href=\"{}\">\n\
         </head>\n\
         <body>\n<main class=\"page stack gap-lg pad-xl\">\n{body}\n</main>\n</body>\n</html>\n",
        escape(title),
        crate::embed::stylesheet_href(),
    )
}

/// **The one rejection's page**, rendered at lanyard with no `Location` header,
/// naming the offending value and stating the rule in one sentence.
///
/// Both callers are the same boundary pointed at a different parameter:
/// `/oidc/authorize`'s `redirect_uri` and `/oidc/end_session`'s
/// `post_logout_redirect_uri`. The audience is the developer reading it, which
/// is exactly who needed it — the RP is never contacted, so "an error the RP can
/// read" was never achievable for this class of failure.
pub fn rejected_page(headline: &str, value: Option<&str>, rule: &str, reason: &str) -> Response {
    let named = match value {
        Some(value) => format!("<p><code>{}</code></p>\n", escape(value)),
        None => String::new(),
    };
    let body = format!(
        "<div class=\"warn card stack gap-sm pad-lg border\">\n\
         <h1 class=\"text-h2\">{}</h1>\n{named}\
         <p class=\"text-body\">{}</p>\n\
         </div>\n\
         <p class=\"lede text-body\">{}</p>\n\
         <footer class=\"text-small\">lanyard · <a href=\"/_/\">persona picker</a> · \
         <a href=\"/_/log\">live log</a></footer>\n",
        escape(headline),
        escape(rule),
        escape(reason),
    );
    html(
        StatusCode::BAD_REQUEST,
        page("lanyard — request rejected", &body),
    )
}

/// `text/html` with the status the caller chose. Used for the picker and for
/// the one rejection's rendered `400`.
pub fn html(status: StatusCode, body: String) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_five_dangerous_characters_are_escaped() {
        assert_eq!(
            escape(r#"<script>alert("1" & '2')</script>"#),
            "&lt;script&gt;alert(&quot;1&quot; &amp; &#39;2&#39;)&lt;/script&gt;"
        );
    }

    #[test]
    fn ordinary_text_including_non_ascii_passes_through_untouched() {
        assert_eq!(escape("Mira Okonkwo — admin"), "Mira Okonkwo — admin");
        assert_eq!(escape(""), "");
    }

    /// The ampersand has to go first, or `<` becomes `&amp;lt;`. Escaping an
    /// already-escaped string is a different bug, and this pins which one this
    /// function has.
    #[test]
    fn escaping_is_not_applied_twice_to_its_own_output() {
        assert_eq!(escape("&lt;"), "&amp;lt;");
    }

    /// The Phase 4 assertion, narrowed exactly as far as the spec says and no
    /// further: the `<link>` is now expected, and the **no script** rule stands
    /// for every server-rendered page because those pages are in the login
    /// path.
    #[test]
    fn a_server_rendered_page_links_the_hashed_stylesheet_and_ships_no_script() {
        let out = page("lanyard", "<p>hello</p>");
        assert!(out.starts_with("<!doctype html>"));
        assert!(out.contains("<p>hello</p>"));
        assert!(
            out.contains("<link rel=\"stylesheet\" href=\"/_/assets/app."),
            "the href comes from the embedded manifest: {out:.400}"
        );
        assert!(out.contains(".css\">"), "{out:.400}");
        assert!(
            !out.contains("<style"),
            "the design system is not repeated per render: {out:.400}"
        );
        assert!(
            !out.contains("<script"),
            "no script tag on a server-rendered page"
        );
    }

    /// Nothing is fetched from anywhere but lanyard — criterion 22, asserted at
    /// the one place a page could name another host.
    #[test]
    fn a_page_names_no_host_but_lanyards_own() {
        let out = page("lanyard", "<p>hello</p>");
        assert!(!out.contains("//fonts.googleapis.com"), "{out}");
        assert!(!out.contains("http://"), "{out}");
        assert!(!out.contains("https://"), "{out}");
    }

    #[test]
    fn a_page_title_is_escaped_like_everything_else() {
        assert!(page("<script>", "").contains("<title>&lt;script&gt;</title>"));
    }
}
