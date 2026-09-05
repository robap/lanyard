//! The six deliberate failure modes, and the only place their names exist.
//!
//! North star 4: producing a bad token is a feature, not a config hack. Three of
//! the six are claims ([`crate::oidc::issue`] applies them last, after the
//! caller's overrides), three are the header or the signature
//! ([`crate::oidc::jws`] applies those). Every surface — the grant, the seam,
//! the CLI — turns a string into a `Flaw` here and passes it along; none of them
//! builds a flawed token itself, because a second way to break a token is a
//! second way for the two to disagree.
//!
//! **Each flaw breaks exactly one thing.** A resource server stops at the first
//! check it fails, so a token that is both expired and wrongly signed says
//! nothing about which check ran.

/// Which single thing about the token is wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flaw {
    /// `iat`, `nbf` and `exp` all shift back by [`EXPIRED_SHIFT`].
    Expired,
    /// `aud` becomes `wrong-<requested>`.
    WrongAud,
    /// `iss` becomes [`WRONG_ISSUER`].
    WrongIss,
    /// The low bit of the signature's last byte is flipped.
    BadSignature,
    /// An unsecured JWT: `{"alg":"none","typ":"JWT"}` and an empty third segment.
    AlgNone,
    /// A real RS256 signature under a `kid` that is in no JWKS anywhere.
    UnknownKid,
}

/// An hour, not a second. Phase 2 measured a stock `AddJwtBearer` accepting a
/// lanyard token for 363 seconds — `exp` plus five minutes of default
/// `ClockSkew` — so a token expired by seconds is a negative test that passes
/// for the wrong reason. An hour clears that and every other default tolerance
/// we know of, and still reads as "expired" rather than "ancient".
pub const EXPIRED_SHIFT: u64 = 3600;

/// `.test` is a reserved TLD that is guaranteed not to resolve, so a client that
/// tries to fetch discovery from the token's `iss` fails immediately rather than
/// reaching something on the internet.
pub const WRONG_ISSUER: &str = "https://wrong-issuer.example.test";

/// A `kid` that is in no JWKS anywhere. Key rotation is not on the roadmap, so
/// this never has to avoid colliding with a second real key.
pub const UNKNOWN_KID: &str = "lanyard-unknown-kid";

/// The `aud` a `wrong-aud` token carries when none was requested, or when the
/// one that was requested is not a string.
pub const WRONG_AUDIENCE: &str = "wrong-audience";

impl Flaw {
    /// Declaration order is the order the six appear in the README, the CLI
    /// help and the rejection message — one list, written once.
    pub const ALL: [Flaw; 6] = [
        Flaw::Expired,
        Flaw::WrongAud,
        Flaw::WrongIss,
        Flaw::BadSignature,
        Flaw::AlgNone,
        Flaw::UnknownKid,
    ];

    /// The wire value, which is also the CLI flag with `--` in front.
    pub fn as_str(self) -> &'static str {
        match self {
            Flaw::Expired => "expired",
            Flaw::WrongAud => "wrong-aud",
            Flaw::WrongIss => "wrong-iss",
            Flaw::BadSignature => "bad-signature",
            Flaw::AlgNone => "alg-none",
            Flaw::UnknownKid => "unknown-kid",
        }
    }

    /// A known parameter with an unrecognized value is a refusal, not an ignored
    /// parameter (unlike an unknown parameter, which OAuth says to ignore):
    /// `flaw=expried` quietly minting a perfectly good token means a test suite
    /// that reports six passes and proves nothing. The `Err` is the
    /// `error_description` both surfaces return verbatim.
    pub fn parse(value: &str) -> Result<Flaw, String> {
        Flaw::ALL
            .into_iter()
            .find(|flaw| flaw.as_str() == value)
            .ok_or_else(|| {
                format!(
                    "unknown flaw {value:?}; expected one of {}",
                    Flaw::ALL
                        .iter()
                        .map(|flaw| flaw.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_wire_name_round_trips() {
        for flaw in Flaw::ALL {
            assert_eq!(Flaw::parse(flaw.as_str()), Ok(flaw));
        }
    }

    #[test]
    fn the_six_names_are_the_documented_ones() {
        let names: Vec<&str> = Flaw::ALL.iter().map(|f| f.as_str()).collect();
        assert_eq!(
            names,
            [
                "expired",
                "wrong-aud",
                "wrong-iss",
                "bad-signature",
                "alg-none",
                "unknown-kid"
            ]
        );
    }

    /// A typo'd negative test that quietly mints a good token is the failure
    /// this refusal exists to prevent, so the message has to name both the
    /// mistake and every way to spell it right.
    #[test]
    fn a_typo_is_rejected_with_a_message_naming_it_and_all_six() {
        let message = Flaw::parse("expried").unwrap_err();
        assert!(message.contains("expried"), "{message}");
        for name in Flaw::ALL.map(|f| f.as_str()) {
            assert!(message.contains(name), "{name} missing from: {message}");
        }
    }
}
