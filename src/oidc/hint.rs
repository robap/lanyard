//! `id_token_hint`: whose session is this `prompt=none` a renew *for*?
//!
//! **Why this is not [`crate::oidc::jws::verify`].** That function enforces
//! `exp`, and an `id_token_hint` is *expected* to be expired — it is the ID
//! token from the very session an RP is asking to renew, and by the time the
//! access token is near enough expiry to trigger a silent renew the ID token
//! beside it may already be past its own. Verifying a hint against `exp` would
//! turn "your session is still good" into `invalid_request` for the one flow
//! the parameter exists to serve.
//!
//! So: signature, `iss`, `sub`. **No `exp`, no `nbf`, no `aud`, no `azp`**
//! (Phase 9 spec, open question 2). `aud` would be the `client_id`, which the
//! request already carries and lanyard does not register anyway; `azp` is a
//! claim lanyard never mints. What is left is the only question the parameter
//! can answer for a provider with no client registry: *does this hint name the
//! same person this browser is logged in as?*
//!
//! The signature is still checked, and that is not ceremony. Without it any
//! caller could name any subject and find out whether this browser is logged in
//! as them — a probe, not a hint.

use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};

use crate::b64;
use crate::keys::SigningKey;

/// The `sub` of a token lanyard signed, or why the hint could not be read.
///
/// The error strings are `invalid_request` descriptions: a hint that will not
/// verify is a malformed request, not a login that failed.
pub fn subject_of(key: &SigningKey, issuer: &str, token: &str) -> Result<String, String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("the id_token_hint is not a compact JWS of three segments".to_string());
    }

    let header: Value = serde_json::from_slice(
        &b64::decode(parts[0])
            .map_err(|_| "the id_token_hint header is not base64url".to_string())?,
    )
    .map_err(|_| "the id_token_hint header is not JSON".to_string())?;

    // Refused by name rather than by falling through a signature check it would
    // never reach: an unsecured JWT would otherwise be a way to assert any
    // subject at all, and a hint is exactly where that would be convenient.
    match header["alg"].as_str() {
        Some("RS256") => {}
        Some("none") => return Err("the id_token_hint is unsigned (alg: none)".to_string()),
        Some(other) => {
            return Err(format!(
                "the id_token_hint is signed with {other}, not RS256"
            ))
        }
        None => return Err("the id_token_hint header names no alg".to_string()),
    }

    let signature = b64::decode(parts[2])
        .map_err(|_| "the id_token_hint signature is not base64url".to_string())?;
    let digest = Sha256::digest(format!("{}.{}", parts[0], parts[1]).as_bytes());
    key.private()
        .to_public_key()
        .verify(rsa::Pkcs1v15Sign::new::<Sha256>(), &digest, &signature)
        .map_err(|_| {
            "the id_token_hint signature does not verify against lanyard's key".to_string()
        })?;

    let claims: Map<String, Value> = serde_json::from_slice(
        &b64::decode(parts[1])
            .map_err(|_| "the id_token_hint payload is not base64url".to_string())?,
    )
    .map_err(|_| "the id_token_hint payload is not a JSON object".to_string())?;

    if claims.get("iss").and_then(Value::as_str) != Some(issuer) {
        return Err(format!(
            "the id_token_hint was issued by someone other than {issuer}"
        ));
    }

    // **And that is the end of the checks.** No `exp`, no `nbf`, no `aud`: see
    // the module comment. A hint that got this far names somebody, and who it
    // names is the only thing `/authorize` wants from it.
    claims
        .get("sub")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "the id_token_hint has no sub, so it names nobody".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Clock;
    use crate::keys::DEFAULT_DEV_KEY_PEM;
    use crate::oidc::flaw::Flaw;
    use crate::oidc::jws;

    const ISSUER: &str = "http://127.0.0.1:9500/oidc";

    fn key() -> SigningKey {
        SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap()
    }

    /// A real ID token as [`crate::oidc::issue`] would have minted it, `ttl`
    /// seconds after — or, negative, before — now.
    fn id_token(key: &SigningKey, sub: &str, ttl: i64) -> String {
        let mut claims = Map::new();
        claims.insert("sub".into(), Value::from(sub));
        let now = Clock::real().now();
        let iat = (now as i64 - ttl.max(0)) as u64;
        let exp = (now as i64 + ttl) as u64;
        claims.insert("iss".into(), Value::from(ISSUER));
        claims.insert("iat".into(), Value::from(iat));
        claims.insert("exp".into(), Value::from(exp));
        claims.insert("aud".into(), Value::from("billing-web"));
        jws::sign(key, &claims, None).unwrap()
    }

    /// **The whole reason this module exists.** A hint ten minutes past its
    /// `exp` is the normal case — it is the token from the session being
    /// renewed — and it must still name its subject.
    #[test]
    fn a_hint_ten_minutes_past_its_exp_still_names_its_subject() {
        let key = key();
        let stale = id_token(&key, "ada", -600);
        assert!(
            jws::verify(&key, Clock::real(), ISSUER, &stale).is_err(),
            "the fixture really is expired, or this test proves nothing"
        );
        assert_eq!(subject_of(&key, ISSUER, &stale).unwrap(), "ada");
    }

    #[test]
    fn a_fresh_hint_names_its_subject_too() {
        let key = key();
        assert_eq!(
            subject_of(&key, ISSUER, &id_token(&key, "mira", 300)).unwrap(),
            "mira"
        );
    }

    /// Without this, `id_token_hint` is a probe: anybody could name any subject
    /// and learn from the answer whether this browser is logged in as them.
    #[test]
    fn a_hint_signed_by_somebody_else_is_refused() {
        let mine = key();
        // The same fixture `keys.rs` uses for "somebody else's key".
        let theirs = SigningKey::from_pem(include_str!("../../tests/data/other-key.pem")).unwrap();
        let forged = id_token(&theirs, "ada", 300);
        let err = subject_of(&mine, ISSUER, &forged).unwrap_err();
        assert!(err.contains("does not verify"), "{err}");
    }

    /// Phase 3 can mint an unsecured JWT on purpose. A hint is exactly the kind
    /// of place an `alg: none` token would like to be believed.
    #[test]
    fn an_unsigned_hint_is_refused_by_name() {
        let key = key();
        let mut claims = Map::new();
        claims.insert("sub".into(), Value::from("ada"));
        claims.insert("iss".into(), Value::from(ISSUER));
        let unsigned = jws::sign(&key, &claims, Some(Flaw::AlgNone)).unwrap();
        let err = subject_of(&key, ISSUER, &unsigned).unwrap_err();
        assert!(err.contains("unsigned"), "{err}");
    }

    /// The two header shapes a hand-rolled or another-provider's token arrives
    /// in. lanyard signs RS256 and only RS256, so anything else is refused
    /// **naming what it saw** — "the id_token_hint is signed with HS256" is a
    /// fixable sentence and a bare `invalid_request` is not.
    #[test]
    fn a_hint_whose_alg_is_not_rs256_is_refused_naming_the_alg() {
        let key = key();
        let payload = b64::encode(br#"{"sub":"ada","iss":"http://127.0.0.1:9500/oidc"}"#);

        let hs256 = format!(
            "{}.{payload}.c2ln",
            b64::encode(br#"{"alg":"HS256","typ":"JWT"}"#)
        );
        let err = subject_of(&key, ISSUER, &hs256).unwrap_err();
        assert!(err.contains("signed with HS256"), "{err}");
        assert!(err.contains("not RS256"), "{err}");

        let headerless = format!("{}.{payload}.c2ln", b64::encode(br#"{"typ":"JWT"}"#));
        let err = subject_of(&key, ISSUER, &headerless).unwrap_err();
        assert!(err.contains("names no alg"), "{err}");
    }

    #[test]
    fn a_hint_from_another_issuer_is_refused() {
        let key = key();
        let mut claims = Map::new();
        claims.insert("sub".into(), Value::from("ada"));
        claims.insert("iss".into(), Value::from("https://elsewhere.test"));
        let foreign = jws::sign(&key, &claims, None).unwrap();
        let err = subject_of(&key, ISSUER, &foreign).unwrap_err();
        assert!(err.contains("issued by someone other than"), "{err}");
    }

    #[test]
    fn a_hint_with_no_sub_is_refused_rather_than_naming_nobody() {
        let key = key();
        let mut claims = Map::new();
        claims.insert("iss".into(), Value::from(ISSUER));
        let subjectless = jws::sign(&key, &claims, None).unwrap();
        let err = subject_of(&key, ISSUER, &subjectless).unwrap_err();
        assert!(err.contains("no sub"), "{err}");
    }

    #[test]
    fn something_that_is_not_a_token_is_refused_rather_than_panicking() {
        let key = key();
        for garbage in ["", "abc", "a.b", "a.b.c.d", "!!!.!!!.!!!"] {
            assert!(subject_of(&key, ISSUER, garbage).is_err(), "{garbage:?}");
        }
    }

    /// The claim that pins the difference from [`jws::verify`] itself: the two
    /// disagree about exactly one token, and it is the expired one.
    #[test]
    fn the_only_disagreement_with_jws_verify_is_about_expiry() {
        let key = key();
        for (sub, ttl) in [("ada", 300i64), ("mira", -600)] {
            let token = id_token(&key, sub, ttl);
            let hint = subject_of(&key, ISSUER, &token);
            let full = jws::verify(&key, Clock::real(), ISSUER, &token);
            assert_eq!(hint.unwrap(), sub);
            assert_eq!(full.is_ok(), ttl > 0, "ttl {ttl}");
        }
    }
}
