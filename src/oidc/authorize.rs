//! `GET`/`POST /oidc/authorize` — the endpoint where every other IdP keeps a
//! client registry, a redirect allowlist and a consent screen.
//!
//! lanyard has none of the three. Any `client_id`, any `client_secret`, any
//! `redirect_uri` **whose host is loopback**. That single rejection is the whole
//! security boundary (north star 1) and it lives in
//! [`crate::oidc::redirect_uri`].
//!
//! This handler validates and **stores**; `/_/` renders and chooses;
//! `/oidc/token` exchanges. The picker is reached by a redirect rather than
//! rendered here, at the cost of one extra hop, so that `/oidc` stays
//! protocol-only (CONCEPT §3) — and so that the URL a developer is looking at
//! while picking a person is the URL they would visit to look at the persona
//! list.
//!
//! **Two classes of error, and the difference is not cosmetic.** A missing
//! `client_id` or a `redirect_uri` we refused renders a `400` here, because RFC
//! 6749 §4.1.2.1 says an error must not be sent to an address the server has
//! just decided not to trust — doing so would make the one rejection
//! decorative. Everything else redirects, so the RP's own error handling runs.

use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use url::Url;

use crate::app::SharedState;
use crate::log_detail::{attach, LogDetail};
use crate::oidc::code::{Challenge, ChallengeMethod, CodeRecord};
use crate::oidc::redirect_uri;
use crate::oidc::scope::Scopes;
use crate::oidc::token::Form;
use crate::persona::Persona;
use crate::session;
use crate::store::Selection;
use crate::ui::html;

/// How the authorization response gets back to the RP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseMode {
    Query,
    FormPost,
}

/// What the RP asked for about showing a login screen. Anything else in
/// `prompt` — `consent`, a vendor extension — is ignored, per the spec's rule
/// for unrecognized parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// `login` and `select_account` both mean "ask again", and lanyard has one
    /// screen to ask with, so they are one variant.
    Ask,
    /// Render nothing, ever. A code if there is a remembered selection,
    /// `login_required` if there is not.
    None,
}

/// Everything `/authorize` accepted, held under an unguessable id while the
/// human looks at the picker.
#[derive(Debug, Clone)]
pub struct AuthRequest {
    pub client_id: String,
    pub redirect_uri: Url,
    /// Echoed back byte-for-byte when it was sent, and absent when it was not.
    /// Never invented.
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub scope: Scopes,
    /// The raw string, echoed in the token response's `scope`.
    pub scope_raw: Option<String>,
    pub response_mode: ResponseMode,
    pub challenge: Option<Challenge>,
    /// Becomes the **access token's** `aud`, exactly as on Phase 2's grant. Not
    /// the ID token's — that is the `client_id`.
    pub audience: Option<String>,
    pub prompt: Option<Prompt>,
    pub max_age: Option<u64>,
}

pub fn route() -> axum::routing::MethodRouter<SharedState> {
    // Both verbs: OIDC Core §3.1.2.1 allows either, and some SDKs use the
    // second.
    axum::routing::get(authorize_get).post(authorize_post)
}

async fn authorize_get(
    State(state): State<SharedState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    logged(
        &state,
        &headers,
        Form::from_query(query.as_deref().unwrap_or_default()),
    )
}

async fn authorize_post(
    State(state): State<SharedState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    body: Bytes,
) -> Response {
    let form = Form::parse(&body).chain(Form::from_query(query.as_deref().unwrap_or_default()));
    logged(&state, &headers, form)
}

/// The envelope, added on the way out: the `client_id` and every parameter as
/// it was decoded. What only the handler knew — the persona a remembered
/// session resolved to, the host a `redirect_uri` was refused for, the OAuth
/// error a redirect carried — is already attached, and
/// [`LogDetail`]'s first-writer-wins merge leaves it alone.
fn logged(state: &SharedState, headers: &HeaderMap, form: Form) -> Response {
    let mut request = crate::log_detail::request_map(form.pairs());
    // **Decided, not sent.** `response_mode` is absent from most requests and
    // means `query`; a log that echoed the absence would make a developer
    // looking at a `form_post` bug guess at what lanyard chose.
    request
        .entry("response_mode")
        .or_insert_with(|| serde_json::Value::from(mode_of(&form)));

    let client_id = form.get("client_id").map(str::to_string);

    // **Resolved unconditionally, and here rather than deeper in.** `/authorize`
    // is the request that decides whether the picker is needed, so it is the
    // request a warning about the persona sources has to ride back on — and it
    // must do so whether or not this login ever looked a persona up.
    let warnings: Vec<String> = state
        .personas
        .resolve()
        .warnings
        .iter()
        .map(ToString::to_string)
        .collect();

    let response = authorize(state, headers, form);
    attach(
        response,
        LogDetail {
            client_id,
            request: Some(request),
            warnings: (!warnings.is_empty()).then_some(warnings),
            ..LogDetail::default()
        },
    )
}

/// The response mode this request resolves to, for the log. An unsupported
/// value is echoed rather than corrected to `query`: the request is refused,
/// and the log describes the request.
fn mode_of(form: &Form) -> &str {
    form.get("response_mode").unwrap_or("query")
}

fn authorize(state: &SharedState, headers: &HeaderMap, form: Form) -> Response {
    let request = match parse(&form) {
        Ok(request) => request,
        Err(Rejection::Rendered(page)) => return *page,
        Err(Rejection::Redirected(response)) => return *response,
    };

    // **Per `client_id`.** This is CONCEPT §3's multi-project claim becoming
    // observable: three apps on three ports against one instance, and choosing
    // Mira in one does not disturb the Ada the other is logged in as.
    let session_id = session::from_headers(headers);
    let (remembered, always_ask) = match &session_id {
        Some(id) => {
            let sessions = state.stores.sessions.lock().expect("sessions");
            (
                sessions.selection(id, &request.client_id).cloned(),
                sessions.always_ask(id),
            )
        }
        None => (None, false),
    };

    // A remembered selection is usable only if the browser has not asked to be
    // asked, the request has not asked to be asked, and it is not older than
    // the `max_age` the request will accept.
    let usable = remembered.filter(|selection| {
        !always_ask
            && request.prompt != Some(Prompt::Ask)
            && request.max_age.is_none_or(|max_age| {
                state.clock.now().saturating_sub(selection.auth_time) <= max_age
            })
    });

    // **`prompt=none` renders nothing, ever.** Not because silent renew works
    // here — lanyard's `Lax` cookie is not sent on a third-party iframe
    // navigation, so a real SPA renew gets exactly this `login_required` — but
    // because a picker inside a hidden iframe is a login screen nobody can
    // click, and that is the wrong behavior on day one (spec, open question 6).
    //
    // `always_ask` and a stale `max_age` both land here as `login_required`,
    // which is the coherent reading: the answer is "you have to ask", and
    // `prompt=none` is the request not to.
    if request.prompt == Some(Prompt::None) {
        return match usable.and_then(|selection| resolve(state, &request.client_id, &selection)) {
            Some((persona, auth_time)) => with_optional_cookie(
                complete(state, request, persona, auth_time, session_id.clone()),
                session_id,
            ),
            None => redirect_error(
                &request.redirect_uri,
                request.state.as_deref(),
                "login_required",
                "prompt=none was sent and this browser has no usable selection for \
                 this client_id",
            ),
        };
    }

    if let Some((persona, auth_time)) =
        usable.and_then(|selection| resolve(state, &request.client_id, &selection))
    {
        return with_optional_cookie(
            complete(state, request, persona, auth_time, session_id.clone()),
            session_id,
        );
    }

    let id = state
        .stores
        .pending
        .lock()
        .expect("pending store lock")
        .insert(request);

    // Relative on purpose: lanyard's issuer is fixed configuration, but the
    // address a browser reached it at may not be the issuer's, and a redirect to
    // the picker must land on the host the human is already looking at.
    found(&format!("/_/?req={id}"))
}

/// A remembered selection names a persona by id, and the persona list can have
/// changed under it — a file edited, or lanyard restarted with a different one.
/// A selection that no longer names anybody falls through to the picker rather
/// than failing the login.
/// Scoped by the request's own `client_id`: a persona that has since been
/// scoped to a *different* application no longer names anybody **for this
/// login**, and falls through to the picker like any other id that stopped
/// resolving.
fn resolve(state: &SharedState, client_id: &str, selection: &Selection) -> Option<(Persona, u64)> {
    state
        .personas
        .resolve()
        .get(&selection.persona_id, Some(client_id))
        .map(|sourced| (sourced.persona.clone(), selection.auth_time))
}

/// Mint the code and hand back the authorization response. Both the picker's
/// `POST /_/pick` and the remembered-session skip above end here, so a login
/// that showed the picker and one that did not are byte-identical to the RP.
pub fn complete(
    state: &SharedState,
    request: AuthRequest,
    persona: Persona,
    auth_time: u64,
    session_id: Option<String>,
) -> Response {
    let code = state
        .stores
        .codes
        .lock()
        .expect("codes")
        .insert(CodeRecord {
            persona: persona.clone(),
            auth_time,
            request: request.clone(),
            // Carried through the code so a refresh token minted from it knows
            // which browser session **Log out** revokes it with.
            session_id,
        });
    // **Who this login is for.** Attached here rather than at either caller,
    // because both of them end here: a login that showed the picker and one
    // that used a remembered selection are byte-identical to the RP, and they
    // are one line apart in the log for the same reason.
    attach(
        deliver(&request, &code),
        LogDetail {
            client_id: Some(request.client_id.clone()),
            detail: Some(
                [("persona".to_string(), Value::from(persona.id.clone()))]
                    .into_iter()
                    .collect(),
            ),
            ..LogDetail::default()
        },
    )
}

fn with_optional_cookie(response: Response, session_id: Option<String>) -> Response {
    match session_id {
        Some(id) => crate::ui::with_session_cookie(response, &id),
        None => response,
    }
}

/// Either a page lanyard renders itself, or a redirect carrying an OAuth error
/// to an address it has decided to trust.
/// Boxed because both variants are a whole `Response`, and an unboxed one makes
/// every `?` in `parse` move 128 bytes on the happy path.
enum Rejection {
    Rendered(Box<Response>),
    Redirected(Box<Response>),
}

/// [`rejected`] always builds a [`Rejection::Rendered`]; this unwraps it so the
/// one call site that has more to say can attach to the page it built.
fn unwrap_rendered(rejection: Rejection) -> Box<Response> {
    match rejection {
        Rejection::Rendered(page) => page,
        Rejection::Redirected(response) => response,
    }
}

fn parse(form: &Form) -> Result<AuthRequest, Rejection> {
    // ---- the two that never redirect (RFC 6749 §4.1.2.1) -------------------

    let client_id = form.get("client_id").ok_or_else(|| {
        rejected(
            "lanyard needs a client_id",
            None,
            "lanyard does not register clients, so any client_id at all is accepted — \
             but one has to be sent. It is what a browser session is keyed on, and an \
             app that omits it would share a session with every other app.",
        )
    })?;

    let raw_redirect = form.get("redirect_uri").ok_or_else(|| {
        rejected(
            "lanyard needs a redirect_uri",
            None,
            &redirect_uri::rule("redirect_uri"),
        )
    })?;

    // The parser's own sentence is carried into the page as well as the rule,
    // because it covers what the one-line rule does not — a URL that will not
    // parse at all, or a scheme rather than a host.
    let redirect_uri = redirect_uri::check("redirect_uri", raw_redirect).map_err(|message| {
        // **Criterion 12.** Nothing was redirected and the RP was never
        // contacted, so the log is the only place this failure exists at all.
        // The host is named separately from the whole URL because the host is
        // what the rule is about.
        let host = Url::parse(raw_redirect)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string));
        Rejection::Rendered(Box::new(attach(
            *unwrap_rendered(rejected(
                &message,
                Some(raw_redirect),
                &redirect_uri::rule("redirect_uri"),
            )),
            LogDetail {
                error: Some("invalid_request".to_string()),
                error_description: Some(message.clone()),
                detail: Some(
                    [(
                        "redirect_uri".to_string(),
                        serde_json::json!({
                            "presented": raw_redirect,
                            "host": host,
                        }),
                    )]
                    .into_iter()
                    .collect(),
                ),
                ..LogDetail::default()
            },
        )))
    })?;

    // ---- everything below here redirects ----------------------------------

    let state = form.get("state").map(str::to_string);
    let fail = |error: &str, description: String| {
        Rejection::Redirected(Box::new(redirect_error(
            &redirect_uri,
            state.as_deref(),
            error,
            &description,
        )))
    };

    // A request object is a whole feature, and pretending to accept one would
    // mean silently ignoring every parameter inside it.
    if form.get("request").is_some() {
        return Err(fail(
            "request_not_supported",
            "lanyard does not accept a request object; send the parameters in the query"
                .to_string(),
        ));
    }
    if form.get("request_uri").is_some() {
        return Err(fail(
            "request_uri_not_supported",
            "lanyard does not fetch a request object; send the parameters in the query".to_string(),
        ));
    }

    match form.get("response_type") {
        Some("code") => {}
        Some(other) => {
            // Phase 0's obligation #4: .NET's default `ResponseType` is
            // `id_token`, so a `dotnet new` app with otherwise correct settings
            // sends the implicit flow and gets a bare protocol noun back. The
            // sentence is emitted only when `id_token` was actually asked for,
            // because that is the one case where it is the diagnosis rather
            // than a guess about which stack is calling.
            let dotnet = if other.split_whitespace().any(|t| t == "id_token") {
                " In .NET set options.ResponseType = \"code\"."
            } else {
                ""
            };
            return Err(fail(
                "unsupported_response_type",
                format!(
                    "response_type {other:?} is not supported; lanyard implements the \
                     authorization code flow only.{dotnet}"
                ),
            ));
        }
        None => {
            return Err(fail(
                "invalid_request",
                "response_type is required, and lanyard implements the authorization \
                 code flow only, so it has to be \"code\""
                    .to_string(),
            ))
        }
    }

    let response_mode = match form.get("response_mode") {
        None | Some("query") => ResponseMode::Query,
        Some("form_post") => ResponseMode::FormPost,
        Some(other) => {
            return Err(fail(
                "unsupported_response_mode",
                format!(
                    "response_mode {other:?} is not supported; lanyard supports \
                     query and form_post"
                ),
            ))
        }
    };

    // A method without a challenge is a half-supplied PKCE, and accepting it
    // would mean silently minting a code nothing can redeem.
    let challenge = match (
        form.get("code_challenge"),
        form.get("code_challenge_method"),
    ) {
        (None, None) => None,
        (None, Some(method)) => {
            return Err(fail(
                "invalid_request",
                format!("code_challenge_method {method:?} was sent without a code_challenge"),
            ))
        }
        (Some(value), method) => match ChallengeMethod::parse(method) {
            Ok(method) => Some(Challenge {
                value: value.to_string(),
                method,
            }),
            Err(message) => return Err(fail("invalid_request", message)),
        },
    };

    let max_age = match form.get("max_age") {
        None => None,
        Some(raw) => match raw.parse::<u64>() {
            Ok(seconds) => Some(seconds),
            Err(_) => {
                return Err(fail(
                    "invalid_request",
                    format!("max_age must be a whole number of seconds, got {raw:?}"),
                ))
            }
        },
    };

    let scope_raw = form.get("scope").map(str::to_string);

    Ok(AuthRequest {
        client_id: client_id.to_string(),
        redirect_uri,
        state,
        nonce: form.get("nonce").map(str::to_string),
        scope: Scopes::parse(scope_raw.as_deref()),
        scope_raw,
        response_mode,
        challenge,
        // Both spellings: `audience` is the Auth0 convention most developers
        // have seen, `resource` is RFC 8707's. Same as Phase 2's grant.
        audience: form
            .get("audience")
            .or_else(|| form.get("resource"))
            .map(str::to_string),
        prompt: match form.get("prompt") {
            Some("login") | Some("select_account") => Some(Prompt::Ask),
            Some("none") => Some(Prompt::None),
            _ => None,
        },
        max_age,
    })
}

/// The successful authorization response, delivered the way the request asked
/// for it.
///
/// Both `/_/pick` and `/authorize`'s remembered-session skip end here, so a
/// login that showed the picker and one that did not produce a byte-identical
/// response to the RP.
pub fn deliver(request: &AuthRequest, code: &str) -> Response {
    match request.response_mode {
        ResponseMode::Query => {
            let mut url = request.redirect_uri.clone();
            {
                let mut pairs = url.query_pairs_mut();
                pairs.append_pair("code", code);
                if let Some(state) = &request.state {
                    pairs.append_pair("state", state);
                }
            }
            found(url.as_str())
        }
        ResponseMode::FormPost => {
            let mut fields = vec![("code", code)];
            if let Some(state) = &request.state {
                fields.push(("state", state.as_str()));
            }
            crate::ui::html::html(
                StatusCode::OK,
                crate::ui::form_post::page(request.redirect_uri.as_str(), &fields),
            )
        }
    }
}

/// A `302` back to the RP carrying an OAuth error, so its own error handling
/// runs rather than the developer seeing a lanyard page for a problem in their
/// app.
///
/// Errors always go back as query parameters, even when `response_mode` asked
/// for `form_post`: an error page that auto-submits itself is harder to read
/// than a URL, and half of these errors are *about* the response mode.
pub fn redirect_error(
    redirect_uri: &Url,
    state: Option<&str>,
    error: &str,
    description: &str,
) -> Response {
    let mut url = redirect_uri.clone();
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("error", error);
        pairs.append_pair("error_description", description);
        // Only when one was sent. Never invented — an RP that did not send
        // `state` and receives one has been told something untrue about its own
        // request.
        if let Some(state) = state {
            pairs.append_pair("state", state);
        }
    }
    // The RP will read this out of its query string; the developer reads it off
    // the log without having to look in a browser's address bar first.
    attach(
        found(url.as_str()),
        LogDetail {
            error: Some(error.to_string()),
            error_description: Some(description.to_string()),
            ..LogDetail::default()
        },
    )
}

/// `302 Found`, spelled out rather than via `Redirect`, which is `303` in axum
/// and would turn the acceptance criteria's `302` into a lie.
pub fn found(location: &str) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
}

/// The rendered `400`, wrapped as a [`Rejection`]. The page itself is
/// [`html::rejected_page`], which `/oidc/end_session` renders too — the one
/// rejection has one look as well as one function.
fn rejected(headline: &str, value: Option<&str>, rule: &str) -> Rejection {
    Rejection::Rendered(Box::new(html::rejected_page(
        headline,
        value,
        rule,
        // RFC 6749 §4.1.2.1: an authorization server must not redirect an error
        // to a `redirect_uri` it has not accepted. Redirecting to an address you
        // have just decided not to trust is what would make the boundary
        // decorative.
        "Nothing was sent to that address. RFC 6749 §4.1.2.1 says an authorization \
         server must not redirect an error to a redirect_uri it has not accepted, and \
         this is the one thing lanyard does not accept.",
    )))
}
