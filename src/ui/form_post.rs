//! `response_mode=form_post`: a page whose single form submits itself.
//!
//! It exists because Phase 0 watched a .NET app set its correlation and nonce
//! cookies `SameSite=None` "because it uses response_mode=form_post". Rather
//! than assume that only applied to the implicit response type it was
//! defaulting to, lanyard implements the mode.
//!
//! **This is the only JavaScript in the login path, and it degrades to one
//! click.** The submit button inside `<noscript>` is not a courtesy: criterion
//! 26 completes the whole flow with JavaScript disabled, and a form_post
//! response that needed a script would be the one place that failed.

use crate::ui::html;

/// The auto-submitting page. `action` is a `redirect_uri` that has already
/// passed the loopback check; the fields are the authorization response.
pub fn page(action: &str, fields: &[(&str, &str)]) -> String {
    let inputs: String = fields
        .iter()
        .map(|(name, value)| {
            format!(
                "<input type=\"hidden\" name=\"{}\" value=\"{}\">\n",
                html::escape(name),
                html::escape(value)
            )
        })
        .collect();

    let body = format!(
        "<h1>Signing you in…</h1>\n\
         <form id=\"lanyard-form-post\" method=\"post\" action=\"{}\">\n\
         {inputs}\
         <noscript><p class=\"lede\">Your browser is not running JavaScript, so this \
         last step needs a click.</p>\n\
         <button class=\"submit\" type=\"submit\">Continue</button></noscript>\n\
         </form>\n\
         <script>document.getElementById('lanyard-form-post').submit();</script>\n",
        html::escape(action),
    );
    html::page("lanyard — signing you in", &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_posts_the_authorization_response_to_the_redirect_uri() {
        let out = page(
            "http://localhost:5000/signin-oidc",
            &[("code", "abc"), ("state", "s1")],
        );
        assert!(out.contains("method=\"post\" action=\"http://localhost:5000/signin-oidc\""));
        assert!(out.contains("name=\"code\" value=\"abc\""));
        assert!(out.contains("name=\"state\" value=\"s1\""));
    }

    /// Criterion 26 in miniature: with scripting off, the page still has a
    /// button that finishes the login.
    #[test]
    fn there_is_a_visible_submit_button_inside_noscript() {
        let out = page("http://localhost:5000/cb", &[("code", "abc")]);
        let noscript = out.split("<noscript>").nth(1).unwrap();
        let noscript = noscript.split("</noscript>").next().unwrap();
        assert!(noscript.contains("type=\"submit\""), "{noscript}");
    }

    /// The values are attacker-influenced in exactly the way a persona name is
    /// not — `state` is whatever the RP sent.
    #[test]
    fn field_values_are_escaped_so_a_state_cannot_close_the_input() {
        let out = page(
            "http://localhost:5000/cb",
            &[("state", r#""><script>x</script>"#)],
        );
        assert!(!out.contains("<script>x</script>"), "{out}");
        assert!(out.contains("&quot;&gt;&lt;script&gt;"), "{out}");
    }
}
