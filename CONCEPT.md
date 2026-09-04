# lanyard — concept

A local OIDC/OAuth2 provider for development. Single static binary, nothing to configure, one instance serving every project on the machine.

Status: pre-implementation. This document captures the design intent and the decisions already made, so that the ones still open are visible.

Working name. `lanyard` is taken on crates.io by an unrelated FFI string library, which does not block us — see [Naming and distribution](#naming-and-distribution).

---

## 1. The problem

Local development against an identity provider is disproportionately painful. The two available options are both bad:

**Run a real IdP locally.** Keycloak, FusionAuth, Zitadel, Authentik. Each needs realms, applications, client IDs, secrets, redirect URI allowlists, and user records configured before a single login works. That is an afternoon per project, repeated for every new checkout and every teammate, and it lives in a Docker container that must stay running.

**Stub it out.** Add `if (isDev)` branches that skip auth entirely. Fast, and it means the auth path — the one with the security consequences — is the least-tested code in the application.

The configuration burden exists for a good reason in production: a real IdP must be closed by default. In development that same property is pure cost. **lanyard's central bet is that a development IdP should be open by default.**

## 2. The pitch

> **A local OIDC provider with nothing to configure.** Any client id, any localhost redirect, any audience — accepted, no registration. Pick a persona in the browser, or mint a bearer token from the command line.

Supporting paragraph:

> Personas live in a YAML file you commit; the server is a single static binary you can delete and forget. It also mints tokens that are deliberately wrong — expired, wrong audience, bad signature — so the half of your auth code that rejects things gets tested too.

Framing rules:

- **Do not say "mock" or "fake."** The tokens are real RS256 tokens served from a real JWKS endpoint. "Mock" invites the reader to assume it is shallow. Cubby says "S3-compatible object store," not "S3 mock," for the same reason.
- **Do not name Keycloak or FusionAuth.** "Nothing to configure" lands with anyone who has spent an afternoon in an admin console, and it ages better than a direct comparison.
- **Do not lead with "accepts any client id."** Several competing tools already lead with exactly that. Keep it as a feature, not the headline.

## 3. Core design decisions

### Accept everything

No client registration. No redirect URI allowlist. No audience registration. Any `client_id`, any `client_secret`, any `redirect_uri` that resolves to localhost or 127.0.0.1.

Localhost-only redirects is the security boundary that makes this defensible rather than reckless, and it is one line to explain in a README.

This is what produces the multi-project property: three services on three ports use one running instance with zero setup between them.

### Personas, not passwords

The login screen is a list of people you click. No password field. Plus a "mint one now" panel for a custom user with arbitrary claims.

A **"no roles / empty claims"** persona ships by default. That is the user that breaks applications and the one nobody remembers to create.

### Issuance is one function

The browser flow, the token endpoint, the CLI, and the test seam are all thin callers of a single function that takes a persona plus overrides and returns signed tokens.

Getting this wrong means claim-building logic in three places that drift apart. This is the most important internal structure decision in the project.

### Keep the persona model protocol-neutral

A persona has: a stable id, an email, a display name, roles, and a bag of arbitrary attributes. It does **not** have `sub`, `preferred_username`, or a `claims` key. The mapping from persona to OIDC claims happens at issue time, inside the OIDC module.

This is the only insurance needed against a possible SAML or LDAP future. No plugin traits, no `Protocol` abstraction, no registry — those cost real time now to serve a hypothetical later, and are usually designed wrong without a second implementation to check against.

Same discipline for routing: mount OIDC under `/oidc/...` from day one, keep `/_/` for the UI and its JSON seam, leave the root free. Costs nothing, avoids a breaking URL change later.

## 4. Scope

### In

**Endpoints:** discovery (`.well-known/openid-configuration`), JWKS, `/authorize`, `/token`, `/userinfo`, `/end_session`, `/introspect`, `/revoke`.

`/end_session` matters — RP-initiated logout is a common trip-up and is missing from many mocks. `/revoke` is a dozen lines and some SDKs probe for it.

**Grants:** authorization code with PKCE, authorization code with client secret, refresh token, client credentials. Device code if CLI apps come up.

**Not:** implicit, resource owner password credentials. Unless something we own needs them.

### Out

SAML 2.0, LDAP, WS-Federation, CAS.

SAML in particular is not a variant of what we are building — it is a separate stack with XML assertions, XML digital signatures, canonicalization, and metadata exchange. It is the single biggest thing we are choosing not to do, and it is where an outside contributor could add the most value later. The protocol-neutral persona model above is what keeps that door open.

## 5. The two halves

### Browser flow

For developing UIs. Redirect in, pick a persona, redirect out.

### CLI token minting

For everything else. Probably the larger half of daily use, and the shorter path to being useful since there is no redirect dance to get right.

```
$ lanyard token --as ada --aud billing-api --scope 'orders:read orders:write'
eyJhbGciOiJSUzI1NiIsImtpZCI6...

$ curl -H "Authorization: Bearer $(lanyard token --as ada --aud billing-api)" \
       localhost:8080/orders
```

Supporting commands:

```
$ eval "$(lanyard env --as ada --aud billing-api)"     # exports BEARER_TOKEN
$ lanyard proxy --port 8081 --target localhost:8080 --as ada --aud billing-api
```

The proxy attaches a fresh bearer to every request passing through it. That is what lets us keep aggressively short token lifetimes without making interactive exploration irritating.

Underneath, the CLI uses the real `client_credentials` grant against `/oidc/token`, so the token arrives the way a production token arrives. If the keypair lives on disk, the CLI can also sign directly with no server running.

**Audiences follow the same no-registration rule as redirect URIs.** Any `aud` string is accepted. An `--aud` that does not match what the API expects should still mint successfully — that is a test case, not an error.

## 6. Features

### Deliberate failure tokens

First-class flags, not a config edit:

```
$ lanyard token --as ada --aud billing-api --expired
$ lanyard token --as ada --aud billing-api --wrong-aud
$ lanyard token --as ada --aud billing-api --wrong-iss
$ lanyard token --as ada --aud billing-api --bad-signature
$ lanyard token --as ada --aud billing-api --alg-none
$ lanyard token --as ada --aud billing-api --unknown-kid
```

Each mints a token that is structurally valid and semantically wrong. Testing that an API *accepts* a good token is the easy half and everyone does it; testing that it correctly *rejects* the bad ones is the half that never gets written, because producing those tokens is annoying enough that people skip it.

Six lines is a complete negative-path suite for any resource server. **This block belongs in the README above the login screenshot.**

Some competing tools can be coaxed into producing a bad token by editing config. None advertise it as the point.

### Claim profiles

The same persona emitted shaped like Auth0, Cognito, Okta, or Entra ID. Roles land in `cognito:groups` vs a namespaced `https://app/roles` vs plain `roles`. The claim-mapping code written locally is then the code that runs in production.

**Entra first**, since it is the daily target. Its distinctive shapes: `tid` for tenant, `oid` as the stable user object id (which is *not* `sub`), `roles` for app roles, `scp` as a space-delimited string rather than an array, `preferred_username` rather than `email`. Applications written against Entra read `oid` and `tid` specifically; a persona that only emits `sub` will send them down an unexpected branch. The `v1.0` vs `v2.0` endpoint distinction matters if any project is still on the older one.

### Opaque access token mode

Half of all application code validates the wrong token — ID token where it means access token, or vice versa. Auth0 famously returns an opaque access token unless the client passes an `audience` parameter, so an app built against a JWT-always provider breaks on migration. Being able to return an opaque token is a good bug-finder.

### Short lifetimes by default

60-second access tokens out of the box, plus an "expire this session now" control in the UI.

Long-lived dev tokens mean the refresh path never runs locally and ships untested. Resist the inevitable request for `--ttl 30d` for Postman collections; if it goes in, make it loud in the log line.

### Silent renew (`prompt=none`)

SPAs do this in a hidden iframe. It is a common source of production breakage and nearly impossible to test against a mock that does not implement it. Return `login_required` correctly when there is no session. In v1.

Note this is also the strongest argument for eventual HTTPS support — see [section 9](#9-https-later-but-design-for-it).

### Test seam

```
POST /_/api/token
```

Post the claims you want, get a signed token back, no browser involved. This is what makes lanyard a test fixture rather than a login screen — the same role `--seed` plays for cubby.

**Build it in week one, not as a later addition**, because it determines the structure: the browser flow becomes a thin caller of the same function the test seam calls.

### Live request log

Every `/authorize` and `/token` with decoded parameters and resulting claims. "Why did my login fail" answered in the tool's own output.

Because lanyard is a machine-wide singleton, this becomes a view across every project being worked on at once — more useful than a per-project log, not less.

### `lanyard doctor`

Checks the resolved issuer, tries to reach itself at that address, compares internal and external ports, reports clock skew against the host, confirms the signing key is persisted, and (later) verifies a real TLS handshake.

Roughly ten minutes of work per check, and it collapses the entire Docker gotcha category into one command whose output can be pasted into an issue. Nothing in the competitive set has one.

### Smaller items

- CORS on `/token` and `/jwks` — public SPA clients call these from the browser. Accept any localhost origin.
- Sessions keyed per `client_id`, so the second project on the machine does not silently inherit the first project's persona. Plus an "always ask" toggle.
- Emit `nonce`, `at_hash`, and `c_hash` correctly. Emitting them wrong belongs in the failure-token list.

## 7. Running and distribution

### Native binary is the recommended path

This inverts cubby's emphasis, deliberately. Cubby is per-project: its data dir sits beside the code, it is gitignored, it dies with the Compose stack. Correct for cubby.

lanyard is machine-wide by design — that was the point of accepting any client id. A container that must always be running means Docker Desktop must always be running, which is a real cost people notice on a laptop. A 10 MB static binary in a launchd plist costs nothing.

Docker stays first-class for CI and for teams whose whole dev environment is already Compose.

### Service management built in

```
$ lanyard service install     # writes the launchd/systemd unit, enables it
$ lanyard service status
$ lanyard service logs
$ lanyard service uninstall
```

A launchd agent in `~/Library/LaunchAgents` on macOS, a systemd **user** unit on Linux (user, not system — no sudo, inherits the session), a scheduled task on Windows. Perhaps 200 lines, and it removes the biggest reason people abandon always-on tools.

`lanyard serve` stays a plain foreground process; the service unit is a thin wrapper around it. **No forking, no pidfiles, no separate daemon mode.** Foreground-first means it debugs like any other program and behaves identically in CI.

Socket activation (launchd `Sockets`, systemd socket units) is the elegant version — the OS holds the port, first connection starts the process. Nice later; ship boring always-on first.

### One fixed port, forever

A machine-wide service that sometimes lands on 9500 and sometimes 9501 defeats its own purpose, because the issuer string is baked into every project's config. Fail loudly on conflict rather than falling back, and have `doctor` report what is squatting on it.

### Channels

In rough priority order:

1. **GitHub release binaries** — macOS arm64/x86_64, Linux arm64/x86_64, Windows. The floor.
2. **Homebrew tap** — `brew install robap/tap/lanyard`, which also gives `brew services start lanyard` for free. Personal taps have no naming conflicts; homebrew-core has notability requirements we will not meet at launch.
3. **Container image** on GHCR for Compose users and CI.
4. **curl installer** for the README one-liner.

`cargo-dist` produces most of 1, 2, and 4 from a GitHub Actions workflow and will save a weekend of release plumbing.

**crates.io is not needed.** `cargo install` reaches Rust developers, who are a small slice of the audience for a local OIDC provider — our own stack is PHP and .NET. Nobody installs an S3 emulator via cargo either.

## 8. Docker gotchas

Worse for an IdP than for an object store, because identity has more things that must string-match exactly. These need to be written down in the README rather than papered over — that is the one thing navikt's README does better than most.

### The issuer is one string and there are two addresses

The browser reaches lanyard at `localhost:9500`. An API container reaches it at `lanyard:9500` on the Compose network. But `iss` is a single claim, discovery advertises a single `issuer`, and every conforming RP does an exact string comparison.

**Do not derive the issuer per-request from the `Host` header.** It works beautifully until a single flow crosses the boundary — which is every browser login flow — and then it is a confusing failure instead of an obvious one.

Instead, make one name resolve identically from both sides:

- `host.docker.internal`, which is Docker Desktop behavior; native Linux needs `--add-host=host.docker.internal:host-gateway` explicitly.
- Or pick `lanyard`, add `127.0.0.1 lanyard` to the host's `/etc/hosts`, and use `http://lanyard:9500` everywhere. One line of setup, then everything matches.

Whichever, **the tool should say so**: print the resolved issuer at startup, and when a request arrives with a `Host` that does not match the configured issuer, log explicitly that the token just minted will be rejected by an RP configured for that issuer. That converts a 45-minute debugging session into a glance.

### Port must match on both sides of the colon

Because the issuer string contains the port. `-p 9500:9500` is fine; `-p 9500:8080` produces an issuer correct for exactly one of the two audiences. Default them equal and warn on mismatch.

### `localhost` inside a container is the container

The most common mistake, and it produces a connection-refused that reads as "the IdP is down." One README line next to the Compose snippet.

### `http://localhost` is a secure context; `http://lanyard` is not

Browsers treat `localhost` as trustworthy for cookies, `SameSite=None`, crypto APIs, and service workers. A hostname on a plain HTTP origin gets none of that. A session cookie that works when hitting lanyard directly can silently fail on a Compose hostname, and the symptom is "it asks me to pick a persona every single time."

Use `SameSite=Lax`, never set `Secure` in HTTP mode, and document it.

### Clock drift

Docker Desktop's VM clock can lag after the laptop sleeps. With production-length tokens nobody notices; with 60-second TTLs, every token arrives already expired or not yet valid. Add a few seconds of `nbf` leeway, and make "rejected as expired, but the clock looks skewed" a specific diagnosis rather than a generic 401.

### Ephemeral signing keys

If the keypair is generated at startup and lives only in the container, every `docker compose up` rotates it. Saved Postman tokens stop working and any RP with cached JWKS fails signature validation for no visible reason.

Do both: **persist the keypair** in the mounted data dir (like cubby's `meta.sqlite`), and **derive a deterministic default dev key** so a fresh container with no volume still produces the same `kid` and public key every time. Stable-by-default is the right bias for a dev tool.

### Ordinary hygiene

amd64 and arm64 images so Apple Silicon does not crawl under QEMU. Distroless static. A `LANYARD_BIND` env var so the container binds `0.0.0.0` while the native binary stays on loopback. Rootless Podman file ownership. A health endpoint so `depends_on: condition: service_healthy` works and apps do not race the IdP at startup.

## 9. HTTPS: later, but design for it

**HTTP is the default and stays the default.** Zero-setup startup is what makes lanyard worth using; making TLS mandatory reintroduces the setup cost the tool exists to eliminate.

But two real reasons TLS will be needed eventually, neither cosmetic:

1. Some OIDC libraries hard-refuse a non-HTTPS issuer. (.NET is the one in our stack most likely to — see [section 10](#10-our-stack-specifically).)
2. **Silent renew.** `prompt=none` in a hidden iframe makes the session cookie third-party. Third-party requires `SameSite=None`, which requires `Secure`, which requires HTTPS. Over plain HTTP, silent renew either does not work or works only through browser leniency that will not hold in production. That is the class of bug that ships.

The port is irrelevant to TLS — `https://localhost:9500` is completely ordinary. 443 only matters as the default when the port is omitted.

### Design: a local CA, not a self-signed leaf

`dotnet dev-certs https --trust` is the right model to copy for ergonomics, but use a CA rather than a single trusted leaf. Generate the CA once, install it into trust stores once, then mint leaf certs freely without ever touching the trust store again. mkcert and Caddy both work this way. It also means lanyard can issue for whatever hostname was chosen to solve the Docker problem, not just `localhost`.

```
$ lanyard trust
  Generating local CA at ~/.config/lanyard/ca/
  Installing into system keychain… (sudo required once)
  Installing into NSS store (Firefox, Chromium)…
  Issuing leaf for localhost, 127.0.0.1, ::1, lanyard
  Trusted. Issuer is now https://localhost:9500
```

`rcgen` handles generation. Put every name in the leaf's SANs up front so regeneration is never needed.

### The trust store work is the actual project

- macOS: `security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain`.
- Linux: CA into `/usr/local/share/ca-certificates/` then `update-ca-certificates`, or the `/etc/pki/ca-trust/` equivalent.
- Windows: root store via certutil.
- **Firefox and Chromium on Linux do not use the OS store.** They use NSS databases under `~/.pki/nssdb` and each Firefox profile directory, requiring `certutil` from nss-tools. This is the part everyone forgets — read mkcert's source before writing our own.

### Runtimes do not inherit OS trust

Trusting the CA at the OS level does not mean the application trusts it:

| Runtime | What it needs |
|---|---|
| Node | `NODE_EXTRA_CA_CERTS=/path/to/ca.pem` (bundles its own CA list) |
| Python `requests` | `REQUESTS_CA_BUNDLE` or `SSL_CERT_FILE` (uses certifi) |
| Java | `keytool -importcert` into its own `cacerts` |
| Go | mostly works — uses the system store |
| Containers | own filesystem, own CA bundle — must be mounted in |

`lanyard trust` should print the exact env var line for each detected runtime, and `doctor` should verify a real TLS handshake. "The cert is installed" and "your Node app trusts it" are different facts; only the second matters.

### Two cautions

**Serve both once a CA exists** — HTTP on 9500, HTTPS on 9501, always. Switching becomes one character in a config rather than a restart with a flag, and two projects can disagree about which they want.

**The CA private key is a genuine MITM capability for the machine.** Mode 0600, config dir only, never in a project directory, never in a container image. Say so plainly in the README; mkcert's docs are refreshingly blunt about this and ours should be too.

**Detect an existing mkcert CA.** Many developers already have one installed and trusted. Issuing from it is a zero-prompt path to HTTPS and avoids adding a second root for no reason.

## 10. Our stack specifically

### .NET

Most likely to complain about HTTP. The OIDC handler's `RequireHttpsMetadata` defaults to `true`, so discovery over `http://` fails outright:

```csharp
options.RequireHttpsMetadata = builder.Environment.IsDevelopment() ? false : true;
```

Write the ternary, not a bare `false` — it is easy to forget it is environment-gated and ship it.

Two more .NET specifics:

- It validates that the token's `iss` exactly matches the discovery document's `issuer`. Everything in [section 8](#8-docker-gotchas) lands here first, with an error message that is not forthcoming about which string mismatched.
- It is strict about correlation and nonce cookies, and sets them `SameSite=None` in some configurations — which needs `Secure`, which needs HTTPS. **"Correlation failed" errors locally are usually this**, and it is the concrete case where `lanyard trust` becomes necessary sooner than expected.

### PHP

Probably `league/oauth2-client` (generic OAuth2) or `jumbojett/openid-connect-php` (full OIDC). Both are more relaxed about `http://` than .NET. jumbojett has `setHttpUpgradeInsecureRequests(false)` for exactly this. Neither should block us.

### Cheap validation before writing code

Point an existing project at `oidc-provider-mock` (via pipx) or the Soluto `oidc-server-mock` container — both run over HTTP. Twenty minutes tells us whether .NET needs anything beyond `RequireHttpsMetadata = false`, and whether PHP has any opinion at all. That is real signal about where the HTTPS work sits on the roadmap, at almost no cost.

## 11. Examples directory

```
examples/
  curl/              # no framework at all — discovery, client credentials, decode, call
  dotnet-web/        # OIDC login, cookie session
  dotnet-api/        # JWT bearer validation
  php-web/
  node-api/
  node-spa/          # public client, PKCE, silent renew
  python-api/
  go-api/
```

A directory listing showing five languages side by side is itself the message: nobody arriving at the repo wonders whether their stack is supported. This is a stronger fix for language-leakage than adding snippets to the README.

Principles:

- **Split by role, not just language.** "Log in with a browser" and "validate a bearer token" are different integrations with different failure modes, and most people only need one.
- **Runnable, not illustrative.** Clone, two commands, watch a login work — against a fresh lanyard with no setup. A snippet in a folder is just a README snippet with extra steps.
- **`curl/` comes first**, alphabetically and conceptually. Zero dependencies, works everywhere, proves the tool without committing anyone to a framework.
- **Each demonstrates the failure tokens**, not just the happy path. A `test.sh` doing good → 200, expired → 401, wrong audience → 401, no token → 401. Four lines, shows off the differentiating feature in context.
- **Run them in CI as a matrix**, one job per example. That turns `examples/` from documentation that rots into an executable compatibility promise — the same pattern as cubby's conformance matrix. It also gives outside contributors a good shape: "doesn't work with Spring Boot" is answered by a new directory and a new CI job, not a patch to the core.
- **Keep dependencies minimal.** A .NET example pinned to a specific SDK, or a Node one with a 400-line lockfile, will break in eight months and read as abandonment. Renovate or Dependabot on this directory.
- **Do not seed them from our own projects.** Examples should be deliberately boring and read like a tutorial, not like production code carrying our conventions.

**Start with two:** `curl/` and `dotnet-web/`. .NET because it is the one most likely to need `RequireHttpsMetadata = false` and we will be debugging it anyway. Two solid entries beat six half-working ones, and empty slots are a good "help wanted" signal.

## 12. Competitive position

The space is more crowded than it first appears.

**Real IdPs run locally:** Keycloak, FusionAuth, Zitadel, Authentik, Logto, Ory Hydra, Dex.

**Purpose-built dev/test servers:** navikt/mock-oauth2-server (the most mature — Kotlin, scriptable, multi-issuer, ships a browser debugger), Soluto/oidc-server-mock (.NET container, env-var config), oidc-provider-mock (Python, pipx or container, accepts any client by default), bluecatengineering/mock-oidc-provider (Node, npx), stackables/oauth-tester (deliberately permissive, also hosted), appvia/mock-oidc-user-server and its many forks, oauth-mocks (stateless GitHub/Google mocks).

**Libraries:** node-oidc-provider (the certified JS implementation most Node mocks are built on), WireMock.

**Adjacent:** jwt.io, oauth.tools, oauth2c.

Three claims survive that landscape:

1. **Single static binary.** Everything above is a container, a JVM artifact, an npm package, or a Python install. "Runs from a container" is table stakes; "runs from nothing" is not.
2. **CLI token minting usable from any language.** navikt has `issueToken()`, but it is a Kotlin API for JVM tests. A `lanyard token --as ada --aud billing-api` that drops into a curl in any project appears genuinely unoccupied.
3. **Deliberate failure tokens as advertised flags.**

Read navikt's README closely before building. It is the most thorough thing in the category and its Docker networking notes will save real time.

## 13. README lessons

navikt's first line — "Scriptable OAuth2/OpenID Connect server for JVM tests and Docker Compose" — describes the artifact's shape, not the reader's pain. It is precise if you already know what problem you have. A naive reader cannot tell whether it is a production IdP, a proxy, a test library, or a load generator. "Scriptable" is the word doing the most damage: it means the most to the maintainers and the least to everyone else.

**The test for a first line:** can a stranger finish the sentence "oh, this is for when I…" after reading it?

### Screenshots are the pitch

For a tool with a UI, the screenshot answers "what is this" in about 400 milliseconds, which is roughly how long we have. The persona picker is unusually well-suited: a login screen that is just a list of people with an "or mint one now" box below explains the entire product without a word of prose.

- **Directly under the tagline**, before installation. Not in a "Screenshots" section halfway down.
- **Light mode.** Dark screenshots look great to their author and murky to everyone else; GitHub's own default is light. Use `<picture>` with `prefers-color-scheme` for both.
- **Realistic content, never placeholder.** `ada@example.test` with an `admin` badge tells a story; `user1@test.com` does not.
- **Crop tight.** A full browser window with tabs and an address bar wastes most of the pixels.
- **A GIF for anything with motion.** For lanyard: clicking "Ada Bell" and watching the decoded claims appear in the live log.
- **Commit the images to the repo**, not an image host, so they survive.

### Lead with language-neutral examples

```
$ lanyard serve
  Issuer  → http://127.0.0.1:9500
  Web UI  → http://127.0.0.1:9500/_/

$ curl -H "Authorization: Bearer $(lanyard token --as ada --aud billing-api)" \
       localhost:8080/orders
```

Curl and a URL are universal — nobody reading that wonders whether it works with their stack. Language-specific config goes in `docs/` or a collapsed `<details>`, and there should be several so none looks like *the* language. Our implementation language belongs in the install instructions and nowhere else. (Cubby already does this well: `aws --endpoint-url` is language-neutral and Rust appears only under installation.)

### Show the contrast in numbers

A getting-started section that is literally two commands and no admin console visit. That contrast against a Docker-and-configure IdP is the strongest argument, and it works better shown than claimed.

## 14. Naming and distribution

`lanyard` on crates.io is taken by an unrelated MPL-licensed FFI library providing UTF-8 equivalents of `CStr`/`CString`, currently at 0.1.3. Not something anyone would confuse with an identity provider.

**Crate name and binary name are independent:**

```toml
[package]
name = "lanyard-cli"

[[bin]]
name = "lanyard"
path = "src/main.rs"
```

Well-worn path: `fd` publishes as `fd-find`, `dust` as `du-dust`, `delta` as `git-delta`, `btm` as `bottom`. Nobody thinks about the crate name after the install line.

Avoid a `-kit` suffix — it suggests a library or a collection of parts rather than a thing you run.

The names that actually matter are all available: the binary (ours to choose), `github.com/robap/lanyard`, `ghcr.io/robap/lanyard`, and a Homebrew tap `robap/tap/lanyard`.

**Remaining check:** search GitHub and the wider web for existing developer tools named lanyard. Registry availability is easy to verify; an established project with the same name and a similar audience is the thing that would actually cause confusion, and crates.io will not reveal it.

## 15. Open questions

### Where do personas come from?

The one genuinely unresolved design question, and it needs deciding early because it determines whether the persona file has a client-scope field — adding that later is a migration.

A machine-wide daemon started at login is not sitting in any project directory. But personas travelling with the repo was one of the better properties of the original design.

| Option | How | Trade-off |
|---|---|---|
| **Global only** | Single `~/.config/lanyard/users.yaml` | Simplest, fine for one developer. Personas stop travelling with the repo. |
| **Registration** | `lanyard link` from a project records the path; the daemon watches and merges each registered project's `lanyard.yaml` | Personas travel again. Costs one setup command per project, plus staleness when directories move. |
| **Client-id namespacing** | Every project already sends a distinct `client_id`; a project file declares `client: billing-web` and its personas appear only for that client | A clean picker per project instead of a merged list of forty people. |

**Current lean:** linking plus client-id namespacing, with a global file as fallback so the zero-config path still works on a fresh machine.

### Others

- **HTTPS priority.** Depends entirely on what the .NET and PHP validation in [section 10](#10-our-stack-specifically) turns up. Do that first.
- **Whether to reserve `lanyard-cli` on crates.io defensively.** Ten minutes if we want it; otherwise skip.

## 16. Relationship to cubby

lanyard is the second instance of a pattern cubby established: emulate a service locally as a single zero-config binary with a debugger UI, deterministic fixtures, and executable conformance against real SDKs.

By the time lanyard ships, the same skeleton will have been written twice:

- bind-address env var
- health endpoint
- data dir with a self-`.gitignore`
- `doctor` scaffolding
- live event stream on stdout plus SSE plus ndjson
- embedded UI with no Node on the build path
- conformance runner against real SDKs

That is an argument for extracting a shared crate — and unlike lanyard itself, a library's consumers *are* Rust developers, so that one genuinely belongs on crates.io. Not a v1 concern.

"Here is the toolkit for building local service fakes" is a more interesting artifact than any single fake.

### Cubby follow-ups from this thinking

Add screenshots: the live request log mid-multipart, the bucket browser beside a terminal showing the same files under `ls`, and the object detail with the presigned-URL generator. The second one is cubby's central thesis as a picture.
