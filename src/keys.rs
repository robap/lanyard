//! The signing key: one load path, one file.
//!
//! A fresh install with no data dir must produce the same `kid` and public key
//! every time (CONCEPT §8), so the default is a fixed RSA-2048 key compiled into
//! the binary rather than a seeded generation — seeded generation is
//! deterministic only for as long as the keygen candidate search never changes,
//! which is not a promise any crate makes across versions.
//!
//! **That default key is public.** It is committed to this repository, so anyone
//! with the binary can forge a token this JWKS will validate. Correct for a
//! development IdP, and the reason the default bind is loopback.

use std::fs;
use std::path::{Path, PathBuf};

use rsa::pkcs8::DecodePrivateKey;
use rsa::traits::PublicKeyParts as _;
use rsa::RsaPrivateKey;

use crate::b64;
use crate::runtime::Runtime;

/// Committed on purpose. See the module comment.
pub const DEFAULT_DEV_KEY_PEM: &str = include_str!("default-dev-key.pem");

pub const KEY_FILE: &str = "signing-key.pem";

pub struct SigningKey {
    private: RsaPrivateKey,
    kid: String,
    n: String,
    e: String,
}

/// Hand-written so a stray `{:?}` can never put private material in a log.
impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SigningKey")
            .field("kid", &self.kid)
            .finish()
    }
}

impl SigningKey {
    /// Parse a PKCS#8 PEM private key.
    pub fn from_pem(pem: &str) -> Result<Self, String> {
        let private = RsaPrivateKey::from_pkcs8_pem(pem)
            .map_err(|e| format!("not a PKCS#8 RSA private key: {e}"))?;
        let n = b64::encode(private.n().to_bytes_be());
        let e = b64::encode(private.e().to_bytes_be());
        let kid = thumbprint(&n, &e);
        Ok(SigningKey { private, kid, n, e })
    }

    pub fn kid(&self) -> &str {
        &self.kid
    }

    pub fn private(&self) -> &RsaPrivateKey {
        &self.private
    }

    /// The public half, as it appears in `/oidc/jwks`. Never contains `d` or any
    /// other private field.
    pub fn public_jwk(&self) -> serde_json::Value {
        serde_json::json!({
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": self.kid,
            "n": self.n,
            "e": self.e,
        })
    }
}

/// Bootstrap the data dir and load the key from it.
///
/// If `<data_dir>/signing-key.pem` exists it is the authority; otherwise the
/// built-in default is written there first. Either way the key that gets used is
/// the one read back off disk, so there is exactly one load path.
pub fn load_or_create(data_dir: &Path) -> Result<SigningKey, String> {
    let path = key_path(data_dir);

    if !path.exists() {
        fs::create_dir_all(data_dir).map_err(|e| {
            denied_or(e, data_dir, |e| {
                format!("cannot create data dir {}: {e}", data_dir.display())
            })
        })?;

        let gitignore = data_dir.join(".gitignore");
        if !gitignore.exists() {
            fs::write(&gitignore, "*\n").map_err(|e| {
                denied_or(e, data_dir, |e| {
                    format!("cannot write {}: {e}", gitignore.display())
                })
            })?;
        }

        write_private(&path, data_dir, DEFAULT_DEV_KEY_PEM)?;
    }

    let pem =
        fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;

    // Never fall back to the default key here. A token signed by a key the
    // operator does not expect is the single most confusing failure this tool
    // can produce.
    SigningKey::from_pem(&pem).map_err(|e| format!("{}: {e}", path.display()))
}

/// Write at mode 0600 from the start, rather than creating world-readable and
/// tightening afterwards.
fn write_private(path: &Path, data_dir: &Path, contents: &str) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| {
            denied_or(e, data_dir, |e| {
                format!("cannot write {}: {e}", path.display())
            })
        })?;
    file.write_all(contents.as_bytes()).map_err(|e| {
        denied_or(e, data_dir, |e| {
            format!("cannot write {}: {e}", path.display())
        })
    })
}

/// A permission error on the data directory gets the guardrail; anything else
/// gets the caller's ordinary message.
///
/// **`Permission denied (os error 13)` on a path the developer did not choose
/// is the class of failure Phase 8 exists to delete**, and in a container it is
/// almost always one thing: a bind mount whose ownership was not mapped.
fn denied_or(
    error: std::io::Error,
    data_dir: &Path,
    ordinary: impl FnOnce(std::io::Error) -> String,
) -> String {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => {
            not_writable(data_dir, Runtime::detect(), current_uid())
        }
        _ => ordinary(error),
    }
}

/// The message, as a pure function of the three things it depends on, so every
/// branch of it is testable without a read-only directory or a container.
pub fn not_writable(data_dir: &Path, runtime: Runtime, uid: Option<u32>) -> String {
    let who = match uid {
        Some(uid) => format!(" by uid {uid}"),
        None => String::new(),
    };
    let where_ = match runtime {
        Runtime::Host => String::new(),
        _ => " (this process is in a container)".to_string(),
    };

    // **The runtime's own remedy first.** Rootless podman maps the container's
    // root to the invoking user and every other uid into a subuid range, so a
    // directory you own appears inside as root's and uid 65532 cannot write it.
    // Under rootful Docker the same mount behaves the other way around, which
    // is why the two commands are opposites rather than variants.
    let remedies = match runtime {
        Runtime::Docker => [
            "docker run --user $(id -u):$(id -g)  …    # docker",
            "podman run -v ./lanyard-data:/data:U …    # podman, rootless",
        ],
        _ => [
            "podman run -v ./lanyard-data:/data:U …    # podman, rootless",
            "docker run --user $(id -u):$(id -g)  …    # docker",
        ],
    };

    // Built line by line rather than as one format string: the message's shape
    // — an indented block of two commands to paste — is the point of it, and a
    // formatter that rewrapped a string literal would quietly destroy that.
    [
        format!("{} is not writable{who}{where_}.", data_dir.display()),
        "  A bind-mounted data directory needs its ownership mapped. Try:".to_string(),
        format!("      {}", remedies[0]),
        format!("      {}", remedies[1]),
        "  Or drop the volume entirely: the default signing key is deterministic, so a".to_string(),
        "  container with no volume already produces the same kid every time.".to_string(),
    ]
    .join("\n")
}

/// This process's uid, without a libc dependency.
///
/// `/proc/self` is owned by the process that is looking at it, which is the one
/// fact needed here — and the one the error message is useless without, because
/// "not writable" with no uid leaves the reader guessing which side to change.
fn current_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt as _;
    fs::metadata("/proc/self").ok().map(|m| m.uid())
}

pub fn key_path(data_dir: &Path) -> PathBuf {
    data_dir.join(KEY_FILE)
}

/// RFC 7638 JWK thumbprint. The hashed input is a hand-built string —
/// exactly `{"e":"…","kty":"RSA","n":"…"}`, lexicographic member order, no
/// whitespace — because a `serde_json` object would only be right by accident.
pub fn thumbprint(n: &str, e: &str) -> String {
    use sha2::{Digest as _, Sha256};

    let canonical = format!(r#"{{"e":"{e}","kty":"RSA","n":"{n}"}}"#);
    b64::encode(Sha256::digest(canonical.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// The worked example from RFC 7638 §3.1. Checking the thumbprint against a
    /// second implementation rather than against ourselves: a wrong
    /// serialization still yields a *stable* kid, so a self-consistency test
    /// would pass while the value is non-standard.
    const RFC7638_N: &str = "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw";
    const RFC7638_THUMBPRINT: &str = "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs";

    #[test]
    fn thumbprint_matches_the_rfc_7638_worked_example() {
        assert_eq!(thumbprint(RFC7638_N, "AQAB"), RFC7638_THUMBPRINT);
    }

    #[test]
    fn the_built_in_default_key_parses_and_is_rsa_2048() {
        let key = SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap();
        let jwk = key.public_jwk();
        assert_eq!(jwk["kty"], "RSA");
        assert_eq!(jwk["e"], "AQAB");
        // 2048 bits of modulus, base64url-encoded without padding.
        let n = jwk["n"].as_str().unwrap();
        assert_eq!(b64::decode(n).unwrap().len(), 256, "not a 2048-bit modulus");
        assert!(!n.contains('='), "base64url must be unpadded");
    }

    #[test]
    fn the_public_jwk_carries_no_private_material() {
        let key = SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap();
        let jwk = key.public_jwk();
        let obj = jwk.as_object().unwrap();
        for private in ["d", "p", "q", "dp", "dq", "qi", "oth"] {
            assert!(!obj.contains_key(private), "jwk leaks {private}");
        }
        assert_eq!(jwk["use"], "sig");
        assert_eq!(jwk["alg"], "RS256");
        assert_eq!(jwk["kid"], key.kid());
    }

    #[test]
    fn bootstrap_writes_the_key_its_gitignore_and_mode_600() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("nested").join("lanyard");

        let key = load_or_create(&data_dir).unwrap();

        let pem = key_path(&data_dir);
        assert!(pem.exists());
        assert_eq!(
            mode(&pem),
            0o600,
            "the signing key must not be world-readable"
        );

        let gitignore = fs::read_to_string(data_dir.join(".gitignore")).unwrap();
        assert!(
            gitignore.contains('*'),
            "data dir must ignore its own contents"
        );

        // Written from the built-in constant, so the kid is the stable one.
        assert_eq!(
            key.kid(),
            SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap().kid()
        );
    }

    #[test]
    fn a_key_already_on_disk_is_the_authority() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        fs::write(key_path(&data_dir), OTHER_KEY_PEM).unwrap();

        let key = load_or_create(&data_dir).unwrap();
        assert_eq!(
            key.kid(),
            SigningKey::from_pem(OTHER_KEY_PEM).unwrap().kid()
        );
        assert_ne!(
            key.kid(),
            SigningKey::from_pem(DEFAULT_DEV_KEY_PEM).unwrap().kid(),
            "the file on disk was ignored in favour of the default"
        );
        // Loading it must not rewrite it.
        assert_eq!(
            fs::read_to_string(key_path(&data_dir)).unwrap(),
            OTHER_KEY_PEM
        );
    }

    #[test]
    fn a_second_run_reloads_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let first = load_or_create(dir.path()).unwrap();
        let second = load_or_create(dir.path()).unwrap();
        assert_eq!(first.kid(), second.kid());
        assert_eq!(first.public_jwk(), second.public_jwk());
    }

    #[test]
    fn a_garbage_key_file_is_fatal_and_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(key_path(dir.path()), "not a key\n").unwrap();

        let err = load_or_create(dir.path()).unwrap_err();
        assert!(
            err.contains(key_path(dir.path()).to_str().unwrap()),
            "error must name the file: {err}"
        );
    }

    #[test]
    fn a_well_formed_but_non_rsa_key_is_fatal_and_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(key_path(dir.path()), EC_KEY_PEM).unwrap();

        let err = load_or_create(dir.path()).unwrap_err();
        assert!(
            err.contains(key_path(dir.path()).to_str().unwrap()),
            "error must name the file: {err}"
        );
    }

    /// **The class of failure Phase 8 exists to delete.** A bare
    /// `Permission denied (os error 13)` on a path the developer did not choose
    /// tells them nothing; this names the directory, the uid, and both
    /// runtimes' fix.
    #[test]
    fn an_unwritable_data_dir_names_the_directory_the_uid_and_both_fixes() {
        let message = not_writable(Path::new("/data"), Runtime::Podman, Some(65532));

        assert!(
            message.starts_with("/data is not writable by uid 65532"),
            "{message}"
        );
        assert!(message.contains("in a container"), "{message}");
        assert!(message.contains("-v ./lanyard-data:/data:U"), "{message}");
        assert!(message.contains("--user $(id -u):$(id -g)"), "{message}");
        assert!(
            message.contains("no volume already produces the same kid"),
            "the answer is often that the volume was never needed: {message}"
        );
        assert!(!message.contains("os error"), "{message}");
    }

    /// Rootless podman and rootful Docker map ownership in **opposite**
    /// directions, so the runtime's own remedy goes first rather than being one
    /// of two equal options.
    #[test]
    fn the_detected_runtimes_remedy_is_the_first_one_offered() {
        let podman = not_writable(Path::new("/data"), Runtime::Podman, Some(65532));
        assert!(
            podman.find(":U").unwrap() < podman.find("--user").unwrap(),
            "{podman}"
        );

        let docker = not_writable(Path::new("/data"), Runtime::Docker, Some(65532));
        assert!(
            docker.find("--user").unwrap() < docker.find(":U").unwrap(),
            "{docker}"
        );
    }

    /// Outside a container the same mount problem is possible and the sentence
    /// about being in one would be a lie.
    #[test]
    fn on_the_host_the_message_does_not_claim_to_be_in_a_container() {
        let message = not_writable(Path::new("/srv/lanyard"), Runtime::Host, Some(1000));
        assert!(!message.contains("in a container"), "{message}");
        assert!(
            message.contains("/srv/lanyard is not writable by uid 1000"),
            "{message}"
        );
    }

    /// The real path, through `load_or_create`, on a directory nothing can
    /// write to.
    #[test]
    fn load_or_create_on_an_unwritable_directory_gives_the_guardrail() {
        use std::os::unix::fs::PermissionsExt as _;

        // root writes anywhere, so there is nothing to observe.
        if current_uid() == Some(0) {
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).unwrap();
        fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o500)).unwrap();

        let err = load_or_create(&data_dir).unwrap_err();

        assert!(err.contains(data_dir.to_str().unwrap()), "{err}");
        assert!(err.contains("is not writable"), "{err}");
        assert!(
            !err.contains("os error 13"),
            "no bare errno, which is the whole point: {err}"
        );

        // Left as we found it, so the tempdir can be cleaned up.
        fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o700)).unwrap();
    }

    /// A permission error is one failure; every other one keeps the message it
    /// had, because "not writable" would be wrong about them.
    #[test]
    fn a_non_permission_failure_keeps_its_ordinary_message() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the data directory should be: `create_dir_all` fails,
        // but not for want of permission.
        let data_dir = dir.path().join("in-the-way");
        fs::write(&data_dir, "not a directory").unwrap();

        let err = load_or_create(&data_dir).unwrap_err();
        assert!(!err.contains("is not writable"), "{err}");
    }

    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// A second, throwaway RSA-2048 key, so "the file wins" is observable.
    const OTHER_KEY_PEM: &str = include_str!("../tests/data/other-key.pem");
    /// Valid PKCS#8, wrong algorithm — the near-miss that a fallback-to-default
    /// would silently paper over.
    const EC_KEY_PEM: &str = include_str!("../tests/data/ec-key.pem");
}
