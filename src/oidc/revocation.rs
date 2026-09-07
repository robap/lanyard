//! What a presented token *is*, and the two ways to kill one.
//!
//! `/oidc/introspect` and `/oidc/revoke` are the same question asked twice —
//! "what is this string?" — and they must not be able to answer it differently.
//! So they ask here. `/oidc/userinfo` and the refresh grant consult the same
//! revocation set for the same reason: `/introspect` saying `active: false`
//! while `/userinfo` hands over claims would be lanyard disagreeing with itself.
//!
//! **Access tokens stay stateless JWTs.** Nothing in this file stores one.
//! Revoking an access token records its `jti` until the moment the token would
//! have expired anyway; revoking a refresh token drops the record *and* records
//! its id, so a replay can be told it was revoked rather than that it never
//! existed — which [`crate::store::Lookup::Spent`] cannot say, because it
//! already means something else.

use serde_json::{Map, Value};

use crate::app::SharedState;
use crate::clock::Clock;
use crate::oidc::jws;
use crate::store::{Lookup, RefreshRecord};

/// The three things a string presented to `/introspect` or `/revoke` can be.
///
/// There is no fourth variant for *why* something was unrecognized. RFC 7662
/// §2.2 is explicit that an inactive response reveals nothing else, and
/// diagnosing which kind of nothing it was is Phase 6's log rather than this
/// endpoint's job.
///
/// Boxed payloads: the claim map and the refresh record are both large, and an
/// unboxed enum makes every `Unrecognized` the same size as the biggest arm.
pub enum Presented {
    /// One of lanyard's own JWTs, verified and not revoked. An ID token lands
    /// here too, which is honest: it is one of ours and it introspects like one.
    Access(Box<Map<String, Value>>),
    /// A live refresh token, and the grant it stands for.
    Refresh(Box<RefreshRecord>),
    /// Expired, revoked, foreign, malformed, or a string nobody ever issued.
    Unrecognized,
}

/// Resolve `token` against the signing key, the refresh store, and the
/// revocation set.
///
/// A JWT is tried first because it is self-describing: a string that verifies
/// against lanyard's key, names lanyard as its issuer and has not expired is an
/// access token, whatever `token_type_hint` claimed. Only a string that is not
/// one of ours is looked for in the refresh store.
pub fn resolve(state: &SharedState, token: &str) -> Presented {
    // `verify` already refuses an expired token, a foreign one, an unsigned
    // one and a string that is not a compact JWS — every "no" this endpoint
    // owes, decided in the one place that decides it.
    if let Ok(claims) = jws::verify(&state.key, state.clock, &state.config.issuer, token) {
        let jti = claims
            .get("jti")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if is_revoked(state, jti) {
            return Presented::Unrecognized;
        }
        return Presented::Access(Box::new(claims));
    }

    // Not one of ours as a JWT, so it may be an opaque refresh token. Peeked
    // rather than taken: introspecting a refresh token must not spend it.
    let record = {
        let refresh = state.stores.refresh.lock().expect("refresh");
        match refresh.peek(token) {
            Lookup::Found(record) => Some(record.clone()),
            Lookup::Expired | Lookup::Spent | Lookup::Unknown => None,
        }
    };

    match record {
        Some(record) if !is_revoked(state, token) => Presented::Refresh(Box::new(record)),
        _ => Presented::Unrecognized,
    }
}

/// Whether this `jti` has been revoked.
pub fn is_revoked(state: &SharedState, jti: &str) -> bool {
    state.stores.revoked.lock().expect("revoked").contains(jti)
}

/// Revoke whatever `token` turns out to be, and say nothing about which it was.
///
/// **No cascade in either direction.** RFC 7009 §2.1 says the server MAY revoke
/// an access token's refresh token and vice versa; lanyard does not, because it
/// does not track the linkage and inventing one to support a MAY is how a dev
/// tool grows a subsystem nobody asked for.
pub fn revoke(state: &SharedState, token: &str) {
    match resolve(state, token) {
        // Stateless, so the only thing that can be remembered is the `jti` —
        // and only until the moment the token would have died on its own.
        Presented::Access(claims) => {
            let (Some(jti), Some(exp)) = (
                claims.get("jti").and_then(Value::as_str),
                claims.get("exp").and_then(Value::as_u64),
            ) else {
                return;
            };
            state
                .stores
                .revoked
                .lock()
                .expect("revoked")
                .revoke(jti, deadline_for(state.clock, exp));
        }
        // Dropped *and* recorded: the record going away is what stops the
        // refresh working, and the id being recorded is what lets the next
        // attempt be told it was revoked rather than that it never existed.
        Presented::Refresh(_) => {
            state.stores.refresh.lock().expect("refresh").remove(token);
            state
                .stores
                .revoked
                .lock()
                .expect("revoked")
                .revoke(token, std::time::Instant::now() + crate::store::REFRESH_TTL);
        }
        Presented::Unrecognized => {}
    }
}

/// The moment a token whose `exp` is `exp` would have died on its own.
///
/// The stores hold [`std::time::Instant`]s and a token's `exp` is unix seconds,
/// so the conversion is by difference from now — and it saturates, because a
/// token already past its `exp` needs no remembering at all. [`Revoked::revoke`]
/// records nothing for a deadline that has passed.
///
/// [`Revoked::revoke`]: crate::store::Revoked::revoke
fn deadline_for(clock: Clock, exp: u64) -> std::time::Instant {
    std::time::Instant::now() + std::time::Duration::from_secs(exp.saturating_sub(clock.now()))
}

/// Revoke the refresh tokens a browser session was issued — all of them for
/// **Log out**, or one `client_id`'s for **Expire now**.
///
/// Keyed on the record rather than on a list of ids the session remembered,
/// because **rotation makes such a list stale the moment it is written**: the
/// token a session was issued at login is not the token it holds an hour later.
///
/// The two locks are taken in sequence and never nested — the refresh store's
/// guard is dropped before the revoked set's is taken.
pub fn revoke_session_refresh_tokens(
    state: &SharedState,
    session_id: &str,
    client_id: Option<&str>,
) {
    let dropped = {
        let mut refresh = state.stores.refresh.lock().expect("refresh");
        refresh.retain(|record| {
            let mine = record.session_id.as_deref() == Some(session_id)
                && client_id.is_none_or(|client_id| record.client_id == client_id);
            !mine
        })
    };
    let mut revoked = state.stores.revoked.lock().expect("revoked");
    for id in dropped {
        revoked.revoke(&id, std::time::Instant::now() + crate::store::REFRESH_TTL);
    }
}

/// **Expire now**: every unexpired token this session holds for one
/// `client_id`, dead, with the selection left alone.
///
/// The access tokens come from what the session recorded being issued; the
/// refresh tokens come from a scan. Both are revoked, because an app whose
/// access token died and whose refresh token still worked would renew straight
/// past the thing this control exists to make observable (CONCEPT §6).
pub fn expire_client(state: &SharedState, session_id: &str, client_id: &str) {
    let now = state.clock.now();
    let issued = {
        let sessions = state.stores.sessions.lock().expect("sessions");
        sessions.issuances_for(session_id, client_id, now)
    };
    {
        let mut revoked = state.stores.revoked.lock().expect("revoked");
        for issuance in issued {
            revoked.revoke(&issuance.jti, deadline_for(state.clock, issuance.exp));
        }
    }
    revoke_session_refresh_tokens(state, session_id, Some(client_id));
}
