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
//! Dispatch on `grant_type` has two arms, and Phase 4 added the second one as
//! an arm rather than as a second handler — which is the point. A browser login
//! and a `client_credentials` grant mint through the same `issue()` call with
//! the same claim table, and criterion 23 is the assertion that they still do.
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
use crate::oidc::flaw::Flaw;
use crate::oidc::issue::{self, DEFAULT_TTL, ID_TOKEN_TTL};
use crate::oidc::scope::ClaimFilter;
use crate::store::Lookup;

/// Named once so the two "what is supported" messages cannot drift apart.
const SUPPORTED: &str = "client_credentials and authorization_code";

pub fn route() -> axum::routing::MethodRouter<SharedState> {
    // `post` alone: axum answers `GET /oidc/token` with 405 and an `Allow`
    // header, which is the correct answer and not one we have to write.
    axum::routing::post(token)
}

async fn token(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Response {
    let form = Form::parse(&body);

    // An empty `grant_type=` is the same mistake as no `grant_type` at all, and
    // both are `invalid_request` rather than `unsupported_grant_type`: nothing
    // was named, so nothing can be unsupported.
    match form.get("grant_type") {
        None => bad_request(
            "invalid_request",
            format!("grant_type is required; this build supports {SUPPORTED}"),
        ),
        Some("client_credentials") => client_credentials(&state, &headers, &form),
        Some("authorization_code") => authorization_code(&state, &form),
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
    let persona = match form.get("persona") {
        Some(id) => match state.personas.get(id) {
            Some(persona) => Some(persona),
            None => {
                return bad_request(
                    "invalid_request",
                    format!("no persona with id {id:?} is loaded"),
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
    if let Some(client_id) = form
        .get("client_id")
        .map(str::to_string)
        .or_else(|| basic_client_id(headers))
    {
        overrides.insert("client_id".into(), Value::from(client_id));
    }

    match issue::issue(
        &state.key,
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
            ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
        }
        Err(message) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "issuance_failed", "error_description": message })),
        )
            .into_response(),
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
            return invalid_grant(&format!(
                "redirect_uri {presented:?} does not match the one this code was \
                 issued against, {:?}",
                record.request.redirect_uri.as_str()
            ));
        }
    }

    // **PKCE is genuinely verified, and this is not a contradiction of north
    // star 1.** Accept-everything is about registration — who you say you are,
    // where you say you want to come back to. It was never about skipping the
    // cryptography. A local IdP that rubber-stamps a wrong `code_verifier` lets
    // a broken PKCE implementation ship.
    if let Some(challenge) = &record.request.challenge {
        let Some(verifier) = form.get("code_verifier") else {
            return invalid_grant(&format!(
                "this code was issued against a {} code_challenge, so a \
                 code_verifier is required",
                challenge.method.as_str()
            ));
        };
        if !challenge.verify(verifier) {
            return invalid_grant(&format!(
                "the code_verifier does not match the {} code_challenge this code \
                 was issued against",
                challenge.method.as_str()
            ));
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
    if request.scope.is_openid() {
        let mut registered = Map::new();
        // **The client_id, not the API audience.** An ID token says "this
        // person authenticated to you"; an access token says "bearer may act at
        // that API". Conflating the two is the mistake this line exists to not
        // make.
        registered.insert("aud".into(), Value::from(request.client_id.clone()));
        // May legitimately predate `iat` — that is what a remembered session
        // means, and an RP checking `max_age` needs the real moment.
        registered.insert("auth_time".into(), Value::from(record.auth_time));
        if let Some(nonce) = &request.nonce {
            registered.insert("nonce".into(), Value::from(nonce.clone()));
        }
        registered.insert(
            "at_hash".into(),
            Value::from(issue::half_hash(&access.token)),
        );
        // Emitted although OIDC Core requires it only for `code id_token`. It
        // costs a hash, and an RP that validates it should find it valid —
        // which is a better test of that RP than never exercising the path.
        registered.insert("c_hash".into(), Value::from(issue::half_hash(code)));

        match issue::issue(
            &state.key,
            &state.config.issuer,
            Some(&record.persona),
            &registered,
            ID_TOKEN_TTL,
            None,
            &ClaimFilter::ByScope(request.scope.clone()),
        ) {
            // The ID token also carries `nbf` and `jti`, because `claims_at`
            // adds them to everything. Both are legal registered claims, and
            // stripping them would mean a second claim path for the sake of two
            // fields no relying party objects to. Decided, not accidental.
            Ok(id_token) => body["id_token"] = Value::from(id_token.token),
            Err(message) => return issuance_failed(message),
        }
    }

    ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

/// `400` in the OAuth shape, with a description that names what failed.
fn invalid_grant(description: &str) -> Response {
    bad_request("invalid_grant", description.to_string())
}

fn issuance_failed(message: String) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": "issuance_failed", "error_description": message })),
    )
        .into_response()
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
fn bad_request(error: &str, description: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response()
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
