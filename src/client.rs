//! The CLI's half: a real `client_credentials` grant against a running lanyard.
//!
//! **The CLI does not sign.** It posts to `/oidc/token` like any SDK would, so
//! the token arrives the way a production token arrives — through an endpoint,
//! with an OAuth-shaped response around it (north star 4). Offline signing is
//! post-v1 precisely because a second signing path would let the two drift.

/// What the CLI sends as `client_id`, and therefore what lands in the token's
/// `client_id` claim.
pub const CLI_CLIENT_ID: &str = "lanyard-cli";

/// Where the CLI reaches lanyard — **an address, not the issuer.**
///
/// `LANYARD_ISSUER` is deliberately not consulted. In the Docker case CONCEPT §8
/// describes, the issuer is `http://lanyard:9500/oidc`: a name that resolves
/// inside the container network and nowhere else. A CLI that derived its target
/// from the issuer would be unreachable in exactly the setup the issuer setting
/// exists to support.
pub fn resolve_url(
    explicit: Option<&str>,
    get: impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    if let Some(url) = explicit.filter(|u| !u.is_empty()) {
        return Ok(trim(url));
    }
    if let Some(url) = get("LANYARD_URL").filter(|u| !u.is_empty()) {
        return Ok(trim(&url));
    }
    // Same variable the server reads, so `LANYARD_PORT=9600 lanyard serve` in
    // one shell and `lanyard token` in another with the same variable exported
    // agree without a second setting. A value the server would refuse to start
    // on is refused here too, rather than silently becoming 9500.
    let port = match get("LANYARD_PORT").filter(|p| !p.is_empty()) {
        Some(raw) => raw
            .parse::<u16>()
            .map_err(|_| format!("LANYARD_PORT is not a port number: {raw}"))?,
        None => crate::config::DEFAULT_PORT,
    };
    Ok(format!("http://127.0.0.1:{port}"))
}

/// A trailing slash would make the posted URL `…//oidc/token`, which most
/// servers tolerate and none should have to.
fn trim(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

/// What the caller wants minted. `persona` is not optional: the endpoint and the
/// seam both allow a persona-less token, but the CLI's job is the persona case,
/// and a required flag beats a silent `sub`-less token that fails validation
/// three layers away.
pub struct MintRequest<'a> {
    pub url: &'a str,
    pub persona: &'a str,
    /// What the grant names itself as, and therefore **which personas it can
    /// see**. Defaults to [`CLI_CLIENT_ID`]; `--client` is how a shell reaches
    /// a persona a project scoped to its own application.
    pub client_id: &'a str,
    pub audience: Option<&'a str>,
    pub scope: Option<&'a str>,
    /// The wire value of a [`crate::oidc::flaw::Flaw`], passed straight through.
    /// The CLI does not decide what a flawed token looks like; it asks for one.
    pub flaw: Option<&'a str>,
}

/// What went wrong, in the three shapes a user can act on. `Display` is what
/// they read on stderr, so each variant is written as a sentence rather than as
/// a type name.
///
/// Shared by `token`/`env` and by `logs`: both are the CLI reaching a running
/// lanyard over HTTP, and "nothing listening at …" is the same sentence and the
/// same fix whichever subcommand hit it.
#[derive(Debug)]
pub enum MintError {
    /// Nothing is listening. By far the most common failure, and the one where
    /// naming both the address tried and the fix saves the most time.
    Unreachable(String),
    /// Reachable, but the exchange failed — DNS, or an `https://` URL against a
    /// binary with no TLS stack linked (HTTPS is post-v1). Still names the URL.
    Transport { url: String, message: String },
    /// lanyard said no, and said why. The description is surfaced verbatim, so
    /// the unknown-persona id reaches the user.
    Rejected(String),
    /// Something answered, but not like lanyard. Usually the wrong `--url`.
    Unexpected {
        url: String,
        status: u16,
        body: String,
    },
}

impl std::fmt::Display for MintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MintError::Unreachable(url) => {
                write!(
                    f,
                    "nothing listening at {url} — is `lanyard serve` running?"
                )
            }
            MintError::Transport { url, message } => {
                write!(f, "cannot reach lanyard at {url}: {message}")
            }
            MintError::Rejected(description) => write!(f, "{description}"),
            MintError::Unexpected { url, status, body } => {
                write!(f, "{url} answered {status}, which is not a token: {body}")
            }
        }
    }
}

impl std::error::Error for MintError {}

/// Perform the grant. Note what this does *not* do: read the discovery document
/// to find the token endpoint. It is the same binary that serves it — a round
/// trip to learn its own route buys nothing and adds a failure mode.
pub async fn mint(request: &MintRequest<'_>) -> Result<String, MintError> {
    let endpoint = format!("{}/oidc/token", request.url);

    let mut form = vec![
        ("grant_type", "client_credentials"),
        ("client_id", request.client_id),
        ("persona", request.persona),
    ];
    if let Some(audience) = request.audience {
        form.push(("audience", audience));
    }
    if let Some(scope) = request.scope {
        form.push(("scope", scope));
    }
    // One more form field, which is the whole of `--bad-signature`: the server
    // breaks the token, the CLI asks it to.
    if let Some(flaw) = request.flaw {
        form.push(("flaw", flaw));
    }

    let response = reqwest::Client::new()
        .post(&endpoint)
        .form(&form)
        .send()
        .await
        .map_err(|e| transport_error(request.url, &e))?;

    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|e| transport_error(request.url, &e))?;

    if (200..300).contains(&status) {
        return match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(json) => match json["access_token"].as_str() {
                Some(token) => Ok(token.to_string()),
                None => Err(unexpected(request.url, status, &body)),
            },
            Err(_) => Err(unexpected(request.url, status, &body)),
        };
    }

    // The OAuth error shape both this endpoint and the seam already speak.
    match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(json) => match json["error_description"]
            .as_str()
            .or_else(|| json["error"].as_str())
        {
            Some(description) => Err(MintError::Rejected(description.to_string())),
            None => Err(unexpected(request.url, status, &body)),
        },
        Err(_) => Err(unexpected(request.url, status, &body)),
    }
}

/// `lanyard logs` — follow the running singleton's event stream and print it.
///
/// **Bare reads the SSE stream and renders it with [`crate::events::pretty`]**,
/// the same function `lanyard serve` prints with, so a second terminal shows
/// what the first is showing. `--json` reads `?format=ndjson` and passes each
/// line through untouched, because the thing on the other end of that pipe is
/// `jq` and re-encoding could only lose.
///
/// No lanyard running is an **error**, not an empty stream: a tool that exits
/// `0` having printed nothing is a tool that lies about having looked.
pub async fn logs(url: &str, json: bool) -> Result<(), MintError> {
    let endpoint = match json {
        true => format!("{url}/_/api/events?format=ndjson"),
        false => format!("{url}/_/api/events"),
    };

    let mut response = reqwest::Client::new()
        .get(&endpoint)
        .send()
        .await
        .map_err(|e| transport_error(url, &e))?;

    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let body = response.text().await.unwrap_or_default();
        return Err(unexpected(url, status, &body));
    }

    // Framed by hand rather than through a line-oriented codec: a chunk is not
    // a line, and a frame split across two reads has to survive the join.
    let mut buffered = String::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| transport_error(url, &e))?
    {
        buffered.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(end) = buffered.find('\n') {
            let line: String = buffered.drain(..=end).collect();
            print_line(line.trim_end_matches(['\r', '\n']), json);
        }
    }

    // The stream only ends when the other end goes away. Reporting that is the
    // same stance as refusing to start: a `logs` that returns quietly looks
    // exactly like one that is still following.
    Err(MintError::Transport {
        url: url.to_string(),
        message: "the event stream ended — lanyard stopped".to_string(),
    })
}

/// One line off the wire, rendered for whichever surface asked.
fn print_line(line: &str, json: bool) {
    if json {
        // Already one JSON object per line, markers included: `jq` gets to
        // decide what a `{"dropped":N}` means to it.
        if !line.is_empty() {
            println!("{line}");
        }
        return;
    }

    // SSE: `id:` and blank lines carry no payload, and the payload is on
    // `data:`.
    let Some(payload) = line.strip_prefix("data: ") else {
        return;
    };
    match serde_json::from_str::<crate::events::Event>(payload) {
        Ok(event) => println!("{}", crate::events::pretty(&event)),
        // Not an event: the clear directive, or a `dropped` marker on its own
        // event type. A hole must be visible here too — that is the whole point
        // of the marker.
        Err(_) => {
            if let Some(dropped) = serde_json::from_str::<serde_json::Value>(payload)
                .ok()
                .and_then(|v| v["dropped"].as_u64())
            {
                println!("-- dropped {dropped} events; this reader could not keep up --");
            }
        }
    }
}

fn transport_error(url: &str, error: &reqwest::Error) -> MintError {
    if error.is_connect() {
        MintError::Unreachable(url.to_string())
    } else {
        MintError::Transport {
            url: url.to_string(),
            message: error.to_string(),
        }
    }
}

/// The body goes into the message because "not a token" is unhelpful on its own,
/// but a whole HTML error page on stderr is worse than none of it.
fn unexpected(url: &str, status: u16, body: &str) -> MintError {
    let mut body: String = body.chars().take(200).collect();
    body = body.split_whitespace().collect::<Vec<_>>().join(" ");
    MintError::Unexpected {
        url: url.to_string(),
        status,
        body,
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
    fn the_default_target_is_the_loopback_default_port() {
        assert_eq!(
            resolve_url(None, env(&[])).unwrap(),
            "http://127.0.0.1:9500"
        );
    }

    #[test]
    fn lanyard_url_wins_over_the_default_and_the_flag_wins_over_both() {
        assert_eq!(
            resolve_url(None, env(&[("LANYARD_URL", "http://127.0.0.1:9600")])).unwrap(),
            "http://127.0.0.1:9600"
        );
        assert_eq!(
            resolve_url(
                Some("http://elsewhere:1234"),
                env(&[("LANYARD_URL", "http://127.0.0.1:9600")])
            )
            .unwrap(),
            "http://elsewhere:1234"
        );
    }

    #[test]
    fn the_port_variable_moves_the_default_target() {
        assert_eq!(
            resolve_url(None, env(&[("LANYARD_PORT", "9600")])).unwrap(),
            "http://127.0.0.1:9600"
        );
    }

    #[test]
    fn a_non_numeric_port_is_an_error_naming_the_variable() {
        let err = resolve_url(None, env(&[("LANYARD_PORT", "nine")])).unwrap_err();
        assert!(err.contains("LANYARD_PORT"), "{err}");
    }

    #[test]
    fn a_trailing_slash_is_trimmed() {
        assert_eq!(
            resolve_url(Some("http://127.0.0.1:9500/"), env(&[])).unwrap(),
            "http://127.0.0.1:9500"
        );
        assert_eq!(
            resolve_url(None, env(&[("LANYARD_URL", "http://127.0.0.1:9600/")])).unwrap(),
            "http://127.0.0.1:9600"
        );
    }

    /// CONCEPT §8's Docker issuer resolves nowhere useful from a shell, so the
    /// CLI must not derive its target from it.
    #[test]
    fn the_issuer_is_not_the_target() {
        assert_eq!(
            resolve_url(None, env(&[("LANYARD_ISSUER", "http://lanyard:9500/oidc")])).unwrap(),
            "http://127.0.0.1:9500"
        );
    }

    /// An exported-but-empty variable is the shell's way of saying nothing.
    #[test]
    fn an_empty_setting_reads_as_absent() {
        assert_eq!(
            resolve_url(Some(""), env(&[("LANYARD_URL", "")])).unwrap(),
            "http://127.0.0.1:9500"
        );
    }
}
