//! JWS serialization by hand, with `rsa` doing only the signature.
//!
//! Not a JWT library, deliberately. Phase 3 needs a genuine `alg: none` token
//! and a deliberately broken signature, and every correct JWT library makes both
//! impossible — reaching for one now buys a second, divergent code path in two
//! phases, which is exactly the drift north star 3 exists to prevent.
//!
//! **This is the bill Phase 1 ran up, and this module pays it.** Three of the
//! six [`Flaw`]s are not expressible as claims, so they arrive here; the other
//! three are [`crate::oidc::issue`]'s. `sign` takes the same `Flaw` the wire
//! speaks rather than a private mode enum of its own, because a second
//! vocabulary needs a mapping table and a mapping table is the seam two names
//! drift through.

use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};

use crate::b64;
use crate::clock::{format_duration, format_offset, round_about, Clock, LEEWAY, SKEW_VAR};
use crate::keys::SigningKey;
use crate::oidc::flaw::{self, Flaw};

/// RFC 7515 §A.5's unsecured JWT header, written out rather than serialized.
///
/// `serde_json` without `preserve_order` emits object keys sorted, and `alg`
/// sorts before `typ` by luck rather than by design — a feature flag somewhere
/// downstream could quietly reorder it. A token that must decode to exactly
/// these bytes is a literal.
const UNSECURED_HEADER: &str = r#"{"alg":"none","typ":"JWT"}"#;

/// Serialize and sign `claims` as a compact JWS, optionally breaking exactly one
/// thing about the result.
///
/// The signature covers the literal ASCII of `header.payload` exactly as it
/// appears in the returned token. Re-serializing the payload in order to sign it
/// yields a token that verifies against our own code and fails in `jose`.
pub fn sign(
    key: &SigningKey,
    claims: &Map<String, Value>,
    flaw: Option<Flaw>,
) -> Result<String, String> {
    let payload =
        b64::encode(serde_json::to_vec(claims).map_err(|e| format!("cannot encode claims: {e}"))?);

    // An unsecured JWT is not RS256 with a rewritten header: there is no key
    // involved and no third segment, so this returns before any signing happens.
    if flaw == Some(Flaw::AlgNone) {
        return Ok(format!("{}.{payload}.", b64::encode(UNSECURED_HEADER)));
    }

    // A real RS256 signature under a `kid` that is in no JWKS anywhere, which is
    // what makes it a test of whether the relying party honours `kid` at all.
    let kid = match flaw {
        Some(Flaw::UnknownKid) => flaw::UNKNOWN_KID,
        _ => key.kid(),
    };
    let header = serde_json::json!({
        "alg": "RS256",
        "typ": "JWT",
        "kid": kid,
    });

    let signing_input = format!(
        "{}.{payload}",
        b64::encode(serde_json::to_vec(&header).map_err(|e| format!("cannot encode header: {e}"))?),
    );

    let digest = Sha256::digest(signing_input.as_bytes());
    let mut signature = key
        .private()
        .sign(rsa::Pkcs1v15Sign::new::<Sha256>(), &digest)
        .map_err(|e| format!("cannot sign: {e}"))?;

    // Flipped on the signature bytes, after signing — never on the encoded
    // string, and never by re-serializing anything. That is the only way the
    // header stays byte-identical to a good token's, which is what makes key
    // lookup succeed so the failure lands on the signature check.
    if flaw == Some(Flaw::BadSignature) {
        if let Some(last) = signature.last_mut() {
            *last ^= 1;
        }
    }

    Ok(format!("{signing_input}.{}", b64::encode(signature)))
}

/// Verify a token as **lanyard's own**: signature, `iss`, `nbf` and `exp`.
///
/// It does not check `aud`, because there is no client to check it against — a
/// bearer token arriving at UserInfo was issued for whatever API the request
/// named, and UserInfo is not that API.
///
/// **It does check `nbf`, and that is a Phase 8 change.** This comment used to
/// say the opposite, on the grounds that lanyard never issues a future `nbf`;
/// `LANYARD_CLOCK_SKEW` is exactly what makes that false, and a token that is
/// not valid yet is the skew symptom most worth naming. Both bounds get
/// [`LEEWAY`] in both directions, so a relying party a second or two out is not
/// told its one-second-old token is dead.
///
/// **This is the second place lanyard says no**, and the reason is PKCE's: an
/// app that reads `/userinfo` with an expired token has a bug, and a mock that
/// answers anyway hides it. What changed in Phase 8 is not *whether* it says
/// no but *what it says*: three distinct sentences instead of one, because
/// "expired" and "your clocks disagree" are different bugs with different
/// fixes.
pub fn verify(
    key: &SigningKey,
    clock: Clock,
    issuer: &str,
    token: &str,
) -> Result<Map<String, Value>, String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("the token is not a compact JWS of three segments".to_string());
    }

    let header: Value = serde_json::from_slice(
        &b64::decode(parts[0]).map_err(|_| "the token header is not base64url".to_string())?,
    )
    .map_err(|_| "the token header is not JSON".to_string())?;

    // An unsecured JWT is refused by name rather than by falling through a
    // signature check it would never reach. Phase 3 can mint one on purpose,
    // and this endpoint is where it should stop.
    match header["alg"].as_str() {
        Some("RS256") => {}
        Some("none") => return Err("the token is unsigned (alg: none)".to_string()),
        Some(other) => return Err(format!("the token is signed with {other}, not RS256")),
        None => return Err("the token header names no alg".to_string()),
    }

    let signature =
        b64::decode(parts[2]).map_err(|_| "the token signature is not base64url".to_string())?;
    let digest = Sha256::digest(format!("{}.{}", parts[0], parts[1]).as_bytes());
    key.private()
        .to_public_key()
        .verify(rsa::Pkcs1v15Sign::new::<Sha256>(), &digest, &signature)
        .map_err(|_| "the token signature does not verify against lanyard's key".to_string())?;

    let claims: Map<String, Value> = serde_json::from_slice(
        &b64::decode(parts[1]).map_err(|_| "the token payload is not base64url".to_string())?,
    )
    .map_err(|_| "the token payload is not a JSON object".to_string())?;

    if claims.get("iss").and_then(Value::as_str) != Some(issuer) {
        return Err(format!(
            "the token was issued by someone other than {issuer}"
        ));
    }

    let now = clock.now();

    // **`nbf` first**, because "not valid yet" is the more specific diagnosis:
    // a token from a clock five minutes ahead has an `exp` five minutes in the
    // future too, so the expiry check would pass it silently.
    if let Some(nbf) = claims.get("nbf").and_then(Value::as_u64) {
        if nbf > now.saturating_add(LEEWAY) {
            // `issue()` backdates `nbf` by the leeway, and `iss` has already
            // been checked to be ours — so adding it back reconstructs the
            // issuing clock's own `iat`, which is the number a developer can
            // act on.
            let ahead = nbf.saturating_add(LEEWAY).saturating_sub(now);
            return Err(format!(
                "the token is not valid yet — the clock that issued it is ahead of this one \
                 by about {}. Check the clocks on both sides; run lanyard doctor{}",
                format_duration(round_about(ahead)),
                own_skew(clock),
            ));
        }
    }

    match claims.get("exp").and_then(Value::as_u64) {
        Some(exp) if exp.saturating_add(LEEWAY) > now => {}
        Some(exp) => {
            return Err(format!(
                "the token expired {} ago{}",
                format_duration(now.saturating_sub(exp)),
                own_skew(clock),
            ))
        }
        None => return Err("the token has no exp".to_string()),
    }

    Ok(claims)
}

/// The parenthetical a **skewed** process adds to every refusal, and nothing at
/// all otherwise.
///
/// A `401` whose real cause is a variable the developer exported an hour ago is
/// the bare rejection this phase exists to delete — so the process that is
/// lying about the time is the one that has to say so.
fn own_skew(clock: Clock) -> String {
    if !clock.is_skewed() {
        return String::new();
    }
    format!(
        " (this process's clock is skewed by {} via {SKEW_VAR})",
        format_offset(clock.skew_seconds())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::DEFAULT_DEV_KEY_PEM;

    fn claims() -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("sub".into(), Value::from("ada"));
        m.insert("iss".into(), Value::from("http://127.0.0.1:9500/oidc"));
        m
    }

    fn segments(token: &str) -> Vec<&str> {
        token.split('.').collect()
    }

    fn key() -> SigningKey {
        SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap()
    }

    #[test]
    fn the_header_names_rs256_and_the_keys_own_kid() {
        let key = key();
        let token = sign(&key, &claims(), None).unwrap();
        let header: Value =
            serde_json::from_slice(&b64::decode(token.split('.').next().unwrap()).unwrap())
                .unwrap();

        assert_eq!(header["alg"], "RS256");
        assert_eq!(header["typ"], "JWT");
        assert_eq!(
            header["kid"],
            key.kid(),
            "a token whose kid is not in the JWKS is unverifiable"
        );
    }

    #[test]
    fn the_payload_round_trips_the_claims_it_was_given() {
        let token = sign(&key(), &claims(), None).unwrap();
        let payload: Value =
            serde_json::from_slice(&b64::decode(token.split('.').nth(1).unwrap()).unwrap())
                .unwrap();

        assert_eq!(payload["sub"], "ada");
        assert_eq!(payload["iss"], "http://127.0.0.1:9500/oidc");
    }

    #[test]
    fn every_segment_is_unpadded_base64url() {
        let token = sign(&key(), &claims(), None).unwrap();
        let segments: Vec<&str> = token.split('.').collect();
        assert_eq!(segments.len(), 3, "compact JWS is three segments");
        for s in segments {
            assert!(!s.is_empty());
            assert!(!s.contains('='), "padding breaks verification: {s}");
            assert!(!s.contains('+') && !s.contains('/'), "not url-safe: {s}");
        }
    }

    #[test]
    fn the_signature_covers_the_bytes_actually_emitted() {
        let key = key();
        let token = sign(&key, &claims(), None).unwrap();

        let last_dot = token.rfind('.').unwrap();
        let signing_input = &token[..last_dot];
        let signature = b64::decode(&token[last_dot + 1..]).unwrap();

        let digest = Sha256::digest(signing_input.as_bytes());
        key.private()
            .to_public_key()
            .verify(rsa::Pkcs1v15Sign::new::<Sha256>(), &digest, &signature)
            .expect("signature must cover the literal header.payload string in the token");
    }

    // ----------------------------------------------------------- the flaws --

    /// RFC 7515 §A.5's unsecured JWT, byte for byte. No `kid`, because a token
    /// with no signature has no key — and nothing named RS256 anywhere.
    #[test]
    fn alg_none_emits_the_exact_unsecured_header() {
        let token = sign(&key(), &claims(), Some(Flaw::AlgNone)).unwrap();
        let header = b64::decode(segments(&token)[0]).unwrap();

        assert_eq!(
            String::from_utf8(header).unwrap(),
            r#"{"alg":"none","typ":"JWT"}"#
        );
        assert!(!token.contains("RS256"));
    }

    /// Three segments, the third empty — an unsecured JWT, not RS256 with a
    /// rewritten header. There are no signature bytes at all.
    #[test]
    fn alg_none_has_no_signature_bytes_and_keeps_its_claims() {
        let token = sign(&key(), &claims(), Some(Flaw::AlgNone)).unwrap();
        let parts = segments(&token);

        assert_eq!(parts.len(), 3);
        assert_eq!(parts[2], "", "the third segment is the empty string");

        let payload: Value = serde_json::from_slice(&b64::decode(parts[1]).unwrap()).unwrap();
        assert_eq!(payload["sub"], "ada", "all claims are still right");
    }

    /// A genuine test of whether the relying party honours `kid` at all: the
    /// signature is real, so a library that ignores `kid` and tries every key in
    /// the JWKS will accept this token. Finding that out about your stack is the
    /// point.
    #[test]
    fn unknown_kid_names_a_key_nobody_has_and_signs_with_the_real_one() {
        let key = key();
        let token = sign(&key, &claims(), Some(Flaw::UnknownKid)).unwrap();
        let parts = segments(&token);
        let header: Value = serde_json::from_slice(&b64::decode(parts[0]).unwrap()).unwrap();

        assert_eq!(header["alg"], "RS256");
        assert_eq!(header["kid"], flaw::UNKNOWN_KID);
        assert_ne!(header["kid"], key.kid());

        let signature = b64::decode(parts[2]).unwrap();
        assert_eq!(signature.len(), 256);
        let digest = Sha256::digest(format!("{}.{}", parts[0], parts[1]).as_bytes());
        key.private()
            .to_public_key()
            .verify(rsa::Pkcs1v15Sign::new::<Sha256>(), &digest, &signature)
            .expect("the signature is real; only the kid is wrong");
    }

    /// Byte-identical header, so key lookup succeeds and the failure lands on
    /// the signature — which is the only reason this flaw tells you anything.
    /// Byte-identical only because the flaw is applied to the signature bytes
    /// after signing, never by re-serializing anything.
    #[test]
    fn bad_signature_keeps_a_byte_identical_header_and_payload() {
        let key = key();
        let good = sign(&key, &claims(), None).unwrap();
        let bad = sign(&key, &claims(), Some(Flaw::BadSignature)).unwrap();

        assert_eq!(segments(&bad)[0], segments(&good)[0], "header must match");
        assert_eq!(segments(&bad)[1], segments(&good)[1], "payload must match");
        assert_ne!(segments(&bad)[2], segments(&good)[2]);
    }

    #[test]
    fn bad_signature_flips_one_bit_of_a_full_length_signature() {
        let key = key();
        let good = b64::decode(segments(&sign(&key, &claims(), None).unwrap())[2]).unwrap();
        let bad =
            b64::decode(segments(&sign(&key, &claims(), Some(Flaw::BadSignature)).unwrap())[2])
                .unwrap();

        assert_eq!(bad.len(), 256, "still a full RSA-2048 signature");
        assert_eq!(bad.len(), good.len());
        assert_eq!(&bad[..255], &good[..255], "only the last byte moves");
        assert_eq!(bad[255], good[255] ^ 1, "the low bit, and only the low bit");

        let parts = segments(&sign(&key, &claims(), Some(Flaw::BadSignature)).unwrap())
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let digest = Sha256::digest(format!("{}.{}", parts[0], parts[1]).as_bytes());
        assert!(
            key.private()
                .to_public_key()
                .verify(rsa::Pkcs1v15Sign::new::<Sha256>(), &digest, &bad)
                .is_err(),
            "the whole point is that this does not verify"
        );
    }
}

#[cfg(test)]
mod verify_tests {
    use super::*;
    use crate::keys::DEFAULT_DEV_KEY_PEM;

    const ISSUER: &str = "http://127.0.0.1:9500/oidc";

    fn key() -> SigningKey {
        SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap()
    }

    fn now() -> u64 {
        Clock::real().now()
    }

    fn token_with(iss: &str, exp: u64, flaw: Option<Flaw>) -> String {
        let mut claims = Map::new();
        claims.insert("sub".into(), Value::from("ada"));
        claims.insert("iss".into(), Value::from(iss));
        claims.insert("exp".into(), Value::from(exp));
        sign(&key(), &claims, flaw).unwrap()
    }

    #[test]
    fn a_good_token_verifies_and_hands_back_its_claims() {
        let claims = verify(
            &key(),
            Clock::real(),
            ISSUER,
            &token_with(ISSUER, now() + 60, None),
        )
        .unwrap();
        assert_eq!(claims["sub"], "ada");
    }

    /// The reason this endpoint says no at all: a mock that answers an expired
    /// token hides the bug in the app that presented it.
    #[test]
    fn an_expired_token_is_refused_and_says_so() {
        let err = verify(
            &key(),
            Clock::real(),
            ISSUER,
            &token_with(ISSUER, now() - 60, None),
        )
        .unwrap_err();
        assert!(err.contains("expired"), "{err}");
    }

    /// **lanyard allows itself the same 5 seconds it backdates `nbf` by.** A
    /// relying party whose clock is a second or two behind must not be told its
    /// one-second-old token is dead.
    #[test]
    fn a_token_a_few_seconds_past_exp_is_still_accepted() {
        let claims = verify(
            &key(),
            Clock::real(),
            ISSUER,
            &token_with(ISSUER, now() - 3, None),
        )
        .expect("three seconds past exp is inside the leeway");
        assert_eq!(claims["sub"], "ada");
    }

    /// And a token well past it is refused **naming how long ago** — the
    /// difference between "your token is dead" and "your clocks are wrong".
    #[test]
    fn a_token_well_past_exp_says_how_long_ago_it_expired() {
        let err = verify(
            &key(),
            Clock::real(),
            ISSUER,
            &token_with(ISSUER, now() - 252, None),
        )
        .unwrap_err();
        assert!(err.contains("expired"), "{err}");
        // Either side of a second boundary, because the clock can tick between
        // building the token and verifying it. The point is that the elapsed
        // time is *named*, to the second, rather than that it is 252 exactly.
        assert!(
            err.contains("4m12s ago") || err.contains("4m13s ago"),
            "{err}"
        );
    }

    /// The skew symptom, named. A token minted by a process five minutes ahead
    /// is *not valid yet*, and saying "expired" or nothing at all is the bare
    /// `401` this whole phase exists to delete.
    #[test]
    fn a_token_from_a_clock_that_is_ahead_is_not_valid_yet() {
        let token = minted_at(Clock::skewed(300));

        let err = verify(&key(), Clock::real(), ISSUER, &token).unwrap_err();
        assert!(err.contains("not valid yet"), "{err}");
        assert!(
            !err.contains("4m5"),
            "\"about 4m57s\" undoes its own hedge: {err}"
        );
        assert!(err.contains("ahead of this one"), "{err}");
        assert!(err.contains("5m0s"), "the offset, not just the fact: {err}");
        assert!(err.contains("lanyard doctor"), "{err}");
        assert!(!err.contains("expired"), "not 'expired': {err}");
    }

    /// A token a couple of seconds into the future is a clock that is a couple
    /// of seconds out, not a diagnosis worth a `401`.
    #[test]
    fn a_token_a_few_seconds_early_is_accepted() {
        assert!(verify(&key(), Clock::real(), ISSUER, &minted_at(Clock::skewed(3))).is_ok());
    }

    /// **A dev tool may lie about the time; it may not do so quietly.** A
    /// process running skewed says so in every refusal it hands out, because
    /// the alternative is a `401` whose real cause is a variable the developer
    /// exported an hour ago.
    #[test]
    fn a_skewed_process_names_its_own_skew_in_the_refusal() {
        // Five minutes past the *skewed* process's own clock, which is ten
        // minutes past the machine's — the arithmetic the message reports.
        let err = verify(
            &key(),
            Clock::skewed(-300),
            ISSUER,
            &token_with(ISSUER, now() - 600, None),
        )
        .unwrap_err();
        assert!(err.contains("expired"), "{err}");
        assert!(err.contains("LANYARD_CLOCK_SKEW"), "{err}");
        assert!(err.contains("-5m0s"), "{err}");
    }

    /// The clock is the *process's*, not the machine's: a server five minutes
    /// behind is what makes a perfectly good token look unborn.
    #[test]
    fn a_skewed_process_names_its_skew_on_the_not_valid_yet_message_too() {
        let err = verify(
            &key(),
            Clock::skewed(-300),
            ISSUER,
            &minted_at(Clock::real()),
        )
        .unwrap_err();
        assert!(err.contains("not valid yet"), "{err}");
        assert!(err.contains("LANYARD_CLOCK_SKEW"), "{err}");
    }

    /// A whole token as `issue()` would have minted it on `clock` — `nbf`
    /// backdated by the leeway and all — so the tests above exercise the shape
    /// lanyard actually emits rather than one assembled by hand.
    fn minted_at(clock: Clock) -> String {
        crate::oidc::issue::issue(
            &key(),
            clock,
            ISSUER,
            None,
            &Map::new(),
            60,
            None,
            &crate::oidc::scope::ClaimFilter::Unfiltered,
        )
        .unwrap()
        .token
    }

    #[test]
    fn a_token_from_another_issuer_is_refused() {
        let err = verify(
            &key(),
            Clock::real(),
            ISSUER,
            &token_with("https://elsewhere.test", now() + 60, None),
        )
        .unwrap_err();
        assert!(err.contains("issued by someone other than"), "{err}");
    }

    /// Phase 3's three signature-shaped flaws, meeting the one endpoint that
    /// checks signatures. Each is refused, and each says which thing was wrong.
    #[test]
    fn the_flawed_tokens_are_refused_by_name() {
        let expired = sign_flawed(Flaw::Expired);
        assert!(verify(&key(), Clock::real(), ISSUER, &expired)
            .unwrap_err()
            .contains("expired"));

        let unsigned = sign_flawed(Flaw::AlgNone);
        assert!(verify(&key(), Clock::real(), ISSUER, &unsigned)
            .unwrap_err()
            .contains("unsigned"));

        let broken = sign_flawed(Flaw::BadSignature);
        assert!(verify(&key(), Clock::real(), ISSUER, &broken)
            .unwrap_err()
            .contains("does not verify"));

        let wrong_iss = sign_flawed(Flaw::WrongIss);
        assert!(verify(&key(), Clock::real(), ISSUER, &wrong_iss)
            .unwrap_err()
            .contains("issued by someone"));
    }

    /// A real signature under a `kid` nobody has still verifies here, because
    /// lanyard has exactly one key and looks nothing up. That is honest rather
    /// than a gap: `--unknown-kid` is a test of *the relying party's* key
    /// handling, and lanyard is not the relying party.
    #[test]
    fn an_unknown_kid_still_verifies_because_the_signature_is_real() {
        let token = sign_flawed(Flaw::UnknownKid);
        assert!(verify(&key(), Clock::real(), ISSUER, &token).is_ok());
    }

    fn sign_flawed(flaw: Flaw) -> String {
        let claims = crate::oidc::issue::claims_at(
            now(),
            ISSUER,
            None,
            &Map::new(),
            60,
            "test-jti",
            Some(flaw),
            &crate::oidc::scope::ClaimFilter::Unfiltered,
        );
        sign(&key(), &claims, Some(flaw)).unwrap()
    }

    #[test]
    fn something_that_is_not_a_token_is_refused_rather_than_panicking() {
        for garbage in ["", "abc", "a.b", "a.b.c.d", "!!!.!!!.!!!"] {
            assert!(
                verify(&key(), Clock::real(), ISSUER, garbage).is_err(),
                "{garbage:?}"
            );
        }
    }
}
