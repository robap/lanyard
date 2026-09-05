//! JWS serialization by hand, with `rsa` doing only the signature.
//!
//! Not a JWT library, deliberately. Phase 3 needs a genuine `alg: none` token
//! and a deliberately broken signature, and every correct JWT library makes both
//! impossible — reaching for one now buys a second, divergent code path in two
//! phases, which is exactly the drift north star 3 exists to prevent.

use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};

use crate::b64;
use crate::keys::SigningKey;

/// Serialize and sign `claims` as a compact JWS.
///
/// The signature covers the literal ASCII of `header.payload` exactly as it
/// appears in the returned token. Re-serializing the payload in order to sign it
/// yields a token that verifies against our own code and fails in `jose`.
pub fn sign(key: &SigningKey, claims: &Map<String, Value>) -> Result<String, String> {
    let header = serde_json::json!({
        "alg": "RS256",
        "typ": "JWT",
        "kid": key.kid(),
    });

    let signing_input = format!(
        "{}.{}",
        b64::encode(serde_json::to_vec(&header).map_err(|e| format!("cannot encode header: {e}"))?),
        b64::encode(serde_json::to_vec(claims).map_err(|e| format!("cannot encode claims: {e}"))?),
    );

    let digest = Sha256::digest(signing_input.as_bytes());
    let signature = key
        .private()
        .sign(rsa::Pkcs1v15Sign::new::<Sha256>(), &digest)
        .map_err(|e| format!("cannot sign: {e}"))?;

    Ok(format!("{signing_input}.{}", b64::encode(signature)))
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

    fn key() -> SigningKey {
        SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap()
    }

    #[test]
    fn the_header_names_rs256_and_the_keys_own_kid() {
        let key = key();
        let token = sign(&key, &claims()).unwrap();
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
        let token = sign(&key(), &claims()).unwrap();
        let payload: Value =
            serde_json::from_slice(&b64::decode(token.split('.').nth(1).unwrap()).unwrap())
                .unwrap();

        assert_eq!(payload["sub"], "ada");
        assert_eq!(payload["iss"], "http://127.0.0.1:9500/oidc");
    }

    #[test]
    fn every_segment_is_unpadded_base64url() {
        let token = sign(&key(), &claims()).unwrap();
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
        let token = sign(&key, &claims()).unwrap();

        let last_dot = token.rfind('.').unwrap();
        let signing_input = &token[..last_dot];
        let signature = b64::decode(&token[last_dot + 1..]).unwrap();

        let digest = Sha256::digest(signing_input.as_bytes());
        key.private()
            .to_public_key()
            .verify(rsa::Pkcs1v15Sign::new::<Sha256>(), &digest, &signature)
            .expect("signature must cover the literal header.payload string in the token");
    }
}
