# Changelog

The version headings are what `dist` lifts into a GitHub release body, so this
file is the release notes — write it for somebody arriving at the release page,
not for somebody reading the commit log.

Versions follow [semantic versioning](https://semver.org/). Until `1.0.0` the
endpoints are settled but the CLI's output shapes are not.

## 0.1.0

**The first release.** Everything below has existed since the phase that built
it; what is new here is that it is a file you can download.

### The provider

- `lanyard serve` starts an OpenID Connect provider on `127.0.0.1:9500` with a
  discovery document, a JWKS, and a signing key that is the same on every
  machine — no realm to create, no client to register, no admin console.
- **No client registry.** Any `client_id` works, invented at the point of use.
  `redirect_uri` must be loopback, which is the one security boundary.
- Authorization code with PKCE, `client_credentials`, refresh tokens,
  `/oidc/userinfo`, `/oidc/introspect`, `/oidc/revoke`, `/oidc/end_session`,
  and `prompt=none` for the SPA silent-renew iframe.
- Three built-in people — `ada`, `mira`, `nobody` — and a persona picker at
  `/_/` with no password field. The choice is remembered per `client_id`, so
  three applications can be signed in as three different people at once.

### The CLI

- `lanyard token --as ada --aud billing-api` prints a bearer token; `lanyard env`
  prints it as an `export` for `eval`.
- **Deliberate failure tokens** — expired, wrong issuer, wrong audience, bad
  signature — so the unhappy path is something you can ask for rather than wait
  for.
- `lanyard logs` follows a running lanyard's request log, and `/_/log` is the
  same thing in a browser, naming what went wrong in the provider's own words.
- `lanyard link` records a project's `lanyard.yml`, so a repository's personas
  travel with it and load for that project's `client_id` only.
- `lanyard doctor` checks the machine and says what is wrong with it — the
  issuer's host, the port, the container name gotchas, the key file's mode.

### Getting it

- macOS and Linux, arm64 and x86_64. **The Linux builds are statically linked
  musl**, so they run on distributions older than the machine that built them.
- `brew install robap/tap/lanyard`, a `curl … | sh` installer, or
  `docker run -p 9500:9500 ghcr.io/robap/lanyard` — a multi-arch image of about
  10 MB with no shell in it.
- Windows is reached through WSL2 or that image; there is no native Windows
  binary yet. The README's `## Install` section has the line for each.

### Read this first

The default signing key is public and committed to this repository on purpose.
Anyone with a checkout can mint a token your application will accept, and
`/oidc/token` needs no authentication at all. That is the point, and it is why
the default bind is loopback. Never point a staging or production service at it.
