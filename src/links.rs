//! The links registry: `~/.config/lanyard/links.yaml`, or wherever
//! `LANYARD_LINKS` points.
//!
//! A list of **persona files** — the exact paths `lanyard link` was given.
//! Nothing is inferred from a directory, so what the registry holds is what the
//! developer typed, and a project may call its file whatever it likes.
//!
//! It lives in the *config* directory beside `users.yaml`, not in the data
//! directory: the data directory carries its own `.gitignore`, is documented as
//! deletable, and regenerates the same signing key when it is. Losing every
//! link because somebody cleared a cache would be a bad surprise.
//!
//! **Absent means empty**, and that is the one place this differs from
//! [`crate::config::PersonasSource`]: pointing `LANYARD_PERSONAS` at a file that
//! is not there leaves you with an empty picker, which is why it is fatal;
//! having no links is the normal condition of a fresh machine.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::LinksSource;

/// The registry, in file order. **The order is the precedence tie-break**, so it
/// is a list and it is preserved: first-linked wins, and rewriting the file must
/// not reshuffle it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Links {
    #[serde(default)]
    pub links: Vec<PathBuf>,
}

impl Links {
    /// Read the registry. An absent file is zero links, not an error; a
    /// malformed one **is** an error, so that `lanyard link` refuses rather
    /// than overwriting something a human hand-edited.
    pub fn load(source: &LinksSource) -> Result<Links, String> {
        let path = source.path();
        if !path.exists() {
            return Ok(Links::default());
        }
        let yaml = std::fs::read_to_string(path)
            .map_err(|e| format!("{}: cannot read links file: {e}", path.display()))?;
        Self::parse(&yaml, path)
    }

    pub fn parse(yaml: &str, path: &Path) -> Result<Links, String> {
        // An empty file is an empty registry rather than a parse error: a
        // registry emptied by `lanyard unlink` is a normal state to be in.
        if yaml.trim().is_empty() {
            return Ok(Links::default());
        }
        serde_norway::from_str(yaml)
            .map_err(|e| format!("{}: invalid links file: {e}", path.display()))
    }

    /// Write it back, creating the config directory if this is the first link.
    pub fn save(&self, source: &LinksSource) -> Result<(), String> {
        let path = source.path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("{}: cannot create directory: {e}", parent.display()))?;
        }
        let yaml = serde_norway::to_string(self)
            .map_err(|e| format!("{}: cannot serialize links: {e}", path.display()))?;
        std::fs::write(path, yaml)
            .map_err(|e| format!("{}: cannot write links file: {e}", path.display()))
    }

    /// **Idempotent, and appends.** Linking an already-linked file is a
    /// success that changes nothing — including its position, because that
    /// position is what makes first-linked-wins stable.
    pub fn add(&mut self, file: PathBuf) -> bool {
        if self.links.contains(&file) {
            return false;
        }
        self.links.push(file);
        true
    }

    pub fn remove(&mut self, file: &Path) -> bool {
        let before = self.links.len();
        self.links.retain(|held| held != file);
        self.links.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(dir: &tempfile::TempDir) -> LinksSource {
        LinksSource(dir.path().join("lanyard").join("links.yaml"))
    }

    /// The zero-config state of a fresh machine, and it must not be an error —
    /// `lanyard serve` and `lanyard link` both start here.
    #[test]
    fn an_absent_registry_reads_as_zero_links() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Links::load(&source(&dir)).unwrap(), Links::default());
    }

    #[test]
    fn a_registry_round_trips_through_the_file_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let source = source(&dir);

        let mut links = Links::default();
        assert!(links.add(PathBuf::from("/home/dev/code/billing/lanyard.yaml")));
        assert!(links.add(PathBuf::from("/home/dev/code/ops-console/lanyard.yaml")));
        links.save(&source).unwrap();

        let read = Links::load(&source).unwrap();
        assert_eq!(
            read.links,
            [
                PathBuf::from("/home/dev/code/billing/lanyard.yaml"),
                PathBuf::from("/home/dev/code/ops-console/lanyard.yaml")
            ],
            "order is the precedence tie-break, so it is preserved"
        );
    }

    #[test]
    fn adding_an_already_linked_file_changes_nothing() {
        let mut links = Links::default();
        links.add(PathBuf::from("/a"));
        links.add(PathBuf::from("/b"));
        assert!(!links.add(PathBuf::from("/a")), "already there");
        assert_eq!(links.links, [PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn removing_takes_the_entry_out_and_says_whether_it_was_there() {
        let mut links = Links::default();
        links.add(PathBuf::from("/a"));
        assert!(links.remove(Path::new("/a")));
        assert!(!links.remove(Path::new("/a")));
        assert!(links.links.is_empty());
    }

    /// Hand-editable, so a hand-edit that went wrong must be reported rather
    /// than silently replaced by whatever `lanyard link` was about to write.
    #[test]
    fn a_malformed_registry_is_an_error_that_names_the_file() {
        let err = Links::parse("linkz:\n  - /a\n", Path::new("/tmp/links.yaml")).unwrap_err();
        assert!(err.contains("linkz"), "{err}");
        assert!(err.contains("/tmp/links.yaml"), "{err}");
    }

    /// A registry emptied by `unlink` is a normal state, whether it was written
    /// as `links: []` or left blank by a human.
    #[test]
    fn an_empty_file_is_an_empty_registry() {
        assert_eq!(
            Links::parse("", Path::new("/tmp/links.yaml")).unwrap(),
            Links::default()
        );
    }

    /// The config directory may not exist at all on a fresh machine, and the
    /// first `lanyard link` is what creates it.
    #[test]
    fn saving_creates_the_config_directory() {
        let dir = tempfile::tempdir().unwrap();
        let source = source(&dir);
        let mut links = Links::default();
        links.add(PathBuf::from("/home/dev/code/billing/lanyard.yaml"));
        links.save(&source).unwrap();
        assert!(source.path().exists());
    }
}
