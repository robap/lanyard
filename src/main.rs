use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand};
use lanyard_cli::app::{self, AppState};
use lanyard_cli::client::{self, MintRequest};
use lanyard_cli::clock::Clock;
use lanyard_cli::links::Links;
use lanyard_cli::oidc::flaw::Flaw;
use lanyard_cli::persona::Personas;
use lanyard_cli::registry::Registry;
use lanyard_cli::store::Stores;
use lanyard_cli::{banner, config::Config, keys};

#[derive(Parser)]
#[command(
    name = "lanyard",
    version,
    about = "A local OIDC provider with nothing to configure"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the provider in the foreground
    Serve,
    /// Mint a bearer token and print it
    Token(MintArgs),
    /// Print `export BEARER_TOKEN=...` for `eval`
    Env(MintArgs),
    /// Follow the live request log of a running lanyard
    Logs(LogsArgs),
    /// Record a project's persona file so its personas load
    Link(LinkArgs),
    /// Remove a persona file from the registry
    Unlink(LinkArgs),
    /// List every linked persona file, its client, and its personas
    Links,
    /// Check this machine's lanyard and say what is wrong with it
    Doctor(DoctorArgs),
}

#[derive(Args)]
struct DoctorArgs {
    /// Where lanyard is listening. An address, not the issuer
    #[arg(long, value_name = "URL")]
    url: Option<String>,

    /// Treat every WARN as a failure. For CI
    #[arg(long)]
    strict: bool,

    /// Print nothing; only set the exit code. What the image's HEALTHCHECK runs
    #[arg(long)]
    quiet: bool,
}

#[derive(Args)]
struct LinkArgs {
    /// The project's persona file, e.g. ./lanyard.yaml. **Named, never
    /// discovered**: nothing is inferred from the working directory
    #[arg(value_name = "FILE")]
    file: PathBuf,
}

#[derive(Args)]
struct LogsArgs {
    /// One JSON object per event, for `jq`. Bare prints what `lanyard serve` prints
    #[arg(long)]
    json: bool,

    /// Where lanyard is listening. An address, not the issuer
    #[arg(long, value_name = "URL")]
    url: Option<String>,
}

#[derive(Args)]
struct MintArgs {
    /// Persona to mint for
    #[arg(long = "as", value_name = "PERSONA")]
    persona: String,

    /// The `aud` claim. Any value mints — an API that rejects it is the point
    #[arg(long, value_name = "AUDIENCE")]
    aud: Option<String>,

    /// Mint as this `client_id`, which is what reaches a persona scoped to it
    #[arg(long, value_name = "CLIENT_ID")]
    client: Option<String>,

    /// Space-delimited scopes
    #[arg(long, value_name = "SCOPE")]
    scope: Option<String>,

    /// Where lanyard is listening. An address, not the issuer
    #[arg(long, value_name = "URL")]
    url: Option<String>,

    // The six deliberate failure modes. One per token: a resource server
    // reports only the first check it fails, so `--expired --wrong-aud` tests
    // strictly less than either flag alone. `group` is what makes clap say so,
    // naming both flags, before anything reaches the network.
    /// Mint a token that expired an hour ago
    #[arg(long, group = "flaw")]
    expired: bool,

    /// Mint a token whose `aud` is `wrong-<requested>`
    #[arg(long, group = "flaw")]
    wrong_aud: bool,

    /// Mint a token issued by someone else
    #[arg(long, group = "flaw")]
    wrong_iss: bool,

    /// Mint a correctly-shaped token with one bit of the signature flipped
    #[arg(long, group = "flaw")]
    bad_signature: bool,

    /// Mint an unsecured JWT: `alg: none`, no signature at all
    #[arg(long, group = "flaw")]
    alg_none: bool,

    /// Mint a real signature under a `kid` that is in no JWKS
    #[arg(long, group = "flaw")]
    unknown_kid: bool,
}

impl MintArgs {
    /// The wire value the server understands, which is the flag with its `--`
    /// removed. The names live in [`Flaw`] and nowhere else, so a flag and a
    /// form field cannot drift apart.
    fn flaw(&self) -> Option<&'static str> {
        [
            (self.expired, Flaw::Expired),
            (self.wrong_aud, Flaw::WrongAud),
            (self.wrong_iss, Flaw::WrongIss),
            (self.bad_signature, Flaw::BadSignature),
            (self.alg_none, Flaw::AlgNone),
            (self.unknown_kid, Flaw::UnknownKid),
        ]
        .into_iter()
        .find(|(asked_for, _)| *asked_for)
        .map(|(_, flaw)| flaw.as_str())
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    // **`doctor` owns its own exit code.** The report *is* the output, and a
    // second `lanyard: …` line under a `FAIL` would be noise — on a probe that
    // prints nothing by design, it would be the only output there was.
    if let Command::Doctor(args) = &cli.command {
        return doctor(args).await;
    }

    let result = match cli.command {
        Command::Serve => serve().await,
        // Both write one line to stdout and nothing else. A stray banner or
        // timing line here silently poisons `$(lanyard token ...)`, and every
        // test still passes except a real curl.
        Command::Token(args) => mint(&args).await.map(|token| println!("{token}")),
        // Single-quoted: a JWT is base64url and dots, so the quoting is
        // belt-and-braces — but `eval` runs whatever we print, and this is the
        // one line lanyard hands a shell to execute.
        Command::Env(args) => mint(&args)
            .await
            .map(|token| println!("export BEARER_TOKEN='{token}'")),
        // Prints for as long as it runs and only ever returns an error: the
        // stream ends when lanyard stops, and saying so beats exiting quietly.
        Command::Logs(args) => follow(&args).await,
        // **None of these three needs a running server.** They write and read a
        // file; a running `lanyard serve` picks the change up on its next
        // resolve, and a not-yet-running one reads it at startup.
        Command::Link(args) => link(&args),
        Command::Unlink(args) => unlink(&args),
        Command::Links => list_links(),
        // Handled above, where it can return its own exit code.
        Command::Doctor(_) => unreachable!(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("lanyard: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Nothing is written to stdout unless this returns `Ok`. On failure the message
/// goes to stderr and the exit code is non-zero, so
/// `curl -H "Authorization: Bearer $(lanyard token ...)"` sends an empty bearer
/// rather than the words "connection refused" as a credential.
async fn mint(args: &MintArgs) -> Result<String, String> {
    let url = client::resolve_url(args.url.as_deref(), |key| std::env::var(key).ok())?;
    client::mint(&MintRequest {
        url: &url,
        persona: &args.persona,
        // The CLI's own name unless asked otherwise — deriving it from the
        // working directory is deliberately post-v1 (spec, open question 1):
        // the `client_id` claim of a minted token must not depend on where the
        // shell happened to be, on a command whose contract is that stdout is
        // the token and nothing else.
        client_id: args.client.as_deref().unwrap_or(client::CLI_CLIENT_ID),
        audience: args.aud.as_deref(),
        scope: args.scope.as_deref(),
        flaw: args.flaw(),
    })
    .await
    .map_err(|e| e.to_string())
}

/// The eighth subcommand: six checks, one line each, and an exit code a
/// container health probe can depend on.
///
/// **Only a `FAIL` is non-zero.** The image's `HEALTHCHECK` is this command, so
/// a container must not stay unhealthy over a warning that may well be
/// deliberate — see [`lanyard_cli::doctor::Report::failed`].
async fn doctor(args: &DoctorArgs) -> ExitCode {
    let url = match client::resolve_url(args.url.as_deref(), |key| std::env::var(key).ok()) {
        Ok(url) => url,
        Err(message) => {
            // Before any check runs, so this is the ordinary error path rather
            // than a `FAIL` line about a URL that was never formed.
            eprintln!("lanyard: {message}");
            return ExitCode::FAILURE;
        }
    };

    let report = lanyard_cli::doctor::run(&url, &Config::from_env()).await;
    if !args.quiet {
        print!("{}", report.render());
    }
    if report.failed(args.strict) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// The fourth subcommand, and the second thing the CLI does over HTTP against
/// the running singleton rather than in-process.
async fn follow(args: &LogsArgs) -> Result<(), String> {
    let url = client::resolve_url(args.url.as_deref(), |key| std::env::var(key).ok())?;
    client::logs(&url, args.json)
        .await
        .map_err(|e| e.to_string())
}

/// The file a project commits. Printed when there is not one, because a
/// developer who ran `lanyard link` in the wrong place wants the file, not a
/// pointer to the README.
const MINIMAL_PROJECT_FILE: &str = "\n\
    A project's personas live in a lanyard.yaml in its root. A minimal one:\n\n\
    \x20 personas:\n\
    \x20   - id: dev-admin\n\
    \x20     name: Dev Admin\n\
    \x20     email: dev-admin@example.test\n\
    \x20     roles: [admin]\n";

/// The file the argument names, canonicalized — a relative path in a registry
/// read by a daemon with a different working directory means nothing.
///
/// **Never inferred.** `lanyard link` records the file you named and no other:
/// a command that silently picks up whatever happens to be underfoot is one you
/// have to check afterwards, and the `client_id`s in a token are not a good
/// place to find out you linked the wrong repository.
fn named_file(args: &LinkArgs) -> Result<PathBuf, String> {
    let named = &args.file;
    if named.is_dir() {
        return Err(format!(
            "{} is a directory — name the persona file itself:\n\n  lanyard link {}\n",
            named.display(),
            named.join(DEFAULT_FILE_NAME).display(),
        ));
    }
    std::fs::canonicalize(named).map_err(|e| format!("{}: {e}", named.display()))
}

/// What a project conventionally calls its persona file. A convention and not a
/// rule — `link` records the file it was given, whatever it is called — but it
/// is what the errors suggest and what the docs use.
const DEFAULT_FILE_NAME: &str = "lanyard.yaml";

/// **Validates before recording.** A file that does not parse is fatal *here*,
/// which is exactly what lets it be a warning at serve time: a machine-wide
/// daemon must not die because one of ten projects has a typo, and the moment
/// the error is cheapest is while the developer is standing in the directory.
fn link(args: &LinkArgs) -> Result<(), String> {
    let config = Config::from_env()?;

    if !args.file.exists() {
        return Err(format!(
            "{}: no such file\n{MINIMAL_PROJECT_FILE}",
            args.file.display()
        ));
    }
    let file = named_file(args)?;
    let personas = Personas::parse(
        &std::fs::read_to_string(&file)
            .map_err(|e| format!("{}: cannot read: {e}", file.display()))?,
        &file,
    )?;

    let mut links = Links::load(&config.links)?;
    links.add(file.clone());
    links.save(&config.links)?;

    let ids: Vec<&str> = personas.list.iter().map(|p| p.id.as_str()).collect();
    let scope = match &personas.client {
        Some(client) => format!(" for client {client}"),
        None => String::new(),
    };
    println!(
        "linked {} — {} persona{}{scope}",
        file.display(),
        ids.len(),
        if ids.len() == 1 { "" } else { "s" },
    );
    if !ids.is_empty() {
        println!("  {}", ids.join(", "));
    }
    Ok(())
}

/// The mirror of `link`, and it must work on a file that is **gone**: a link to
/// a project you deleted is exactly the one you most want to prune, and
/// canonicalizing a path that no longer exists fails. So a path that cannot be
/// resolved falls back to the path as typed, which is what the registry holds.
fn unlink(args: &LinkArgs) -> Result<(), String> {
    let config = Config::from_env()?;
    let file = named_file(args).unwrap_or_else(|_| args.file.clone());

    let mut links = Links::load(&config.links)?;
    if !links.remove(&file) {
        return Err(format!("{} is not linked", file.display()));
    }
    links.save(&config.links)?;
    println!("unlinked {}", file.display());
    Ok(())
}

/// One line per registry entry — **including the broken ones**. A stale link
/// becomes actionable here rather than merely warned about, which is the whole
/// reason this subcommand exists.
fn list_links() -> Result<(), String> {
    let config = Config::from_env()?;
    let links = Links::load(&config.links)?;

    // **The registry's own path first.** `LANYARD_LINKS` moves it, and "which
    // file did this read" is the question behind half of "why is that persona
    // not there" — so the answer is on the first line rather than in the docs.
    println!("{}\n", config.links.path().display());

    if links.links.is_empty() {
        println!("Nothing linked. lanyard link <path>/lanyard.yaml");
        return Ok(());
    }

    // Two passes so the columns line up around the widest real path rather than
    // around a guess. A registry of absolute paths has no useful fixed width,
    // and a table whose second column moves per row is one you read twice.
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for file in &links.links {
        let (client, detail) = match std::fs::read_to_string(file) {
            Ok(yaml) => match Personas::parse(&yaml, file) {
                Ok(personas) => (
                    personas.client.unwrap_or_else(|| "—".to_string()),
                    personas
                        .list
                        .iter()
                        .map(|p| p.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
                Err(message) => ("broken".to_string(), message),
            },
            // The OS error text ("No such file or directory (os error 2)") is
            // noise in a column whose whole job is to be scannable.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                ("missing".to_string(), "(no such file)".to_string())
            }
            Err(e) => ("unreadable".to_string(), format!("({e})")),
        };
        rows.push((file.display().to_string(), client, detail));
    }

    // Counted in characters, not bytes: the `—` a project with no `client:`
    // gets is one column and three bytes, and a byte-width table is visibly out.
    let width = |s: &String| s.chars().count();
    let path_width = rows.iter().map(|(path, ..)| width(path)).max().unwrap_or(0);
    let client_width = rows
        .iter()
        .map(|(_, client, _)| width(client))
        .max()
        .unwrap_or(0);
    for (path, client, detail) in &rows {
        println!("{path:<path_width$}  {client:<client_width$}  {detail}");
    }
    Ok(())
}

async fn serve() -> Result<(), String> {
    let config = Config::from_env()?;
    // Built once and handed to the state, the bus and every mint. Two places
    // that read the clock is the same drift north star 3 exists to prevent, one
    // layer down.
    let clock = Clock::skewed(config.skew);

    // Everything that can be fatal happens before the port is taken, so a bad
    // key or a bad personas file never leaves a half-started server listening.
    let key = keys::load_or_create(&config.data_dir)?;
    // Still fatal, and still before the port is taken: a malformed global file
    // must not start a server. What changed in Phase 7 is that this is a check
    // rather than the value — the registry re-reads it, and every other source,
    // on demand.
    Personas::load(&config.personas)?;
    let personas = Registry::new(config.personas.clone(), config.links.clone());
    // Resolved once here so the banner can name every source — and so a broken
    // linked project is on the screen before the first request rather than
    // after it.
    let resolved = personas.resolve();

    let addr = config.listen_addr();
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("cannot listen on {addr}: {e}"))?;

    print!(
        "{}",
        banner::render(&banner::Banner {
            issuer: &config.issuer,
            ui: &config.ui_url(),
            log: &config.log_url(),
            listen: &addr,
            data_dir: &config.data_dir.display().to_string(),
            kid: key.kid(),
            clock,
            sources: &resolved.sources,
            warnings: &resolved.warnings,
        })
    );

    let state = Arc::new(AppState {
        config,
        key,
        clock,
        personas,
        stores: Stores::default(),
        events: lanyard_cli::events::EventBus::new(clock),
        hosts: lanyard_cli::hosts::HostSightings::default(),
    });

    axum::serve(listener, app::router(state))
        .await
        .map_err(|e| format!("server stopped: {e}"))
}
