//! `POST /oidc/token` — the OAuth 2.0 token endpoint.
//!
//! **This file builds no claims.** It parses a form, maps a handful of
//! parameters into an overrides map, and calls [`crate::oidc::issue`] — the same
//! function, with the same persona→claims table, that the test seam calls. Any
//! claim shaping that appears here is the drift north star 3 exists to prevent.
//!
//! The form is parsed by hand rather than through axum's `Form` extractor for
//! the same reason [`crate::seam::parse_claims`] hand-rolls its JSON: every 4xx
//! on this endpoint has to be ours and carry the OAuth
//! `{"error", "error_description"}` shape, not axum's 415 or 422.
//!
//! Dispatch on `grant_type` has three arms, each added as an arm rather than as
//! a second handler — which is the point. A browser login, a
//! `client_credentials` grant and a refresh all mint through the same `issue()`
//! call with the same claim table, and criterion 23 is the assertion that they
//! still do.
//!
//! Phase 3's `flaw` parameter is parsed here and applied nowhere: it is turned
//! into a [`Flaw`] and handed to the same one function. The response says
//! nothing about it — an SDK reading this body should see nothing unusual,
//! because the whole point is that the token looks ordinary until it is
//! validated.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine as _;
use serde_json::{json, Map, Value};

use crate::app::SharedState;
use crate::log_detail::{attach, LogDetail};
use crate::oidc::flaw::Flaw;
use crate::oidc::issue::{self, DEFAULT_TTL, ID_TOKEN_TTL};
use crate::oidc::scope::{ClaimFilter, Scopes, OFFLINE_ACCESS};
use crate::store::{Issuance, Lookup, RefreshRecord};

/// Named once so the two "what is supported" messages cannot drift apart.
const SUPPORTED: &str = "client_credentials, authorization_code and refresh_token";

pub fn route() -> axum::routing::MethodRouter<SharedState> {
    // `post` alone: axum answers `GET /oidc/token` with 405 and an `Allow`
    // header, which is the correct answer and not one we have to write.
    axum::routing::post(token)
}

async fn token(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Response {
    let form = Form::parse(&body);
    let response = dispatch(&state, &headers, &form);

    // **The envelope, added on the way out.** The arms have already attached
    // whatever only they knew — the reason for a refusal, the client off a code
    // record, the tokens that were minted — and [`LogDetail`]'s first-writer-
    // wins merge means none of it is disturbed by this.
    attach(
        response,
        LogDetail {
            client_id: form
                .get("client_id")
                .map(str::to_string)
                .or_else(|| basic_client_id(&headers)),
            grant_type: form.get("grant_type").map(str::to_string),
            request: Some(crate::log_detail::request_map(form.pairs())),
            flaw: form.get("flaw").map(str::to_string),
            ..LogDetail::default()
        },
    )
}

fn dispatch(state: &SharedState, headers: &HeaderMap, form: &Form) -> Response {
    // An empty `grant_type=` is the same mistake as no `grant_type` at all, and
    // both are `invalid_request` rather than `unsupported_grant_type`: nothing
    // was named, so nothing can be unsupported.
    match form.get("grant_type") {
        None => bad_request(
            "invalid_request",
            format!("grant_type is required; this build supports {SUPPORTED}"),
        ),
        Some("client_credentials") => client_credentials(state, headers, form),
        Some("authorization_code") => authorization_code(state, form),
        Some("refresh_token") => refresh_token(state, form),
        Some(other) => bad_request(
            "unsupported_grant_type",
            format!("grant_type {other:?} is not supported; this build supports {SUPPORTED}"),
        ),
    }
}

/// Client authentication is accepted in **any** form — HTTP Basic, form
/// parameters, or entirely absent — and checked in none. That is north star 1
/// arriving at the endpoint where every other IdP would put a client registry,
/// and it is what produces the multi-project property.
fn client_credentials(state: &SharedState, headers: &HeaderMap, form: &Form) -> Response {
    let client_id = form
        .get("client_id")
        .map(str::to_string)
        .or_else(|| basic_client_id(headers));

    // **Scoped like the picker is.** `lanyard token` mints from the same set
    // the picker shows, which is the whole reason `--client` exists.
    let people = state.personas.resolve();
    let persona = match form.get("persona") {
        Some(id) => match people.get(id, client_id.as_deref()) {
            Some(sourced) => Some(&sourced.persona),
            // **The useful refusal**, through the funnel every other one goes
            // through: a persona that exists but belongs to another
            // application says so, and names the flag that reaches it.
            None => {
                return bad_request(
                    "invalid_request",
                    people.missing_persona(id, client_id.as_deref()),
                )
            }
        },
        None => None,
    };

    // A known parameter with an unrecognized value is a refusal, unlike an
    // unknown parameter, which OAuth says to ignore and this endpoint does:
    // `flaw=expried` quietly minting a good token is a negative test suite
    // reporting a pass and proving nothing.
    let flaw = match form.get("flaw") {
        Some(raw) => match Flaw::parse(raw) {
            Ok(flaw) => Some(flaw),
            Err(message) => return bad_request("invalid_request", message),
        },
        None => None,
    };

    let mut overrides = Map::new();
    // `audience` is the Auth0 convention most developers have seen; `resource`
    // is the standards-track one (RFC 8707). Accepting both means a real SDK
    // works whichever it sends. Only when asked for: a token with no `aud` is
    // legitimate and useful — it is what an API that forgets to validate
    // audience will happily accept.
    if let Some(audience) = form.get("audience").or_else(|| form.get("resource")) {
        overrides.insert("aud".into(), Value::from(audience));
    }
    let scope = form.get("scope");
    if let Some(scope) = scope {
        overrides.insert("scope".into(), Value::from(scope));
    }
    // Emitted only when the request supplied one, matching what a real
    // `client_credentials` token carries and giving Phase 6's log and Phase 7's
    // namespacing something honest to key on.
    if let Some(client_id) = &client_id {
        overrides.insert("client_id".into(), Value::from(client_id.clone()));
    }

    match issue::issue(
        &state.key,
        state.clock,
        &state.config.issuer,
        persona,
        &overrides,
        DEFAULT_TTL,
        flaw,
        // Access tokens are never scope-filtered: OIDC Core §5.4 specifies
        // claims-per-scope for the ID token and UserInfo, and nothing specifies
        // it here. Phase 2's contract stays byte-identical.
        &ClaimFilter::Unfiltered,
    ) {
        Ok(issued) => {
            // The token's actual remaining life, floored at zero, rather than
            // the TTL it was asked for: `flaw=expired` reporting 60 seconds it
            // does not have is the one place this envelope could lie. Computed
            // from the moment `issue` used, because reading the clock a second
            // time here makes the ordinary case report 59 or 61 at random.
            let expires_in = issued
                .claims
                .get("exp")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .saturating_sub(issued.issued_at);
            let mut body = json!({
                "access_token": issued.token,
                "token_type": "Bearer",
                "expires_in": expires_in,
            });
            // Present only when one was asked for. No `id_token`: client
            // credentials has no user authentication event, so returning one
            // would be a lie an SDK might believe. No `refresh_token` either.
            if let Some(scope) = scope {
                body["scope"] = Value::from(scope);
            }
            let response = ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response();
            attach(response, minted(&[("access_token", &issued.token)]))
        }
        Err(message) => issuance_failed(message),
    }
}

/// The `issued` payload: the decoded header and payload of every token that
/// came out, keyed by the response field it was returned as.
fn minted(tokens: &[(&str, &str)]) -> LogDetail {
    let mut issued = Map::new();
    for (name, jwt) in tokens {
        if let Some(decoded) = crate::log_detail::decoded_token(jwt) {
            issued.insert((*name).to_string(), decoded);
        }
    }
    LogDetail {
        issued: Some(issued),
        ..LogDetail::default()
    }
}

/// The browser flow's other end. The human already clicked; this redeems what
/// that click produced.
///
/// **Six distinct `invalid_grant` descriptions**, because Phase 6's log has to
/// be able to say "which parameter mismatched" and it can only report what this
/// phase distinguishes. Every one of them is a real check: PKCE, single use,
/// expiry and the `redirect_uri` match cost nothing and are exactly the checks
/// whose absence makes a mock diverge from the real thing.
fn authorization_code(state: &SharedState, form: &Form) -> Response {
    let Some(code) = form.get("code") else {
        return bad_request(
            "invalid_request",
            "code is required for grant_type=authorization_code".to_string(),
        );
    };

    let record = {
        let mut codes = state.stores.codes.lock().expect("codes");
        match codes.take(code) {
            Lookup::Found(record) => record,
            Lookup::Spent => {
                return invalid_grant(
                    "that authorization code has already been exchanged; \
                     codes are single-use",
                )
            }
            Lookup::Expired => {
                return invalid_grant(
                    "that authorization code has expired; codes live 60 seconds, \
                     which is the redirect and the exchange and nothing else",
                )
            }
            Lookup::Unknown => {
                return invalid_grant(
                    "no such authorization code; it was never issued, or lanyard \
                     was restarted since it was",
                )
            }
        }
    };

    // Compared as URLs rather than as strings, so a client that reassembles the
    // value it sent does not fail on a trailing slash it never meant.
    //
    // **Absent is not a mismatch.** RFC 6749 §4.1.3 says to require it, but
    // requiring a parameter an SDK chose not to resend is a registration-shaped
    // "no" on an endpoint that has none, and it catches no bug. A *wrong* one
    // does catch a bug, and that is the case below.
    if let Some(presented) = form.get("redirect_uri") {
        let matches = url::Url::parse(presented)
            .map(|url| url == record.request.redirect_uri)
            .unwrap_or(false);
        if !matches {
            return invalid_grant_for(
                &record.request.client_id,
                &format!(
                    "redirect_uri {presented:?} does not match the one this code was \
                     issued against, {:?}",
                    record.request.redirect_uri.as_str()
                ),
            );
        }
    }

    // **PKCE is genuinely verified, and this is not a contradiction of north
    // star 1.** Accept-everything is about registration — who you say you are,
    // where you say you want to come back to. It was never about skipping the
    // cryptography. A local IdP that rubber-stamps a wrong `code_verifier` lets
    // a broken PKCE implementation ship.
    if let Some(challenge) = &record.request.challenge {
        let Some(verifier) = form.get("code_verifier") else {
            return invalid_grant_for(
                &record.request.client_id,
                &format!(
                    "this code was issued against a {} code_challenge, so a \
                     code_verifier is required",
                    challenge.method.as_str()
                ),
            );
        };
        if !challenge.verify(verifier) {
            // **The three values, side by side.** The sentence says which
            // parameter is wrong; this says *how* — and the recorded and
            // computed challenges differ visibly at the character the typo is
            // in. That is criterion 10, and it is the reason this phase exists.
            let response = invalid_grant_for(
                &record.request.client_id,
                &format!(
                    "the code_verifier does not match the {} code_challenge this code \
                     was issued against",
                    challenge.method.as_str()
                ),
            );
            return attach(
                response,
                LogDetail {
                    detail: Some(
                        json!({
                            "pkce": {
                                "method": challenge.method.as_str(),
                                "verifier_presented": verifier,
                                "challenge_computed": challenge.transform(verifier),
                                "challenge_recorded": challenge.value,
                            }
                        })
                        .as_object()
                        .cloned()
                        .expect("a JSON object literal"),
                    ),
                    ..LogDetail::default()
                },
            );
        }
    }
    // A `code_verifier` sent when no challenge was recorded is ignored, exactly
    // as any other unrecognized parameter is.

    let request = &record.request;

    let mut overrides = Map::new();
    // The access token's `aud` is the API audience the authorization request
    // named — **not** the `client_id`. The `client_id` is the ID token's `aud`,
    // and conflating the two is the mistake this comment exists to prevent.
    if let Some(audience) = &request.audience {
        overrides.insert("aud".into(), Value::from(audience.clone()));
    }
    if let Some(scope) = &request.scope_raw {
        overrides.insert("scope".into(), Value::from(scope.clone()));
    }
    overrides.insert("client_id".into(), Value::from(request.client_id.clone()));

    let access = match issue::issue(
        &state.key,
        state.clock,
        &state.config.issuer,
        Some(&record.persona),
        &overrides,
        DEFAULT_TTL,
        // Phase 3's `flaw` is out of scope on this arm: the spec defers a
        // flawed ID token to a later phase, and a flawed access token from a
        // browser login has no way to be asked for.
        None,
        // Not filtered. OIDC Core §5.4 specifies claims-per-scope for the ID
        // token and UserInfo and says nothing about an access token — so this
        // stays byte-identical to what `lanyard token --as ada` mints, which is
        // criterion 23.
        &ClaimFilter::Unfiltered,
    ) {
        Ok(issued) => issued,
        Err(message) => return issuance_failed(message),
    };

    let expires_in = access
        .claims
        .get("exp")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .saturating_sub(access.issued_at);

    let mut body = json!({
        "access_token": access.token,
        "token_type": "Bearer",
        "expires_in": expires_in,
    });
    if let Some(scope) = &request.scope_raw {
        body["scope"] = Value::from(scope.clone());
    }

    // **A second call to the same function**, made after the access token
    // because `at_hash` is a hash *of* the access token. Two calls, one
    // `issue()`, one persona→claims table — which is north star 3's whole claim,
    // and criterion 23 is what fails if an ID token's claims ever get assembled
    // anywhere else.
    //
    // Only when `openid` was asked for. A request without it is a plain OAuth
    // 2.0 code flow and gets a plain OAuth 2.0 response, which is honest and is
    // a thing worth being able to test.
    //
    // `Some(code)`: `c_hash` is emitted although OIDC Core requires it only for
    // `code id_token`. It costs a hash, and an RP that validates it should find
    // it valid — which is a better test of that RP than never exercising the
    // path.
    if request.scope.is_openid() {
        let grant = Grant {
            persona: &record.persona,
            client_id: &request.client_id,
            scope: &request.scope,
            auth_time: record.auth_time,
            nonce: request.nonce.as_deref(),
        };
        match id_token(state, &grant, &access.token, Some(code)) {
            Ok(token) => body["id_token"] = Value::from(token),
            Err(message) => return issuance_failed(message),
        }
    }

    // **`offline_access` gates the refresh token exactly as `openid` gates the
    // ID token.** Auth0, Okta and Entra all require it; an app that forgets it
    // in production gets no refresh token and breaks, and a lanyard that hands
    // one over unasked hides exactly that bug.
    if request.scope.has(OFFLINE_ACCESS) {
        let refresh = state
            .stores
            .refresh
            .lock()
            .expect("refresh")
            .insert(RefreshRecord {
                persona: record.persona.clone(),
                client_id: request.client_id.clone(),
                scope: request.scope.clone(),
                scope_raw: request.scope_raw.clone(),
                audience: request.audience.clone(),
                // The **original** authentication, carried forward unchanged: a
                // refresh is not a new login, and OIDC Core §12.2 says so.
                auth_time: record.auth_time,
                nonce: request.nonce.clone(),
                session_id: record.session_id.clone(),
            });
        body["refresh_token"] = Value::from(refresh);
    }

    record_issuance(
        state,
        record.session_id.as_deref(),
        &request.client_id,
        &access,
    );

    // **The `client_id` comes off the code record, not off a form field.** RFC
    // 6749 does not require an RP to resend it on the exchange, and several do
    // not — reading the form here would leave the exchange half of a login
    // unlabelled in a log whose whole readability rests on that column.
    let mut detail = minted_pair(&access.token, body.get("id_token").and_then(Value::as_str));
    detail.client_id = Some(request.client_id.clone());
    attach(
        ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response(),
        detail,
    )
}

/// An access token and, when `openid` was asked for, the ID token beside it.
fn minted_pair(access: &str, id_token: Option<&str>) -> LogDetail {
    match id_token {
        Some(id_token) => minted(&[("access_token", access), ("id_token", id_token)]),
        None => minted(&[("access_token", access)]),
    }
}

/// Note what a browser session was just issued, so **Expire now** has something
/// to revoke. A grant with no session — a curl-only code flow — records
/// nothing, which is not an error: there is no browser to expire it from.
fn record_issuance(
    state: &SharedState,
    session_id: Option<&str>,
    client_id: &str,
    access: &issue::Issued,
) {
    let Some(session_id) = session_id else {
        return;
    };
    let (Some(jti), Some(exp)) = (
        access.claims.get("jti").and_then(Value::as_str),
        access.claims.get("exp").and_then(Value::as_u64),
    ) else {
        return;
    };
    state
        .stores
        .sessions
        .lock()
        .expect("sessions")
        .record_issuance(
            session_id,
            Issuance {
                client_id: client_id.to_string(),
                jti: jti.to_string(),
                exp,
            },
            access.issued_at,
        );
}

/// **The third arm, and the third caller of [`issue::issue`].** OIDC Core §12.2.
///
/// Client authentication is accepted in any form and checked in none, as on the
/// other two arms. Four distinct `invalid_grant` descriptions — unknown,
/// expired, already exchanged, revoked — because those are four different
/// things a developer needs told apart, and Phase 6's log can only report what
/// this phase distinguishes.
fn refresh_token(state: &SharedState, form: &Form) -> Response {
    let Some(presented) = form.get("refresh_token") else {
        return bad_request(
            "invalid_request",
            "refresh_token is required for grant_type=refresh_token".to_string(),
        );
    };

    // **Revocation is checked before the store**, because revoking removes the
    // record: asking the store first would answer "no such refresh token" about
    // one that was deliberately killed, which is the one thing criterion 18 is
    // about.
    if crate::oidc::revocation::is_revoked(state, presented) {
        return invalid_grant(
            "that refresh token has been revoked, either at /oidc/revoke or by \
             logging out of lanyard",
        );
    }

    // **Peeked, not taken.** The scope check below can still refuse, and a
    // refusal must not spend the token: an app that asked for the wrong scope
    // gets to fix its request and try again with the token it already holds.
    // The exchange happens further down, once nothing is left to refuse.
    let record = {
        let refresh = state.stores.refresh.lock().expect("refresh");
        match refresh.peek(presented) {
            Lookup::Found(record) => record.clone(),
            Lookup::Spent => {
                return invalid_grant(
                    "that refresh token has already been exchanged; lanyard rotates \
                     refresh tokens, so each one works once and the response carries \
                     its replacement",
                )
            }
            Lookup::Expired => {
                return invalid_grant(
                    "that refresh token has expired; refresh tokens live eight hours, \
                     which is a working day",
                )
            }
            Lookup::Unknown => {
                return invalid_grant(
                    "no such refresh token; it was never issued, or lanyard was \
                     restarted since it was",
                )
            }
        }
    };

    // **Scope may narrow and may not widen** (RFC 6749 §6). An absent `scope`
    // means the whole grant, which is what an SDK that sends none means. A scope
    // that was never granted names itself in the refusal, because a developer
    // reading `invalid_scope` has to know which of the values they sent was the
    // problem. Same class of check as PKCE: free, catches a real bug, and
    // nothing to do with registration.
    let (scope, scope_raw) = match form.get("scope") {
        None => (record.scope.clone(), record.scope_raw.clone()),
        Some(requested) => {
            let requested = Scopes::parse(Some(requested));
            if let Some(ungranted) = record.scope.missing_from(&requested) {
                return bad_request(
                    "invalid_scope",
                    format!(
                        "scope {ungranted:?} was not granted to this refresh token, and a \
                         refresh may narrow a grant but not widen it (RFC 6749 §6); this \
                         grant is {:?}",
                        record.scope.to_raw()
                    ),
                );
            }
            let raw = requested.to_raw();
            (requested, Some(raw))
        }
    };

    let mut overrides = Map::new();
    // The stored audience, and the scope this refresh is for: a refresh mints
    // the same grant again, or a narrower one, never a new one.
    if let Some(audience) = &record.audience {
        overrides.insert("aud".into(), Value::from(audience.clone()));
    }
    if let Some(scope_raw) = &scope_raw {
        overrides.insert("scope".into(), Value::from(scope_raw.clone()));
    }
    overrides.insert("client_id".into(), Value::from(record.client_id.clone()));

    let access = match issue::issue(
        &state.key,
        state.clock,
        &state.config.issuer,
        Some(&record.persona),
        &overrides,
        DEFAULT_TTL,
        None,
        &ClaimFilter::Unfiltered,
    ) {
        Ok(issued) => issued,
        Err(message) => return issuance_failed(message),
    };

    let expires_in = access
        .claims
        .get("exp")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .saturating_sub(access.issued_at);

    let mut body = json!({
        "access_token": access.token,
        "token_type": "Bearer",
        "expires_in": expires_in,
    });
    if let Some(scope_raw) = &scope_raw {
        body["scope"] = Value::from(scope_raw.clone());
    }

    let grant = Grant {
        persona: &record.persona,
        client_id: &record.client_id,
        // The **narrowed** set, so it filters the ID token's claims too: a
        // refresh that dropped `email` must not hand one back.
        scope: &scope,
        auth_time: record.auth_time,
        nonce: record.nonce.as_deref(),
    };
    // **No `c_hash`**: there is no code this time. Everything else is the code
    // arm's ID token, built by the same helper so the two cannot drift.
    if scope.is_openid() {
        match id_token(state, &grant, &access.token, None) {
            Ok(token) => body["id_token"] = Value::from(token),
            Err(message) => return issuance_failed(message),
        }
    }

    // **Rotation**, and the moment the presented token is spent: nothing above
    // can refuse any more, so taking it here is what makes a refusal cost
    // nothing and a success cost exactly one token. The replacement carries the
    // narrowed grant — a narrowing is permanent, which is RFC 6749 §6's reading
    // and the shape an app has to handle against Auth0 and Okta too.
    let rotated = {
        let mut refresh = state.stores.refresh.lock().expect("refresh");
        refresh.take(presented);
        refresh.insert(RefreshRecord {
            scope: scope.clone(),
            scope_raw: scope_raw.clone(),
            ..record.clone()
        })
    };
    body["refresh_token"] = Value::from(rotated);

    let mut detail = minted_pair(&access.token, body.get("id_token").and_then(Value::as_str));
    // As on the code arm: the client comes off the stored grant. A refresh is
    // the request least likely to name itself.
    detail.client_id = Some(record.client_id.clone());

    record_issuance(
        state,
        record.session_id.as_deref(),
        &record.client_id,
        &access,
    );

    attach(
        ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response(),
        detail,
    )
}

/// What both user-facing grants have in common: who authenticated, for which
/// application, when, and under which scope.
///
/// It exists so the ID token is assembled **once**. The code arm builds one of
/// these from an [`crate::oidc::authorize::AuthRequest`] and a
/// [`crate::oidc::code::CodeRecord`]; the refresh arm builds one from a
/// [`RefreshRecord`]. Two assemblies of the same claim set is exactly the drift
/// north star 3 exists to prevent, and it would be invisible until the day an
/// RP compared a refreshed ID token against the original.
struct Grant<'a> {
    persona: &'a crate::persona::Persona,
    client_id: &'a str,
    scope: &'a crate::oidc::scope::Scopes,
    auth_time: u64,
    nonce: Option<&'a str>,
}

/// **A second call to the same `issue()`**, made after the access token because
/// `at_hash` is a hash *of* the access token.
///
/// `code` is `Some` only on the authorization code arm. OIDC Core requires
/// `c_hash` only for `code id_token`, and lanyard emits it anyway there because
/// it costs a hash and an RP that validates it should find it valid — but a
/// refresh has no code, so inventing a `c_hash` would be a hash of nothing.
fn id_token(
    state: &SharedState,
    grant: &Grant,
    access_token: &str,
    code: Option<&str>,
) -> Result<String, String> {
    let mut registered = Map::new();
    // **The client_id, not the API audience.** An ID token says "this person
    // authenticated to you"; an access token says "bearer may act at that API".
    // Conflating the two is the mistake this line exists to not make.
    registered.insert("aud".into(), Value::from(grant.client_id.to_string()));
    // May legitimately predate `iat` — that is what a remembered session means,
    // and on a refresh it is the *original* authentication, which OIDC Core
    // §12.2 requires.
    registered.insert("auth_time".into(), Value::from(grant.auth_time));
    if let Some(nonce) = grant.nonce {
        registered.insert("nonce".into(), Value::from(nonce.to_string()));
    }
    registered.insert(
        "at_hash".into(),
        Value::from(issue::half_hash(access_token)),
    );
    if let Some(code) = code {
        registered.insert("c_hash".into(), Value::from(issue::half_hash(code)));
    }

    // The ID token also carries `nbf` and `jti`, because `claims_at` adds them
    // to everything. Both are legal registered claims, and stripping them would
    // mean a second claim path for the sake of two fields no relying party
    // objects to. Decided, not accidental.
    issue::issue(
        &state.key,
        state.clock,
        &state.config.issuer,
        Some(grant.persona),
        &registered,
        ID_TOKEN_TTL,
        None,
        &ClaimFilter::ByScope(grant.scope.clone()),
    )
    .map(|issued| issued.token)
}

/// `400` in the OAuth shape, with a description that names what failed.
fn invalid_grant(description: &str) -> Response {
    bad_request("invalid_grant", description.to_string())
}

/// The same refusal, **labelled with the application it was for**.
///
/// Once a code record has been taken, lanyard knows whose login just failed
/// even though the exchange form never said — and "`client_id` on every event,
/// always" is what makes the log readable when three projects are running at
/// once. A refusal that dropped the label would be the one row a developer
/// could not attribute, on the page they opened to attribute it.
fn invalid_grant_for(client_id: &str, description: &str) -> Response {
    attach(
        invalid_grant(description),
        LogDetail {
            client_id: Some(client_id.to_string()),
            ..LogDetail::default()
        },
    )
}

/// The other way out that is not a success. Logged for `bad_request`'s reason:
/// an error nobody can see is the failure this phase exists to end, and a `500`
/// is the one a developer is least equipped to guess at.
fn issuance_failed(message: String) -> Response {
    let response = (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": "issuance_failed", "error_description": message.clone() })),
    )
        .into_response();
    attach(
        response,
        LogDetail {
            error: Some("issuance_failed".to_string()),
            error_description: Some(message),
            ..LogDetail::default()
        },
    )
}

/// The username half of `Authorization: Basic`, which RFC 6749 §2.3.1 defines as
/// the client id. Nothing here validates: a header that will not decode simply
/// yields no client id, because a malformed credential is still an accepted one.
fn basic_client_id(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let encoded = raw
        .strip_prefix("Basic ")
        .or_else(|| raw.strip_prefix("basic "))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let credentials = String::from_utf8(decoded).ok()?;
    let username = credentials.split(':').next()?;
    // §2.3.1 says both halves are form-urlencoded before being base64'd, so a
    // client id with a space arrives as `%20` and must be undone.
    Some(form_decode(username))
}

fn form_decode(raw: &str) -> String {
    serde_urlencoded::from_str::<Vec<(String, String)>>(&format!("v={raw}"))
        .ok()
        .and_then(|pairs| pairs.into_iter().next())
        .map(|(_, value)| value)
        .unwrap_or_else(|| raw.to_string())
}

/// `400` in the OAuth error shape the seam already uses.
///
/// **This is the funnel every refusal on this endpoint passes through**, so
/// attaching the error here is what makes Phase 6's "no error can be returned
/// without being logged" a property of the code rather than a habit. The four
/// arms above add the `client_id` and the parameters on the way out; this adds
/// the reason, and [`LogDetail`]'s first-writer-wins merge keeps them apart.
fn bad_request(error: &str, description: String) -> Response {
    let response = (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": error, "error_description": description.clone() })),
    )
        .into_response();
    attach(
        response,
        LogDetail {
            error: Some(error.to_string()),
            error_description: Some(description),
            ..LogDetail::default()
        },
    )
}

/// The decoded form, kept as ordered pairs so a repeated parameter resolves to
/// its first occurrence rather than to whichever one a map happened to keep.
///
/// `/oidc/authorize` parses its query string and its POST body with this too.
/// The two endpoints have to agree that an empty value is an absent one — `-d
/// scope=` and `?scope=` are the same shrug — and the only way two endpoints
/// agree about that forever is by sharing the code that decides it.
pub(crate) struct Form(Vec<(String, String)>);

impl Form {
    /// The decoded pairs, for the log. `/oidc/authorize` reads this too — the
    /// event's `request` is the parameters the handler acted on, so it comes
    /// off the same parse rather than off a second one.
    pub(crate) fn pairs(&self) -> &[(String, String)] {
        &self.0
    }

    /// Infallible, and that is an observed fact rather than an assumption:
    /// `serde_urlencoded` decoding into pairs is lossy on invalid UTF-8 and
    /// lenient about stray `%` escapes, so no byte string fails. A body that is
    /// not a form at all therefore arrives as unknown parameters and falls out
    /// as `invalid_request` for the missing `grant_type` — still our shape,
    /// never axum's 415 or 422.
    ///
    /// The content-type header is deliberately not consulted. Rejecting a
    /// well-formed body over its label would be the one registration-shaped
    /// "no" on an endpoint whose whole point is that it has none.
    pub(crate) fn parse(body: &Bytes) -> Self {
        Form(serde_urlencoded::from_bytes::<Vec<(String, String)>>(body).unwrap_or_default())
    }

    /// The same decoding for a query string, which arrives as a `&str` rather
    /// than as bytes.
    pub(crate) fn from_query(query: &str) -> Self {
        Form(serde_urlencoded::from_str::<Vec<(String, String)>>(query).unwrap_or_default())
    }

    /// Body parameters win over query parameters of the same name: OIDC Core
    /// §3.1.2.1 puts them in the body on a `POST`, and a client that sends both
    /// meant the body.
    pub(crate) fn chain(mut self, other: Form) -> Self {
        self.0.extend(other.0);
        self
    }

    /// An empty value is the same as an absent one. `-d scope=` should not echo
    /// an empty `scope`, and `-d grant_type=` should not name an unsupported
    /// grant of `""`.
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, value)| key == name && !value.is_empty())
            .map(|(_, value)| value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic(value: &str) -> Option<String> {
        let mut headers = HeaderMap::new();
        headers.insert(axum::http::header::AUTHORIZATION, value.parse().unwrap());
        basic_client_id(&headers)
    }

    fn encode(credentials: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(credentials)
    }

    #[test]
    fn the_basic_username_is_the_client_id() {
        assert_eq!(
            basic(&format!("Basic {}", encode("billing-web:unchecked"))),
            Some("billing-web".to_string())
        );
    }

    #[test]
    fn a_percent_encoded_basic_username_is_decoded() {
        // RFC 6749 §2.3.1 form-urlencodes each half before base64.
        assert_eq!(
            basic(&format!("Basic {}", encode("billing%20web:s%3Acret"))),
            Some("billing web".to_string())
        );
    }

    #[test]
    fn a_secretless_basic_credential_still_yields_a_client_id() {
        assert_eq!(
            basic(&format!("Basic {}", encode("billing-web"))),
            Some("billing-web".to_string())
        );
    }

    /// A credential we cannot read is still an accepted one — it yields no
    /// client id rather than a 401. Nothing on this endpoint rejects.
    #[test]
    fn an_unreadable_credential_yields_no_client_id_rather_than_an_error() {
        assert_eq!(basic("Bearer something"), None);
        assert_eq!(basic("Basic !!!not-base64!!!"), None);
        assert_eq!(basic(&format!("Basic {}", encode(""))), Some(String::new()));
    }

    /// An empty value is an absent one, so `-d scope=` echoes nothing and
    /// `-d grant_type=` names no grant.
    #[test]
    fn an_empty_form_value_reads_as_absent() {
        let form = Form::parse(&Bytes::from_static(b"grant_type=&scope=orders%3Aread"));
        assert_eq!(form.get("grant_type"), None);
        assert_eq!(form.get("scope"), Some("orders:read"));
    }

    /// Ordered pairs, not a map: a repeated parameter resolves to its first
    /// occurrence rather than to whichever one a map happened to keep.
    #[test]
    fn a_repeated_parameter_resolves_to_the_first() {
        let form = Form::parse(&Bytes::from_static(b"persona=ada&persona=mira"));
        assert_eq!(form.get("persona"), Some("ada"));
    }
}
