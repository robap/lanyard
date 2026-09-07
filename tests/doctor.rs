//! `lanyard doctor` driven as the real binary, against a real server — because
//! the exit-code contract is what the image's `HEALTHCHECK` depends on, and an
//! exit code is not something an in-process test observes.

mod support;

use std::process::Output;

/// The binary with a clean environment, so nothing the operator exported can
/// change what `doctor` decides. `tests/cli.rs` explains the three exceptions.
fn doctor(url: Option<&str>, args: &[&str]) -> Output {
    doctor_env(url, args, &[])
}

fn doctor_env(url: Option<&str>, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_lanyard"));
    command
        .env_clear()
        .env("HOME", "/nonexistent")
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .envs(env.iter().copied())
        .arg("doctor")
        .args(args);
    if let Some(url) = url {
        command.env("LANYARD_URL", url);
    }
    if let Ok(profile) = std::env::var("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command.output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn line(out: &str, check: &str) -> String {
    out.lines()
        .find(|l| l.trim_start().starts_with(check))
        .unwrap_or_else(|| panic!("no {check} line in:\n{out}"))
        .to_string()
}

/// Part of criterion 5: against a server whose issuer is the address it is
/// listening on, every check passes and the report names what it reached.
#[tokio::test(flavor = "multi_thread")]
async fn doctor_against_a_working_setup_is_all_ok_and_exits_zero() {
    let (base, _bus) = support::spawn_self_consistent().await;
    // A real data dir with the built-in key in it, because the Signing key
    // check reads the file rather than asking the server about it.
    let data = tempfile::tempdir().unwrap();
    lanyard_cli::keys::load_or_create(data.path()).unwrap();

    let out = doctor_env(
        Some(&base),
        &[],
        &[("LANYARD_DATA_DIR", data.path().to_str().unwrap())],
    );
    let report = stdout(&out);

    assert!(out.status.success(), "{report}");
    assert!(report.starts_with("lanyard doctor\n"), "{report}");
    assert!(line(&report, "Config").contains("OK"), "{report}");
    let reachable = line(&report, "Reachable");
    assert!(reachable.contains("OK"), "{report}");
    assert!(
        reachable.contains(&base),
        "the address it reached: {report}"
    );
    assert!(reachable.contains("lanyard 0.1.0"), "{report}");
    assert!(line(&report, "Issuer").contains("OK"), "{report}");
    let jwks = line(&report, "JWKS");
    assert!(jwks.contains("OK"), "{report}");
    assert!(
        jwks.contains(&format!("{base}/oidc/jwks")),
        "the address advertised, not the one configured: {report}"
    );
    assert!(jwks.contains("1 key"), "{report}");
    assert!(line(&report, "Clock").contains("OK"), "{report}");
    let key = line(&report, "Signing key");
    assert!(key.contains("OK"), "{report}");
    assert!(
        key.contains("stable across a wiped data dir"),
        "the answer to the ephemeral-keys gotcha: {report}"
    );
    assert!(
        !report.contains("FAIL") && !report.contains("WARN"),
        "{report}"
    );
    assert_eq!(
        report.lines().count(),
        7,
        "one heading and six checks, no more: {report}"
    );
}

/// Criterion 13's diagnosis half. **The pairing is the criterion**: the `401`
/// is watched in the same session, and this is the half that says why.
#[tokio::test(flavor = "multi_thread")]
async fn a_skewed_server_earns_a_clock_warning_with_the_sixty_second_arithmetic() {
    let (base, _bus) = support::spawn_skewed(-300).await;

    let out = doctor(Some(&base), &[]);
    let report = stdout(&out);

    assert!(out.status.success(), "a wrong clock is a warning: {report}");
    let clock = line(&report, "Clock");
    assert!(clock.contains("WARN"), "{report}");
    assert!(clock.contains("5m0s"), "{report}");
    assert!(clock.contains("behind"), "{report}");
    assert!(report.contains("4m0s expired"), "{report}");
    assert!(
        report.contains("LANYARD_CLOCK_SKEW=-5m0s on purpose"),
        "the server admits it: {report}"
    );
}

/// **The one lie this phase exists to prevent**, driven as the binary: a
/// developer with the variable exported must not be told the clocks agree.
#[tokio::test(flavor = "multi_thread")]
async fn doctor_ignores_the_skew_variable_in_its_own_environment() {
    let (base, _bus) = support::spawn_skewed(-300).await;

    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_lanyard"));
    let out = command
        .env_clear()
        .env("HOME", "/nonexistent")
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("LANYARD_URL", &base)
        // The same skew the server is running. A doctor that applied it would
        // measure zero and report that all is well.
        .env("LANYARD_CLOCK_SKEW", "-5m")
        .arg("doctor")
        .output()
        .unwrap();
    let report = String::from_utf8(out.stdout).unwrap();

    assert!(line(&report, "Clock").contains("WARN"), "{report}");
    assert!(report.contains("5m0s"), "{report}");
    assert!(
        report.contains("doctor ignores it and reads the true clock"),
        "{report}"
    );
}

/// Criterion 8's shape, without a container: the running server's issuer names
/// a port `doctor` did not dial. **`WARN`, exit `0`; `--strict`, non-zero.**
#[tokio::test(flavor = "multi_thread")]
async fn an_issuer_on_another_port_warns_names_both_and_still_exits_zero() {
    // The ordinary harness keeps the canonical `:9500` issuer while binding an
    // ephemeral port, which is exactly the mismatch this check reports.
    let (base, _bus) = support::spawn_logged().await;
    let dialled = base.strip_prefix("http://").unwrap().to_string();

    let out = doctor(Some(&base), &[]);
    let report = stdout(&out);

    assert!(
        out.status.success(),
        "a mismatch may be deliberate, so plain doctor is zero: {report}"
    );
    let issuer = line(&report, "Issuer");
    assert!(issuer.contains("WARN"), "{report}");
    assert!(issuer.contains("127.0.0.1:9500"), "{report}");
    assert!(issuer.contains(&dialled), "{report}");
    assert!(
        report.contains("iss=http://127.0.0.1:9500/oidc"),
        "the value that will be minted: {report}"
    );
    assert!(report.contains("rejects it"), "{report}");

    let strict = doctor(Some(&base), &["--strict"]);
    assert!(
        !strict.status.success(),
        "--strict promotes it: {}",
        stdout(&strict)
    );
}

/// Criterion 6. **`doctor` needs no running server to be useful**: `Config`
/// still reports, `Reachable` fails naming the exact address, and nothing below
/// it invents a value it could not have measured.
#[tokio::test(flavor = "multi_thread")]
async fn with_nothing_listening_reachable_fails_and_config_still_reports() {
    // A port nothing is on. Bound and released, so it is free and known.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let url = format!("http://{addr}");

    let out = doctor(Some(&url), &[]);
    let report = stdout(&out);

    assert!(!out.status.success(), "a FAIL must be non-zero: {report}");
    assert!(line(&report, "Config").contains("OK"), "{report}");
    let reachable = line(&report, "Reachable");
    assert!(reachable.contains("FAIL"), "{report}");
    assert!(
        reachable.contains(&url),
        "the exact address it tried: {report}"
    );
    for guessed in ["Issuer", "JWKS", "Clock"] {
        assert!(
            !report.contains(&format!("{guessed}  ")),
            "{guessed} cannot have been measured: {report}"
        );
    }
    // …and the one check that needs no server still reports.
    assert!(line(&report, "Signing key").contains("OK"), "{report}");
}

/// **What the image's `HEALTHCHECK` runs.** There is no shell and no curl in
/// the image, so the exit code is the entire signal.
#[tokio::test(flavor = "multi_thread")]
async fn quiet_prints_nothing_and_only_sets_the_exit_code() {
    let (base, _bus) = support::spawn_logged().await;

    let out = doctor(Some(&base), &["--quiet"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "", "not one byte");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let out = doctor(Some(&format!("http://{addr}")), &["--quiet"]);
    assert!(!out.status.success());
    assert_eq!(stdout(&out), "");
}

/// A config `serve` would refuse is a `FAIL` before anything is dialled.
#[tokio::test(flavor = "multi_thread")]
async fn a_config_serve_would_refuse_fails_and_names_the_variable() {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_lanyard"));
    let out = command
        .env_clear()
        .env("HOME", "/nonexistent")
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("LANYARD_CLOCK_SKEW", "soon")
        .args(["doctor", "--quiet"])
        .output()
        .unwrap();

    assert!(!out.status.success());
}
