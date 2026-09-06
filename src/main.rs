use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand};
use lanyard_cli::app::{self, AppState};
use lanyard_cli::client::{self, MintRequest};
use lanyard_cli::oidc::flaw::Flaw;
use lanyard_cli::persona::Personas;
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
        audience: args.aud.as_deref(),
        scope: args.scope.as_deref(),
        flaw: args.flaw(),
    })
    .await
    .map_err(|e| e.to_string())
}

/// The fourth subcommand, and the second thing the CLI does over HTTP against
/// the running singleton rather than in-process.
async fn follow(args: &LogsArgs) -> Result<(), String> {
    let url = client::resolve_url(args.url.as_deref(), |key| std::env::var(key).ok())?;
    client::logs(&url, args.json)
        .await
        .map_err(|e| e.to_string())
}

async fn serve() -> Result<(), String> {
    let config = Config::from_env()?;

    // Everything that can be fatal happens before the port is taken, so a bad
    // key or a bad personas file never leaves a half-started server listening.
    let key = keys::load_or_create(&config.data_dir)?;
    let personas = Personas::load(&config.personas)?;

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
            personas: &personas,
        })
    );

    let state = Arc::new(AppState {
        config,
        key,
        personas,
        stores: Stores::default(),
        events: lanyard_cli::events::EventBus::new(),
    });

    axum::serve(listener, app::router(state))
        .await
        .map_err(|e| format!("server stopped: {e}"))
}
