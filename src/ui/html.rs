//! HTML by hand: an escaping helper, a document shell, and one stylesheet
//! pulled in with `include_str!`.
//!
//! No template engine and no bundler, which is criterion 29: on a machine with
//! no `node` and no `npm` on `PATH`, `cargo build --release` produces a binary
//! that serves a styled picker. The CSS is inlined into every page rather than
//! served from a `<link>` — one round trip, no cache to be stale, and nothing
//! for a CDN to be asked for.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

const CSS: &str = include_str!("lanyard.css");

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
         <title>{}</title>\n\
         <style>\n{CSS}</style>\n\
         </head>\n\
         <body>\n<main>\n{body}\n</main>\n</body>\n</html>\n",
        escape(title),
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

    #[test]
    fn a_page_carries_its_own_css_and_no_link_or_script_tag() {
        let out = page("lanyard", "<p>hello</p>");
        assert!(out.starts_with("<!doctype html>"));
        assert!(out.contains("<p>hello</p>"));
        assert!(
            out.contains("prefers-color-scheme"),
            "the stylesheet is inlined"
        );
        assert!(!out.contains("<link"), "nothing is fetched: {out:.200}");
        assert!(!out.contains("<script"), "no script tag anywhere");
    }

    #[test]
    fn a_page_title_is_escaped_like_everything_else() {
        assert!(page("<script>", "").contains("<title>&lt;script&gt;</title>"));
    }
}
