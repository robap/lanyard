//! **The one function.** North star 3: the browser flow (Phase 4), the CLI
//! (Phase 2), `/oidc/token` (Phase 5) and the test seam are all thin callers of
//! this. Nothing outside this module builds claims.
//!
//! The persona → claim mapping lives here rather than on [`Persona`], because
//! the persona model stays protocol-neutral (CONCEPT §3).

use serde_json::{Map, Value};

use crate::clock::{Clock, LEEWAY};
use crate::keys::SigningKey;
use crate::oidc::flaw::{self, Flaw};
use crate::oidc::jws;
use crate::oidc::scope::ClaimFilter;
use crate::persona::Persona;

/// Access tokens are 60 seconds by default. Long-lived dev tokens mean the
/// refresh path never runs locally (CONCEPT §6).
pub const DEFAULT_TTL: u64 = 60;

/// **The one lifetime in this project that is not 60 seconds, and it is
/// deliberate.**
///
/// An access token is a credential and its short life is the entire point of
/// CONCEPT §6 — a 60-second access token that gets rejected is lanyard working.
/// An ID token is an authentication *receipt*: consumed once at login and then
/// exchanged for the RP's own cookie. A 60-second one makes `oidc-client-ts`
/// consider the user expired seconds after signing in, which tests Phase 9's
/// renew path rather than this phase's login path. Five minutes is still
/// aggressive by any production standard.
pub const ID_TOKEN_TTL: u64 = 300;

/// OIDC Core §3.1.3.6's `at_hash` / `c_hash`: base64url of the **leftmost 128
/// bits** of the SHA-256 of the ASCII of the value.
///
/// 128 bits because the hash length is half the digest of the signature
/// algorithm's hash, and RS256 hashes with SHA-256. Emitting a full digest is
/// the classic way to produce a hash every library rejects.
pub fn half_hash(value: &str) -> String {
    use sha2::{Digest as _, Sha256};
    let digest = Sha256::digest(value.as_bytes());
    crate::b64::encode(&digest[..16])
}

/// A minted token, the exact claims that were signed, and the moment they were
/// minted at.
///
/// `issued_at` is here rather than read again by the caller because
/// `/oidc/token`'s `expires_in` is `exp - issued_at`: a second clock read makes
/// the ordinary case report 59 or 61 at random.
pub struct Issued {
    pub token: String,
    pub claims: Map<String, Value>,
    pub issued_at: u64,
}

/// Mint a signed token and hand back the exact claims that were signed.
#[allow(clippy::too_many_arguments)]
pub fn issue(
    key: &SigningKey,
    clock: Clock,
    issuer: &str,
    persona: Option<&Persona>,
    overrides: &Map<String, Value>,
    ttl: u64,
    flaw: Option<Flaw>,
    filter: &ClaimFilter,
) -> Result<Issued, String> {
    let issued_at = clock.now();
    let claims = claims_at(
        issued_at,
        issuer,
        persona,
        overrides,
        ttl,
        &new_jti(),
        flaw,
        filter,
    );
    let token = jws::sign(key, &claims, flaw)?;
    Ok(Issued {
        token,
        claims,
        issued_at,
    })
}

/// The claim set, as a pure function of its inputs so the time-dependent parts
/// are testable.
///
/// Eight arguments, two over clippy's threshold, and deliberately not bundled
/// into a parameters struct. This is the one function every token in the project
/// comes out of (north star 3), and its argument list *is* the contract: a new
/// caller has to answer "which persona, which overrides, how long, flawed how,
/// filtered how" before it can call. A struct with `Default` would let a caller
/// skip a question, and the question it would skip is the filter — which is
/// precisely the one whose wrong answer leaks a persona's email into a token
/// that did not ask for it. `now` and `jti` are the impurity, injected so the
/// rest is testable.
#[allow(clippy::too_many_arguments)]
pub fn claims_at(
    now: u64,
    issuer: &str,
    persona: Option<&Persona>,
    overrides: &Map<String, Value>,
    ttl: u64,
    jti: &str,
    flaw: Option<Flaw>,
    filter: &ClaimFilter,
) -> Map<String, Value> {
    // 1. The persona, mapped per the spec's table and filtered by scope.
    //    **Step 1 only.** The filter never reaches the registered claims below
    //    and never reaches the overrides: `email` scope decides whether Ada's
    //    address is in the token, not whether the token has an `iss`.
    let mut claims = match persona {
        Some(p) => persona_claims(p, filter),
        None => Map::new(),
    };

    // 2. The registered claims. After attributes, so an attribute cannot quietly
    //    move the issuer — the body is the documented override channel.
    claims.insert("iss".into(), Value::from(issuer));
    claims.insert("iat".into(), Value::from(now));
    // **`nbf` is backdated, and only `nbf`.** A relying party whose clock is a
    // second or two behind must not reject a token that is a second old.
    // Extending `exp` would have the same effect and is the wrong lever: it
    // would silently lengthen a TTL that is 60 seconds on purpose (CONCEPT §6),
    // and a 60-second token that gets rejected is lanyard working. `iat` is a
    // statement of fact and does not move either.
    claims.insert("nbf".into(), Value::from(now.saturating_sub(LEEWAY)));
    claims.insert("exp".into(), Value::from(now.saturating_add(ttl)));
    claims.insert("jti".into(), Value::from(jti));

    // 3. The body wins over everything, `iss` and `exp` included. That is what
    //    keeps Phase 3's failure flags sugar over this function rather than a
    //    second code path.
    for (key, value) in overrides {
        claims.insert(key.clone(), value.clone());
    }

    // 4. The flaw, which outranks even the body — the one reversal of step 3's
    //    rule. `flaw=expired` returning a live token because the caller happened
    //    to post an `exp` is a negative test that quietly passes, which is worse
    //    than no flag at all (north star 4). Anyone who wants an exact `exp` can
    //    still post one, without a flaw.
    match flaw {
        // The whole token shifts back, lifetime intact: a token issued now that
        // expired an hour ago is a shape no IdP produces, and some libraries
        // reject it for the wrong reason.
        Some(Flaw::Expired) => {
            let iat = now.saturating_sub(flaw::EXPIRED_SHIFT);
            claims.insert("iat".into(), Value::from(iat));
            // Leeway and all, so `nbf` is five seconds before `iat` in every
            // token lanyard mints rather than in most of them.
            claims.insert("nbf".into(), Value::from(iat.saturating_sub(LEEWAY)));
            claims.insert("exp".into(), Value::from(iat.saturating_add(ttl)));
        }
        // `wrong-<requested>` cannot collide with what was asked for and reads
        // correctly in the refusal. An `aud` that is absent — or an array, which
        // the seam's body can post — has no string to prefix, so it is replaced.
        Some(Flaw::WrongAud) => {
            let aud = match claims.get("aud").and_then(Value::as_str) {
                Some(requested) => format!("wrong-{requested}"),
                None => flaw::WRONG_AUDIENCE.to_string(),
            };
            claims.insert("aud".into(), Value::from(aud));
        }
        Some(Flaw::WrongIss) => {
            claims.insert("iss".into(), Value::from(flaw::WRONG_ISSUER));
        }
        // The other three are the header and the signature, and belong to
        // `jws::sign`. Their claims are exactly what an unflawed request would
        // have produced, which is why the seam echoes the flaw it was asked for.
        Some(Flaw::BadSignature) | Some(Flaw::AlgNone) | Some(Flaw::UnknownKid) | None => {}
    }

    claims
}

/// The persona → claims table, and the only copy of it.
///
/// Extracted so `/userinfo` can read the same table without minting anything.
/// A second mapping written next to the UserInfo handler is exactly the drift
/// north star 3 exists to prevent — and it would be invisible until the day an
/// RP compared the ID token against the UserInfo response.
pub fn persona_claims(p: &Persona, filter: &ClaimFilter) -> Map<String, Value> {
    let mut claims = Map::new();
    let mut put = |key: &str, value: Value| {
        if filter.allows(key) {
            claims.insert(key.to_string(), value);
        }
    };

    put("sub", Value::from(p.id.clone()));

    if let Some(email) = &p.email {
        put("email", Value::from(email.clone()));
        put("email_verified", Value::Bool(true));
    }
    // `preferred_username` and `name` are both `profile`-scope claims, so
    // they travel together: a persona with no display identity has no
    // username to prefer. This is what keeps `nobody` carrying `sub` and the
    // registered claims and nothing else (CONCEPT §3) — a picker or header
    // that renders `preferred_username` must actually break on `nobody`,
    // which is the entire reason that persona exists.
    //
    // It is also why the filter must not panic or 500 on an absent claim:
    // `nobody` under `openid email profile` yields `sub` and nothing else, and
    // that login still has to succeed.
    if let Some(name) = &p.name {
        put("name", Value::from(name.clone()));
        put("preferred_username", Value::from(p.id.clone()));
    }
    if !p.roles.is_empty() {
        put("roles", Value::from(p.roles.clone()));
    }
    for (key, value) in &p.attributes {
        put(key, value.clone());
    }
    claims
}

fn new_jti() -> String {
    uuid::Uuid::new_v4().to_string()
}
