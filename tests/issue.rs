//! The one issuance function: the persona → claim mapping, override
//! precedence, and the stability of the default key's `kid`.

use serde_json::{json, Map, Value};

use lanyard_cli::keys::{SigningKey, DEFAULT_DEV_KEY_PEM};
use lanyard_cli::oidc::flaw::{Flaw, EXPIRED_SHIFT, WRONG_ISSUER};
use lanyard_cli::oidc::issue::{claims_at, issue, DEFAULT_TTL};
use lanyard_cli::persona::Personas;

const ISSUER: &str = "http://127.0.0.1:9500/oidc";
const NOW: u64 = 1_700_000_000;

fn overrides(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn mint(persona: Option<&str>, over: Value, ttl: u64) -> Map<String, Value> {
    flawed(persona, over, ttl, None)
}

fn flawed(persona: Option<&str>, over: Value, ttl: u64, flaw: Option<Flaw>) -> Map<String, Value> {
    let personas = Personas::builtin();
    let p = persona.map(|id| personas.get(id).unwrap());
    claims_at(NOW, ISSUER, p, &overrides(over), ttl, "test-jti", flaw)
}

#[test]
fn the_registered_claims_are_always_present() {
    let c = mint(None, json!({"sub": "ada"}), DEFAULT_TTL);

    assert_eq!(c["iss"], ISSUER);
    assert_eq!(c["iat"], NOW);
    assert_eq!(c["nbf"], NOW, "nbf tracks iat");
    assert_eq!(c["exp"], NOW + DEFAULT_TTL);
    assert_eq!(c["jti"], "test-jti");
}

#[test]
fn exp_follows_the_requested_ttl() {
    let c = mint(None, json!({}), 300);
    assert_eq!(c["exp"].as_u64().unwrap() - c["iat"].as_u64().unwrap(), 300);
}

/// `aud` is minted only when asked for. An `aud` that matches no real API is a
/// test case, not an error (CONCEPT §5).
#[test]
fn aud_appears_only_when_requested() {
    assert!(mint(None, json!({}), DEFAULT_TTL).get("aud").is_none());
    assert_eq!(
        mint(None, json!({"aud": "billing-api"}), DEFAULT_TTL)["aud"],
        "billing-api"
    );
}

#[test]
fn a_persona_maps_onto_the_documented_claims() {
    let c = mint(Some("ada"), json!({}), DEFAULT_TTL);

    assert_eq!(c["sub"], "ada");
    assert_eq!(c["preferred_username"], "ada");
    assert_eq!(c["email"], "ada@example.test");
    assert_eq!(c["email_verified"], true);
    assert_eq!(c["name"], "Ada Bell");
    assert_eq!(c["roles"], json!(["admin", "user"]));
}

/// The user that breaks applications: `sub` and the registered claims, and
/// nothing else at all.
#[test]
fn nobody_mints_sub_and_the_registered_claims_only() {
    let c = mint(Some("nobody"), json!({}), DEFAULT_TTL);

    assert_eq!(c["sub"], "nobody");
    for absent in [
        "email",
        "email_verified",
        "name",
        "roles",
        "preferred_username",
    ] {
        assert!(c.get(absent).is_none(), "nobody must not carry {absent}");
    }

    let mut keys: Vec<&str> = c.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["exp", "iat", "iss", "jti", "nbf", "sub"]);
}

#[test]
fn attributes_become_top_level_claims() {
    let yaml =
        "personas:\n  - id: ada\n    attributes:\n      department: platform\n      level: 7\n";
    let personas = Personas::parse(yaml, std::path::Path::new("/tmp/users.yaml")).unwrap();
    let c = claims_at(
        NOW,
        ISSUER,
        personas.get("ada"),
        &Map::new(),
        DEFAULT_TTL,
        "test-jti",
        None,
    );

    assert_eq!(c["department"], "platform");
    assert_eq!(c["level"], 7);
}

#[test]
fn body_claims_win_over_persona_claims() {
    let c = mint(
        Some("ada"),
        json!({"email": "other@example.test"}),
        DEFAULT_TTL,
    );
    assert_eq!(c["email"], "other@example.test");
    assert_eq!(
        c["email_verified"], true,
        "the rest of the persona survives"
    );
}

/// Overriding `iss` and `exp` is deliberate: it is what makes Phase 3's
/// `--expired` and `--wrong-iss` thin sugar over this one function rather than a
/// second code path.
#[test]
fn body_claims_win_over_the_registered_claims_too() {
    let c = mint(
        None,
        json!({"iss": "http://evil.test", "exp": 1, "iat": 2, "jti": "fixed"}),
        DEFAULT_TTL,
    );
    assert_eq!(c["iss"], "http://evil.test");
    assert_eq!(c["exp"], 1);
    assert_eq!(c["iat"], 2);
    assert_eq!(c["jti"], "fixed");
}

#[test]
fn issuing_signs_the_claims_it_returns() {
    let key = SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap();
    let personas = Personas::builtin();
    let issued = issue(
        &key,
        ISSUER,
        personas.get("ada"),
        &overrides(json!({"aud": "billing-api"})),
        DEFAULT_TTL,
        None,
    )
    .unwrap();
    let (token, claims) = (issued.token, issued.claims);

    let payload = token.split('.').nth(1).unwrap();
    let decoded: Value =
        serde_json::from_slice(&lanyard_cli::b64::decode(payload).unwrap()).unwrap();
    assert_eq!(
        decoded,
        Value::Object(claims),
        "the echoed claims must be the signed claims"
    );
    assert_eq!(decoded["aud"], "billing-api");
    assert_eq!(decoded["sub"], "ada");
}

#[test]
fn each_token_gets_its_own_jti() {
    let key = SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap();
    let a = issue(&key, ISSUER, None, &Map::new(), DEFAULT_TTL, None).unwrap();
    let b = issue(&key, ISSUER, None, &Map::new(), DEFAULT_TTL, None).unwrap();
    assert_ne!(a.claims["jti"], b.claims["jti"]);
}

/// North star 4: a rotating key silently breaks every cached JWKS, so the
/// built-in key's `kid` is pinned against a known-good value rather than
/// against itself. Cross-checked against `jose`'s `calculateJwkThumbprint`.
#[test]
fn the_default_keys_kid_is_the_known_good_thumbprint() {
    let key = SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap();
    assert_eq!(key.kid(), "TXntCt2biz2Bj578hZocZOb2A2nQV9JfrBvFN55QWpU");
}

// ------------------------------------------------- the flaw is applied last --

/// Phase 1 wrote "the body wins over everything"; the flaw is the one input that
/// outranks it. `flaw=expired` returning a live token because the body happened
/// to carry an `exp` is a negative test that quietly passes — the failure mode
/// this whole phase exists to prevent (north star 4).
#[test]
fn expired_outranks_an_exp_the_body_posted() {
    let c = flawed(
        Some("ada"),
        json!({"exp": NOW + 99_999, "iat": NOW, "nbf": NOW}),
        DEFAULT_TTL,
        Some(Flaw::Expired),
    );

    assert_eq!(c["iat"], NOW - EXPIRED_SHIFT);
    assert_eq!(c["nbf"], NOW - EXPIRED_SHIFT, "nbf moves with iat");
    assert_eq!(
        c["exp"],
        NOW - EXPIRED_SHIFT + DEFAULT_TTL,
        "the lifetime stays 60 seconds; the whole token shifts back"
    );
    assert!(c["exp"].as_u64().unwrap() < NOW, "still expired");
}

/// A token issued now that expired an hour ago is a shape no IdP produces, and
/// some libraries reject it for the wrong reason.
#[test]
fn expired_keeps_the_lifetime_and_moves_the_whole_token() {
    let c = flawed(Some("ada"), json!({}), 300, Some(Flaw::Expired));
    assert_eq!(c["exp"].as_u64().unwrap() - c["iat"].as_u64().unwrap(), 300);
    assert_eq!(NOW - c["exp"].as_u64().unwrap(), EXPIRED_SHIFT - 300);
    assert_eq!(c["sub"], "ada", "only the timestamps are wrong");
    assert_eq!(c["iss"], ISSUER);
}

/// `wrong-<requested>` cannot collide with what was asked for, needs no
/// collision check, and reads correctly in .NET's refusal message.
#[test]
fn wrong_aud_prefixes_the_audience_that_was_asked_for() {
    let c = flawed(
        None,
        json!({"aud": "billing-api"}),
        DEFAULT_TTL,
        Some(Flaw::WrongAud),
    );
    assert_eq!(c["aud"], "wrong-billing-api");
    assert_eq!(c["iss"], ISSUER, "only aud is wrong");
}

#[test]
fn wrong_aud_with_no_audience_asked_for_is_wrong_audience() {
    let c = flawed(None, json!({}), DEFAULT_TTL, Some(Flaw::WrongAud));
    assert_eq!(c["aud"], "wrong-audience");
}

/// The seam's body can post an array, and `wrong-` prefixed onto a list is not a
/// string: it is replaced wholesale.
#[test]
fn a_non_string_aud_is_replaced_wholesale() {
    let c = flawed(
        None,
        json!({"aud": ["billing-api", "orders-api"]}),
        DEFAULT_TTL,
        Some(Flaw::WrongAud),
    );
    assert_eq!(c["aud"], "wrong-audience");
}

#[test]
fn wrong_iss_outranks_an_iss_the_body_posted() {
    let c = flawed(
        None,
        json!({"iss": "http://evil.test", "aud": "billing-api"}),
        DEFAULT_TTL,
        Some(Flaw::WrongIss),
    );
    assert_eq!(c["iss"], WRONG_ISSUER);
    assert_eq!(c["aud"], "billing-api", "only iss is wrong");
    assert_eq!(c["exp"], NOW + DEFAULT_TTL, "the lifetime is untouched");
}

/// The three header-level flaws are not claims. `claims` for them is exactly
/// what an unflawed request would have produced, which is why the seam echoes
/// the `flaw` it was asked for — it is the only way a fixture can tell.
#[test]
fn the_header_level_flaws_leave_the_claims_alone() {
    let good = mint(Some("ada"), json!({"aud": "billing-api"}), DEFAULT_TTL);
    for flaw in [Flaw::BadSignature, Flaw::AlgNone, Flaw::UnknownKid] {
        assert_eq!(
            flawed(
                Some("ada"),
                json!({"aud": "billing-api"}),
                DEFAULT_TTL,
                Some(flaw)
            ),
            good,
            "{} must not touch the claims",
            flaw.as_str()
        );
    }
}

/// `expires_in` is `exp - issued_at`, and reading the clock a second time in the
/// handler would make the ordinary case report 59 or 61 at random.
#[test]
fn issuing_hands_back_the_moment_it_used() {
    let key = SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap();
    let issued = issue(&key, ISSUER, None, &Map::new(), DEFAULT_TTL, None).unwrap();
    assert_eq!(
        issued.claims["exp"].as_u64().unwrap() - issued.issued_at,
        DEFAULT_TTL
    );
}

#[test]
fn issuing_expired_signs_the_shifted_claims() {
    let key = SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap();
    let issued = issue(
        &key,
        ISSUER,
        None,
        &Map::new(),
        DEFAULT_TTL,
        Some(Flaw::Expired),
    )
    .unwrap();

    let payload: Value = serde_json::from_slice(
        &lanyard_cli::b64::decode(issued.token.split('.').nth(1).unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(payload, Value::Object(issued.claims));
    assert!(payload["exp"].as_u64().unwrap() < issued.issued_at);
}
