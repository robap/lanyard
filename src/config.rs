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

/// Where the links registry lives.
///
/// **No explicit/default split, unlike [`PersonasSource`]**: an absent registry
/// is the zero-config state whichever way its path was chosen. Pointing
/// `LANYARD_PERSONAS` at a file that is not there leaves you with an empty
/// picker, which is why that case is fatal; having no links is the normal
/// condition of a fresh machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinksSource(pub PathBuf);

impl LinksSource {
    pub fn path(&self) -> &PathBuf {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub issuer: String,
    pub bind: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub personas: PersonasSource,
    pub links: LinksSource,
    /// `LANYARD_CLOCK_SKEW`, in signed seconds. `0` for every ordinary run —
    /// and the reason [`crate::clock::Clock`] is a value on `AppState` rather
    /// than a global.
    pub skew: i64,
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

        // The seventh deliberate flaw. Fatal on garbage rather than ignored,
        // because a variable that silently did nothing would be indisting-
        // uishable from a clock that agrees — which is the one lie this whole
        // phase exists to prevent.
        let skew = match get(crate::clock::SKEW_VAR) {
            Some(raw) => crate::clock::parse_skew(&raw)?,
            None => 0,
        };

        // Beside `users.yaml`, which is the other file that answers "where do
        // personas come from".
        let links = match get("LANYARD_LINKS") {
            Some(raw) => LinksSource(PathBuf::from(raw)),
            None => LinksSource(
                base_dir(&get, "XDG_CONFIG_HOME", ".config")?
                    .join("lanyard")
                    .join("links.yaml"),
            ),
        };

        Ok(Config {
            issuer,
            bind,
            port,
            data_dir,
            personas,
            links,
            skew,
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

    pub fn authorization_endpoint(&self) -> String {
        format!("{}/authorize", self.issuer)
    }

    pub fn userinfo_endpoint(&self) -> String {
        format!("{}/userinfo", self.issuer)
    }

    /// **.NET builds its logout redirect by reading this URL out of the
    /// discovery document**, and silently builds no redirect when it is absent.
    /// The endpoint and its advertisement ship together or the one-liner in the
    /// spike is quietly a no-op.
    pub fn end_session_endpoint(&self) -> String {
        format!("{}/end_session", self.issuer)
    }

    pub fn introspection_endpoint(&self) -> String {
        format!("{}/introspect", self.issuer)
    }

    pub fn revocation_endpoint(&self) -> String {
        format!("{}/revoke", self.issuer)
    }

    /// The persona picker, for the banner to print. Derived from the issuer
    /// rather than from the bind address, because the issuer is the one address
    /// everything else in this file is derived from — and because a banner that
    /// printed `0.0.0.0:9500` would print a URL that does not open.
    pub fn ui_url(&self) -> String {
        format!("{}/_/", self.issuer.trim_end_matches("/oidc"))
    }

    /// The live request log. Derived from the issuer for `ui_url`'s reason, and
    /// printed by the banner — which, since Phase 1, must never print a URL
    /// that 404s.
    pub fn log_url(&self) -> String {
        format!("{}/_/log", self.issuer.trim_end_matches("/oidc"))
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
        assert_eq!(
            c.authorization_endpoint(),
            "http://127.0.0.1:9500/oidc/authorize"
        );
        assert_eq!(c.userinfo_endpoint(), "http://127.0.0.1:9500/oidc/userinfo");
        assert_eq!(
            c.end_session_endpoint(),
            "http://127.0.0.1:9500/oidc/end_session"
        );
        assert_eq!(
            c.introspection_endpoint(),
            "http://127.0.0.1:9500/oidc/introspect"
        );
        assert_eq!(c.revocation_endpoint(), "http://127.0.0.1:9500/oidc/revoke");
        // The banner prints this and criterion 25 fetches it, so the `/oidc`
        // segment has to come back off.
        assert_eq!(c.ui_url(), "http://127.0.0.1:9500/_/");
        assert_eq!(c.log_url(), "http://127.0.0.1:9500/_/log");
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
        assert_eq!(
            c.ui_url(),
            "http://127.0.0.1:9500/_/",
            "the banner must print a URL that opens, not the wildcard bind"
        );
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
        assert_eq!(
            c.links.path(),
            &PathBuf::from("/home/dev/.config/lanyard/links.yaml"),
            "the links registry sits beside users.yaml, not in the data directory"
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
        assert_eq!(
            c.links.path(),
            &PathBuf::from("/xdg/config/lanyard/links.yaml")
        );
    }

    #[test]
    fn explicit_paths_win_and_are_marked_explicit() {
        let c = Config::resolve(env(&[
            ("HOME", "/home/dev"),
            ("LANYARD_DATA_DIR", "/tmp/throwaway"),
            ("LANYARD_PERSONAS", "/tmp/users.yaml"),
            ("LANYARD_LINKS", "/tmp/links.yaml"),
        ]))
        .unwrap();
        assert_eq!(c.data_dir, PathBuf::from("/tmp/throwaway"));
        assert_eq!(
            c.personas,
            PersonasSource::Explicit(PathBuf::from("/tmp/users.yaml"))
        );
        assert_eq!(c.links, LinksSource(PathBuf::from("/tmp/links.yaml")));
    }

    /// The seventh deliberate flaw, and the only knob that makes lanyard lie
    /// about the time — so it is parsed as strictly as the port is.
    #[test]
    fn the_clock_skew_is_off_by_default_and_parses_when_set() {
        let c = Config::resolve(env(&[("HOME", "/home/dev")])).unwrap();
        assert_eq!(c.skew, 0, "an ordinary run has no skew");

        for (raw, seconds) in [("-5m", -300), ("+90s", 90), ("300", 300)] {
            let c = Config::resolve(env(&[("HOME", "/home/dev"), ("LANYARD_CLOCK_SKEW", raw)]))
                .unwrap();
            assert_eq!(c.skew, seconds, "{raw}");
        }
    }

    #[test]
    fn a_garbage_clock_skew_is_fatal_and_names_the_variable() {
        let err = Config::resolve(env(&[
            ("HOME", "/home/dev"),
            ("LANYARD_CLOCK_SKEW", "soon"),
        ]))
        .unwrap_err();
        assert!(err.contains("LANYARD_CLOCK_SKEW"), "{err}");
    }

    #[test]
    fn no_home_and_no_xdg_is_fatal() {
        let err = Config::resolve(env(&[])).unwrap_err();
        assert!(err.contains("HOME"), "{err}");
    }
}
