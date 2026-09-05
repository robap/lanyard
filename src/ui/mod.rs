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
use crate::oidc::token::Form;
use crate::persona::Persona;
use crate::session;
use crate::store::{Lookup, Selection};

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route("/", get(picker))
        .route("/pick", post(pick))
        .route("/session", post(session_settings))
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
    let session_id = session::from_headers(&headers);
    let always_ask = session_id.as_deref().is_some_and(|id| {
        state
            .stores
            .sessions
            .lock()
            .expect("sessions")
            .always_ask(id)
    });

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
                    "<div class=\"warn\">\n<h1>That login is no longer in progress</h1>\n\
                     <p>An authorization request is held for five minutes, and lanyard \
                     holds them in memory, so either the five minutes ran out or lanyard \
                     was restarted. Start the login again from your application.</p>\n</div>\n{}",
                    persona_list(&state, None, always_ask)
                ),
            ),
        ),
        (Some(id), Some(Some(client_id))) => html::html(
            StatusCode::OK,
            html::page(
                "lanyard — pick a person",
                &persona_list(&state, Some((id, &client_id)), always_ask),
            ),
        ),
        // The banner's `UI →` line lands here, so it has to be honest about
        // there being nothing to do.
        _ => html::html(
            StatusCode::OK,
            html::page(
                "lanyard — personas",
                &persona_list(&state, None, always_ask),
            ),
        ),
    }
}

/// The list of people, as buttons when a login is waiting on one and as plain
/// cards when nothing is.
fn persona_list(state: &SharedState, req: Option<(&str, &str)>, always_ask: bool) -> String {
    let mut out = String::new();

    match req {
        Some((_, client_id)) => out.push_str(&format!(
            "<h1>Who are you signing in as?</h1>\n\
             <p class=\"lede\"><code>{}</code> is waiting. There is no password — \
             pick a person.</p>\n",
            html::escape(client_id)
        )),
        None => out.push_str(
            "<h1>Personas</h1>\n\
             <p class=\"lede\">No login is in progress. These are the people lanyard \
             can sign you in as; start a login from your application to pick one.</p>\n",
        ),
    }

    for persona in &state.personas.list {
        out.push_str(&persona_row(persona, req.map(|(id, _)| id)));
    }

    out.push_str(&mint_panel(req.map(|(id, _)| id)));
    out.push_str(&always_ask_panel(req.map(|(id, _)| id), always_ask));
    out.push_str(
        "<footer>lanyard · sessions live in memory, so restarting lanyard logs \
         everybody out.</footer>\n",
    );
    out
}

fn persona_row(persona: &Persona, req: Option<&str>) -> String {
    // Every one of these is developer-supplied and every one of them is
    // escaped. A persona file can come out of a fixture generator, and a picker
    // that executes its own persona list is a bad look for a tool whose pitch
    // is "it catches your bugs" (criterion 27).
    let name = match &persona.name {
        Some(name) => format!("<span class=\"name\">{}</span>", html::escape(name)),
        None => "<span class=\"name none\">(no name)</span>".to_string(),
    };
    let email = match &persona.email {
        Some(email) => format!("<span class=\"email\">{}</span>", html::escape(email)),
        None => "<span class=\"email none\">(no email)</span>".to_string(),
    };
    let roles = if persona.roles.is_empty() {
        "<span class=\"roles none\">(no roles)</span>".to_string()
    } else {
        format!(
            "<span class=\"roles\">{}</span>",
            html::escape(&persona.roles.join(", "))
        )
    };
    let inner = format!(
        "{name}\n<span class=\"id\">{}</span>\n{email}\n{roles}\n",
        html::escape(&persona.id)
    );

    match req {
        Some(req) => format!(
            "<form class=\"persona\" method=\"post\" action=\"/_/pick\">\n\
             <input type=\"hidden\" name=\"req\" value=\"{}\">\n\
             <input type=\"hidden\" name=\"persona\" value=\"{}\">\n\
             <button type=\"submit\">\n{inner}</button>\n</form>\n",
            html::escape(req),
            html::escape(&persona.id),
        ),
        None => format!("<div class=\"card persona\">\n{inner}</div>\n"),
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
        "<h2>Or mint one now</h2>\n\
         <form class=\"card\" method=\"post\" action=\"/_/pick\">\n\
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
         <button class=\"submit\" type=\"submit\">Sign in as this person</button>\n\
         <p class=\"lede\">A one-off is not remembered: the next login from this \
         application shows this page again.</p>\n\
         </form>\n",
        html::escape(req),
    )
}

/// Its own little form, on both versions of the page, so the flag can be turned
/// off without starting a login — and so that turning it on does not need
/// JavaScript to also submit whichever persona form the human happens to be
/// looking at.
fn always_ask_panel(req: Option<&str>, always_ask: bool) -> String {
    let checked = if always_ask { " checked" } else { "" };
    let back = match req {
        Some(req) => format!(
            "<input type=\"hidden\" name=\"req\" value=\"{}\">\n",
            html::escape(req)
        ),
        None => String::new(),
    };
    format!(
        "<h2>This browser</h2>\n\
         <form class=\"card\" method=\"post\" action=\"/_/session\">\n{back}\
         <div class=\"check\">\
         <input id=\"always-ask\" type=\"checkbox\" name=\"always_ask\" value=\"1\"{checked}>\
         <label for=\"always-ask\">Always ask which person, even when this browser \
         already chose one for an application</label></div>\n\
         <button class=\"submit\" type=\"submit\">Save</button>\n</form>\n"
    )
}

// -------------------------------------------------------------- the pick --

async fn pick(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Response {
    let form = Form::parse(&body);

    let Some(req) = form.get("req") else {
        return expired_page(&state);
    };

    // Consumed: a pending request is answered once. Reloading the picker peeks;
    // submitting it takes.
    let request = {
        let mut store = state.stores.pending.lock().expect("pending");
        match store.take(req) {
            Lookup::Found(request) => request,
            _ => return expired_page(&state),
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
                            "<div class=\"warn\"><h1>No persona with id <code>{}</code> \
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
                            "<div class=\"warn\"><h1>{}</h1></div>\n",
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
        authorize::complete(&state, request, persona, auth_time),
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

fn expired_page(state: &SharedState) -> Response {
    html::html(
        StatusCode::BAD_REQUEST,
        html::page(
            "lanyard — that login is no longer in progress",
            &format!(
                "<div class=\"warn\">\n<h1>That login is no longer in progress</h1>\n\
                 <p>The request was already answered, or its five minutes ran out, or \
                 lanyard was restarted. Start the login again from your \
                 application.</p>\n</div>\n{}",
                persona_list(state, None, false)
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

    // Back to the page the human was on, with the login still in progress if
    // there was one.
    let back = match form.get("req") {
        Some(req) => format!("/_/?req={req}"),
        None => "/_/".to_string(),
    };
    with_session_cookie(authorize::found(&back), &session_id)
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
