use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use lanyard_cli::app::{self, AppState};
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
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Serve => serve().await,
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("lanyard: {message}");
            ExitCode::FAILURE
        }
    }
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
