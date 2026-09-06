//! `GET`/`POST /oidc/end_session` — OIDC RP-Initiated Logout.
//!
//! **There are two sessions, and this is what ends both.** The RP drops its own
//! cookie and navigates here; lanyard drops the whole browser session, expires
//! `lanyard_session`, and sends the browser back. It is a redirect chain rather
//! than a back-channel call because `lanyard_session` lives in the browser and
//! only a top-level navigation carries it.
//!
//! **Logging out never fails.** No session, an unknown session, a session from
//! before a restart: all of them are the same `302` or the same page. There is
//! no state in which a logout request produces an error the user has to read,
//! except the one rejection — `post_logout_redirect_uri` has to be loopback,
//! which is [`redirect_uri::check`] pointed at a second parameter.
//!
//! **No confirmation screen, ever.** RP-Initiated Logout §2 says the OP *should*
//! ask the user to confirm when there is no valid `id_token_hint`. lanyard has
//! no consent screen and this is the same argument: a prompt nobody can automate
//! past is a prompt in the way of a test.
//!
//! **It drops everything, not one `client_id`.** Every real IdP has one SSO
//! session and clears all of it, and diverging from production is the thing this
//! project exists not to do. The per-`client_id` variant lives on `/_/` as
//! **Forget**, where a developer reaches for it deliberately rather than where
//! an SDK would reach it by accident.

use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::Value;

use crate::app::SharedState;
use crate::oidc::authorize::found;
use crate::oidc::redirect_uri;
use crate::oidc::revocation;
use crate::oidc::token::Form;
use crate::session;
use crate::ui::html;

/// The parameter the one rejection is pointed at here, named once so the check,
/// the rule and the page cannot spell it differently.
const PARAMETER: &str = "post_logout_redirect_uri";

pub fn route() -> axum::routing::MethodRouter<SharedState> {
    // Both verbs: RP-Initiated Logout §2 allows either, .NET sends the first
    // and some SDKs send the second.
    axum::routing::get(end_session_get).post(end_session_post)
}

async fn end_session_get(
    State(state): State<SharedState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    end_session(
        &state,
        &headers,
        Form::from_query(query.as_deref().unwrap_or_default()),
    )
}

async fn end_session_post(
    State(state): State<SharedState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    body: Bytes,
) -> Response {
    let form = Form::parse(&body).chain(Form::from_query(query.as_deref().unwrap_or_default()));
    end_session(&state, &headers, form)
}

fn end_session(state: &SharedState, headers: &HeaderMap, form: Form) -> Response {
    let response = logout(state, headers, &form);
    // The application named by the hint, and the fact that a session actually
    // ended. "Why am I still signed in" is a question this line answers.
    crate::log_detail::attach(
        response,
        crate::log_detail::LogDetail {
            client_id: form
                .get("id_token_hint")
                .and_then(audience_of)
                .or_else(|| form.get("client_id").map(str::to_string)),
            request: Some(crate::log_detail::request_map(form.pairs())),
            ..crate::log_detail::LogDetail::default()
        },
    )
}

fn logout(state: &SharedState, headers: &HeaderMap, form: &Form) -> Response {
    // **The rejection comes first, and it logs nobody out.** A logout lanyard
    // refused is a request it never acted on; ending the session and then
    // rendering an error would leave the browser signed out of a page that says
    // the request failed.
    let target = match form.get(PARAMETER) {
        None => None,
        Some(raw) => match redirect_uri::check(PARAMETER, raw) {
            Ok(url) => Some(url),
            Err(message) => {
                // Refused, and **nobody was logged out** — which is exactly the
                // outcome the log has to record, or a developer reads a
                // rejection and assumes the session survived by accident.
                return crate::log_detail::attach(
                    html::rejected_page(
                        &message,
                        Some(raw),
                        &redirect_uri::rule(PARAMETER),
                        "Nothing was sent to that address and nobody was logged out. \
                         Sending a browser to an address lanyard has just decided not to \
                         trust is what would make the boundary decorative, and this is the \
                         one thing lanyard does not accept.",
                    ),
                    crate::log_detail::LogDetail {
                        error: Some("invalid_request".to_string()),
                        error_description: Some(message),
                        detail: Some(
                            [("signed_out".to_string(), Value::Bool(false))]
                                .into_iter()
                                .collect(),
                        ),
                        ..crate::log_detail::LogDetail::default()
                    },
                );
            }
        },
    };

    // Dropped **before** the response is written. The record is what matters;
    // the cookie below is what makes the logout readable in `curl -i`.
    let mut signed_out = false;
    if let Some(session_id) = session::from_headers(headers) {
        signed_out = true;
        state
            .stores
            .sessions
            .lock()
            .expect("sessions")
            .remove(&session_id);
        // Open question 2: a "full logout" that leaves a working refresh token
        // in an app's local storage is not one.
        revocation::revoke_session_refresh_tokens(state, &session_id, None);
    }

    // `id_token_hint` is **read, never required**, and `client_id` is ignored
    // except for display. A hint that is expired, foreign, malformed or absent
    // changes nothing about the outcome — it only decides whether the rendered
    // page can name the application.
    let application = form
        .get("id_token_hint")
        .and_then(audience_of)
        .or_else(|| form.get("client_id").map(str::to_string));

    let response = match target {
        Some(mut url) => {
            // Echoed byte-for-byte when sent, absent when not. Never invented.
            if let Some(state_param) = form.get("state") {
                url.query_pairs_mut().append_pair("state", state_param);
            }
            found(url.as_str())
        }
        // An RP that sends no return address has to land somewhere, and landing
        // on a blank page or a bare `302` to `/` would be lanyard being
        // unhelpful at exactly the moment a developer is watching.
        None => signed_out_page(application.as_deref()),
    };

    crate::log_detail::attach(
        with_cleared_cookie(response),
        crate::log_detail::LogDetail {
            detail: Some(
                [("signed_out".to_string(), Value::Bool(signed_out))]
                    .into_iter()
                    .collect(),
            ),
            ..crate::log_detail::LogDetail::default()
        },
    )
}

fn signed_out_page(application: Option<&str>) -> Response {
    let named = match application {
        Some(application) => format!(
            "<p class=\"text-body\">The application that sent you here was \
             <code>{}</code>. It has its own session cookie, and only it can clear that — \
             this page is about lanyard's.</p>\n",
            html::escape(application)
        ),
        None => String::new(),
    };
    let body = format!(
        "<h1 class=\"text-h2\">You are signed out of lanyard</h1>\n\
         <p class=\"lede text-body\">This browser's lanyard session is gone: every persona \
         it had chosen, for every application, and every refresh token it was issued. The \
         next login from any application will show the picker.</p>\n{named}\
         <footer class=\"text-small\">lanyard · <a href=\"/_/\">persona picker</a> · \
         <a href=\"/_/log\">live log</a></footer>\n"
    );
    html::html(StatusCode::OK, html::page("lanyard — signed out", &body))
}

/// The `aud` of an ID token, **without verifying it**. That is the point: an
/// expired, foreign or garbage hint has to log out exactly as an absent one
/// does, so this can only ever fail into `None`.
fn audience_of(hint: &str) -> Option<String> {
    let payload = hint.split('.').nth(1)?;
    let claims: Value = serde_json::from_slice(&crate::b64::decode(payload).ok()?).ok()?;
    match claims.get("aud")? {
        Value::String(one) => Some(one.clone()),
        // An `aud` array is legal; the first entry is the one an RP would read
        // as itself.
        Value::Array(many) => many.first()?.as_str().map(str::to_string),
        _ => None,
    }
}

fn with_cleared_cookie(mut response: Response) -> Response {
    if let Ok(value) = header::HeaderValue::from_str(&session::clear_cookie()) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(claims: serde_json::Value) -> String {
        let payload = crate::b64::encode(serde_json::to_vec(&claims).unwrap());
        format!("header.{payload}.signature")
    }

    /// Read for display and nothing else, so every way of being unreadable is
    /// `None` rather than an error — criterion 9 is that all of them log out
    /// identically.
    #[test]
    fn the_hints_audience_is_read_and_anything_unreadable_is_simply_absent() {
        assert_eq!(
            audience_of(&hint(serde_json::json!({ "aud": "billing-web" }))),
            Some("billing-web".to_string())
        );
        assert_eq!(
            audience_of(&hint(
                serde_json::json!({ "aud": ["billing-web", "other"] })
            )),
            Some("billing-web".to_string())
        );
        for unreadable in ["", "garbage", "a.b", "a.!!!.c", "a.e30.c"] {
            assert_eq!(audience_of(unreadable), None, "{unreadable}");
        }
    }
}
