//! The authorization code record, and the PKCE challenge it may be bound to.
//!
//! **Verification lives here rather than at the token endpoint** for the same
//! reason the claim table lives in `issue`: there is one place that decides
//! whether a code is good, and `/oidc/token` reads its answer.

use sha2::{Digest as _, Sha256};

use crate::b64;
use crate::oidc::authorize::AuthRequest;
use crate::persona::Persona;

/// RFC 7636's two transformations. `plain` is here because RFC 7636 §4.4.1
/// defines it as the default when `code_challenge_method` is omitted, and an
/// SDK that omits it is an SDK a local IdP has to be able to log in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeMethod {
    S256,
    Plain,
}

impl ChallengeMethod {
    /// An absent method is `plain` (RFC 7636 §4.3), which is a default rather
    /// than a guess.
    pub fn parse(raw: Option<&str>) -> Result<ChallengeMethod, String> {
        match raw {
            None | Some("plain") => Ok(ChallengeMethod::Plain),
            Some("S256") => Ok(ChallengeMethod::S256),
            Some(other) => Err(format!(
                "code_challenge_method {other:?} is not supported; \
                 lanyard supports S256 and plain"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ChallengeMethod::S256 => "S256",
            ChallengeMethod::Plain => "plain",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    pub value: String,
    pub method: ChallengeMethod,
}

impl Challenge {
    /// **Genuinely verified, and that is not a contradiction of north star 1.**
    /// Accept-everything is about registration — who you say you are, where you
    /// say you want to come back to. It was never about skipping the
    /// cryptography. A local IdP that rubber-stamps a wrong `code_verifier`
    /// lets a broken PKCE implementation ship, and production is a bad place to
    /// find that out.
    pub fn verify(&self, verifier: &str) -> bool {
        let presented = match self.method {
            ChallengeMethod::Plain => verifier.to_string(),
            ChallengeMethod::S256 => b64::encode(Sha256::digest(verifier.as_bytes())),
        };
        // Not constant-time, on purpose and with a straight face: the challenge
        // is a public value the client just sent us, and the thing being
        // compared is single-use for 60 seconds on loopback. A timing-safe
        // compare here would be security theatre in a tool whose signing key is
        // published in its own repository.
        presented == self.value
    }
}

/// What a code stands for: who was chosen, when they were chosen, and the
/// request that is waiting for them.
///
/// The persona is **owned**, not an id, because "mint one now" produces an
/// identity that is in no file and has to travel the identical path to a loaded
/// one (spec, open question 10).
pub struct CodeRecord {
    pub persona: Persona,
    /// Unix seconds. Becomes the ID token's `auth_time`, and may predate `iat`
    /// when the selection was remembered from an earlier login.
    pub auth_time: u64,
    pub request: AuthRequest,
    /// The browser session this login belongs to, carried so that a refresh
    /// token minted from this code knows which **Log out** revokes it. Absent
    /// for a login that never had a session — which a curl-only code flow does
    /// not, and which is not an error.
    pub session_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7636 §B's worked example, so this is checked against the RFC rather
    /// than against itself.
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    #[test]
    fn an_absent_method_is_plain_per_rfc_7636() {
        assert_eq!(ChallengeMethod::parse(None), Ok(ChallengeMethod::Plain));
        assert_eq!(
            ChallengeMethod::parse(Some("plain")),
            Ok(ChallengeMethod::Plain)
        );
        assert_eq!(
            ChallengeMethod::parse(Some("S256")),
            Ok(ChallengeMethod::S256)
        );
    }

    #[test]
    fn an_unknown_method_is_refused_and_names_itself() {
        let err = ChallengeMethod::parse(Some("S512")).unwrap_err();
        assert!(err.contains("S512"), "{err}");
        assert!(err.contains("S256"), "and says what is supported: {err}");
        // Case matters: RFC 7636 spells it S256, and `s256` is a different
        // string that a strict provider would reject — so lanyard does too,
        // rather than teaching a client a spelling production will refuse.
        assert!(ChallengeMethod::parse(Some("s256")).is_err());
    }

    #[test]
    fn s256_matches_the_rfc_worked_example() {
        let challenge = Challenge {
            value: CHALLENGE.to_string(),
            method: ChallengeMethod::S256,
        };
        assert!(challenge.verify(VERIFIER));
    }

    #[test]
    fn one_character_different_does_not_verify() {
        let challenge = Challenge {
            value: CHALLENGE.to_string(),
            method: ChallengeMethod::S256,
        };
        let mut wrong = VERIFIER.to_string();
        wrong.replace_range(0..1, "e");
        assert!(!challenge.verify(&wrong), "{wrong} must not verify");
        assert!(!challenge.verify(""));
    }

    #[test]
    fn plain_compares_the_verifier_to_the_challenge_literally() {
        let challenge = Challenge {
            value: VERIFIER.to_string(),
            method: ChallengeMethod::Plain,
        };
        assert!(challenge.verify(VERIFIER));
        assert!(!challenge.verify(CHALLENGE));
    }

    /// The S256 challenge is unpadded base64url. A padded or standard-alphabet
    /// encoding would verify against nothing a real client sends.
    #[test]
    fn the_s256_transformation_is_unpadded_base64url() {
        let computed = b64::encode(Sha256::digest(VERIFIER.as_bytes()));
        assert_eq!(computed, CHALLENGE);
        assert!(!computed.contains('=') && !computed.contains('+') && !computed.contains('/'));
    }
}
