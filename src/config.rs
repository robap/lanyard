//! Everything the process needs from its environment, resolved exactly once at
//! startup. In particular the issuer: CONCEPT §8 is emphatic that it must never
//! be derived per-request from the `Host` header.

use std::path::PathBuf;

pub const DEFAULT_PORT: u16 = 9500;
pub const DEFAULT_BIND: &str = "127.0.0.1";

/// Where the persona list came from, so the banner can say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonasSource {
    /// `LANYARD_PERSONAS` was set; the file must exist.
    Explicit(PathBuf),
    /// The conventional `~/.config/lanyard/users.yaml`, which may be absent.
    Default(PathBuf),
}

impl PersonasSource {
    pub fn path(&self) -> &PathBuf {
        match self {
            PersonasSource::Explicit(p) | PersonasSource::Default(p) => p,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub issuer: String,
    pub bind: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub personas: PersonasSource,
}

impl Config {
    /// Resolve from the real process environment.
    pub fn from_env() -> Result<Self, String> {
        Self::resolve(|key| std::env::var(key).ok())
    }

    /// Resolve from an arbitrary lookup, so this is testable without mutating
    /// process-global state.
    pub fn resolve(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let port = match get("LANYARD_PORT") {
            Some(raw) => raw
                .parse::<u16>()
                .map_err(|_| format!("LANYARD_PORT is not a port number: {raw}"))?,
            None => DEFAULT_PORT,
        };

        let bind = get("LANYARD_BIND").unwrap_or_else(|| DEFAULT_BIND.to_string());

        // The `/oidc` path segment is part of the issuer: OIDC Discovery defines
        // the document's path as issuer + /.well-known/openid-configuration, and
        // the document lives under /oidc.
        let issuer = match get("LANYARD_ISSUER") {
            Some(raw) => raw.trim_end_matches('/').to_string(),
            None => format!("http://127.0.0.1:{port}/oidc"),
        };

        let data_dir = match get("LANYARD_DATA_DIR") {
            Some(raw) => PathBuf::from(raw),
            None => base_dir(&get, "XDG_DATA_HOME", ".local/share")?.join("lanyard"),
        };

        let personas = match get("LANYARD_PERSONAS") {
            Some(raw) => PersonasSource::Explicit(PathBuf::from(raw)),
            None => PersonasSource::Default(
                base_dir(&get, "XDG_CONFIG_HOME", ".config")?
                    .join("lanyard")
                    .join("users.yaml"),
            ),
        };

        Ok(Config {
            issuer,
            bind,
            port,
            data_dir,
            personas,
        })
    }

    pub fn listen_addr(&self) -> String {
        format!("{}:{}", self.bind, self.port)
    }

    pub fn jwks_uri(&self) -> String {
        format!("{}/jwks", self.issuer)
    }

    pub fn token_endpoint(&self) -> String {
        format!("{}/token", self.issuer)
    }
}

/// XDG by hand rather than via `directories`, which would return
/// `~/Library/Application Support/…` on macOS and quietly contradict the spec's
/// `~/.config/lanyard/users.yaml`. Revisit at Phase 10.
fn base_dir(
    get: &impl Fn(&str) -> Option<String>,
    xdg_var: &str,
    home_relative: &str,
) -> Result<PathBuf, String> {
    if let Some(raw) = get(xdg_var).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(raw));
    }
    match get("HOME").filter(|v| !v.is_empty()) {
        Some(home) => Ok(PathBuf::from(home).join(home_relative)),
        None => Err(format!("neither {xdg_var} nor HOME is set")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| {
            owned
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn issuer_defaults_to_loopback_with_the_oidc_segment() {
        let c = Config::resolve(env(&[("HOME", "/home/dev")])).unwrap();
        assert_eq!(c.issuer, "http://127.0.0.1:9500/oidc");
        assert_eq!(c.jwks_uri(), "http://127.0.0.1:9500/oidc/jwks");
        assert_eq!(c.token_endpoint(), "http://127.0.0.1:9500/oidc/token");
    }

    #[test]
    fn default_issuer_follows_the_port() {
        let c = Config::resolve(env(&[("HOME", "/home/dev"), ("LANYARD_PORT", "8080")])).unwrap();
        assert_eq!(c.issuer, "http://127.0.0.1:8080/oidc");
        assert_eq!(c.port, 8080);
    }

    #[test]
    fn explicit_issuer_wins_over_the_port() {
        let c = Config::resolve(env(&[
            ("HOME", "/home/dev"),
            ("LANYARD_PORT", "8080"),
            ("LANYARD_ISSUER", "http://lanyard:9500/oidc"),
        ]))
        .unwrap();
        assert_eq!(c.issuer, "http://lanyard:9500/oidc");
    }

    #[test]
    fn a_trailing_slash_on_the_issuer_is_dropped() {
        let c = Config::resolve(env(&[
            ("HOME", "/home/dev"),
            ("LANYARD_ISSUER", "http://lanyard:9500/oidc/"),
        ]))
        .unwrap();
        assert_eq!(c.issuer, "http://lanyard:9500/oidc");
    }

    #[test]
    fn bind_defaults_to_loopback_and_does_not_move_the_issuer() {
        let c = Config::resolve(env(&[("HOME", "/home/dev")])).unwrap();
        assert_eq!(c.listen_addr(), "127.0.0.1:9500");

        let c =
            Config::resolve(env(&[("HOME", "/home/dev"), ("LANYARD_BIND", "0.0.0.0")])).unwrap();
        assert_eq!(c.listen_addr(), "0.0.0.0:9500");
        assert_eq!(c.issuer, "http://127.0.0.1:9500/oidc");
    }

    #[test]
    fn a_non_numeric_port_is_fatal_and_names_the_variable() {
        let err =
            Config::resolve(env(&[("HOME", "/home/dev"), ("LANYARD_PORT", "nine")])).unwrap_err();
        assert!(err.contains("LANYARD_PORT"), "{err}");
    }

    #[test]
    fn paths_come_from_xdg_then_home() {
        let c = Config::resolve(env(&[("HOME", "/home/dev")])).unwrap();
        assert_eq!(c.data_dir, PathBuf::from("/home/dev/.local/share/lanyard"));
        assert_eq!(
            c.personas.path(),
            &PathBuf::from("/home/dev/.config/lanyard/users.yaml")
        );

        let c = Config::resolve(env(&[
            ("HOME", "/home/dev"),
            ("XDG_DATA_HOME", "/xdg/data"),
            ("XDG_CONFIG_HOME", "/xdg/config"),
        ]))
        .unwrap();
        assert_eq!(c.data_dir, PathBuf::from("/xdg/data/lanyard"));
        assert_eq!(
            c.personas.path(),
            &PathBuf::from("/xdg/config/lanyard/users.yaml")
        );
    }

    #[test]
    fn explicit_paths_win_and_are_marked_explicit() {
        let c = Config::resolve(env(&[
            ("HOME", "/home/dev"),
            ("LANYARD_DATA_DIR", "/tmp/throwaway"),
            ("LANYARD_PERSONAS", "/tmp/users.yaml"),
        ]))
        .unwrap();
        assert_eq!(c.data_dir, PathBuf::from("/tmp/throwaway"));
        assert_eq!(
            c.personas,
            PersonasSource::Explicit(PathBuf::from("/tmp/users.yaml"))
        );
    }

    #[test]
    fn no_home_and_no_xdg_is_fatal() {
        let err = Config::resolve(env(&[])).unwrap_err();
        assert!(err.contains("HOME"), "{err}");
    }
}
