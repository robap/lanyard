use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand};
use lanyard_cli::app::{self, AppState};
use lanyard_cli::client::{self, MintRequest};
use lanyard_cli::persona::Personas;
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
    })
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
    });

    axum::serve(listener, app::router(state))
        .await
        .map_err(|e| format!("server stopped: {e}"))
}
