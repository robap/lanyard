//! The client half, driven the way a shell drives it.
//!
//! Every test here points at a router on an ephemeral port via `LANYARD_URL`,
//! because port 9500 is global to the machine. The binary under test is the real
//! one cargo built (`CARGO_BIN_EXE_lanyard`), not a library call dressed up as
//! one: the thing being pinned is what lands on stdout, what lands on stderr and
//! what the exit code is — and `$(lanyard token …)` reads exactly that.

use std::process::Output;
use std::sync::Arc;

use lanyard_cli::app::{self, AppState};
use lanyard_cli::client::{self, MintRequest};
use lanyard_cli::config::Config;
use lanyard_cli::keys::{SigningKey, DEFAULT_DEV_KEY_PEM};
use lanyard_cli::persona::Personas;
use lanyard_cli::registry::Registry;
use lanyard_cli::store::Stores;

async fn spawn() -> String {
    spawn_with(Personas::builtin()).await
}

async fn spawn_with(personas: Personas) -> String {
    let config = Config::resolve(|key| match key {
        "HOME" => Some("/nonexistent".to_string()),
        _ => None,
    })
    .unwrap();
    let key = SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap();
    let state = Arc::new(AppState {
        config,
        key,
        personas: Registry::fixed(personas),
        stores: Stores::default(),
        events: lanyard_cli::events::EventBus::new(),
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app::router(state)).await.unwrap();
    });
    format!("http://{addr}")
}

/// A port nothing is listening on, for the "no lanyard running" cases. Bound and
/// dropped, so it is free and almost certainly still free a moment later.
async fn dead_url() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn payload_of(token: &str) -> serde_json::Value {
    let segment = token.split('.').nth(1).expect("compact JWS");
    serde_json::from_slice(&lanyard_cli::b64::decode(segment).unwrap()).unwrap()
}

/// Run the real binary with a clean environment: no `LANYARD_*` leaking in from
/// the operator's shell, and a `HOME` that does not exist, so nothing here can
/// read or write real dotfiles. The three exceptions — `HOME`, `PATH` and, under
/// a coverage run, `LLVM_PROFILE_FILE` — are put back deliberately below.
///
/// This blocks the calling thread, so every test that calls it while a router is
/// running in-process must be a `multi_thread` one — a single-threaded runtime
/// would sit here forever waiting for a server it is itself supposed to poll.
fn run(url: Option<&str>, args: &[&str]) -> Output {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_lanyard"));
    command
        .env_clear()
        .env("HOME", "/nonexistent")
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .args(args);
    if let Some(url) = url {
        command.env("LANYARD_URL", url);
    }
    // The one variable that is not the operator's, put back after `env_clear`.
    //
    // `cargo llvm-cov` uses `LLVM_PROFILE_FILE` to tell an instrumented binary
    // where to write its coverage profile. A child that loses it falls back to
    // LLVM's default filename and drops a `default_*.profraw` in whatever
    // directory the test happened to run from — the repository root. Those
    // profiles are never merged, so `main.rs` reports 0% coverage that these
    // twenty-odd tests actually give it, and the stray files land in
    // `git status`. Absent outside a coverage run, in which case nothing is set
    // and the environment stays as clean as the doc comment says.
    if let Ok(profile) = std::env::var("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command.output().unwrap()
}

/// `lanyard logs` never exits on its own — the stream is open until lanyard
/// stops — so it cannot be driven by [`run`], which waits for a process to end.
/// This starts it, collects lines off its stdout until `lines` have arrived or
/// the deadline passes, then kills it and gives back what it printed.
fn run_logs(url: &str, args: &[&str], lines: usize) -> Vec<String> {
    use std::io::BufRead as _;

    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_lanyard"));
    command
        .env_clear()
        .env("HOME", "/nonexistent")
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("LANYARD_URL", url)
        .args(args)
        .stdout(std::process::Stdio::piped());
    if let Ok(profile) = std::env::var("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    let mut child = command.spawn().unwrap();

    let out = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(out).lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    });

    let mut collected = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while collected.len() < lines {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            break;
        }
        match rx.recv_timeout(left) {
            Ok(line) => collected.push(line),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    collected
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

// ----------------------------------------------------------- client::mint --

#[tokio::test]
async fn mint_performs_a_real_grant_and_returns_the_access_token() {
    let base = spawn().await;
    let token = client::mint(&MintRequest {
        url: &base,
        client_id: client::CLI_CLIENT_ID,
        persona: "ada",
        audience: Some("billing-api"),
        scope: Some("orders:read"),
        flaw: None,
    })
    .await
    .unwrap();

    let claims = payload_of(&token);
    assert_eq!(claims["sub"], "ada");
    assert_eq!(claims["aud"], "billing-api");
    assert_eq!(claims["scope"], "orders:read");
    // The CLI is a client like any other, and says so.
    assert_eq!(claims["client_id"], "lanyard-cli");
}

/// The message every new user hits first. It has to name the URL that was tried
/// and the fix, because "connection refused" alone does not say which address.
#[tokio::test]
async fn nothing_listening_names_the_url_and_the_fix() {
    let url = dead_url().await;
    let error = client::mint(&MintRequest {
        url: &url,
        client_id: client::CLI_CLIENT_ID,
        persona: "ada",
        audience: None,
        scope: None,
        flaw: None,
    })
    .await
    .unwrap_err();

    let message = error.to_string();
    assert!(message.contains(&url), "must name the URL tried: {message}");
    assert!(
        message.contains("lanyard serve"),
        "must name the fix: {message}"
    );
}

/// The server's `error_description` reaches the user verbatim, which is how the
/// unknown-persona id gets in front of them.
#[tokio::test]
async fn a_rejected_grant_surfaces_the_servers_description() {
    let base = spawn().await;
    let error = client::mint(&MintRequest {
        url: &base,
        client_id: client::CLI_CLIENT_ID,
        persona: "nope",
        audience: None,
        scope: None,
        flaw: None,
    })
    .await
    .unwrap_err();

    assert!(error.to_string().contains("nope"), "{error}");
}

/// The six flags are one more form field on the grant the CLI already posts —
/// **the CLI still does not sign.** A local string edit for `--bad-signature`
/// would be the second signing path this whole phase exists to not have.
#[tokio::test]
async fn mint_posts_the_flaw_as_one_more_form_field() {
    let base = spawn().await;
    let token = client::mint(&MintRequest {
        url: &base,
        client_id: client::CLI_CLIENT_ID,
        persona: "ada",
        audience: Some("billing-api"),
        scope: None,
        flaw: Some("wrong-aud"),
    })
    .await
    .unwrap();

    assert_eq!(payload_of(&token)["aud"], "wrong-billing-api");
}

// ------------------------------------------------------- lanyard token/env --

/// A JWT is three dot-separated base64url segments, and nothing else may share
/// the line: `$(lanyard token …)` goes straight into an `Authorization` header.
fn assert_is_one_bare_token(out: &str) {
    let line = out
        .strip_suffix('\n')
        .expect("exactly one trailing newline");
    assert!(!line.contains('\n'), "more than one line: {out:?}");
    let segments: Vec<&str> = line.split('.').collect();
    assert_eq!(segments.len(), 3, "not a compact JWS: {line:?}");
    for segment in segments {
        assert!(!segment.is_empty(), "empty segment in {line:?}");
        assert!(
            segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "not base64url: {segment:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn token_prints_the_token_and_nothing_else() {
    let base = spawn().await;
    let output = run(
        Some(&base),
        &["token", "--as", "ada", "--aud", "billing-api"],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        stderr(&output),
        "",
        "no banner, no timing, no minted-for line"
    );
    assert_is_one_bare_token(&stdout(&output));

    let claims = payload_of(stdout(&output).trim());
    assert_eq!(claims["sub"], "ada");
    assert_eq!(claims["aud"], "billing-api");
}

#[tokio::test(flavor = "multi_thread")]
async fn token_passes_scope_through_and_omits_aud_when_not_asked() {
    let base = spawn().await;

    let output = run(
        Some(&base),
        &[
            "token",
            "--as",
            "ada",
            "--scope",
            "orders:read orders:write",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let claims = payload_of(stdout(&output).trim());
    assert_eq!(claims["scope"], "orders:read orders:write");
    assert!(
        claims.as_object().unwrap().get("aud").is_none(),
        "no --aud was given: {claims}"
    );
}

/// Any `--aud` mints. Rejection is the API's job, not minting's — that is a test
/// case, not an error (CONCEPT §5).
#[tokio::test(flavor = "multi_thread")]
async fn an_audience_no_api_expects_still_mints() {
    let base = spawn().await;
    let output = run(
        Some(&base),
        &["token", "--as", "ada", "--aud", "not-a-real-api"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(payload_of(stdout(&output).trim())["aud"], "not-a-real-api");
}

/// The discipline that keeps `curl -H "Bearer $(lanyard token …)"` from sending
/// an error message as a credential: a 401 with a garbage token is a better
/// outcome than an API receiving the words "connection refused".
#[tokio::test(flavor = "multi_thread")]
async fn failure_writes_nothing_at_all_to_stdout() {
    let unreachable = dead_url().await;
    let base = spawn().await;

    let cases = [
        (
            "nothing listening",
            run(Some(&unreachable), &["token", "--as", "ada"]),
        ),
        (
            "unknown persona",
            run(Some(&base), &["token", "--as", "nope"]),
        ),
    ];

    for (label, output) in cases {
        assert!(!output.status.success(), "{label} should exit non-zero");
        assert_eq!(output.stdout.len(), 0, "{label} wrote to stdout");
        assert!(
            !stderr(&output).is_empty(),
            "{label} said nothing on stderr"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_listening_tells_the_user_where_and_what_to_run() {
    let unreachable = dead_url().await;
    let output = run(Some(&unreachable), &["token", "--as", "ada"]);
    let message = stderr(&output);
    assert!(message.contains(&unreachable), "{message}");
    assert!(message.contains("lanyard serve"), "{message}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_persona_names_the_id_on_stderr() {
    let base = spawn().await;
    let output = run(Some(&base), &["token", "--as", "nope"]);
    assert!(stderr(&output).contains("nope"), "{}", stderr(&output));
}

/// `--client` is not a new concept: it sets the `client_id` on a grant lanyard
/// already accepts from anyone. It is what reaches a scoped persona from a
/// shell, and it lands in the token's `client_id` claim like any other.
#[tokio::test(flavor = "multi_thread")]
async fn client_reaches_a_scoped_persona_and_lands_in_the_claim() {
    let base = spawn_with(
        Personas::parse(
            "personas:\n  - id: dev-admin\n    client: billing-web\n",
            std::path::Path::new("/home/dev/code/billing/lanyard.yaml"),
        )
        .unwrap(),
    )
    .await;

    let output = run(
        Some(&base),
        &[
            "token",
            "--as",
            "dev-admin",
            "--client",
            "billing-web",
            "--aud",
            "billing-api",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));

    let claims = payload_of(stdout(&output).trim());
    assert_eq!(claims["sub"], "dev-admin");
    assert_eq!(claims["client_id"], "billing-web");
    assert_eq!(claims["aud"], "billing-api");
}

/// The default is unchanged, and it is what the refusal above names.
#[tokio::test(flavor = "multi_thread")]
async fn without_client_the_grant_still_says_lanyard_cli() {
    let base = spawn().await;
    let output = run(Some(&base), &["token", "--as", "ada"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        payload_of(stdout(&output).trim())["client_id"],
        "lanyard-cli"
    );
}

/// `env` takes the same flag: the two subcommands share [`MintArgs`], and a
/// scoped persona that `token` could reach and `env` could not would be a
/// difference with no reason behind it.
#[tokio::test(flavor = "multi_thread")]
async fn env_takes_client_too() {
    let base = spawn_with(
        Personas::parse(
            "personas:\n  - id: dev-admin\n    client: billing-web\n",
            std::path::Path::new("/tmp/lanyard.yaml"),
        )
        .unwrap(),
    )
    .await;

    let output = run(
        Some(&base),
        &["env", "--as", "dev-admin", "--client", "billing-web"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let token = stdout(&output)
        .strip_prefix("export BEARER_TOKEN='")
        .and_then(|rest| rest.strip_suffix("'\n"))
        .unwrap()
        .to_string();
    assert_eq!(payload_of(&token)["sub"], "dev-admin");
}

/// **The message that makes namespacing usable rather than maddening.**
///
/// The CLI sends `client_id=lanyard-cli`, so a persona scoped to `billing-web`
/// is invisible to it. `no such persona` would be true and useless — the file
/// is right there. Written by the server at its one refusal funnel and
/// surfaced verbatim, so there is one copy of the sentence.
#[tokio::test(flavor = "multi_thread")]
async fn a_persona_scoped_to_another_client_says_where_it_is_and_which_flag_reaches_it() {
    let base = spawn_with(
        Personas::parse(
            "personas:\n  - id: dev-admin\n    client: billing-web\n",
            std::path::Path::new("/home/dev/code/billing/lanyard.yaml"),
        )
        .unwrap(),
    )
    .await;

    let output = run(Some(&base), &["token", "--as", "dev-admin"]);
    assert!(!output.status.success(), "must exit non-zero");
    assert_eq!(stdout(&output), "", "stdout stays empty on a refusal");
    assert_eq!(
        stderr(&output),
        "lanyard: no persona \"dev-admin\" for client \"lanyard-cli\" — it is defined in \
         /home/dev/code/billing/lanyard.yaml scoped to client \"billing-web\". \
         Retry with --client billing-web\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn env_prints_a_single_quoted_export_line() {
    let base = spawn().await;
    let output = run(Some(&base), &["env", "--as", "ada", "--aud", "billing-api"]);
    assert!(output.status.success(), "{}", stderr(&output));

    let line = stdout(&output);
    let token = line
        .strip_prefix("export BEARER_TOKEN='")
        .and_then(|rest| rest.strip_suffix("'\n"))
        .unwrap_or_else(|| panic!("not a single-quoted export line: {line:?}"));
    assert_eq!(payload_of(token)["sub"], "ada");
}

/// `eval "$(…)"` of an empty string is a no-op, so a failed mint leaves the
/// shell exactly as it was.
#[tokio::test(flavor = "multi_thread")]
async fn env_prints_nothing_on_stdout_when_minting_fails() {
    let base = spawn().await;
    let output = run(Some(&base), &["env", "--as", "nope"]);

    assert!(!output.status.success());
    assert_eq!(output.stdout.len(), 0, "eval would run this");
    assert!(stderr(&output).contains("nope"), "{}", stderr(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn as_is_required_and_its_absence_is_a_usage_error() {
    let base = spawn().await;
    for subcommand in ["token", "env"] {
        let output = run(Some(&base), &[subcommand]);
        assert!(!output.status.success(), "{subcommand} without --as");
        assert_eq!(output.stdout.len(), 0, "{subcommand} without --as");
        assert!(stderr(&output).contains("--as"), "{}", stderr(&output));
    }
}

// ------------------------------------------------------------ lanyard logs --

/// Criterion 3. One client id per event, as each happens — the shape
/// `lanyard logs --json | jq .client_id` reads.
#[tokio::test(flavor = "multi_thread")]
async fn logs_json_prints_one_json_object_per_event() {
    let base = spawn().await;
    let url = base.clone();
    // Minted from another thread while the child is already connected, so this
    // is the live half of the stream rather than the replay.
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        for persona in ["ada", "mira"] {
            run(Some(&url), &["token", "--as", persona]);
        }
    });

    let lines = run_logs(&base, &["logs", "--json"], 2);
    assert!(lines.len() >= 2, "{lines:#?}");
    for line in &lines {
        let event: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON: {line:?} ({e})"));
        assert_eq!(event["client_id"], "lanyard-cli", "{event}");
        assert_eq!(event["endpoint"], "/oidc/token");
    }
}

/// Criteria 4 and 6 together: the same lines `lanyard serve` prints on its own
/// stdout, and three already-finished mints print before the stream goes live.
#[tokio::test(flavor = "multi_thread")]
async fn logs_replays_finished_requests_in_the_serve_format() {
    let base = spawn().await;
    for persona in ["ada", "mira", "ada"] {
        run(
            Some(&base),
            &["token", "--as", persona, "--aud", "billing-api"],
        );
    }

    let lines = run_logs(&base, &["logs"], 3);
    assert_eq!(
        lines.len(),
        3,
        "the three already-finished mints: {lines:#?}"
    );
    for line in &lines {
        assert!(line.contains("lanyard-cli"), "{line}");
        assert!(line.contains("POST /oidc/token"), "{line}");
        assert!(line.contains("client_credentials"), "{line}");
        assert!(line.contains("aud=billing-api"), "{line}");
        assert!(!line.contains('{'), "not JSON without --json: {line}");
    }
}

/// Criterion 7. A tool that exits `0` having printed nothing is a tool that
/// lies about having looked — the README's existing stance, applied to the
/// fourth subcommand.
#[tokio::test(flavor = "multi_thread")]
async fn logs_with_no_lanyard_running_fails_and_names_the_connection() {
    let dead = dead_url().await;
    let output = run(Some(&dead), &["logs"]);
    assert!(!output.status.success(), "must not exit 0");
    assert_eq!(output.stdout.len(), 0, "nothing on stdout");
    let err = stderr(&output);
    assert!(err.contains(&dead), "names the address tried: {err}");
    assert!(err.contains("lanyard serve"), "names the fix: {err}");
}

#[test]
fn help_lists_the_commands_and_the_token_flags() {
    let top = stdout(&run(None, &["--help"]));
    for command in ["serve", "token", "env", "logs"] {
        assert!(
            top.contains(command),
            "`lanyard --help` omits {command}:\n{top}"
        );
    }

    let logs = stdout(&run(None, &["logs", "--help"]));
    for flag in ["--json", "--url"] {
        assert!(
            logs.contains(flag),
            "`lanyard logs --help` omits {flag}:\n{logs}"
        );
    }

    let token = stdout(&run(None, &["token", "--help"]));
    for flag in ["--as", "--aud", "--scope", "--url"] {
        assert!(
            token.contains(flag),
            "`lanyard token --help` omits {flag}:\n{token}"
        );
    }
}

/// Something answered, but not like lanyard — almost always the wrong `--url`.
#[tokio::test]
async fn a_reply_that_is_not_a_token_names_the_url_and_the_status() {
    let base = spawn().await;
    // A real server, a real 404: `<base>/_/oidc/token` is not a route.
    let wrong = format!("{base}/_");
    let error = client::mint(&MintRequest {
        url: &wrong,
        client_id: client::CLI_CLIENT_ID,
        persona: "ada",
        audience: None,
        scope: None,
        flaw: None,
    })
    .await
    .unwrap_err();

    let message = error.to_string();
    assert!(message.contains(&wrong), "{message}");
    assert!(message.contains("404"), "{message}");
}

/// `reqwest` is built with no TLS stack, so `https://` fails at request time
/// rather than at connect time. HTTPS is post-v1; the error must still name the
/// URL, because "unknown scheme" on its own does not say which setting was wrong.
#[tokio::test]
async fn an_https_url_fails_with_a_message_that_still_names_it() {
    let url = "https://127.0.0.1:9500";
    let error = client::mint(&MintRequest {
        url,
        client_id: client::CLI_CLIENT_ID,
        persona: "ada",
        audience: None,
        scope: None,
        flaw: None,
    })
    .await
    .unwrap_err();

    assert!(error.to_string().contains(url), "{error}");
}

// ---------------------------------------------------- the six failure flags --

/// **The CLI still does not sign.** Every flag below is one more form field on
/// the grant it already posts; a local string edit for `--bad-signature` would
/// be the second signing path this whole phase exists to not have.
#[tokio::test(flavor = "multi_thread")]
async fn expired_prints_a_token_whose_exp_is_already_past() {
    let base = spawn().await;
    let output = run(
        Some(&base),
        &["token", "--as", "ada", "--aud", "billing-api", "--expired"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_is_one_bare_token(&stdout(&output));

    let claims = payload_of(stdout(&output).trim());
    assert!(claims["exp"].as_u64().unwrap() < now(), "{claims}");
    assert_eq!(claims["sub"], "ada", "only the clock is wrong");
    assert_eq!(claims["aud"], "billing-api");
}

#[tokio::test(flavor = "multi_thread")]
async fn each_claim_level_flag_damages_exactly_its_own_claim() {
    let base = spawn().await;

    let wrong_aud = run(
        Some(&base),
        &[
            "token",
            "--as",
            "ada",
            "--aud",
            "billing-api",
            "--wrong-aud",
        ],
    );
    let claims = payload_of(stdout(&wrong_aud).trim());
    assert_eq!(claims["aud"], "wrong-billing-api");
    assert_eq!(claims["iss"], "http://127.0.0.1:9500/oidc");

    let no_aud = run(Some(&base), &["token", "--as", "ada", "--wrong-aud"]);
    assert_eq!(payload_of(stdout(&no_aud).trim())["aud"], "wrong-audience");

    let wrong_iss = run(
        Some(&base),
        &[
            "token",
            "--as",
            "ada",
            "--aud",
            "billing-api",
            "--wrong-iss",
        ],
    );
    let claims = payload_of(stdout(&wrong_iss).trim());
    assert_eq!(claims["iss"], "https://wrong-issuer.example.test");
    assert_eq!(claims["aud"], "billing-api", "only iss is wrong");
}

/// `--alg-none` is the one token that is not three non-empty segments, so
/// `assert_is_one_bare_token` deliberately does not apply to it.
#[tokio::test(flavor = "multi_thread")]
async fn alg_none_prints_an_unsecured_jwt_with_an_empty_third_segment() {
    let base = spawn().await;
    let output = run(
        Some(&base),
        &["token", "--as", "ada", "--aud", "billing-api", "--alg-none"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let line = stdout(&output);
    let parts: Vec<&str> = line.trim_end().split('.').collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[2], "");
    assert_eq!(
        String::from_utf8(lanyard_cli::b64::decode(parts[0]).unwrap()).unwrap(),
        r#"{"alg":"none","typ":"JWT"}"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_kid_and_bad_signature_come_back_from_the_server() {
    let base = spawn().await;

    let unknown = run(
        Some(&base),
        &[
            "token",
            "--as",
            "ada",
            "--aud",
            "billing-api",
            "--unknown-kid",
        ],
    );
    let header: serde_json::Value = serde_json::from_slice(
        &lanyard_cli::b64::decode(stdout(&unknown).trim().split('.').next().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(header["kid"], "lanyard-unknown-kid");
    assert_eq!(header["alg"], "RS256");

    let good = run(
        Some(&base),
        &["token", "--as", "ada", "--aud", "billing-api"],
    );
    let bad = run(
        Some(&base),
        &[
            "token",
            "--as",
            "ada",
            "--aud",
            "billing-api",
            "--bad-signature",
        ],
    );
    let good_header = stdout(&good);
    let bad_header = stdout(&bad);
    assert_eq!(
        bad_header.trim().split('.').next().unwrap(),
        good_header.trim().split('.').next().unwrap(),
        "byte-identical header, so the failure lands on the signature"
    );
    assert_eq!(payload_of(bad_header.trim())["sub"], "ada");
}

/// One flaw per token: a resource server reports only the first check it fails,
/// so two flags at once tests strictly less than either alone.
#[tokio::test(flavor = "multi_thread")]
async fn two_flaws_at_once_is_a_usage_error_naming_both() {
    let base = spawn().await;
    let output = run(
        Some(&base),
        &["token", "--as", "ada", "--expired", "--wrong-aud"],
    );

    assert!(!output.status.success());
    assert_eq!(output.stdout.len(), 0, "nothing may reach a header");
    let message = stderr(&output);
    assert!(message.contains("--expired"), "{message}");
    assert!(message.contains("--wrong-aud"), "{message}");
}

#[tokio::test(flavor = "multi_thread")]
async fn env_takes_the_flags_too() {
    let base = spawn().await;
    let output = run(
        Some(&base),
        &["env", "--as", "ada", "--aud", "billing-api", "--expired"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let line = stdout(&output);
    let token = line
        .strip_prefix("export BEARER_TOKEN='")
        .and_then(|rest| rest.strip_suffix("'\n"))
        .unwrap_or_else(|| panic!("not a single-quoted export line: {line:?}"));
    assert!(payload_of(token)["exp"].as_u64().unwrap() < now());
}

#[test]
fn both_help_screens_list_all_six_flags() {
    for subcommand in ["token", "env"] {
        let help = stdout(&run(None, &[subcommand, "--help"]));
        for flag in [
            "--expired",
            "--wrong-aud",
            "--wrong-iss",
            "--bad-signature",
            "--alg-none",
            "--unknown-kid",
        ] {
            assert!(
                help.contains(flag),
                "`lanyard {subcommand} --help` omits {flag}:\n{help}"
            );
        }
    }
}
