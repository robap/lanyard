//! The `/_/` UI: the persona picker, and the pages lanyard renders when it has
//! something to say to the developer rather than to their app.
//!
//! Protocol endpoints live under `/oidc` and the UI lives here (CONCEPT §3).
//! `/authorize` reaches the picker by redirecting to it rather than rendering
//! it, which is what keeps that line intact — and it means the picker can be
//! reloaded or opened in another tab without re-validating an authorization
//! request.
//!
//! **No password field.** There is nothing to authenticate; the list *is* the
//! authentication, and that is the whole product.

pub mod form_post;
pub mod html;

use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;

use crate::app::SharedState;
use crate::oidc::authorize;
use crate::oidc::revocation;
use crate::oidc::token::Form;
use crate::persona::Persona;
use crate::session;
use crate::store::{Lookup, Selection};

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route("/", get(picker))
        .route("/pick", post(pick))
        .route("/session", post(session_settings))
        // Three plain `POST` forms, no JavaScript, same as everything else on
        // this page. `SameSite=Lax` means a cross-site POST arrives without the
        // cookie and therefore acts on no session, which is why none of them
        // needs a CSRF token — the same reasoning Phase 4 recorded for
        // `/_/pick`.
        .route("/logout", post(logout))
        .route("/forget", post(forget))
        .route("/expire", post(expire))
}

// ------------------------------------------------------------ the picker --

/// Public because `/_/` — with the trailing slash — has to be routed at the top
/// level as well as inside the nest. See [`crate::app::router`].
pub async fn picker(
    State(state): State<SharedState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let form = Form::from_query(query.as_deref().unwrap_or_default());
    let browser = Browser::of(&state, session::from_headers(&headers).as_deref());

    let req = form.get("req");
    let pending = req.map(|id| {
        let store = state.stores.pending.lock().expect("pending");
        match store.peek(id) {
            Lookup::Found(request) => Some(request.client_id.clone()),
            _ => None,
        }
    });

    match (req, pending) {
        // A request id that names nothing: the five minutes ran out, or lanyard
        // restarted. Say which two things it could have been rather than
        // rendering an empty picker whose buttons would 400.
        (Some(_), Some(None)) => html::html(
            StatusCode::BAD_REQUEST,
            html::page(
                "lanyard — that login is no longer in progress",
                &format!(
                    "<div class=\"warn card stack gap-sm pad-lg border\">\n\
                     <h1 class=\"text-h2\">That login is no longer in progress</h1>\n\
                     <p>An authorization request is held for five minutes, and lanyard \
                     holds them in memory, so either the five minutes ran out or lanyard \
                     was restarted. Start the login again from your application.</p>\n</div>\n{}",
                    persona_list(&state, None, &browser)
                ),
            ),
        ),
        (Some(id), Some(Some(client_id))) => html::html(
            StatusCode::OK,
            html::page(
                "lanyard — pick a person",
                &persona_list(&state, Some((id, &client_id)), &browser),
            ),
        ),
        // The banner's `UI →` line lands here, so it has to be honest about
        // there being nothing to do.
        _ => html::html(
            StatusCode::OK,
            html::page("lanyard — personas", &persona_list(&state, None, &browser)),
        ),
    }
}

/// The list of people, as buttons when a login is waiting on one and as plain
/// cards when nothing is.
fn persona_list(state: &SharedState, req: Option<(&str, &str)>, browser: &Browser) -> String {
    let mut out = String::new();

    match req {
        Some((_, client_id)) => out.push_str(&format!(
            "<h1 class=\"text-h2\">Who are you signing in as?</h1>\n\
             <p class=\"lede text-body\"><code>{}</code> is waiting. There is no password — \
             pick a person.</p>\n",
            html::escape(client_id)
        )),
        None => out.push_str(
            "<h1 class=\"text-h2\">Personas</h1>\n\
             <p class=\"lede text-body\">No login is in progress. These are the people lanyard \
             can sign you in as; start a login from your application to pick one.</p>\n",
        ),
    }

    for persona in &state.personas.list {
        out.push_str(&persona_row(persona, req.map(|(id, _)| id)));
    }

    out.push_str(&mint_panel(req.map(|(id, _)| id)));
    out.push_str(&this_browser_panel(state, req.map(|(id, _)| id), browser));
    out.push_str(
        "<footer class=\"text-small\">lanyard · sessions live in memory, so restarting \
         lanyard logs everybody out. · <a href=\"/_/log\">live log</a></footer>\n",
    );
    out
}

fn persona_row(persona: &Persona, req: Option<&str>) -> String {
    // Every one of these is developer-supplied and every one of them is
    // escaped. A persona file can come out of a fixture generator, and a picker
    // that executes its own persona list is a bad look for a tool whose pitch
    // is "it catches your bugs" (criterion 27).
    let name = match &persona.name {
        Some(name) => format!("<span class=\"name text-h4\">{}</span>", html::escape(name)),
        None => "<span class=\"name none text-h4\">(no name)</span>".to_string(),
    };
    let email = match &persona.email {
        Some(email) => format!(
            "<span class=\"email text-small\">{}</span>",
            html::escape(email)
        ),
        None => "<span class=\"email none text-small\">(no email)</span>".to_string(),
    };
    let roles = if persona.roles.is_empty() {
        "<span class=\"roles none text-small\">(no roles)</span>".to_string()
    } else {
        format!(
            "<span class=\"roles text-small\">{}</span>",
            html::escape(&persona.roles.join(", "))
        )
    };
    let inner = format!(
        "{name}\n<span class=\"id text-code text-small\">{}</span>\n{email}\n{roles}\n",
        html::escape(&persona.id)
    );

    match req {
        Some(req) => format!(
            "<form class=\"persona\" method=\"post\" action=\"/_/pick\">\n\
             <input type=\"hidden\" name=\"req\" value=\"{}\">\n\
             <input type=\"hidden\" name=\"persona\" value=\"{}\">\n\
             <button class=\"persona-btn stack gap-xs pad-md border text-start\" \
             type=\"submit\">\n\
             {inner}</button>\n</form>\n",
            html::escape(req),
            html::escape(&persona.id),
        ),
        None => format!("<div class=\"card persona stack gap-xs pad-md border\">\n{inner}</div>\n"),
    }
}

/// "Mint one now": an identity that is in no file. It travels the identical
/// path to a loaded persona because the code record owns a whole [`Persona`]
/// rather than an id.
fn mint_panel(req: Option<&str>) -> String {
    let Some(req) = req else {
        return String::new();
    };
    format!(
        "<h2 class=\"text-eyebrow\">Or mint one now</h2>\n\
         <form class=\"card mint stack gap-xs pad-md border\" method=\"post\" \
         action=\"/_/pick\">\n\
         <input type=\"hidden\" name=\"req\" value=\"{}\">\n\
         <label for=\"m-sub\">sub</label>\
         <input id=\"m-sub\" type=\"text\" name=\"sub\" placeholder=\"zed\" required>\n\
         <label for=\"m-name\">name</label>\
         <input id=\"m-name\" type=\"text\" name=\"name\" placeholder=\"Zed Quinn\">\n\
         <label for=\"m-email\">email</label>\
         <input id=\"m-email\" type=\"text\" name=\"email\" placeholder=\"zed@example.test\">\n\
         <label for=\"m-roles\">roles, comma separated</label>\
         <input id=\"m-roles\" type=\"text\" name=\"roles\" placeholder=\"admin, user\">\n\
         <label for=\"m-attrs\">extra claims, as JSON</label>\
         <textarea id=\"m-attrs\" name=\"attributes\" \
         placeholder='{{\"department\": \"ops\"}}'></textarea>\n\
         <button class=\"submit align-self-start\" type=\"submit\">Sign in as this \
         person</button>\n\
         <p class=\"lede text-small\">A one-off is not remembered: the next login from \
         this application shows this page again.</p>\n\
         </form>\n",
        html::escape(req),
    )
}

/// What the **This browser** panel renders: what this browser is signed in as,
/// per `client_id`, and whether it has asked to be asked every time.
///
/// Read once, under one lock, before any page is built — the store has no async
/// surface and no guard here crosses an `.await`.
struct Browser {
    always_ask: bool,
    /// One entry per `client_id`, sorted, so the page does not reorder itself
    /// on reload.
    signed_in: Vec<(String, Selection)>,
}

impl Browser {
    fn of(state: &SharedState, session_id: Option<&str>) -> Browser {
        let Some(session_id) = session_id else {
            return Browser {
                always_ask: false,
                signed_in: Vec::new(),
            };
        };
        let sessions = state.stores.sessions.lock().expect("sessions");
        Browser {
            always_ask: sessions.always_ask(session_id),
            signed_in: sessions.selections(session_id),
        }
    }
}

/// **This browser**: one row per `client_id` this browser is signed in for, and
/// the always-ask flag.
///
/// On both versions of the page — with a login in progress and without — because
/// the question "who does this browser think I am" is the one a developer opens
/// this page to answer, and a login in progress is not a reason to hide it.
fn this_browser_panel(state: &SharedState, req: Option<&str>, browser: &Browser) -> String {
    let mut out = String::from("<h2 class=\"text-eyebrow\">This browser</h2>\n");

    if browser.signed_in.is_empty() {
        out.push_str(
            "<p class=\"lede text-body\">This browser is not signed in to any application. \
             Start a login and pick somebody, and it will be listed here.</p>\n",
        );
    }
    for (client_id, selection) in &browser.signed_in {
        out.push_str(&signed_in_row(state, req, client_id, selection));
    }

    // **Log out of lanyard**: the whole session, every selection and every
    // refresh token it was issued. Offered even with nothing listed, because a
    // browser can hold a session record with no selection in it.
    out.push_str(&format!(
        "<form class=\"card stack gap-sm pad-md border\" method=\"post\" \
         action=\"/_/logout\">\n{}\
         <button class=\"submit danger align-self-start\" type=\"submit\">Log out of \
         lanyard</button>\n\
         <p class=\"lede text-small\">Every application, not just one. Each application \
         still holds its own session cookie, and only that application can clear \
         that.</p>\n</form>\n",
        back_field(req)
    ));

    // Its own little form, so the flag can be turned off without starting a
    // login — and so that turning it on does not need JavaScript to also submit
    // whichever persona form the human happens to be looking at.
    let checked = if browser.always_ask { " checked" } else { "" };
    let back = back_field(req);
    out.push_str(&format!(
        "<form class=\"card stack gap-sm pad-md border\" method=\"post\" \
         action=\"/_/session\">\n{back}\
         <div class=\"check cluster gap-sm align-start\">\
         <input id=\"always-ask\" type=\"checkbox\" name=\"always_ask\" value=\"1\"{checked}>\
         <label for=\"always-ask\">Always ask which person, even when this browser \
         already chose one for an application</label></div>\n\
         <button class=\"submit align-self-start\" type=\"submit\">Save</button>\n\
         </form>\n"
    ));
    out
}

/// One application this browser is signed in to, and who as.
///
/// A `client_id` is developer-supplied like everything else on this page, and
/// goes through the same escaping. A persona that is no longer in the list is
/// named by its id rather than dropped — the selection is real even when the
/// file it came from changed underneath it.
fn signed_in_row(
    state: &SharedState,
    req: Option<&str>,
    client_id: &str,
    selection: &Selection,
) -> String {
    let who = match state.personas.get(&selection.persona_id) {
        Some(persona) => match &persona.name {
            Some(name) => html::escape(name),
            None => html::escape(&persona.id),
        },
        None => format!(
            "{} <span class=\"none\">(no longer in the persona list)</span>",
            html::escape(&selection.persona_id)
        ),
    };
    // **Forget** and **Expire now** are the asymmetry the panel exists to make
    // available: one drops the person and shows the picker next time, the other
    // kills the tokens and keeps the person, so the app's own renew path runs.
    format!(
        "<div class=\"card row cluster align-center gap-md pad-md border\">\n\
         <span class=\"client\"><code>{client}</code></span>\n\
         <span class=\"name\">{who}</span>\n\
         <span class=\"when text-small\">chosen {when}</span>\n\
         <form method=\"post\" action=\"/_/forget\">{back}\
         <input type=\"hidden\" name=\"client_id\" value=\"{client}\">\n\
         <button type=\"submit\" title=\"Drop this application&#39;s persona, so its \
         next login shows the picker\">Forget</button></form>\n\
         <form method=\"post\" action=\"/_/expire\">{back}\
         <input type=\"hidden\" name=\"client_id\" value=\"{client}\">\n\
         <button type=\"submit\" title=\"Kill this application&#39;s live tokens and \
         keep the persona, so its own renew path runs\">Expire now</button></form>\n\
         </div>\n",
        client = html::escape(client_id),
        when = html::escape(&ago(selection.auth_time)),
        back = back_field(req),
    )
}

/// The hidden field that carries a login in progress across one of these forms,
/// so a control on this page never abandons the login the human is in the
/// middle of.
fn back_field(req: Option<&str>) -> String {
    match req {
        Some(req) => format!(
            "<input type=\"hidden\" name=\"req\" value=\"{}\">\n",
            html::escape(req)
        ),
        None => String::new(),
    }
}

/// How long ago, in the words a person would use. `auth_time` is unix seconds
/// and may predate this process, so a negative difference reads as "just now"
/// rather than as an enormous number.
fn ago(unix_seconds: u64) -> String {
    let elapsed = unix_now().saturating_sub(unix_seconds);
    match elapsed {
        0..=44 => "just now".to_string(),
        45..=5399 => {
            let minutes = (elapsed + 30) / 60;
            format!("{minutes} minute{} ago", plural(minutes))
        }
        _ => {
            let hours = (elapsed + 1800) / 3600;
            format!("{hours} hour{} ago", plural(hours))
        }
    }
}

fn plural(n: u64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

// -------------------------------------------------------------- the pick --

async fn pick(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Response {
    let form = Form::parse(&body);
    let browser = Browser::of(&state, session::from_headers(&headers).as_deref());

    let Some(req) = form.get("req") else {
        return expired_page(&state, &browser);
    };

    // Consumed: a pending request is answered once. Reloading the picker peeks;
    // submitting it takes.
    let request = {
        let mut store = state.stores.pending.lock().expect("pending");
        match store.take(req) {
            Lookup::Found(request) => request,
            _ => return expired_page(&state, &browser),
        }
    };

    // A loaded persona is cloned and a one-off is built, and from here they are
    // the same thing — which is what makes "mint one now" a feature rather than
    // a second code path (spec, open question 10).
    let (persona, remember) = match form.get("persona") {
        Some(id) => match state.personas.get(id) {
            Some(persona) => (persona.clone(), true),
            None => {
                return html::html(
                    StatusCode::BAD_REQUEST,
                    html::page(
                        "lanyard — no such persona",
                        &format!(
                            "<div class=\"warn card stack gap-sm pad-lg border\">\
                             <h1 class=\"text-h2\">No persona with id <code>{}</code> \
                             is loaded</h1></div>\n",
                            html::escape(id)
                        ),
                    ),
                )
            }
        },
        None => match one_off(&form) {
            Ok(persona) => (persona, false),
            Err(message) => {
                return html::html(
                    StatusCode::BAD_REQUEST,
                    html::page(
                        "lanyard — that person could not be minted",
                        &format!(
                            "<div class=\"warn card stack gap-sm pad-lg border\">\
                             <h1 class=\"text-h2\">{}</h1></div>\n",
                            html::escape(&message)
                        ),
                    ),
                )
            }
        },
    };

    let auth_time = unix_now();
    let session_id = {
        let mut sessions = state.stores.sessions.lock().expect("sessions");
        let id = sessions.ensure(session::from_headers(&headers).as_deref());
        // An identity from the "mint one now" panel is deliberately not
        // remembered: the session record would have to grow from an id into a
        // claim blob to hold it, and being silently logged in as a one-off you
        // typed once is surprising.
        if remember {
            sessions.remember(
                &id,
                &request.client_id,
                Selection {
                    persona_id: persona.id.clone(),
                    auth_time,
                },
            );
        }
        id
    };

    with_session_cookie(
        authorize::complete(
            &state,
            request,
            persona,
            auth_time,
            Some(session_id.clone()),
        ),
        &session_id,
    )
}

/// The extra-claims textarea is parsed rather than swallowed: a typo in it that
/// silently produced a persona without the claim is the failure this whole
/// project exists not to be.
fn one_off(form: &Form) -> Result<Persona, String> {
    let sub = form
        .get("sub")
        .ok_or_else(|| "a one-off person still needs a sub".to_string())?;

    let attributes = match form.get("attributes") {
        None => serde_json::Map::new(),
        Some(raw) => match serde_json::from_str::<serde_json::Value>(raw) {
            Ok(serde_json::Value::Object(map)) => map,
            Ok(_) => return Err("extra claims must be a JSON object".to_string()),
            Err(e) => return Err(format!("extra claims are not valid JSON: {e}")),
        },
    };

    Ok(Persona {
        id: sub.to_string(),
        name: form.get("name").map(str::to_string),
        email: form.get("email").map(str::to_string),
        roles: form
            .get("roles")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|role| !role.is_empty())
            .map(str::to_string)
            .collect(),
        attributes,
        client: None,
    })
}

fn expired_page(state: &SharedState, browser: &Browser) -> Response {
    html::html(
        StatusCode::BAD_REQUEST,
        html::page(
            "lanyard — that login is no longer in progress",
            &format!(
                "<div class=\"warn card stack gap-sm pad-lg border\">\n\
                 <h1 class=\"text-h2\">That login is no longer in progress</h1>\n\
                 <p>The request was already answered, or its five minutes ran out, or \
                 lanyard was restarted. Start the login again from your \
                 application.</p>\n</div>\n{}",
                persona_list(state, None, browser)
            ),
        ),
    )
}

// ---------------------------------------------------------- this browser --

async fn session_settings(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let form = Form::parse(&body);
    // An unchecked checkbox is not submitted at all, which is how HTML says
    // "off" and is why this reads presence rather than a value.
    let always_ask = form.get("always_ask").is_some();

    let session_id = {
        let mut sessions = state.stores.sessions.lock().expect("sessions");
        let id = sessions.ensure(session::from_headers(&headers).as_deref());
        sessions.set_always_ask(&id, always_ask);
        id
    };

    with_session_cookie(authorize::found(&back_to(&form)), &session_id)
}

/// **Log out of lanyard**: the whole session, and every refresh token it was
/// issued.
///
/// The cookie is expired as well as the record dropped, for `/end_session`'s
/// reason: the record is what matters, and the cookie is what makes the logout
/// readable in a network tab.
async fn logout(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Response {
    let form = Form::parse(&body);
    if let Some(session_id) = session::from_headers(&headers) {
        state
            .stores
            .sessions
            .lock()
            .expect("sessions")
            .remove(&session_id);
        revocation::revoke_session_refresh_tokens(&state, &session_id, None);
    }

    let mut response = authorize::found(&back_to(&form));
    if let Ok(value) = header::HeaderValue::from_str(&session::clear_cookie()) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

/// **Forget**: one `client_id`'s selection, and nothing else. Its next
/// `/authorize` shows the picker; the other rows are untouched.
async fn forget(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Response {
    let form = Form::parse(&body);
    let session_id = {
        let mut sessions = state.stores.sessions.lock().expect("sessions");
        let id = sessions.ensure(session::from_headers(&headers).as_deref());
        if let Some(client_id) = form.get("client_id") {
            sessions.forget(&id, client_id);
        }
        id
    };
    with_session_cookie(authorize::found(&back_to(&form)), &session_id)
}

/// **Expire now**: revoke one `client_id`'s live tokens and **keep the
/// selection**.
///
/// The asymmetry with Forget is the whole point of the control (CONCEPT §6).
/// The question it answers is "what does my application do when its token
/// dies", not "what does the picker look like" — and making the tokens dead
/// without making the person forgotten is the only way to ask that without
/// waiting sixty seconds.
async fn expire(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Response {
    let form = Form::parse(&body);
    let session_id = {
        let mut sessions = state.stores.sessions.lock().expect("sessions");
        sessions.ensure(session::from_headers(&headers).as_deref())
    };
    if let Some(client_id) = form.get("client_id") {
        revocation::expire_client(&state, &session_id, client_id);
    }
    with_session_cookie(authorize::found(&back_to(&form)), &session_id)
}

/// Back to the page the human was on, with the login still in progress if there
/// was one. A control on this page never abandons the login it interrupted.
fn back_to(form: &Form) -> String {
    match form.get("req") {
        Some(req) => format!("/_/?req={req}"),
        None => "/_/".to_string(),
    }
}

/// Attaches the one cookie. Set on every response that creates or touches a
/// session, because a browser that lost it would otherwise be a new browser.
pub fn with_session_cookie(mut response: Response, session_id: &str) -> Response {
    if let Ok(value) = header::HeaderValue::from_str(&session::set_cookie(session_id)) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `auth_time` may predate this process — a remembered selection is
    /// deliberately older than the page rendering it — and a clock that went
    /// backwards must read as "just now" rather than as an enormous number.
    #[test]
    fn ago_reads_in_the_words_a_person_would_use() {
        let now = unix_now();
        assert_eq!(ago(now), "just now");
        assert_eq!(ago(now + 500), "just now", "a future auth_time saturates");
        assert_eq!(ago(now - 44), "just now");
        assert_eq!(ago(now - 60), "1 minute ago");
        assert_eq!(ago(now - 150), "3 minutes ago");
        assert_eq!(ago(now - 7200), "2 hours ago");
    }
}
