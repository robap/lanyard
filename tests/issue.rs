//! The one issuance function: the persona → claim mapping, override
//! precedence, and the stability of the default key's `kid`.

use serde_json::{json, Map, Value};

use lanyard_cli::keys::{SigningKey, DEFAULT_DEV_KEY_PEM};
use lanyard_cli::oidc::issue::{claims_at, issue, DEFAULT_TTL};
use lanyard_cli::persona::Personas;

const ISSUER: &str = "http://127.0.0.1:9500/oidc";
const NOW: u64 = 1_700_000_000;

fn overrides(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn mint(persona: Option<&str>, over: Value, ttl: u64) -> Map<String, Value> {
    let personas = Personas::builtin();
    let p = persona.map(|id| personas.get(id).unwrap());
    claims_at(NOW, ISSUER, p, &overrides(over), ttl, "test-jti")
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
    let (token, claims) = issue(
        &key,
        ISSUER,
        personas.get("ada"),
        &overrides(json!({"aud": "billing-api"})),
        DEFAULT_TTL,
    )
    .unwrap();

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
    let (_, a) = issue(&key, ISSUER, None, &Map::new(), DEFAULT_TTL).unwrap();
    let (_, b) = issue(&key, ISSUER, None, &Map::new(), DEFAULT_TTL).unwrap();
    assert_ne!(a["jti"], b["jti"]);
}

/// North star 4: a rotating key silently breaks every cached JWKS, so the
/// built-in key's `kid` is pinned against a known-good value rather than
/// against itself. Cross-checked against `jose`'s `calculateJwkThumbprint`.
#[test]
fn the_default_keys_kid_is_the_known_good_thumbprint() {
    let key = SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap();
    assert_eq!(key.kid(), "TXntCt2biz2Bj578hZocZOb2A2nQV9JfrBvFN55QWpU");
}
