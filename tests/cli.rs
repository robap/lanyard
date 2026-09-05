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

async fn spawn() -> String {
    let config = Config::resolve(|key| match key {
        "HOME" => Some("/nonexistent".to_string()),
        _ => None,
    })
    .unwrap();
    let key = SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap();
    let state = Arc::new(AppState {
        config,
        key,
        personas: Personas::builtin(),
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
/// read or write real dotfiles.
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
    command.output().unwrap()
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

#[test]
fn help_lists_the_commands_and_the_token_flags() {
    let top = stdout(&run(None, &["--help"]));
    for command in ["serve", "token", "env"] {
        assert!(
            top.contains(command),
            "`lanyard --help` omits {command}:\n{top}"
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
