//! `lanyard link`, `unlink` and `links` — the half of Phase 7 that makes
//! personas travel with a repo.
//!
//! Driven through the real binary, because what is being pinned is what lands
//! on stdout, what lands on stderr, what the exit code is, and what ends up in
//! the registry file. None of these need a running `lanyard serve`: linking
//! writes a file, and a server picks it up on its next resolve.

mod support;

use support::{run, stderr, stdout, ADA_CB};

/// A project directory with a persona file in it, and a registry path in the
/// same temporary tree — so no test here can reach a developer's real
/// `~/.config/lanyard`.
struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        Fixture {
            root: tempfile::tempdir().unwrap(),
        }
    }

    fn registry(&self) -> String {
        self.root.path().join("links.yaml").display().to_string()
    }

    /// A project directory, with `yaml` written into its `lanyard.yaml` unless
    /// `yaml` is `None`. Returns **the file's path** — which is what `link`
    /// takes and what the registry holds.
    fn project(&self, name: &str, yaml: Option<&str>) -> std::path::PathBuf {
        let dir = self.dir(name, yaml);
        dir.join("lanyard.yaml")
    }

    /// The same, when the test needs the directory — to delete it, or to name a
    /// file that is not there.
    fn dir(&self, name: &str, yaml: Option<&str>) -> std::path::PathBuf {
        let dir = self.root.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(yaml) = yaml {
            std::fs::write(dir.join("lanyard.yaml"), yaml).unwrap();
        }
        std::fs::canonicalize(&dir).unwrap()
    }

    fn env(&self) -> Vec<(String, String)> {
        vec![("LANYARD_LINKS".to_string(), self.registry())]
    }

    fn registry_bytes(&self) -> Option<Vec<u8>> {
        std::fs::read(self.root.path().join("links.yaml")).ok()
    }
}

fn link(fixture: &Fixture, args: &[&str]) -> std::process::Output {
    let env = fixture.env();
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    run(&env, args)
}

const BILLING: &str = "client: billing-web\n\
                       personas:\n\
                       \x20 - id: dev-admin\n\
                       \x20 - id: billing-readonly\n\
                       \x20 - id: locked-out\n";

/// Criterion 1.
#[test]
fn link_records_the_absolute_path_and_names_what_it_found() {
    let fixture = Fixture::new();
    let file = fixture.project("billing", Some(BILLING));

    let output = link(&fixture, &["link", file.to_str().unwrap()]);
    assert!(output.status.success(), "{}", stderr(&output));

    let out = stdout(&output);
    assert!(
        out.contains(file.to_str().unwrap()),
        "names the path: {out}"
    );
    assert!(out.contains("billing-web"), "names the client: {out}");
    for id in ["dev-admin", "billing-readonly", "locked-out"] {
        assert!(out.contains(id), "names {id}: {out}");
    }

    let registry = String::from_utf8(fixture.registry_bytes().unwrap()).unwrap();
    assert!(
        registry.contains(file.to_str().unwrap()),
        "the registry holds the file's path, not a directory: {registry}"
    );
}

/// Criterion 2. **The error is also the documentation**: a developer who ran
/// this in the wrong place gets the file they were missing.
#[test]
fn link_without_a_project_file_names_it_and_prints_a_minimal_one() {
    let fixture = Fixture::new();
    let missing = fixture.dir("empty", None).join("lanyard.yaml");

    // A registry that already exists, so "byte-identical to before" is a real
    // assertion rather than "still absent".
    let billing = fixture.project("billing", Some(BILLING));
    link(&fixture, &["link", billing.to_str().unwrap()]);
    let before = fixture.registry_bytes().unwrap();

    let output = link(&fixture, &["link", missing.to_str().unwrap()]);
    assert!(!output.status.success());

    let message = stderr(&output);
    assert!(
        message.contains(missing.to_str().unwrap()),
        "names the file it was given: {message}"
    );
    assert!(message.contains("personas:"), "copyable file: {message}");
    assert!(message.contains("- id:"), "copyable file: {message}");
    assert_eq!(
        fixture.registry_bytes().unwrap(),
        before,
        "nothing recorded"
    );
}

/// Criterion 3. Fatal at link time is what lets it be a warning at serve time:
/// the developer is standing in the directory, which is where the error is
/// cheapest.
#[test]
fn link_refuses_a_project_file_that_does_not_parse() {
    let fixture = Fixture::new();
    let file = fixture.project("dupes", Some("personas:\n  - id: ada\n  - id: ada\n"));

    let output = link(&fixture, &["link", file.to_str().unwrap()]);
    assert!(!output.status.success());

    let message = stderr(&output);
    assert!(message.contains("ada"), "names the id: {message}");
    assert!(
        message.contains(file.to_str().unwrap()),
        "names the path: {message}"
    );
    assert!(
        fixture.registry_bytes().is_none(),
        "a broken project is never recorded"
    );
}

/// Criterion 4.
#[test]
fn linking_twice_succeeds_twice_and_records_one_entry() {
    let fixture = Fixture::new();
    let file = fixture.project("billing", Some(BILLING));

    for _ in 0..2 {
        let output = link(&fixture, &["link", file.to_str().unwrap()]);
        assert!(output.status.success(), "{}", stderr(&output));
    }

    let registry = String::from_utf8(fixture.registry_bytes().unwrap()).unwrap();
    assert_eq!(
        registry.matches(file.to_str().unwrap()).count(),
        1,
        "exactly one entry: {registry}"
    );
}

/// A relative path in a registry read by a daemon with a different working
/// directory means nothing, so what is recorded is always absolute.
#[test]
fn a_linked_path_is_recorded_absolute_and_canonicalized() {
    let fixture = Fixture::new();
    let file = fixture.project("billing", Some(BILLING));

    let indirect = file
        .parent()
        .unwrap()
        .join("..")
        .join("billing")
        .join("lanyard.yaml");
    let output = link(&fixture, &["link", indirect.to_str().unwrap()]);
    assert!(output.status.success(), "{}", stderr(&output));

    let registry = String::from_utf8(fixture.registry_bytes().unwrap()).unwrap();
    assert!(!registry.contains(".."), "canonicalized: {registry}");
    assert!(registry.contains(file.to_str().unwrap()), "{registry}");
}

/// **The file is named, never discovered.** `lanyard link` with nothing to link
/// is a usage error, not an invitation to guess from the working directory: a
/// command that silently records whatever happens to be underfoot is one you
/// have to check afterwards.
#[test]
fn link_with_no_argument_is_a_usage_error_and_records_nothing() {
    let fixture = Fixture::new();
    fixture.project("billing", Some(BILLING));

    let output = link(&fixture, &["link"]);
    assert!(
        !output.status.success(),
        "must not link the working directory"
    );
    assert!(
        stderr(&output).to_lowercase().contains("file"),
        "names what is missing: {}",
        stderr(&output)
    );
    assert!(fixture.registry_bytes().is_none(), "nothing recorded");
}

/// A directory is the shape of the old mistake, so it gets the sentence that
/// fixes it rather than a bare "is a directory" from the OS.
#[test]
fn link_given_a_directory_says_to_name_the_file() {
    let fixture = Fixture::new();
    let dir = fixture.dir("billing", Some(BILLING));

    let output = link(&fixture, &["link", dir.to_str().unwrap()]);
    assert!(!output.status.success());

    let message = stderr(&output);
    assert!(message.contains("directory"), "{message}");
    assert!(
        message.contains(dir.join("lanyard.yaml").to_str().unwrap()),
        "shows the path that would have worked: {message}"
    );
    assert!(fixture.registry_bytes().is_none(), "nothing recorded");
}

/// The filename is a convention, not a rule: what is linked is the file you
/// named, so a project may call it whatever it likes.
#[test]
fn a_persona_file_need_not_be_called_lanyard_yaml() {
    let fixture = Fixture::new();
    let dir = fixture.dir("billing", None);
    let file = dir.join("dev-personas.yaml");
    std::fs::write(&file, BILLING).unwrap();

    let output = link(&fixture, &["link", file.to_str().unwrap()]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(String::from_utf8(fixture.registry_bytes().unwrap())
        .unwrap()
        .contains("dev-personas.yaml"));
}

// ------------------------------------------------------- unlink and links --

/// Criterion 5. The optional argument exists for the second half of this: you
/// cannot `cd` into a directory you deleted, and that is the link you most want
/// to prune.
#[test]
fn unlink_removes_an_entry_including_one_whose_file_is_gone() {
    let fixture = Fixture::new();
    let billing = fixture.project("billing", Some(BILLING));
    let ops = fixture.project("ops", Some("personas:\n  - id: ops-bot\n"));
    link(&fixture, &["link", billing.to_str().unwrap()]);
    link(&fixture, &["link", ops.to_str().unwrap()]);

    let output = link(&fixture, &["unlink", billing.to_str().unwrap()]);
    assert!(output.status.success(), "{}", stderr(&output));
    let registry = String::from_utf8(fixture.registry_bytes().unwrap()).unwrap();
    assert!(!registry.contains(billing.to_str().unwrap()), "{registry}");
    assert!(registry.contains(ops.to_str().unwrap()), "{registry}");

    std::fs::remove_dir_all(ops.parent().unwrap()).unwrap();
    let output = link(&fixture, &["unlink", ops.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "a file that is gone is still unlinkable: {}",
        stderr(&output)
    );
    let registry = String::from_utf8(fixture.registry_bytes().unwrap()).unwrap();
    assert!(!registry.contains(ops.to_str().unwrap()), "{registry}");
}

/// Criterion 6. A stale link becomes **actionable** here rather than merely
/// warned about, so the row that is wrong has to be on the list.
#[test]
fn links_prints_a_row_per_entry_and_says_which_one_is_missing() {
    let fixture = Fixture::new();
    let billing = fixture.project("billing", Some(BILLING));
    let ops = fixture.project("ops", Some("personas:\n  - id: ops-bot\n"));
    let gone = fixture.project("old-thing", Some("personas:\n  - id: ghost\n"));
    for file in [&billing, &ops, &gone] {
        link(&fixture, &["link", file.to_str().unwrap()]);
    }
    std::fs::remove_dir_all(gone.parent().unwrap()).unwrap();

    let output = link(&fixture, &["links"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let out = stdout(&output);

    // **Where the registry lives, first**, then a blank line, then the rows.
    // "which file did it read" is the question behind half of "why is that
    // persona not there", and it is one `LANYARD_LINKS` away from being a
    // surprise.
    let mut lines = out.lines();
    assert_eq!(lines.next(), Some(fixture.registry().as_str()));
    assert_eq!(lines.next(), Some(""), "a blank line under the path");
    assert!(
        lines.next().unwrap().starts_with(billing.to_str().unwrap()),
        "the rows start on the third line: {out}"
    );

    let billing_row = row_for(&out, billing.to_str().unwrap());
    assert!(billing_row.contains("billing-web"), "{billing_row}");
    assert!(billing_row.contains("dev-admin"), "{billing_row}");

    let ops_row = row_for(&out, ops.to_str().unwrap());
    assert!(
        ops_row.contains('—'),
        "no client reads as a dash: {ops_row}"
    );
    assert!(ops_row.contains("ops-bot"), "{ops_row}");

    let gone_row = row_for(&out, gone.to_str().unwrap());
    assert!(
        gone_row.contains("missing"),
        "a file that is gone is listed as missing, not omitted: {gone_row}"
    );
}

fn row_for<'a>(out: &'a str, path: &str) -> &'a str {
    out.lines()
        .find(|line| line.contains(path))
        .unwrap_or_else(|| panic!("no row for {path} in:\n{out}"))
}

/// An empty registry is the normal condition of a fresh machine, so this
/// succeeds and prints nothing rather than reporting a missing file.
#[test]
fn links_on_a_fresh_machine_names_the_registry_and_says_how_to_fill_it() {
    let fixture = Fixture::new();
    let output = link(&fixture, &["links"]);
    assert!(output.status.success(), "an empty registry is not an error");

    let out = stdout(&output);
    let mut lines = out.lines();
    // Still names the path, so "am I even looking at the right registry" is
    // answerable when the answer is "yes, and it is empty".
    assert_eq!(lines.next(), Some(fixture.registry().as_str()));
    assert_eq!(lines.next(), Some(""));
    assert!(
        lines.next().unwrap().contains("lanyard link"),
        "the empty state says what to do: {out}"
    );
}

// ------------------------------------------- what a running server sees --

use lanyard_cli::config::{LinksSource, PersonasSource};
use lanyard_cli::registry::Registry;

impl Fixture {
    /// A registry over this fixture's tree: the global `users.yaml` (absent
    /// unless a test writes one) and whatever `links.yaml` names.
    fn served(&self) -> Registry {
        Registry::new(
            PersonasSource::Default(self.root.path().join("users.yaml")),
            LinksSource(self.root.path().join("links.yaml")),
        )
    }
}

async fn visible(base: &str, client_id: Option<&str>) -> Vec<String> {
    let query = match client_id {
        Some(client_id) => format!("?client_id={client_id}"),
        None => String::new(),
    };
    let body: serde_json::Value = reqwest::get(format!("{base}/_/api/personas{query}"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["personas"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_string())
        .collect()
}

/// The whole of the phase's first half, over HTTP: `lanyard link`, then a
/// server, then a picker with that project's people **and** the built-ins in
/// it. Links add and never subtract.
#[tokio::test]
async fn a_linked_project_shows_up_beside_the_built_ins() {
    let fixture = Fixture::new();
    let billing = fixture.project("billing", Some(BILLING));
    let output = link(&fixture, &["link", billing.to_str().unwrap()]);
    assert!(output.status.success(), "{}", stderr(&output));

    let base = support::spawn_with_registry(fixture.served()).await;
    assert_eq!(
        visible(&base, None).await,
        [
            "ada",
            "mira",
            "nobody",
            "dev-admin",
            "billing-readonly",
            "locked-out"
        ]
    );
}

/// Two projects, two clients, two pickers — and `nobody` still pickable in
/// both, which is criterion 14.
#[tokio::test]
async fn each_project_gets_its_own_picker_and_keeps_the_built_ins() {
    let fixture = Fixture::new();
    let billing = fixture.project(
        "billing",
        Some("client: billing-web\npersonas:\n  - id: dev-admin\n"),
    );
    let php = fixture.project(
        "php",
        Some("client: spike-php\npersonas:\n  - id: qa-bot\n"),
    );
    for file in [&billing, &php] {
        link(&fixture, &["link", file.to_str().unwrap()]);
    }

    let base = support::spawn_with_registry(fixture.served()).await;
    assert_eq!(
        visible(&base, Some("billing-web")).await,
        ["ada", "mira", "nobody", "dev-admin"]
    );
    assert_eq!(
        visible(&base, Some("spike-php")).await,
        ["ada", "mira", "nobody", "qa-bot"]
    );
}

// -------------------------------------------------------- live re-resolve --

/// **No restart, and no `lanyard link` re-run.** The observable requirement of
/// the whole stat-guard decision: edit a linked project's file, load the page,
/// see the change.
#[tokio::test]
async fn editing_a_linked_project_file_is_picked_up_without_a_restart() {
    let fixture = Fixture::new();
    let billing = fixture.project("billing", Some("personas:\n  - id: dev-admin\n"));
    link(&fixture, &["link", billing.to_str().unwrap()]);

    let base = support::spawn_with_registry(fixture.served()).await;
    assert!(!visible(&base, None).await.contains(&"new-hire".to_string()));

    std::fs::write(&billing, "personas:\n  - id: dev-admin\n  - id: new-hire\n").unwrap();

    assert!(
        visible(&base, None).await.contains(&"new-hire".to_string()),
        "the edit is live on the next request"
    );
}

/// `lanyard link` needs no running server, and a running one needs no restart:
/// the registry file is stat-guarded exactly like the persona files are.
#[tokio::test]
async fn linking_a_project_while_serving_is_picked_up_on_the_next_request() {
    let fixture = Fixture::new();
    let base = support::spawn_with_registry(fixture.served()).await;
    assert_eq!(visible(&base, None).await, ["ada", "mira", "nobody"]);

    let billing = fixture.project("billing", Some(BILLING));
    let output = link(&fixture, &["link", billing.to_str().unwrap()]);
    assert!(output.status.success(), "{}", stderr(&output));

    assert!(
        visible(&base, None)
            .await
            .contains(&"dev-admin".to_string()),
        "a project linked mid-flight appears with no restart"
    );
}

/// And unlinking is live in the same way — which is what criterion 23 leans on.
#[tokio::test]
async fn unlinking_while_serving_takes_the_personas_away_again() {
    let fixture = Fixture::new();
    let billing = fixture.project("billing", Some(BILLING));
    link(&fixture, &["link", billing.to_str().unwrap()]);

    let base = support::spawn_with_registry(fixture.served()).await;
    assert!(visible(&base, None)
        .await
        .contains(&"dev-admin".to_string()));

    link(&fixture, &["unlink", billing.to_str().unwrap()]);
    assert_eq!(visible(&base, None).await, ["ada", "mira", "nobody"]);
}

// --------------------------------------------- warnings, not deaths --

async fn warnings(base: &str) -> Vec<String> {
    let body: serde_json::Value = reqwest::get(format!("{base}/_/api/personas"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["warnings"]
        .as_array()
        .expect("a warnings array, always")
        .iter()
        .map(|w| w.as_str().unwrap().to_string())
        .collect()
}

/// **A machine-wide daemon must not die because one of ten projects has a
/// typo** — that would make linking a project an act of sabotage against the
/// other nine. Criterion 21.
#[tokio::test]
async fn a_linked_file_that_stops_parsing_warns_and_takes_only_itself_out() {
    let fixture = Fixture::new();
    let billing = fixture.project("billing", Some("personas:\n  - id: dev-admin\n"));
    let ops = fixture.project("ops", Some("personas:\n  - id: ops-bot\n"));
    for file in [&billing, &ops] {
        link(&fixture, &["link", file.to_str().unwrap()]);
    }

    let base = support::spawn_with_registry(fixture.served()).await;
    assert!(warnings(&base).await.is_empty());

    std::fs::write(
        &billing,
        "personas:\n  - id: dev-admin\n  - id: dev-admin\n",
    )
    .unwrap();

    let listed = visible(&base, None).await;
    assert!(!listed.contains(&"dev-admin".to_string()), "{listed:?}");
    assert_eq!(
        listed,
        ["ada", "mira", "nobody", "ops-bot"],
        "every other source still resolves"
    );

    let listed_warnings = warnings(&base).await;
    assert_eq!(listed_warnings.len(), 1, "{listed_warnings:?}");
    assert!(
        listed_warnings[0].contains("dev-admin"),
        "{listed_warnings:?}"
    );
    assert!(
        listed_warnings[0].contains(billing.to_str().unwrap()),
        "names the file: {listed_warnings:?}"
    );

    // Fixed, and the personas come back — no restart, no re-link.
    std::fs::write(&billing, "personas:\n  - id: dev-admin\n").unwrap();
    assert!(visible(&base, None)
        .await
        .contains(&"dev-admin".to_string()));
    assert!(warnings(&base).await.is_empty());
}

/// Criterion 19's first half. A linked file can be temporarily absent — its
/// directory moved, or on a volume that is not mounted — so the entry stays in
/// the registry and the warning names
/// the path. **The picker can never be emptied by a link going bad**: sources 1
/// or 2 are always still there.
#[tokio::test]
async fn a_linked_file_that_moves_away_warns_and_leaves_the_picker_full() {
    let fixture = Fixture::new();
    let billing = fixture.project("billing", Some(BILLING));
    link(&fixture, &["link", billing.to_str().unwrap()]);

    let base = support::spawn_with_registry(fixture.served()).await;
    let dir = billing.parent().unwrap().to_path_buf();
    let moved = dir.parent().unwrap().join("billing-elsewhere");
    std::fs::rename(&dir, &moved).unwrap();

    let res = reqwest::get(format!("{base}/_/")).await.unwrap();
    assert_eq!(res.status().as_u16(), 200, "the page still renders");

    assert_eq!(visible(&base, None).await, ["ada", "mira", "nobody"]);
    let listed_warnings = warnings(&base).await;
    assert_eq!(listed_warnings.len(), 1, "{listed_warnings:?}");
    assert!(
        listed_warnings[0].contains(billing.to_str().unwrap()),
        "names the missing path: {listed_warnings:?}"
    );

    std::fs::rename(&moved, &dir).unwrap();
    assert!(visible(&base, None)
        .await
        .contains(&"dev-admin".to_string()));
    assert!(warnings(&base).await.is_empty());
}

/// A registry file somebody hand-edited into nonsense must not silently unlink
/// everything: it warns, and the built-ins are still there to pick from.
#[tokio::test]
async fn a_registry_file_that_does_not_parse_warns_rather_than_emptying_the_picker() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root.path().join("links.yaml"), "linkz:\n  - /a\n").unwrap();

    let base = support::spawn_with_registry(fixture.served()).await;
    assert_eq!(visible(&base, None).await, ["ada", "mira", "nobody"]);
    let listed_warnings = warnings(&base).await;
    assert_eq!(listed_warnings.len(), 1, "{listed_warnings:?}");
    assert!(
        listed_warnings[0].contains("links.yaml"),
        "{listed_warnings:?}"
    );
}

// ------------------------------------------------------- the warning band --

async fn page(base: &str, path: &str) -> String {
    reqwest::get(format!("{base}{path}"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
}

/// **The picker is where the missing person is noticed**, so it is where the
/// reason has to be. Criterion 19's and 21's page half.
#[tokio::test]
async fn the_picker_carries_a_band_naming_the_path_and_the_error() {
    let fixture = Fixture::new();
    let billing = fixture.project("billing", Some("personas:\n  - id: dev-admin\n"));
    link(&fixture, &["link", billing.to_str().unwrap()]);

    let base = support::spawn_with_registry(fixture.served()).await;
    let clean = page(&base, "/_/").await;
    assert!(
        !clean.contains("could not be loaded"),
        "no band when nothing is wrong"
    );

    std::fs::write(
        &billing,
        "personas:\n  - id: dev-admin\n  - id: dev-admin\n",
    )
    .unwrap();

    let body = page(&base, "/_/").await;
    assert!(body.contains("could not be loaded"), "{body}");
    assert!(
        body.contains(billing.to_str().unwrap()),
        "the band names the path: {body}"
    );
    assert!(
        body.contains("duplicate persona id"),
        "and the error: {body}"
    );
    assert!(
        body.contains("Ada Bell"),
        "the other sources still list: {body}"
    );
}

/// A file path out of a cloned repo reaches this page, and it goes through the
/// same escaping every persona-supplied string already does. The surface grew
/// in Phase 7; the rule did not change.
#[tokio::test]
async fn a_warning_is_escaped_like_every_other_developer_supplied_string() {
    let fixture = Fixture::new();
    let nasty = fixture.project("<script>alert(1)</script>", Some("personas:\n  - id: x\n"));
    link(&fixture, &["link", nasty.to_str().unwrap()]);
    std::fs::remove_dir_all(nasty.parent().unwrap()).unwrap();

    let base = support::spawn_with_registry(fixture.served()).await;
    let body = page(&base, "/_/").await;
    assert!(body.contains("could not be loaded"), "{body}");
    assert!(!body.contains("<script>alert(1)"), "escaped: {body}");
    assert!(body.contains("&lt;script&gt;alert(1)"), "{body}");
}

// ---------------------------------------------------- warnings on the log --

/// Criterion 19's second half.
///
/// `/_/` emits no event by Phase 6's design — a page is not a decision — so the
/// warning rides on the next *protocol* request, which is the request that was
/// about to get the wrong picker anyway.
#[tokio::test]
async fn a_broken_source_puts_a_warning_on_the_next_authorize_event() {
    let fixture = Fixture::new();
    let billing = fixture.project("billing", Some(BILLING));
    link(&fixture, &["link", billing.to_str().unwrap()]);

    let (base, events) =
        support::spawn_observed_registry(fixture.served(), Default::default()).await;

    let dir = billing.parent().unwrap();
    std::fs::rename(dir, dir.parent().unwrap().join("gone")).unwrap();

    let res = support::authorize(
        &base,
        &format!("client_id=billing-web&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;
    assert_eq!(res.status().as_u16(), 302, "the login still starts");

    let event = support::logged(&events)
        .into_iter()
        .find(|e| e.endpoint == "/oidc/authorize")
        .expect("an authorize event");
    let warnings = event.warnings.clone().expect("warnings ride on the event");
    assert!(
        warnings[0].contains(billing.to_str().unwrap()),
        "naming the path: {warnings:?}"
    );

    // The same sentence on the stdout line `lanyard serve` prints, because it
    // is the same function `lanyard logs` renders with.
    let line = lanyard_cli::events::pretty(&event);
    assert!(line.contains("warning:"), "{line}");
    assert!(line.contains(billing.to_str().unwrap()), "{line}");

    // …and it survives the JSON round trip `lanyard logs --json | jq` reads.
    let json = serde_json::to_value(&event).unwrap();
    assert!(json["warnings"][0]
        .as_str()
        .unwrap()
        .contains("no such file"));
}

/// Nothing wrong, nothing said: **no registry warning** rather than an empty
/// one, so `jq 'select(.warnings)'` is the whole filter.
///
/// Phase 8 put a second kind of warning on this field — the `Host` mismatch —
/// and the harness binds an ephemeral port while the issuer stays `:9500`, so
/// one of those is legitimately here. This test is about the registry, so it
/// asks about the registry.
#[tokio::test]
async fn a_healthy_registry_puts_no_warning_on_an_event() {
    let fixture = Fixture::new();
    let (base, events) =
        support::spawn_observed_registry(fixture.served(), Default::default()).await;
    support::authorize(
        &base,
        &format!("client_id=billing-web&response_type=code&redirect_uri={ADA_CB}"),
    )
    .await;

    let event = support::logged(&events).into_iter().next().unwrap();
    let registry_warnings: Vec<&String> = event
        .warnings
        .iter()
        .flatten()
        .filter(|w| !w.contains("reached as"))
        .collect();
    assert!(registry_warnings.is_empty(), "{registry_warnings:?}");
}

// -------------------------------------------- the unfiltered debugging view --

/// Criterion 10. **Filtering silently would recreate "why is Ada not there"
/// one layer down**, so the page says what it hid and offers the full list
/// without abandoning the login.
#[tokio::test]
async fn the_filtered_picker_says_how_many_it_hid_and_links_to_the_full_list() {
    let fixture = Fixture::new();
    let billing = fixture.project(
        "billing",
        Some("client: billing-web\npersonas:\n  - id: dev-admin\n"),
    );
    let php = fixture.project(
        "php",
        Some("client: spike-php\npersonas:\n  - id: qa-bot\n"),
    );
    for file in [&billing, &php] {
        link(&fixture, &["link", file.to_str().unwrap()]);
    }
    let base = support::spawn_with_registry(fixture.served()).await;

    let http = support::client();
    let res = http
        .get(format!(
            "{base}/oidc/authorize?client_id=billing-web&response_type=code&redirect_uri={ADA_CB}"
        ))
        .send()
        .await
        .unwrap();
    let req = res.headers()["location"]
        .to_str()
        .unwrap()
        .strip_prefix("/_/?req=")
        .unwrap()
        .to_string();

    let body = page(&base, &format!("/_/?req={req}")).await;
    assert!(
        !body.contains("qa-bot"),
        "the clean picker hides it: {body}"
    );
    assert!(
        body.contains("1 other persona is scoped to other applications"),
        "the page says what it hid: {body}"
    );
    assert!(body.contains(&format!("/_/?req={req}&amp;all=1")), "{body}");

    // Following it: the whole list, each row labelled with its client and the
    // file it came from — and the login is still in progress.
    let full = page(&base, &format!("/_/?req={req}&all=1")).await;
    assert!(full.contains("qa-bot"), "{full}");
    assert!(
        full.contains("spike-php"),
        "labelled with its client: {full}"
    );
    assert!(
        full.contains(php.to_str().unwrap()),
        "and with the path of its file: {full}"
    );
    assert!(
        full.contains("name=\"persona\" value=\"dev-admin\""),
        "the login is still pickable: {full}"
    );
    assert!(
        !full.contains("name=\"persona\" value=\"qa-bot\""),
        "but somebody this application cannot see is not a button: {full}"
    );
}

/// The banner's `UI →` line with nothing in progress: the debugging view, and
/// the one a developer opens when the filtered view surprised them.
#[tokio::test]
async fn the_picker_with_no_login_lists_every_source_labelled() {
    let fixture = Fixture::new();
    let billing = fixture.project(
        "billing",
        Some("client: billing-web\npersonas:\n  - id: dev-admin\n"),
    );
    link(&fixture, &["link", billing.to_str().unwrap()]);
    let base = support::spawn_with_registry(fixture.served()).await;

    let body = page(&base, "/_/").await;
    assert!(body.contains("dev-admin"), "{body}");
    assert!(
        body.contains("billing-web"),
        "labelled with its client: {body}"
    );
    assert!(
        body.contains(billing.to_str().unwrap()),
        "and its source path: {body}"
    );
    assert!(
        body.contains("built-in defaults"),
        "including the built-ins: {body}"
    );
}
