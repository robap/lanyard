//! **The one function.** North star 3: the browser flow (Phase 4), the CLI
//! (Phase 2), `/oidc/token` (Phase 5) and the test seam are all thin callers of
//! this. Nothing outside this module builds claims.
//!
//! The persona → claim mapping lives here rather than on [`Persona`], because
//! the persona model stays protocol-neutral (CONCEPT §3).

use serde_json::{Map, Value};

use crate::keys::SigningKey;
use crate::oidc::jws;
use crate::persona::Persona;

/// Access tokens are 60 seconds by default. Long-lived dev tokens mean the
/// refresh path never runs locally (CONCEPT §6).
pub const DEFAULT_TTL: u64 = 60;

/// Mint a signed token and hand back the exact claims that were signed.
pub fn issue(
    key: &SigningKey,
    issuer: &str,
    persona: Option<&Persona>,
    overrides: &Map<String, Value>,
    ttl: u64,
) -> Result<(String, Map<String, Value>), String> {
    let claims = claims_at(unix_now(), issuer, persona, overrides, ttl, &new_jti());
    let token = jws::sign(key, &claims)?;
    Ok((token, claims))
}

/// The claim set, as a pure function of its inputs so the time-dependent parts
/// are testable.
pub fn claims_at(
    now: u64,
    issuer: &str,
    persona: Option<&Persona>,
    overrides: &Map<String, Value>,
    ttl: u64,
    jti: &str,
) -> Map<String, Value> {
    let mut claims = Map::new();

    // 1. The persona, mapped per the spec's table.
    if let Some(p) = persona {
        claims.insert("sub".into(), Value::from(p.id.clone()));

        if let Some(email) = &p.email {
            claims.insert("email".into(), Value::from(email.clone()));
            claims.insert("email_verified".into(), Value::Bool(true));
        }
        // `preferred_username` and `name` are both `profile`-scope claims, so
        // they travel together: a persona with no display identity has no
        // username to prefer. This is what keeps `nobody` carrying `sub` and the
        // registered claims and nothing else (CONCEPT §3) — a picker or header
        // that renders `preferred_username` must actually break on `nobody`,
        // which is the entire reason that persona exists.
        if let Some(name) = &p.name {
            claims.insert("name".into(), Value::from(name.clone()));
            claims.insert("preferred_username".into(), Value::from(p.id.clone()));
        }
        if !p.roles.is_empty() {
            claims.insert("roles".into(), Value::from(p.roles.clone()));
        }
        for (key, value) in &p.attributes {
            claims.insert(key.clone(), value.clone());
        }
    }

    // 2. The registered claims. After attributes, so an attribute cannot quietly
    //    move the issuer — the body is the documented override channel.
    claims.insert("iss".into(), Value::from(issuer));
    claims.insert("iat".into(), Value::from(now));
    claims.insert("nbf".into(), Value::from(now));
    claims.insert("exp".into(), Value::from(now.saturating_add(ttl)));
    claims.insert("jti".into(), Value::from(jti));

    // 3. The body wins over everything, `iss` and `exp` included. That is what
    //    keeps Phase 3's failure flags sugar over this function rather than a
    //    second code path.
    for (key, value) in overrides {
        claims.insert(key.clone(), value.clone());
    }

    claims
}

fn new_jti() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn unix_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
