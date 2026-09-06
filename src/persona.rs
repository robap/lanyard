//! The persona model, kept protocol-neutral (CONCEPT §3): a stable id, an
//! email, a display name, roles, and a bag of arbitrary attributes. No `sub`, no
//! `preferred_username`, no `claims` key — the mapping to OIDC claims happens at
//! issue time, inside [`crate::oidc::issue`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::PersonasSource;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Persona {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default)]
    pub roles: Vec<String>,
    /// The escape hatch for anything the schema does not name, which is what
    /// makes rejecting unknown keys cost nobody anything.
    #[serde(default)]
    pub attributes: Map<String, Value>,
    /// **Which application this persona belongs to.** Read by
    /// [`crate::registry::Sourced::visible_to`] and nowhere else: a persona that
    /// declares one is on that `client_id`'s picker and on nobody else's.
    ///
    /// Carried in the schema since Phase 1 and read from Phase 7, which is what
    /// made namespacing an addition rather than a migration (CONCEPT §15). It is
    /// a label a persona wears, never a record lanyard keeps — nothing is
    /// refused for naming a `client_id` no persona mentions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
}

/// Where the list came from, so the banner can answer "why is Ada not there".
#[derive(Debug, Clone, PartialEq)]
pub enum Origin {
    BuiltIn,
    /// The global `users.yaml`, which replaces the built-ins entirely.
    File(PathBuf),
    /// A linked project's `lanyard.yaml`, which adds and never subtracts.
    Project(PathBuf),
}

#[derive(Debug, Clone)]
pub struct Personas {
    /// File-level `client:`, echoed back untouched. Both levels are preserved
    /// rather than resolved, because Phase 7 has not chosen a shape yet.
    pub client: Option<String>,
    pub list: Vec<Persona>,
    pub origin: Origin,
}

/// The on-disk schema. `deny_unknown_fields` and no `flatten` anywhere: the
/// tempting design of flattening unrecognised keys into `attributes` silently
/// swallows typos, which is the exact failure the fatal-file rule exists to
/// prevent.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersonaFile {
    #[serde(default)]
    client: Option<String>,
    personas: Vec<Persona>,
}

impl Personas {
    /// The three that ship when there is no file.
    pub fn builtin() -> Self {
        Personas {
            client: None,
            list: vec![
                Persona {
                    id: "ada".into(),
                    name: Some("Ada Bell".into()),
                    email: Some("ada@example.test".into()),
                    roles: vec!["admin".into(), "user".into()],
                    attributes: Map::new(),
                    client: None,
                },
                Persona {
                    id: "mira".into(),
                    name: Some("Mira Okonkwo".into()),
                    email: Some("mira@example.test".into()),
                    roles: vec!["user".into()],
                    attributes: Map::new(),
                    client: None,
                },
                // The user that breaks applications. A token for nobody carries
                // `sub` and the registered claims and nothing else.
                Persona {
                    id: "nobody".into(),
                    name: None,
                    email: None,
                    roles: Vec::new(),
                    attributes: Map::new(),
                    client: None,
                },
            ],
            origin: Origin::BuiltIn,
        }
    }

    pub fn load(source: &PersonasSource) -> Result<Self, String> {
        let path = source.path();

        if !path.exists() {
            return match source {
                // Asking for a specific file and not getting it is the same
                // class of mistake as a malformed one.
                PersonasSource::Explicit(_) => {
                    Err(format!("{}: no such personas file", path.display()))
                }
                PersonasSource::Default(_) => Ok(Self::builtin()),
            };
        }

        let yaml = std::fs::read_to_string(path)
            .map_err(|e| format!("{}: cannot read personas file: {e}", path.display()))?;
        Self::parse(&yaml, path)
    }

    pub fn parse(yaml: &str, path: &std::path::Path) -> Result<Self, String> {
        let file: PersonaFile = serde_norway::from_str(yaml)
            .map_err(|e| format!("{}: invalid personas file: {e}", path.display()))?;

        for (i, persona) in file.personas.iter().enumerate() {
            if let Some(first) = file.personas[..i].iter().find(|p| p.id == persona.id) {
                return Err(format!(
                    "{}: duplicate persona id {:?}",
                    path.display(),
                    first.id
                ));
            }
        }

        Ok(Personas {
            client: file.client,
            list: file.personas,
            origin: Origin::File(path.to_path_buf()),
        })
    }

    pub fn get(&self, id: &str) -> Option<&Persona> {
        self.list.iter().find(|p| p.id == id)
    }
}
