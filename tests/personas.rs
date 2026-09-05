//! Persona loading: the built-in defaults, the file schema, and the ways a file
//! is fatal.
//!
//! A malformed file must never fall back to the defaults with a warning. You
//! would believe your personas loaded, and the symptom would appear three
//! redirects later as a name missing from the picker.

use std::path::Path;

use lanyard_cli::config::PersonasSource;
use lanyard_cli::persona::{Origin, Personas};

fn parse(yaml: &str) -> Result<Personas, String> {
    Personas::parse(yaml, Path::new("/tmp/users.yaml"))
}

#[test]
fn the_built_in_defaults_are_ada_mira_and_nobody() {
    let p = Personas::builtin();
    let ids: Vec<&str> = p.list.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["ada", "mira", "nobody"]);
    assert_eq!(p.origin, Origin::BuiltIn);

    let ada = p.get("ada").unwrap();
    assert_eq!(ada.email.as_deref(), Some("ada@example.test"));
    assert_eq!(ada.name.as_deref(), Some("Ada Bell"));
    assert!(ada.roles.contains(&"admin".to_string()));
}

/// CONCEPT §3 calls this one out specifically: the user that breaks
/// applications and the one nobody remembers to create.
#[test]
fn nobody_has_no_email_no_name_and_no_roles() {
    let nobody = Personas::builtin().get("nobody").unwrap().clone();
    assert_eq!(nobody.email, None);
    assert!(nobody.roles.is_empty());
    assert!(nobody.attributes.is_empty());
}

#[test]
fn a_file_replaces_the_built_ins_entirely() {
    let p = parse("personas:\n  - id: solo\n    name: Solo\n").unwrap();
    let ids: Vec<&str> = p.list.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["solo"], "the built-ins must not be merged in");
    assert_eq!(p.origin, Origin::File("/tmp/users.yaml".into()));
}

#[test]
fn client_is_carried_at_both_levels_and_echoed_untouched() {
    let p = parse(
        "client: billing-web\n\
         personas:\n\
         \x20 - id: ada\n\
         \x20 - id: ops\n\
         \x20   client: ops-console\n",
    )
    .unwrap();

    assert_eq!(p.client.as_deref(), Some("billing-web"));
    assert_eq!(p.get("ada").unwrap().client, None);
    assert_eq!(p.get("ops").unwrap().client.as_deref(), Some("ops-console"));
    assert_eq!(p.list.len(), 2, "nothing filters on client until Phase 7");
}

#[test]
fn attributes_are_an_explicit_map_not_a_catch_all() {
    let p = parse(
        "personas:\n\
         \x20 - id: ada\n\
         \x20   attributes:\n\
         \x20     department: platform\n\
         \x20     level: 7\n",
    )
    .unwrap();
    let ada = p.get("ada").unwrap();
    assert_eq!(ada.attributes["department"], "platform");
    assert_eq!(ada.attributes["level"], 7);
}

#[test]
fn an_unknown_key_is_fatal_and_names_the_key_and_the_file() {
    let err = parse("personas:\n  - id: ada\n    rolez: [admin]\n").unwrap_err();
    assert!(err.contains("rolez"), "must name the offending key: {err}");
    assert!(err.contains("/tmp/users.yaml"), "must name the file: {err}");
}

#[test]
fn a_missing_id_is_fatal_and_names_the_file() {
    let err = parse("personas:\n  - name: Ada Bell\n").unwrap_err();
    assert!(err.contains("id"), "must name the missing field: {err}");
    assert!(err.contains("/tmp/users.yaml"), "must name the file: {err}");
}

#[test]
fn a_duplicate_id_is_fatal_and_names_the_id() {
    let err = parse("personas:\n  - id: ada\n  - id: mira\n  - id: ada\n").unwrap_err();
    assert!(err.contains("ada"), "must name the duplicated id: {err}");
    assert!(err.contains("/tmp/users.yaml"), "must name the file: {err}");
}

#[test]
fn an_unknown_top_level_key_is_fatal() {
    let err = parse("clientz: x\npersonas:\n  - id: ada\n").unwrap_err();
    assert!(err.contains("clientz"), "{err}");
}

#[test]
fn an_absent_default_file_yields_the_built_ins() {
    let dir = tempfile::tempdir().unwrap();
    let source = PersonasSource::Default(dir.path().join("users.yaml"));
    let p = Personas::load(&source).unwrap();
    assert_eq!(p.origin, Origin::BuiltIn);
    assert_eq!(p.list.len(), 3);
}

/// Pointing `LANYARD_PERSONAS` at a file that is not there is the same class of
/// mistake as a malformed one: you believe personas loaded and they did not.
#[test]
fn an_absent_explicit_file_is_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.yaml");
    let err = Personas::load(&PersonasSource::Explicit(missing.clone())).unwrap_err();
    assert!(err.contains(missing.to_str().unwrap()), "{err}");
}

#[test]
fn a_present_file_is_loaded_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("users.yaml");
    std::fs::write(&path, "personas:\n  - id: solo\n").unwrap();

    let p = Personas::load(&PersonasSource::Explicit(path.clone())).unwrap();
    assert_eq!(p.origin, Origin::File(path));
    assert_eq!(p.list.len(), 1);
}
