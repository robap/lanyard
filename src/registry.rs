//! Where personas come from, merged, and who is allowed to see them.
//!
//! Phase 1 gave [`crate::persona::Personas`] one job: parse one file. That is
//! still its job. This module holds *many* of them — the built-ins or the global
//! file, plus every linked project — and answers the one question the rest of
//! the process asks: **which people can this `client_id` see right now.**
//!
//! Two shapes carry the module:
//!
//! - **[`Resolved`] is a snapshot, handed out behind an `Arc`.** A handler
//!   resolves once, reads it for the length of the request, and never holds a
//!   lock across an `.await`.
//! - **[`Registry::resolve`] is stat-guarded, not watched.** The observable
//!   requirement is "edit the file, reload the page, see the change"; a
//!   modified-time comparison delivers that without `notify`, its platform
//!   backends, or the atomic-rename problem editors create (north star 5).

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::config::{LinksSource, PersonasSource};
use crate::links::Links;
use crate::persona::{Origin, Persona, Personas};

/// One persona plus the two things the persona itself does not carry: the
/// `client:` that actually applies to it, and the file it came out of.
#[derive(Debug, Clone, PartialEq)]
pub struct Sourced {
    pub persona: Persona,
    /// The **effective** `client:` — the persona's own, or the file's, or none.
    pub client: Option<String>,
    pub origin: Origin,
}

impl Sourced {
    pub fn id(&self) -> &str {
        &self.persona.id
    }

    /// **The visibility rule**, and there is no second copy of it:
    ///
    /// > A persona is visible to a request if it declares no `client:`, or if
    /// > its `client:` equals that request's `client_id`.
    ///
    /// A request with no `client_id` is not an exemption — it sees what an
    /// unrecognised `client_id` sees, which is the unscoped personas.
    pub fn visible_to(&self, client_id: Option<&str>) -> bool {
        match &self.client {
            None => true,
            Some(scoped) => client_id == Some(scoped.as_str()),
        }
    }

    /// Where it came from, as the page and the banner say it.
    pub fn source(&self) -> String {
        source_label(&self.origin)
    }
}

pub fn source_label(origin: &Origin) -> String {
    match origin {
        Origin::BuiltIn => "built-in defaults".to_string(),
        Origin::File(path) | Origin::Project(path) => path.display().to_string(),
    }
}

/// One thing wrong with one source, in the sentence a developer reads.
///
/// A string rather than a struct on purpose: it is rendered on the banner, in a
/// band on `/_/`, on the event stream and in `/_/api/personas` — four surfaces
/// that all want the same sentence, and none of which wants to compose it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Warning(pub String);

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One source's contribution, for the banner: what it is, what it gave, and
/// what went wrong with it if anything. A broken source is listed **with** its
/// error rather than omitted — "why is Ada not there" is answerable from the
/// banner or it is not answerable at all.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceState {
    pub origin: Origin,
    pub ids: Vec<String>,
    pub error: Option<String>,
}

/// A snapshot of every source, merged. Ids may repeat across sources: which one
/// wins depends on who is asking, so the ladder is applied at read time rather
/// than baked into the list.
#[derive(Debug, Default)]
pub struct Resolved {
    all: Vec<Sourced>,
    pub warnings: Vec<Warning>,
    pub sources: Vec<SourceState>,
}

impl Resolved {
    /// Every persona from every source, in source order, unfiltered and
    /// un-deduplicated. The debugging view.
    pub fn all(&self) -> &[Sourced] {
        &self.all
    }

    /// The people this `client_id` can see, one per id, in source order.
    ///
    /// An id collision only matters between two personas **both visible to the
    /// same request**, which is why the ladder is applied here rather than when
    /// the sources were merged.
    pub fn visible_to(&self, client_id: Option<&str>) -> Vec<&Sourced> {
        let mut winners: Vec<usize> = Vec::new();
        for (index, sourced) in self.all.iter().enumerate() {
            if !sourced.visible_to(client_id) {
                continue;
            }
            match winners
                .iter()
                .position(|&held| self.all[held].id() == sourced.id())
            {
                Some(slot) => {
                    if rank(sourced, index) < rank(&self.all[winners[slot]], winners[slot]) {
                        winners[slot] = index;
                    }
                }
                None => winners.push(index),
            }
        }
        // **The winner takes the losing row's place** rather than the list
        // being re-sorted: the position is the id's first appearance across the
        // sources, so a project that shadows `ada` shows its own Ada where Ada
        // already was. A picker that reordered itself on a shadow would be
        // reporting the collision by moving somebody.
        winners.into_iter().map(|i| &self.all[i]).collect()
    }

    /// One visible persona by id.
    pub fn get(&self, id: &str, client_id: Option<&str>) -> Option<&Sourced> {
        self.visible_to(client_id)
            .into_iter()
            .find(|s| s.id() == id)
    }

    /// **Why that persona was not available**, in the sentence that makes this
    /// feature usable rather than maddening.
    ///
    /// `no such persona` is technically true and useless: the persona is right
    /// there in a file the developer can see, and the only thing wrong is which
    /// application asked. So the refusal names the file, the client it is scoped
    /// to, and the flag that fixes it.
    pub fn missing_persona(&self, id: &str, client_id: Option<&str>) -> String {
        let asked_as = match client_id {
            Some(client_id) => format!("for client {client_id:?}"),
            None => "for a request with no client_id".to_string(),
        };
        match self.find_anywhere(id) {
            Some(sourced) => {
                let scoped_to = sourced.client.as_deref().unwrap_or_default();
                format!(
                    "no persona {id:?} {asked_as} — it is defined in {} scoped to client \
                     {scoped_to:?}. Retry with --client {scoped_to}",
                    sourced.source(),
                )
            }
            None => format!("no persona with id {id:?} is loaded"),
        }
    }

    /// The best-ranked persona with this id **regardless of who can see it** —
    /// the lookup behind the useful refusal, which has to be able to say "it is
    /// defined over there, scoped to somebody else".
    pub fn find_anywhere(&self, id: &str) -> Option<&Sourced> {
        self.all
            .iter()
            .enumerate()
            .filter(|(_, s)| s.id() == id)
            .min_by_key(|(i, s)| rank(s, *i))
            .map(|(_, s)| s)
    }
}

/// **The precedence ladder**, as a sort key: a client-scoped persona beats an
/// unscoped one; among equals a project file beats the global file; among
/// project files the one linked first wins.
///
/// First-linked rather than last-linked, so that linking a new project can
/// never silently steal an id out from under a project that already had it.
fn rank(sourced: &Sourced, index: usize) -> (u8, u8, usize) {
    (
        u8::from(sourced.client.is_none()),
        match sourced.origin {
            Origin::Project(_) => 0,
            Origin::BuiltIn | Origin::File(_) => 1,
        },
        index,
    )
}

/// Where the unscoped base list comes from.
enum Global {
    /// A parsed list handed in directly — the test harness, and nothing else.
    Fixed(Personas),
    /// The real thing: the built-ins, or `users.yaml`, re-read when it changes.
    Source(PersonasSource),
}

/// The merged persona sources, and the cache that makes re-reading them cheap.
pub struct Registry {
    global: Global,
    /// Where the linked projects are listed. `None` for the fixed test
    /// registry, which has no files at all.
    links: Option<LinksSource>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    files: HashMap<PathBuf, Cached>,
    /// The registry file gets its own cache slot: it is stat-guarded like every
    /// persona file, but it parses to a list of paths rather than to people.
    links_stamp: Option<Stamp>,
    links: Option<Result<Links, String>>,
    resolved: Option<Arc<Resolved>>,
}

struct Cached {
    stamp: Option<Stamp>,
    /// `None` when the file was not there at all.
    parsed: Option<Result<Personas, String>>,
}

/// Enough of a `stat` to notice an edit. Modified time **and** length, because
/// a filesystem with one-second timestamp resolution can otherwise hide a same
/// second rewrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl Registry {
    /// A registry over one already-parsed list. Nothing is re-read: this is the
    /// in-process construction the test harness uses.
    pub fn fixed(personas: Personas) -> Registry {
        Registry {
            global: Global::Fixed(personas),
            links: None,
            state: Mutex::new(State::default()),
        }
    }

    /// The real registry: the global persona source and every linked project,
    /// each re-read when it changes.
    pub fn new(global: PersonasSource, links: LinksSource) -> Registry {
        Registry {
            global: Global::Source(global),
            links: Some(links),
            state: Mutex::new(State::default()),
        }
    }

    /// The current snapshot, re-resolving only what changed on disk.
    ///
    /// Synchronous, and deliberately so: `std::fs` behind a `Mutex` on files
    /// that are tens of lines long, guarded by a stat, in a loopback dev tool.
    /// `tokio::fs` would put an `.await` inside the lock, which is strictly
    /// worse.
    pub fn resolve(&self) -> Arc<Resolved> {
        let mut state = self.state.lock().expect("registry");

        let mut changed = state.resolved.is_none();
        let mut sources: Vec<Contribution> = Vec::new();
        self.gather(&mut state, &mut sources, &mut changed);

        if !changed {
            if let Some(resolved) = &state.resolved {
                return resolved.clone();
            }
        }

        let registry_error = match &state.links {
            Some(Err(message)) => Some(message.clone()),
            _ => None,
        };
        let resolved = Arc::new(Resolved::merge(sources, registry_error));
        state.resolved = Some(resolved.clone());
        resolved
    }

    /// Every source, in precedence order, each already read (from the cache
    /// when its stamp says nothing moved).
    fn gather(&self, state: &mut State, out: &mut Vec<Contribution>, changed: &mut bool) {
        match &self.global {
            Global::Fixed(personas) => out.push(Contribution::Loaded(personas.clone())),
            Global::Source(source) => {
                let path = source.path().clone();
                match read_cached(state, &path, changed) {
                    Some(Ok(personas)) => out.push(Contribution::Loaded(personas)),
                    Some(Err(message)) => out.push(Contribution::Broken {
                        origin: Origin::File(path),
                        message,
                    }),
                    // Asking for a specific file and not getting it is the same
                    // class of mistake as a malformed one; the conventional path
                    // simply being absent is the zero-config state.
                    None => match source {
                        PersonasSource::Explicit(_) => out.push(Contribution::Broken {
                            origin: Origin::File(path.clone()),
                            message: format!("{}: no such personas file", path.display()),
                        }),
                        PersonasSource::Default(_) => {
                            out.push(Contribution::Loaded(Personas::builtin()))
                        }
                    },
                }
            }
        }

        // **Then the linked projects, in registry order** — the order is the
        // precedence tie-break, so it is read as a list and never reordered.
        for file in self.linked(state, changed) {
            match read_cached(state, &file, changed) {
                Some(Ok(mut personas)) => {
                    // A project file is a `Project` origin rather than a `File`
                    // one, which is the whole of "a project beats the global
                    // file" on the precedence ladder.
                    personas.origin = Origin::Project(file);
                    out.push(Contribution::Loaded(personas));
                }
                Some(Err(message)) => out.push(Contribution::Broken {
                    origin: Origin::Project(file),
                    message,
                }),
                // A linked file can be temporarily absent — its directory
                // moved, or on a volume that is not mounted — so this warns and
                // prunes nothing. `lanyard unlink` is what removes an entry.
                None => out.push(Contribution::Broken {
                    origin: Origin::Project(file.clone()),
                    message: format!("{}: no such file", file.display()),
                }),
            }
        }

        // A source that is gone stops being re-statted; without this the cache
        // is a leak that grows every time a project is unlinked.
        let live: Vec<PathBuf> = out
            .iter()
            .filter_map(|c| match c {
                Contribution::Loaded(personas) => match &personas.origin {
                    Origin::File(path) | Origin::Project(path) => Some(path.clone()),
                    Origin::BuiltIn => None,
                },
                Contribution::Broken { origin, .. } => match origin {
                    Origin::File(path) | Origin::Project(path) => Some(path.clone()),
                    Origin::BuiltIn => None,
                },
            })
            .collect();
        state.files.retain(|path, _| live.contains(path));
    }
}

/// What one source gave this resolve: a parsed list, or the reason it gave
/// nothing. A broken source is carried rather than dropped — it still gets a
/// line on the banner and a warning of its own.
enum Contribution {
    Loaded(Personas),
    Broken { origin: Origin, message: String },
}

impl Resolved {
    fn merge(contributions: Vec<Contribution>, registry_error: Option<String>) -> Resolved {
        let mut all: Vec<Sourced> = Vec::new();
        let mut sources: Vec<SourceState> = Vec::new();
        let mut warnings: Vec<Warning> = registry_error.into_iter().map(Warning).collect();

        for contribution in contributions {
            match contribution {
                Contribution::Loaded(personas) => {
                    sources.push(SourceState {
                        origin: personas.origin.clone(),
                        ids: personas.list.iter().map(|p| p.id.clone()).collect(),
                        error: None,
                    });
                    for persona in personas.list {
                        // A persona-level `client:` wins; a file-level one
                        // applies to everybody who declared none. There is no
                        // third level and no inheritance beyond that.
                        let client = persona.client.clone().or_else(|| personas.client.clone());
                        all.push(Sourced {
                            persona,
                            client,
                            origin: personas.origin.clone(),
                        });
                    }
                }
                Contribution::Broken { origin, message } => {
                    sources.push(SourceState {
                        origin,
                        ids: Vec::new(),
                        error: Some(message.clone()),
                    });
                    warnings.push(Warning(message));
                }
            }
        }

        warnings.extend(shadow_warnings(&all));
        Resolved {
            all,
            warnings,
            sources,
        }
    }
}

/// **One warning per shadowed id**, naming every file that defines it and which
/// one won.
///
/// Two personas with the same id are only ever *both* visible unless they are
/// scoped to two different clients — so that is the one pairing that is not a
/// shadow, and `billing-web`'s `dev-admin` beside `spike-php`'s `dev-admin` is
/// silent by design. Every other pairing resolves deterministically through the
/// ladder, which is why this is a warning and not an error: the login still
/// works, and "why did I get the wrong claims" must not be silent.
fn shadow_warnings(all: &[Sourced]) -> Vec<Warning> {
    let mut out = Vec::new();
    let mut reported: Vec<&str> = Vec::new();

    for (i, sourced) in all.iter().enumerate() {
        if reported.contains(&sourced.id()) {
            continue;
        }
        // Everybody defining this id who collides with at least one other, in
        // source order — which is the order a reader wants them named in.
        let group: Vec<(usize, &Sourced)> = all
            .iter()
            .enumerate()
            .filter(|(_, other)| other.id() == sourced.id())
            .collect();
        let shadowed: Vec<(usize, &Sourced)> = group
            .iter()
            .filter(|(index, one)| {
                group
                    .iter()
                    .any(|(other_index, other)| other_index != index && collides(one, other))
            })
            .copied()
            .collect();
        if shadowed.len() < 2 {
            continue;
        }
        reported.push(sourced.id());

        let winner = shadowed
            .iter()
            .min_by_key(|(index, one)| rank(one, *index))
            .map(|(_, one)| one.source())
            .unwrap_or_default();
        let names: Vec<String> = shadowed.iter().map(|(_, one)| one.source()).collect();
        out.push(Warning(format!(
            "persona {:?} is defined in {}; {winner} wins",
            all[i].id(),
            and_list(&names),
        )));
    }
    out
}

/// "a and b", "a, b and c" — three sources shadowing one id is a real case the
/// moment a project shadows a built-in that another project also shadows, and
/// "a and b and c" reads like a stammer.
fn and_list(names: &[String]) -> String {
    match names.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// Whether two personas with the same id can ever both be visible to one
/// request. Only two personas scoped to two *different* clients cannot.
fn collides(one: &Sourced, other: &Sourced) -> bool {
    match (&one.client, &other.client) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}

/// The parsed contents of `path`, re-read only when its stamp changed. `None`
/// means the file is not there — which is a normal condition, not an error.
fn read_cached(
    state: &mut State,
    path: &Path,
    changed: &mut bool,
) -> Option<Result<Personas, String>> {
    let stamp = stat(path);
    if let Some(entry) = state.files.get(path) {
        if entry.stamp == stamp {
            return entry.parsed.clone();
        }
    }
    *changed = true;

    let parsed = stamp.map(|_| match std::fs::read_to_string(path) {
        Ok(yaml) => Personas::parse(&yaml, path),
        Err(e) => Err(format!(
            "{}: cannot read personas file: {e}",
            path.display()
        )),
    });
    state.files.insert(
        path.to_path_buf(),
        Cached {
            stamp,
            parsed: parsed.clone(),
        },
    );
    parsed
}

impl Registry {
    /// The linked persona files, in registry order, re-read only when the
    /// registry file changes.
    ///
    /// A registry file that does not parse yields no projects **and** an entry
    /// on `state.links` that the caller turns into a warning — a hand-edited
    /// file that went wrong must not silently unlink everything.
    fn linked(&self, state: &mut State, changed: &mut bool) -> Vec<PathBuf> {
        let Some(source) = &self.links else {
            return Vec::new();
        };
        let stamp = stat(source.path());
        if state.links.is_none() || state.links_stamp != stamp {
            *changed = true;
            state.links_stamp = stamp;
            state.links = Some(Links::load(source));
        }
        match &state.links {
            Some(Ok(links)) => links.links.clone(),
            _ => Vec::new(),
        }
    }
}

fn stat(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Stamp {
        modified: meta.modified().ok(),
        len: meta.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::LinksSource;

    fn parse(yaml: &str, path: &str) -> Personas {
        Personas::parse(yaml, Path::new(path)).unwrap()
    }

    fn ids(list: &[&Sourced]) -> Vec<String> {
        list.iter().map(|s| s.id().to_string()).collect()
    }

    #[test]
    fn an_unscoped_persona_is_visible_to_every_client_and_to_none() {
        let r = Registry::fixed(Personas::builtin()).resolve();
        assert_eq!(
            ids(&r.visible_to(Some("billing-web"))),
            ["ada", "mira", "nobody"]
        );
        assert_eq!(ids(&r.visible_to(None)), ["ada", "mira", "nobody"]);
    }

    #[test]
    fn a_scoped_persona_is_visible_only_to_its_own_client() {
        let r = Registry::fixed(parse(
            "personas:\n  - id: ada\n  - id: dev-admin\n    client: billing-web\n",
            "/tmp/users.yaml",
        ))
        .resolve();

        assert_eq!(
            ids(&r.visible_to(Some("billing-web"))),
            ["ada", "dev-admin"]
        );
        assert_eq!(ids(&r.visible_to(Some("node-spa"))), ["ada"]);
        // Not an exemption: no `client_id` sees what an unrecognised one sees.
        assert_eq!(ids(&r.visible_to(None)), ["ada"]);
    }

    #[test]
    fn a_file_level_client_applies_to_every_persona_that_declares_none() {
        let r = Registry::fixed(parse(
            "client: billing-web\npersonas:\n  - id: ada\n  - id: ops\n    client: ops-console\n",
            "/tmp/users.yaml",
        ))
        .resolve();

        assert_eq!(ids(&r.visible_to(Some("billing-web"))), ["ada"]);
        assert_eq!(ids(&r.visible_to(Some("ops-console"))), ["ops"]);
        assert!(r.visible_to(None).is_empty(), "both are scoped");
    }

    #[test]
    fn get_answers_only_with_a_persona_the_caller_can_see() {
        let r = Registry::fixed(parse(
            "personas:\n  - id: ada\n  - id: dev-admin\n    client: billing-web\n",
            "/tmp/users.yaml",
        ))
        .resolve();

        assert!(r.get("dev-admin", Some("billing-web")).is_some());
        assert!(r.get("dev-admin", Some("lanyard-cli")).is_none());
        assert!(r.get("dev-admin", None).is_none());
        assert!(r.get("ada", Some("anything")).is_some());
    }

    /// The lookup behind the useful refusal: it has to reach the persona the
    /// caller could *not* see, or the message can only say "no such persona".
    #[test]
    fn find_anywhere_reaches_a_persona_no_client_can_see() {
        let r = Registry::fixed(parse(
            "personas:\n  - id: dev-admin\n    client: billing-web\n",
            "/home/dev/code/billing/lanyard.yaml",
        ))
        .resolve();

        let found = r.find_anywhere("dev-admin").unwrap();
        assert_eq!(found.client.as_deref(), Some("billing-web"));
        assert_eq!(found.source(), "/home/dev/code/billing/lanyard.yaml");
        assert!(r.find_anywhere("nobody-at-all").is_none());
    }

    #[test]
    fn all_lists_every_persona_labelled_with_its_source() {
        let r = Registry::fixed(parse(
            "client: billing-web\npersonas:\n  - id: ada\n",
            "/tmp/users.yaml",
        ))
        .resolve();

        assert_eq!(r.all().len(), 1);
        assert_eq!(r.all()[0].client.as_deref(), Some("billing-web"));
        assert_eq!(r.all()[0].source(), "/tmp/users.yaml");
    }

    #[test]
    fn an_absent_default_global_file_resolves_to_the_built_ins() {
        let dir = tempfile::tempdir().unwrap();
        let r = Registry::new(
            PersonasSource::Default(dir.path().join("users.yaml")),
            LinksSource(dir.path().join("links.yaml")),
        )
        .resolve();

        assert_eq!(ids(&r.visible_to(None)), ["ada", "mira", "nobody"]);
        assert_eq!(r.sources.len(), 1);
        assert_eq!(r.sources[0].origin, Origin::BuiltIn);
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn a_global_file_replaces_the_built_ins_entirely() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.yaml");
        std::fs::write(&path, "personas:\n  - id: solo\n").unwrap();

        let r = Registry::new(
            PersonasSource::Default(path.clone()),
            LinksSource(dir.path().join("links.yaml")),
        )
        .resolve();
        assert_eq!(ids(&r.visible_to(None)), ["solo"]);
        assert_eq!(r.sources[0].origin, Origin::File(path));
    }

    #[test]
    fn the_refusal_names_the_file_the_client_and_the_flag_that_fixes_it() {
        let r = Registry::fixed(parse(
            "personas:\n  - id: dev-admin\n    client: billing-web\n",
            "/home/dev/code/billing/lanyard.yaml",
        ))
        .resolve();

        let message = r.missing_persona("dev-admin", Some("lanyard-cli"));
        assert_eq!(
            message,
            "no persona \"dev-admin\" for client \"lanyard-cli\" — it is defined in \
             /home/dev/code/billing/lanyard.yaml scoped to client \"billing-web\". \
             Retry with --client billing-web"
        );

        // A persona that is in no file at all gets the plain sentence: there is
        // no path to name and no flag that would help.
        assert_eq!(
            r.missing_persona("ghost", Some("lanyard-cli")),
            "no persona with id \"ghost\" is loaded"
        );
    }

    // ------------------------------------------------- the linked projects --

    /// A registry over a temporary tree: the global source, the links file, and
    /// a helper that writes a project into it.
    struct Tree {
        root: tempfile::TempDir,
    }

    impl Tree {
        fn new() -> Tree {
            Tree {
                root: tempfile::tempdir().unwrap(),
            }
        }

        /// Returns the **file**, which is what the registry holds.
        fn project(&self, name: &str, yaml: &str) -> PathBuf {
            let dir = self.root.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join("lanyard.yaml");
            std::fs::write(&file, yaml).unwrap();
            file
        }

        /// Writes the registry file naming `files`, in order.
        fn links(&self, dirs: &[&PathBuf]) -> LinksSource {
            let path = self.root.path().join("links.yaml");
            let mut links = crate::links::Links::default();
            for dir in dirs {
                links.add((*dir).clone());
            }
            let source = LinksSource(path);
            links.save(&source).unwrap();
            source
        }

        fn registry(&self, links: LinksSource) -> Registry {
            Registry::new(
                PersonasSource::Default(self.root.path().join("users.yaml")),
                links,
            )
        }
    }

    /// **Links add; they never subtract.** A linked project does not suppress
    /// the built-ins the way the global file does — `nobody` is the flagship
    /// persona, and a developer who links a project should not lose it.
    #[test]
    fn a_linked_project_adds_its_personas_beside_the_built_ins() {
        let tree = Tree::new();
        let billing = tree.project("billing", "personas:\n  - id: dev-admin\n");
        let registry = tree.registry(tree.links(&[&billing]));

        let r = registry.resolve();
        assert_eq!(
            ids(&r.visible_to(None)),
            ["ada", "mira", "nobody", "dev-admin"]
        );
        assert_eq!(r.all()[3].origin, Origin::Project(billing.clone()));
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    }

    /// The global file still replaces the built-ins, and the linked project
    /// still adds — the two rules are independent and both hold at once.
    #[test]
    fn a_global_file_and_a_linked_project_are_listed_under_their_own_sources() {
        let tree = Tree::new();
        std::fs::write(
            tree.root.path().join("users.yaml"),
            "personas:\n  - id: solo\n",
        )
        .unwrap();
        let billing = tree.project("billing", "personas:\n  - id: dev-admin\n");
        let registry = tree.registry(tree.links(&[&billing]));

        let r = registry.resolve();
        assert_eq!(ids(&r.visible_to(None)), ["solo", "dev-admin"]);
        assert_eq!(r.sources.len(), 2);
        assert_eq!(
            r.all()[0].source(),
            tree.root.path().join("users.yaml").display().to_string()
        );
        assert_eq!(r.all()[1].source(), billing.display().to_string());
    }

    /// Each project's `client:` scopes only its own people, which is the whole
    /// clean-picker-per-project property.
    #[test]
    fn two_scoped_projects_give_two_different_pickers() {
        let tree = Tree::new();
        let billing = tree.project(
            "billing",
            "client: billing-web\npersonas:\n  - id: dev-admin\n",
        );
        let php = tree.project("php", "client: spike-php\npersonas:\n  - id: qa-bot\n");
        let registry = tree.registry(tree.links(&[&billing, &php]));

        let r = registry.resolve();
        assert_eq!(
            ids(&r.visible_to(Some("billing-web"))),
            ["ada", "mira", "nobody", "dev-admin"]
        );
        assert_eq!(
            ids(&r.visible_to(Some("spike-php"))),
            ["ada", "mira", "nobody", "qa-bot"]
        );
    }

    /// Criterion 22, and the ladder's third rung. **First-linked rather than
    /// last-linked**, so linking a new project can never silently steal an id
    /// out from under a project that already had it.
    #[test]
    fn two_projects_defining_the_same_id_resolve_to_the_first_linked_and_warn() {
        let tree = Tree::new();
        let first = tree.project("first", "personas:\n  - id: ada\n    email: first@x.test\n");
        let second = tree.project(
            "second",
            "personas:\n  - id: ada\n    email: second@x.test\n",
        );
        let registry = tree.registry(tree.links(&[&first, &second]));

        let r = registry.resolve();
        let picked: Vec<&Sourced> = r.visible_to(None);
        assert_eq!(
            ids(&picked),
            ["ada", "mira", "nobody"],
            "one ada, not three"
        );
        assert_eq!(
            r.get("ada", None).unwrap().persona.email.as_deref(),
            Some("first@x.test"),
            "the first-linked project wins"
        );

        assert_eq!(
            r.warnings.len(),
            1,
            "one warning per shadowed id: {:?}",
            r.warnings
        );
        let warning = r.warnings[0].to_string();
        assert!(warning.contains("ada"), "{warning}");
        assert!(warning.contains(first.to_str().unwrap()), "{warning}");
        assert!(warning.contains(second.to_str().unwrap()), "{warning}");
        assert!(
            warning.contains("built-in defaults"),
            "the built-in ada too: {warning}"
        );
    }

    /// The ladder's second rung: among equals a project file beats the global
    /// file.
    #[test]
    fn a_project_beats_the_global_file_for_the_same_id() {
        let tree = Tree::new();
        std::fs::write(
            tree.root.path().join("users.yaml"),
            "personas:\n  - id: ada\n    email: global@x.test\n",
        )
        .unwrap();
        let billing = tree.project(
            "billing",
            "personas:\n  - id: ada\n    email: project@x.test\n",
        );
        let r = tree.registry(tree.links(&[&billing])).resolve();

        assert_eq!(
            r.get("ada", None).unwrap().persona.email.as_deref(),
            Some("project@x.test")
        );
    }

    /// The ladder's first rung, and it outranks the other two: a client-scoped
    /// persona beats an unscoped one, wherever each came from.
    #[test]
    fn a_scoped_persona_beats_an_unscoped_one_of_the_same_id() {
        let tree = Tree::new();
        let unscoped = tree.project(
            "unscoped",
            "personas:\n  - id: ada\n    email: any@x.test\n",
        );
        let scoped = tree.project(
            "scoped",
            "client: billing-web\npersonas:\n  - id: ada\n    email: billing@x.test\n",
        );
        let r = tree.registry(tree.links(&[&unscoped, &scoped])).resolve();

        assert_eq!(
            r.get("ada", Some("billing-web"))
                .unwrap()
                .persona
                .email
                .as_deref(),
            Some("billing@x.test"),
            "scoped wins even though the unscoped one was linked first"
        );
        assert_eq!(
            r.get("ada", Some("node-spa"))
                .unwrap()
                .persona
                .email
                .as_deref(),
            Some("any@x.test"),
            "and for everybody else the unscoped one is the only candidate"
        );
    }

    /// **Two projects that each scope `dev-admin` to their own client do not
    /// collide**: no request can ever see both, so warning about it would be
    /// noise that trains a developer to ignore the band.
    #[test]
    fn the_same_id_scoped_to_two_different_clients_is_not_a_shadow() {
        let tree = Tree::new();
        let a = tree.project("a", "client: billing-web\npersonas:\n  - id: dev-admin\n");
        let b = tree.project("b", "client: spike-php\npersonas:\n  - id: dev-admin\n");
        let r = tree.registry(tree.links(&[&a, &b])).resolve();

        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(ids(&r.visible_to(Some("billing-web"))).len(), 4);
    }

    /// An absent registry file is the zero-config state, not an error.
    #[test]
    fn an_absent_links_file_is_zero_links_and_no_warning() {
        let tree = Tree::new();
        let registry = tree.registry(LinksSource(tree.root.path().join("links.yaml")));
        let r = registry.resolve();
        assert_eq!(ids(&r.visible_to(None)), ["ada", "mira", "nobody"]);
        assert!(r.warnings.is_empty());
    }

    /// The stat guard, from the outside: the change is picked up with no
    /// restart and nothing re-linked.
    #[test]
    fn an_edited_global_file_is_picked_up_on_the_next_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.yaml");
        std::fs::write(&path, "personas:\n  - id: solo\n").unwrap();

        let registry = Registry::new(
            PersonasSource::Default(path.clone()),
            LinksSource(dir.path().join("links.yaml")),
        );
        assert_eq!(ids(&registry.resolve().visible_to(None)), ["solo"]);

        std::fs::write(&path, "personas:\n  - id: solo\n  - id: duo\n").unwrap();
        assert_eq!(ids(&registry.resolve().visible_to(None)), ["solo", "duo"]);
    }

    /// Nothing changed on disk, so the same snapshot comes back — the stat
    /// guard is the whole reason this is not a re-parse per request.
    #[test]
    fn an_unchanged_resolve_hands_back_the_same_snapshot() {
        let registry = Registry::fixed(Personas::builtin());
        assert!(Arc::ptr_eq(&registry.resolve(), &registry.resolve()));
    }
}
