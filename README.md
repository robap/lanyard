# lanyard

A local OIDC provider with nothing to configure.

`lanyard serve` starts an OpenID Connect provider on `127.0.0.1:9500` with a
working discovery document, a stable signing key and three ready-made users. No
realm to create, no client to register, no admin console to visit.
`lanyard token --as ada --aud billing-api` prints a bearer token you can paste
into a `curl`, and pointing a web application at the issuer gets you a login
screen that is a list of people with no password field.

## Logging in

Point any OIDC client at `http://127.0.0.1:9500/oidc`, invent a `client_id`, and
sign in. There is nothing to register.

1. Your app redirects to `/oidc/authorize`.
2. lanyard redirects to its persona picker at `http://127.0.0.1:9500/_/`.
3. You click a person. No password, no consent screen.
4. lanyard redirects back to your app with a `code`, and your app exchanges it
   at `/oidc/token` for an access token and an ID token.

The next login from the same application in the same browser skips step 3 —
lanyard remembers who you picked, **per `client_id`**, so three apps on three
ports can be signed in as three different people at once. Closing the browser
resets it, and so does restarting lanyard.

**Want to see it working before wiring up your own app?**
[`spikes/`](spikes/) holds four small applications — .NET, PHP, a
no-build-step SPA, and a resource server — with a five-minute walkthrough that
gets all of them logged in as different people against one running lanyard.

A minimal .NET client, which is the whole configuration:

```csharp
builder.Services.AddAuthentication(/* … */)
    .AddCookie()
    .AddOpenIdConnect(options =>
    {
        options.Authority = "http://127.0.0.1:9500/oidc";
        options.RequireHttpsMetadata = false;   // lanyard serves HTTP
        options.ResponseType = "code";          // .NET defaults to id_token
        options.ClientId = "billing-web";       // invented here, unknown to lanyard
        options.Scope.Add("email");
        options.Scope.Add("profile");
    });
```

## Install

macOS and Linux, arm64 and x86_64. Pick one line:

```
brew install robap/tap/lanyard
```

```
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/robap/lanyard/releases/latest/download/lanyard-cli-installer.sh | sh
```

```
docker run -p 9500:9500 ghcr.io/robap/lanyard
```

The Linux downloads are statically linked, so they run on distributions older
than the machine that built them. Then:

```
lanyard serve
```

```
lanyard 0.1.0
  Issuer    → http://127.0.0.1:9500/oidc
  UI        → http://127.0.0.1:9500/_/
  Log       → http://127.0.0.1:9500/_/log
  Listening → 127.0.0.1:9500
  Data dir  → /home/you/.local/share/lanyard
  Signing   → kid TXntCt2biz2Bj578hZocZOb2A2nQV9JfrBvFN55QWpU
  Personas  → built-in defaults (ada, mira, nobody)
```

Point your application's OIDC configuration at the issuer and it will find the
rest on its own:

```
curl -s http://127.0.0.1:9500/oidc/.well-known/openid-configuration | jq .
curl -s http://127.0.0.1:9500/oidc/jwks | jq .
```

**`brew services start lanyard` does not work**, and will not until
`lanyard service install` exists. The formula the tap ships has no service
stanza, because it is regenerated from the release on every version and a
hand-added one would survive exactly one of them. `lanyard serve` in the
foreground is the command; the always-on story is a `launchd` agent, a systemd
user unit and a Windows scheduled task written by lanyard itself, and it is
post-v1.

### Windows

**There is no native Windows binary yet**, and there are two ways to run lanyard
on Windows that are not workarounds. The first is WSL2 — install it inside a WSL
distribution with the `curl` line above, and run your application on Windows:

```
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/robap/lanyard/releases/latest/download/lanyard-cli-installer.sh | sh
```

Your application's discovery, JWKS and code exchange reach
`127.0.0.1:9500` through WSL2's localhost forwarding, the browser redirect goes
to the same forwarded port, and the redirect back to your application never
leaves Windows. The whole browser login works, including the persona picker.

The second needs no WSL at all — Docker Desktop or Podman Desktop, and the same
image everyone else runs:

```
docker run -p 9500:9500 ghcr.io/robap/lanyard
```

Two things to know, and they are the same two on both paths:

- **Use `127.0.0.1`, not `localhost`.** The issuer is
  `http://127.0.0.1:9500/oidc` and lanyard does not follow the `Host` header, so
  an application pointed at `localhost:9500` gets tokens that say `127.0.0.1`
  and `doctor` reports a mismatch that is technically correct and, here, only
  confusing. Point both sides at `127.0.0.1`.
- **`LANYARD_BIND=0.0.0.0` is the fallback.** The default loopback bind is
  usually enough, because WSL2's `localhostForwarding` reaches a listener bound
  to `127.0.0.1` inside the VM. Mirrored networking mode and some corporate VPN
  configurations do not, and that is when to bind wider.

### From source

You do not need this to use lanyard; it is here for changing it.

```
cargo build --release
./target/release/lanyard serve
```

**A Rust toolchain is the whole build.** No `node`, no `npm`, no bundler — the
binary is the whole website. The one page that ships JavaScript, `/_/log`, is a
[zero](https://github.com/robap/zero) app whose built output lives in `web/dist/`
and is **committed**; `rust-embed` compiles it in. `zero` itself is a
cargo-installed Rust binary and is needed only to *change* the UI:

```
cargo install zero --locked
zero update -y && zero test && zero lint
zero build            # regenerates web/dist/ — commit it
```

There is no CI gate on whether `web/dist/` is current: pinning CI to one `zero`
version so a byte-identical rebuild could be compared would turn a routine
framework bump into a red build, and a gate like that gets disabled within a
month. What is actually load-bearing is checked instead — CI's
`cargo package --locked` job builds the packaged crate with `web/dist/` embedded
and no `zero` present, and a missing bundle is a `rust-embed` build error rather
than a smaller binary. A stale bundle is a cosmetic wrong-version UI; the
convention carries it. **Regenerate before committing.**

The embedded UI costs about 450 KB of binary: 55 KB of JavaScript, 44 KB of CSS,
and 350 KB of Geist woff2 served from lanyard itself so no page ever asks a CDN
or Google Fonts for anything.

## Read this before you use it

**The default signing key is public and forgeable.** It is committed to this
repository on purpose, so that `kid` and `n` above are the same on every machine
and a wiped data directory does not invalidate a cached JWKS. Anyone with a
checkout can mint a token your application will accept. Never expose lanyard to
an untrusted network, and never point a staging or production service at it.

`POST /_/api/token` and `POST /oidc/token` compound this: between them they mint
any claims for anyone who can reach the port, with no authentication at all. That
is the entire point — a test seam and a provider with no client registry — and it
is why the default bind is loopback. `LANYARD_BIND=0.0.0.0` turns that off, so do
it only on a network you control.

**The issuer is `http://127.0.0.1:9500/oidc`**, path segment included, and it
does not follow the `Host` header. Reaching lanyard at `http://localhost:9500`
still yields a document that says `127.0.0.1`, and a relying party that compares
the `iss` claim against the URL it dialled will reject the token. That mismatch —
not a bug, a deliberate refusal to let a request header decide who the issuer is
— is what `LANYARD_ISSUER` exists for. Set it to whatever name your application
will actually use:

```
LANYARD_ISSUER=http://lanyard:9500/oidc lanyard serve
```

**The discovery document lists only the endpoints that exist.** Today that is
`jwks_uri`, `token_endpoint`, `authorization_endpoint`, `userinfo_endpoint`,
`end_session_endpoint`, `introspection_endpoint` and `revocation_endpoint`,
because advertising an endpoint that returns 404 sends a client down a path that
cannot work. **The rule cuts both ways**, and Phase 5 is where that mattered:
ASP.NET Core's `SignOutAsync` reads `end_session_endpoint` out of the document
and silently builds no redirect when it is missing, so an endpoint that exists
and is not advertised fails exactly as confusingly as one advertised and
missing. The document is also corrected when it turns out to have promised
something: `response_types_supported` is now exactly `["code"]`, because
advertising `id_token` would tell a .NET app that its *default* setting is
supported and then refuse it at request time.

**Any `aud` mints.** `--aud not-a-real-api` is a token, not an error. Audiences
follow the same no-registration rule as everything else, and an audience your API
does not expect is a test case worth having. Rejection happens at the API, not
here.

**A 60-second token stays valid for about six minutes against a stock .NET API.**
`TokenValidationParameters.ClockSkew` defaults to **five minutes**, so a
default-configured `AddJwtBearer` keeps accepting a lanyard token long after it
expired. If you cannot make short-TTL rejection happen locally you will conclude
lanyard's lifetimes are fake; they are not, your resource server is being
generous. One line fixes it:

```csharp
options.TokenValidationParameters.ClockSkew = TimeSpan.Zero;
```

Measured both ways in [`docs/decisions/dotnet-jwt-bearer-settings.md`](docs/decisions/dotnet-jwt-bearer-settings.md).

**lanyard allows itself five seconds, and only five.** Every token is minted with
`nbf` five seconds before `iat` — `iat` and `exp` are untouched, because
extending `exp` would silently lengthen a TTL that is 60 seconds on purpose — and
lanyard's own `/oidc/userinfo`, `/oidc/introspect` and `/oidc/revoke` accept a
token up to five seconds past `exp`. That is enough for a relying party whose
clock is a second or two behind, and nowhere near enough to make the
90-seconds-later rejection stop happening. It is also the opposite end of the
scale from .NET's default five *minutes*.

A refusal says which clock it doubts, rather than returning a bare `401`:

| State of the token | The `401` says |
|---|---|
| `exp` more than 5s past | `the token expired 4m12s ago` |
| `nbf` more than 5s ahead | `the token is not valid yet — the clock that issued it is ahead of this one by about 5m0s. Check the clocks on both sides; run lanyard doctor` |
| either, on a process running skewed | …`(this process's clock is skewed by -5m0s via LANYARD_CLOCK_SKEW)` |

**Scope.** This is Phase 8. There is a discovery document, a JWKS, all three
arms of `/oidc/token` — `client_credentials`, `authorization_code` and
`refresh_token` — plus `/oidc/authorize`, `/oidc/userinfo`,
`/oidc/end_session`, `/oidc/introspect`, `/oidc/revoke`, the persona picker at
`/_/` with its three session controls, browser sessions, the live request log at
`/_/log` and `lanyard logs`, project personas via `lanyard link` and `client:`
namespacing, the `token` and `env` CLI commands, the six deliberate failure
flags, the test seam, `lanyard doctor` and `GET /_/health`, a `Host`-mismatch
warning, and a distroless container image. There is no back-channel or
front-channel logout, no
`sid` in the ID token, no `check_session_iframe`, nothing persisted across a
restart *except the list of linked project directories*, and no consent screen —
a consent screen is client registration in a different costume.

## Configuration

Every setting is an environment variable read once at startup. All are optional.

| Variable | Default | Meaning |
|---|---|---|
| `LANYARD_ISSUER` | `http://127.0.0.1:{port}/oidc` | The `iss` claim and the discovery document's `issuer` |
| `LANYARD_BIND` | `127.0.0.1` | Listen address |
| `LANYARD_PORT` | `9500` | Listen port |
| `LANYARD_DATA_DIR` | `$XDG_DATA_HOME/lanyard` | Signing key lives here |
| `LANYARD_PERSONAS` | `$XDG_CONFIG_HOME/lanyard/users.yaml` | Global persona file; when set, it must exist |
| `LANYARD_LINKS` | `$XDG_CONFIG_HOME/lanyard/links.yaml` | Linked persona files; absent means none, set or not |
| `LANYARD_URL` | `http://127.0.0.1:{port}` | Where `lanyard token`, `lanyard logs` and `lanyard doctor` reach the server |
| `LANYARD_CLOCK_SKEW` | unset | Move this process's clock: `-5m`, `+90s`, `300`. Deliberately wrong, and loud about it |

`LANYARD_CLOCK_SKEW` is the seventh deliberate failure mode, and the only one
that is not a flag on `lanyard token`. It offsets **every** clock read in the
process — the tokens it mints, the timestamps in its log, the `now` it reports on
`/_/health` — so a skewed lanyard is internally consistent and wrong about the
world, which is what a drifted VM actually looks like. It exists because on
native Linux there is no way to skew a container's clock: `CLOCK_REALTIME` is not
virtualized by time namespaces, so a rootless container's clock *is* the host's.

It is loud on purpose. The banner gains a line, `/_/health` reports it, every
expiry rejection names it, and `lanyard doctor` reports it as a `WARN` even when
the two clocks agree by construction. A dev tool may lie about the time; it may
not do so quietly.

```
$ LANYARD_CLOCK_SKEW=-5m lanyard serve
lanyard 0.1.0
  Issuer    → http://127.0.0.1:9500/oidc
  …
  Clock     → skewed -5m0s (LANYARD_CLOCK_SKEW) — tokens minted here are already expired by 4m0s for anything
              on a correct clock
```

`doctor` reads the true clock even when `LANYARD_CLOCK_SKEW` is set in its own
environment, and says so — otherwise a developer with it exported would run a
skewed `serve`, a skewed `doctor`, and be told the clocks agree.

`LANYARD_URL` is an address; `LANYARD_ISSUER` is a string that goes in a token.
They are deliberately separate knobs. Set the issuer to `http://lanyard:9500/oidc`
for a container network and that name resolves nowhere useful from your shell — a
CLI that derived its target from the issuer would be unreachable in exactly the
setup the issuer setting exists to support.

On first run the data directory is created containing `signing-key.pem` (mode
`0600`, seeded from the built-in default key) and a `.gitignore`. Replace
`signing-key.pem` with your own 2048-bit RSA key to get a signing key that is not
public. A `signing-key.pem` that cannot be parsed is fatal — lanyard will not
silently fall back to the default, because a token signed by a key you did not
expect is the most confusing failure this tool can produce.

## Personas

With no persona file, three users ship:

| id | name | email | roles |
|---|---|---|---|
| `ada` | Ada Bell | `ada@example.test` | `admin`, `user` |
| `mira` | Mira Okonkwo | `mira@example.test` | `user` |
| `nobody` | — | — | — |

`nobody` is the useful one. A token for `nobody` carries `sub` and the registered
claims and nothing else — no `email`, no `name`, no `roles`, no
`preferred_username`. It is the user who breaks applications and the one nobody
remembers to create.

Point `LANYARD_PERSONAS` at a YAML file to replace all three:

```yaml
client: billing-web          # optional, file-level
personas:
  - id: ada
    name: Ada Bell
    email: ada@example.test
    roles: [admin, user]
    attributes:              # anything here becomes a top-level claim
      department: platform
  - id: ops
    name: Ops Bot
    client: ops-console      # optional, per-persona
```

The file replaces the built-ins entirely rather than merging with them. A
malformed file — an unknown key, a duplicate `id`, a missing `id` — makes `serve`
exit non-zero naming the offending key and the file path. There is no fallback to
the defaults: a persona that silently failed to load shows up three redirects
later as a name missing from the picker.

## Personas that travel with the repo

A project commits a `lanyard.yaml` — the same schema as above — and each
developer links it once:

```
$ lanyard link ~/code/billing/lanyard.yaml
linked /home/dev/code/billing/lanyard.yaml — 3 personas for client billing-web
  dev-admin, billing-readonly, locked-out

$ lanyard links
/home/dev/.config/lanyard/links.yaml

/home/dev/code/billing/lanyard.yaml    billing-web  dev-admin, billing-readonly, locked-out
/home/dev/code/ops-console/dev.yaml    —            ops-bot
/home/dev/code/old-thing/lanyard.yaml  missing      (no such file)

$ lanyard unlink ~/code/billing/lanyard.yaml
```

`links` leads with the registry's own path, so `LANYARD_LINKS` pointing
somewhere unexpected is visible rather than inferred.

The file is **named, never discovered** — nothing is inferred from the working
directory, and the name `lanyard.yaml` is only a convention. `link` validates
the file, records it absolute, is idempotent, and needs no running server: a
running `lanyard serve` picks the change up on its next request.

### The visibility rule

> **A persona is visible to a request if it declares no `client:`, or if its
> `client:` equals that request's `client_id`.**

Applied at `/oidc/authorize`, `/oidc/token`, `/oidc/userinfo`, `/_/`,
`/_/api/personas` and `/_/api/token`. A persona-level `client:` beats the
file-level one, and a request with no `client_id` sees what an unrecognised one
sees: the unscoped personas.

**It is not registration.** lanyard still accepts any `client_id` from anyone;
the rule filters a list a human reads, it never gates a grant.

### Merging

Three sources: the built-ins, the global file (which still replaces them), and
every linked file in registry order. **Links add; they never subtract** — so
`nobody` survives linking a project, and the picker can never be emptied by a
link going bad. Use `client:` to hide the *other* projects' people.

When two personas visible to the same request share an id: scoped beats
unscoped, then project beats global, then **first-linked wins**. Every
shadowing warns, naming the files and which one won.

### Broken files warn; a broken `users.yaml` is still fatal

A machine-wide daemon must not die because one of ten projects has a typo. The
fatal-ness moves to `lanyard link`, which parses the file and refuses a bad one.
At serve time a file that breaks later, or one that has gone missing, is a
**warning**: that file contributes nothing and everything else still resolves.

Warnings appear in four places — the startup banner, a band on `/_/`, the event
stream (`lanyard logs --json | jq 'select(.warnings)'`, `/_/log`), and
`warnings` on `/_/api/personas`.

### Live, without a watcher

Edit a linked file, reload `/_/`, see the change. Each source is re-read only
when a `stat` says its modified time or length moved — no `notify`, no watcher
thread, no atomic-rename problem.

## Minting a token from the command line

Probably the larger half of daily use, and the shorter path to being useful:
there is no redirect dance to get right.

```
$ lanyard token --as ada --aud billing-api
eyJhbGciOiJSUzI1NiIsImtpZCI6...

$ curl -H "Authorization: Bearer $(lanyard token --as ada --aud billing-api)" \
       localhost:8080/orders
```

`--client` sets the grant's `client_id`, and therefore which personas the CLI
can see. Without it the CLI is `lanyard-cli`, so a scoped persona is invisible —
and the refusal says where it is rather than `no such persona`:

```
$ lanyard token --as dev-admin
lanyard: no persona "dev-admin" for client "lanyard-cli" — it is defined in
  /home/dev/code/billing/lanyard.yaml scoped to client "billing-web".
  Retry with --client billing-web

$ lanyard token --as dev-admin --client billing-web --aud billing-api
```

A project that links a file without declaring `client:` needs none of this.

`lanyard token` prints the token and a newline on stdout and nothing else — no
banner, no timing, no "minted for Ada" — so `$(...)` drops it straight into a
header. On failure **stdout stays empty**, the message goes to stderr and the
exit code is non-zero, because an API receiving the words "connection refused" as
a credential is worse than an API receiving nothing.

```
$ lanyard token --as ada
lanyard: nothing listening at http://127.0.0.1:9500 — is `lanyard serve` running?
```

`lanyard env` prints the same token shaped for `eval`:

```
$ eval "$(lanyard env --as ada --aud billing-api)"
$ echo $BEARER_TOKEN
eyJhbGciOiJSUzI1NiIsImtpZCI6...
```

The variable name is fixed at `BEARER_TOKEN`. When minting fails, `lanyard env`
prints nothing at all on stdout, so `eval "$(...)"` is a no-op that leaves your
shell exactly as it was.

| Flag | Default | Meaning |
|---|---|---|
| `--as` | *required* | Persona to mint for |
| `--aud` | none | The `aud` claim. Any value mints |
| `--scope` | none | Space-delimited, becomes the `scope` claim |
| `--url` | `$LANYARD_URL`, else `http://127.0.0.1:9500` | Where lanyard is listening |
| `--expired` | off | Mint a token that expired an hour ago |
| `--wrong-aud` | off | `aud` becomes `wrong-<requested>` |
| `--wrong-iss` | off | `iss` becomes `https://wrong-issuer.example.test` |
| `--bad-signature` | off | One bit of the signature flipped |
| `--alg-none` | off | An unsecured JWT: `alg: none`, no signature |
| `--unknown-kid` | off | A real signature under a `kid` in no JWKS |

The six failure flags are mutually exclusive with each other and work on `env`
too. See [Deliberately wrong tokens](#deliberately-wrong-tokens).

The CLI does not sign locally. It performs a real `client_credentials` grant
against `/oidc/token`, so the token arrives the way a production token arrives —
through the endpoint an SDK would use. Tokens are 60 seconds and the TTL is not
configurable.

## Deliberately wrong tokens

Testing that your API *accepts* a good token is the easy half. Six flags produce
the tokens that test the other half — each one structurally valid and
semantically wrong in exactly one way:

```sh
API=http://127.0.0.1:5080/orders
code() { curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $1" $API; }

code "$(lanyard token --as ada --aud billing-api)"                  # 200
code "$(lanyard token --as ada --aud billing-api --expired)"        # 401
code "$(lanyard token --as ada --aud billing-api --wrong-aud)"      # 401
code "$(lanyard token --as ada --aud billing-api --wrong-iss)"      # 401
code "$(lanyard token --as ada --aud billing-api --bad-signature)"  # 401
code "$(lanyard token --as ada --aud billing-api --alg-none)"       # 401
code "$(lanyard token --as ada --aud billing-api --unknown-kid)"    # 401 — or 200; see below
```

| Flag | What is wrong | What is still right |
|---|---|---|
| `--expired` | `iat` = `nbf` = an hour ago, `exp` 60s after that | signature, `kid`, `iss`, `aud`, `sub` |
| `--wrong-aud` | `aud` becomes `wrong-<what you asked for>` | signature, `kid`, `iss`, lifetime |
| `--wrong-iss` | `iss` becomes `https://wrong-issuer.example.test` | signature, `kid`, `aud`, lifetime |
| `--bad-signature` | one bit of the signature is flipped | header byte-for-byte, all claims |
| `--alg-none` | header is `{"alg":"none","typ":"JWT"}`, no signature at all | all claims |
| `--unknown-kid` | header `kid` is `lanyard-unknown-kid`, which is in no JWKS | RS256, a real signature, all claims |

**Each flaw breaks exactly one thing**, and that is the whole design. A resource
server stops at the first check it fails, so a token that is both expired and
wrongly signed tells you nothing about which check ran. The flags are mutually
exclusive for the same reason: `--expired --wrong-aud` is a usage error.

**`--expired` is expired by an hour, not by a second**, so it is refused even by
a resource server with a generous clock tolerance — including a stock .NET API,
whose default is five minutes.

**If `--unknown-kid` prints `200`, that is a finding about your stack, not a
bug.** The signature on that token is real; only the `kid` names a key that does
not exist. A library that honours `kid` refuses it — `jose` does, with
`JWKSNoMatchingKey`. A library that falls back to trying every key in the JWKS
accepts it, and **stock .NET `AddJwtBearer` does exactly that** (observed;
`docs/decisions/dotnet-jwt-bearer-settings.md`). Which half of that you are on
is worth knowing, and this flag is how you find out.

Nothing about minting a flawed token changes server state: `/oidc/jwks` still
carries the one real key afterwards. `spikes/dotnet-api/failure-tokens.sh` is
this block with a `PASS`/`FAIL` line per case.

## The authorization endpoint

`GET` or `POST /oidc/authorize`. This is where every other provider keeps a
client registry, a redirect allowlist and a consent screen. lanyard has none of
the three.

| Parameter | Handling |
|---|---|
| `client_id` | **Required.** Any value. It keys the browser session and becomes the ID token's `aud` |
| `redirect_uri` | **Required.** Any value whose host is loopback — [the one rejection](#the-one-rejection) |
| `response_type` | `code` only. Anything else is `unsupported_response_type` |
| `response_mode` | `query` (default) or `form_post`. `fragment` is `unsupported_response_mode` |
| `scope` | Any. `openid` decides whether an ID token is issued; `email` and `profile` gate claims |
| `state` | Echoed back byte-for-byte when sent, absent when not. Never invented |
| `nonce` | Carried into the ID token when sent |
| `code_challenge` / `code_challenge_method` | `S256` or `plain`. An omitted method is `plain` (RFC 7636). Absent means no PKCE |
| `prompt` | `login`, `select_account`, `none`, or absent. Anything else is ignored |
| `max_age` | Compared against the remembered selection's `auth_time` |
| `id_token_hint` | **Only read on `prompt=none`.** Signature and `iss` are checked, `exp` deliberately is not — a hint is expected to be expired. A `sub` that is not the remembered selection's gets `login_required`; a hint that will not verify gets `invalid_request` |
| `audience` / `resource` | Becomes the **access token's** `aud`, as on the `client_credentials` grant |
| `client_secret` | Accepted anywhere it is offered and checked nowhere |
| anything else | Ignored |

`client_id` is required not because lanyard knows it but because a session has to
be keyed on something, and an app that omits it would share a session with every
other app.

Then `grant_type=authorization_code` at [the token endpoint](#the-token-endpoint):

| Parameter | Required | Meaning |
|---|---|---|
| `code` | yes | Single-use, and it lives **60 seconds** |
| `code_verifier` | when a challenge was sent | Genuinely verified |
| `redirect_uri` | no | When sent, it must match the one `/authorize` received |
| `client_id` / `client_secret` | no | Never validated |

```json
{ "access_token": "eyJ…", "token_type": "Bearer", "expires_in": 60,
  "scope": "openid email profile", "id_token": "eyJ…" }
```

`id_token` is present **only when the authorization request's scope contained
`openid`**. Without it this is a plain OAuth 2.0 code flow and gets a plain
OAuth 2.0 response.

`invalid_grant` is a `400`, and its `error_description` says which of the six
things went wrong: the code is unknown, expired, or already exchanged; the
`redirect_uri` does not match; the `code_verifier` is missing; the
`code_verifier` does not match.

**PKCE really is verified, and single-use really is enforced.**
Accept-everything is about *registration* — who you say you are, where you say
you want to come back to. It was never about skipping the cryptography. A local
provider that rubber-stamps a wrong `code_verifier` lets a broken PKCE
implementation ship, and production is a bad place to find that out.

### The one rejection

A `redirect_uri` is accepted when its scheme is `http` or `https` **and its host
is `localhost`, any `*.localhost` name, any `127.0.0.0/8` address, or `[::1]`** —
any port, any path, any query. Everything else is refused.

The host is compared **literally, with no DNS lookup**, so
`http://web.localtest.me:5000/cb` is rejected even though it resolves to
127.0.0.1. A rule enforced by resolution depends on the network, is cacheable, is
different inside a container than outside one, and stops being one line to
explain. (`web.localtest.me` is also where a .NET app's own login breaks anyway,
for an unrelated reason — see [Troubleshooting](#troubleshooting).) Custom
schemes for native apps (`com.example.app:/cb`) and `urn:ietf:wg:oauth:2.0:oob`
are rejected too; when device and native flows come up they bring their own
phase.

A rejected `redirect_uri`, or a missing `client_id`, renders a **`400` at
lanyard** — `text/html`, no `Location` header, the browser stays where it is, and
the page names the value and states the rule. RFC 6749 §4.1.2.1 is explicit that
a server must not redirect an error to an address it has just decided not to
trust; doing so would make the one rejection decorative. Every *other* error
redirects, carrying `error`, `error_description` and the `state` that was sent,
so your app's own error handling runs.

## The persona picker

`http://127.0.0.1:9500/_/` — a list of people, no password field.

- One form per person, so the flow works with **JavaScript disabled**. The only
  script lanyard ships in the login path is the auto-submit on a
  `response_mode=form_post` response, and that page carries a visible button
  inside `<noscript>`.
- **Mint one now** — a panel taking `sub`, `name`, `email`, comma-separated
  `roles`, and a textarea of extra claims as JSON. Submitting it signs you in as
  an identity that is in no file. A one-off is deliberately **not remembered**:
  the next login from that application shows the picker again.
- **Always ask** — a checkbox that forces the picker for every `client_id` in
  this browser. It is on the picker and on the no-login version of the page, so
  it can be turned off without starting a login.
- **This browser** — one row per `client_id` this browser is signed in for,
  naming the person and when they were chosen, each with **Forget** and
  **Expire now**, plus **Log out of lanyard** below them. See
  [Logging out](#three-controls-on-_).
- **Filtered to the people this application can see.** The page says how many it
  hid and offers **Show all** — every persona, labelled with its client and its
  file, without dropping the login. A persona this application cannot see is a
  card there, never a button.
- Visiting `/_/` with no login in progress is that unfiltered view. That is the
  URL the banner prints.
- **A band at the top** when a persona source is broken, naming the path and the
  error.

Persona display strings are HTML-escaped, and so are the `client:` labels and
file paths this page now renders. A persona file is your own, but a project's
`lanyard.yaml` came out of a repository somebody cloned, and a picker that
executes its own persona list is a bad look for a tool whose pitch is "it
catches your bugs".

The page is server-rendered HTML with one stylesheet inlined by `include_str!`.
No template engine, no bundler, no `npm`, nothing generated at build time:
`cargo build --release` on a machine with no `node` on `PATH` produces a binary
that serves a styled picker, and the page requests nothing from anywhere.
`prefers-color-scheme` handles dark mode.

## Sessions

One cookie:

```
lanyard_session=<opaque>; Path=/; HttpOnly; SameSite=Lax
```

**No `Secure`**, ever, while lanyard serves HTTP. A browser silently drops a
`Secure` cookie sent over `http://`, and the symptom is "it asks me to pick a
persona every single time" with nothing anywhere saying why. **No `Max-Age` or
`Expires`** either: it is a browser-session cookie, so closing the browser is a
working reset.

Server-side the cookie maps to a record holding, **per `client_id`**, the persona
chosen and the moment it was chosen. That per-`client_id` scoping is the whole
multi-project property: log in to one app as Ada and another as Mira in the same
browser, and neither disturbs the other.

Three resets, all worth knowing:

- **[Log out](#logging-out).** One click in your application, both sessions gone.
- **Close the browser.** The cookie has no expiry, so it goes.
- **Restart `lanyard serve`.** Sessions, pending authorization requests,
  authorization codes, refresh tokens and revocations all live in memory and all
  die with the process.

The picker appears anyway when "always ask" is set, when `prompt=login` or
`prompt=select_account` is sent, or when `max_age` is sent and the remembered
selection is older than it. `prompt=none` **renders nothing, ever** — it returns
a code if there is a usable remembered selection and redirects with
`error=login_required` if there is not. A picker inside a hidden iframe is a
login screen nobody can click.

### Silent renew, and the one rule that decides whether it works

A SPA with no refresh token renews in a **hidden iframe** pointed at
`/authorize?…&prompt=none`. Whether that works over plain HTTP comes down to one
thing: **does the browser send lanyard's session cookie on that navigation?**

lanyard's cookie is `SameSite=Lax`, and `SameSite` compares the scheme and the
**host** and **ignores the port**. So:

| your app | lanyard's issuer | | silent renew |
|---|---|---|---|
| `http://localhost:5173` | `http://localhost:9500` | same site | **works** |
| `http://127.0.0.1:5173` | `http://127.0.0.1:9500` | same site | **works** |
| `http://localhost:5173` | `http://127.0.0.1:9500` | two sites | `login_required` |
| `http://web.localtest.me:5173` | `http://localhost:9500` | two sites | `login_required` |

**Use the same name on both sides.** The port does not matter and the scheme is
`http` either way, so this is entirely about spelling `localhost` or `127.0.0.1`
consistently.

**lanyard's shipped default is `http://127.0.0.1:9500/oidc`**, and most dev
servers are opened at `http://localhost:<port>` — so out of the box those are
two sites. Either dial your app at `127.0.0.1`, or start lanyard with:

```
LANYARD_ISSUER=http://localhost:9500/oidc lanyard serve
```

`lanyard doctor` prints this as a note under **Issuer** whenever the issuer host
is a loopback IP address. It cannot know what origin your app is served from, so
it states the consequence rather than reaching a verdict.

Measured rather than assumed, in Chrome and Firefox, both directions:
[`docs/decisions/silent-renew-over-http.md`](docs/decisions/silent-renew-over-http.md).

**A renew is not a re-authentication.** The ID token from a `prompt=none` carries
a fresh `iat` and `exp`, the same `sub`, and the **same `auth_time`** — the
moment the human actually picked a person. An `auth_time` that advanced on every
renew would let an RP's `max_age` check pass for ever without anybody
authenticating again.

**`login_required` says which of six things happened**, because "did my cookie
arrive?" is the question and one generic sentence answers it for nobody: no
cookie at all; a cookie with no selection under this `client_id`; "always ask" on;
a selection older than `max_age`; a selection naming a persona the file no longer
has; or an `id_token_hint` naming somebody else. The sentence goes back to your
app in `error_description` **and** into
[`lanyard logs`](#the-live-request-log) — which is where you will actually read
it, because an iframe's query string is not somewhere anybody looks.

**lanyard is deliberately frameable.** There is no `X-Frame-Options` and no
`Content-Security-Policy` on anything, and a test exists whose whole purpose is
to fail the day somebody adds one.

## Logging out

**There are two sessions**, and knowing which one you just ended is most of
this section.

| Cookie you delete by hand | What happens on the next protected page |
|---|---|
| `lanyard_session` only | **Nothing observable.** Still the same person, and the browser never reaches lanyard at all |
| Your application's cookie only | A round trip to lanyard, which still remembers you — so **no picker**, and you are signed straight back in as the same person |
| Both | The picker, at last |

**Why deleting `lanyard_session` by hand does nothing visible.** Your
application holds its own session cookie. While it has one it never asks lanyard
anything, so nothing lanyard forgot can matter. That is not a lanyard bug — it
is what an SSO session *is*, and an application whose log-out does not go
through the provider behaves exactly the same way against Okta. Deleting cookies
by hand is the wrong tool; the right one is the button.

### One click, both sessions

`GET` or `POST /oidc/end_session` — OIDC RP-Initiated Logout, and it is a
browser redirect chain rather than a back-channel call, because
`lanyard_session` lives in the browser and only a top-level navigation carries
it:

```
GET 302  localhost:5000/logout                your app drops its own cookie
GET 302  127.0.0.1:9500/oidc/end_session      lanyard drops the whole session
GET 302  localhost:5000/signout-callback-oidc back at your app, signed out
```

Every SDK ships this as a one-liner and builds the URL from the discovery
document — no lanyard URL appears in any of the spikes:

| Stack | The call |
|---|---|
| ASP.NET Core | `SignOutAsync` over the cookie **and** OpenID Connect schemes |
| `jumbojett/openid-connect-php` | `$oidc->signOut($idToken, $postLogoutRedirect)` |
| `oidc-client-ts` | `mgr.signoutRedirect()` |

| Parameter | Handling |
|---|---|
| `post_logout_redirect_uri` | Optional. **Loopback or a rendered `400`** — [the one rejection](#the-one-rejection), applied to a second parameter |
| `state` | Echoed byte-for-byte when sent, absent when not. Never invented |
| `id_token_hint` | Optional, **never required**. Read for its `aud` so the rendered page can name your application; expired, foreign, malformed and absent all log out identically |
| `client_id` | Ignored except for display |
| `logout_hint`, `ui_locales` | Ignored |

**Logging out never fails.** No session, an unknown session, a session from
before a restart — all of them are the same redirect. There is **no confirmation
screen**, ever: RP-Initiated Logout §2 says the OP *should* ask when there is no
valid `id_token_hint`, and lanyard does not, for the same reason it has no
consent screen. A prompt nobody can automate past is a prompt in the way of a
test.

With no `post_logout_redirect_uri`, lanyard renders its own page saying you are
signed out and linking to `/_/`, rather than leaving you on a blank one. The
response carries `Set-Cookie: lanyard_session=; …; Max-Age=0` either way, so the
log-out is readable in `curl -i` and in a network tab.

### It clears the whole browser session, not one `client_id`

lanyard's session holds a selection per `client_id`, so `/end_session` from one
application *could* drop just that one. **It drops everything** — every
selection, and every refresh token that session was issued. Every real IdP has
one SSO session and clears all of it, and diverging from production is the thing
this project exists not to do. A "full log-out" that leaves a working refresh
token in an application's local storage is not one.

The cost is real and is on the record: **logging out of one application logs you
out of the other two.** If what you wanted was one, that control is **Forget**
on `/_/` — where you reach for it deliberately, rather than on a protocol
endpoint where an SDK would reach it by accident.

### Three controls on `/_/`

The picker's **This browser** section lists one row per `client_id` this browser
is signed in for, naming the person and when they were chosen.

| Control | What it does | What you observe afterwards |
|---|---|---|
| **Log out of lanyard** | The whole session: every selection, every refresh token | The next login from any application shows the picker |
| **Forget** | One `client_id`'s selection | That application's next login shows the picker; the others are untouched |
| **Expire now** | Revokes one `client_id`'s live access and refresh tokens and **keeps the selection** | Your application's next API call gets `401` and its refresh fails — so **its own renew path runs**, rather than the picker appearing |

**Expire now** is the control that answers "what does my application do when its
token dies", and keeping the person is the whole point of it: making the tokens
dead without making the person forgotten is the only way to ask that question
without waiting.

All three are plain `POST` forms. No JavaScript, and no CSRF token — the cookie
is `SameSite=Lax`, so a cross-site POST arrives without it and therefore acts on
no session.

## The live request log

**lanyard already knows why your login failed. This is where it tells you.**

Every request to `/oidc/*` produces one structured event: what was asked, what
was decided, what came out. The `error_description` sentences that used to be
written to a `400` on a back-channel call nobody sees are on it, whole. So is
the `client_id`, on every line — which is what makes one instance serving three
projects readable rather than noise.

There are three surfaces and one stream behind them.

**stdout, always on, no flag.** `lanyard serve` prints one aligned line per
request:

```
14:02:09  billing-web   GET  /oidc/authorize   302    0ms  ada  scope=openid,email,profile  pkce=S256
14:02:11  billing-web   POST /oidc/token       400    3ms  invalid_grant  the code_verifier does not match the S256 code_challenge this code was issued against
14:02:14  lanyard-cli   POST /oidc/token       200    2ms  client_credentials  sub=ada  aud=billing-api  exp=+60s
14:02:19  lanyard-cli   POST /oidc/token       200    2ms  client_credentials  sub=ada  aud=billing-api  exp=+60s  flaw=alg-none
```

Nothing is persisted — `lanyard serve > lanyard.log` is the whole story, and the
log resets on restart. Times are **UTC**: `std::time` has no local offset and a
timezone crate was not worth the dependency.

**`lanyard logs`, in a second terminal.** Connects to the running singleton over
HTTP, the way `lanyard token` performs a real grant rather than signing locally.
Bare, it prints exactly what `lanyard serve` is printing in the first terminal;
`--json` gives one JSON object per line, for `jq`:

```
lanyard logs
lanyard logs --json | jq -r '.client_id + " " + .endpoint'
```

Both replay everything still in the ring before going live, so starting it after
a login that already failed still shows you the failure. With no lanyard running
it exits non-zero and says so, rather than exiting `0` having printed nothing.

**`/_/log`, in a browser.** A row per request, newest first, a `client_id`
filter, pause and clear — and **click a row** for the decoded claims: the header
and payload of every token minted, `exp` as both the epoch integer and a human
time, and, on a PKCE failure, the three values side by side:

```
PKCE (S256)
  code_verifier presented       dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXX
  challenge computed from it    ZtNnvmu4djKPm9mr322ZXBdqrXU41t_xP0Fp3EM3H84
  code_challenge recorded       E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM
```

Which two were meant to be equal is not something you have to be told.

**Warnings about persona sources ride on the events**, not because the request
caused them but because it was answered while they were true — and it is the
request that was about to get the wrong picker. `/_/` emits nothing (a page is
not a decision), so a project file that breaks reaches the log on the next
protocol request:

```
14:07:02  billing-web   GET  /oidc/authorize   302    0ms  dev-admin  warning: /home/dev/code/billing: linked directory does not exist

lanyard logs --json | jq 'select(.warnings)'
```

The field is **absent** when nothing is wrong, so that `select(.warnings)` is
the whole filter. On `/_/log` the same sentences are on the row and, in full, in
the expanded panel.

### The endpoints, if you want the stream yourself

| | |
|---|---|
| `GET /_/api/events` | `text/event-stream`. Replays the ring, then streams live. Honours `Last-Event-ID` |
| `GET /_/api/events?format=ndjson` | The same objects, one JSON per line |
| `POST /_/api/events/clear` | Drains the ring and empties every open `/_/log`. `204` |

```
curl -N 'http://127.0.0.1:9500/_/api/events?format=ndjson' | jq -c '{client_id, endpoint, status, error}'
```

The ring holds the last 1000 events. A reader that connects and stops reading —
a backgrounded tab, a `curl` into a full pipe — never slows a login down: the
fan-out is lossy at the subscriber and never at the source, and a reader that
falls behind gets a `{"dropped": N}` marker naming how many it missed. **A log
with a silent hole in it is worse than no log**, because you conclude the
request never happened.

`/oidc/jwks` and the discovery document emit nothing — they are static, and an
SDK's poll loop on them would drown everything else. Under `/_/`, exactly two
routes emit: `POST /_/pick`, because "why am I signed in as the wrong person" is
a question the log has to answer, and the test seam's `POST /_/api/token`.

### The log prints secrets, and that is deliberate

**It prints authorization codes, refresh tokens, `code_verifier`s, and any
`client_secret` a client sends.** It prints the full decoded claims of every
token minted. Nothing is redacted.

That follows from everything else here. lanyard accepts every `client_secret`
without looking at it, so starring one out would teach you that lanyard checked
something it did not; and a `code_verifier` you cannot see is a PKCE failure you
cannot diagnose. **The log is exactly as sensitive as the tokens `lanyard token`
already prints to your terminal** — 60-second tokens signed by a key whose
private half is published in this repository. Treat a pasted log the way you
would treat a pasted token.

## Scope, and what it filters

| Scope requested | Claims added |
|---|---|
| always | `sub` |
| `email` | `email`, `email_verified` |
| `profile` | `name`, `preferred_username` |
| always | `roles`, and the persona's arbitrary `attributes` |
| `offline_access` | **none** — it adds no claim; it asks for a [refresh token](#refresh-tokens) |

`roles` and `attributes` are unscoped because OIDC defines no scope for them, and
hiding your own custom claims behind a standard scope would be lanyard inventing
a rule. `offline_access` is not a filter at all: it appears in `scopes_supported`
and in the granted `scope` echoed back, and its only effect is whether the token
response carries a `refresh_token`.

**This filter applies to the ID token and to `/userinfo`. It does not apply to
the access token, and that asymmetry is deliberate.** OIDC Core §5.4 specifies
claims-per-scope for exactly those two. Nothing specifies it for an access token
— an access token is not even required to be a JWT, and its `scope` is an
authorization grant for the resource server to read, not a claims filter. So:
filtered where it is specified, unchanged where it is not.

## The ID token

Built by the same function, from the same persona→claims table, as every other
token lanyard mints.

| Claim | Value |
|---|---|
| `iss` | the configured issuer |
| `sub` | the persona id |
| `aud` | **the `client_id`** — not the API audience |
| `iat`, `exp` | now, and now + **300** |
| `auth_time` | when the persona was picked, which may predate `iat` on a remembered session |
| `nonce` | echoed when the request sent one |
| `at_hash`, `c_hash` | base64url of the leftmost 128 bits of `SHA-256` of the access token and of the code |
| persona claims | filtered by scope, per the table above |

**The ID token lives 300 seconds while the access token still lives 60.** They
are not the same kind of thing. The access token is a credential and its short
life is the point — a 60-second access token that gets rejected is lanyard
working. The ID token is an authentication receipt, consumed once at login and
then exchanged for your app's own cookie; a 60-second one makes `oidc-client-ts`
consider the user expired seconds after signing in. It also carries `nbf` and
`jti`, because every lanyard token does; both are legal registered claims and
stripping them would mean a second claim path.

## `/oidc/userinfo`

`GET` or `POST`, with `Authorization: Bearer <access_token>`. Returns the same
scope-filtered persona claims the ID token carried, `sub` always present, and
none of `iss` / `aud` / `exp` / `nonce` / the hashes.

```
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:9500/oidc/userinfo | jq .
```

**This is the second place lanyard says no.** Access tokens are verified as
lanyard's own — signature, `iss` and `exp`, and nothing else, because there is no
client to check. A missing, malformed, unsigned, foreign or expired token gets
`401` with `WWW-Authenticate: Bearer error="invalid_token"`. The reason is PKCE's
reason: an app that reads `/userinfo` with an expired token has a bug, and a mock
that answers anyway hides it.

## Cross-origin requests

The `/oidc` endpoints answer a cross-origin `fetch`: the response echoes the
request's `Origin`, allows `authorization` and `content-type`, and answers
preflight. A single-page app on `http://localhost:5173` can therefore read
discovery and exchange its code without a proxy.

**Credentials are never allowed cross-origin.** A public client authenticates
with a bearer token, not with lanyard's cookie, and allowing credentialed
cross-origin requests would let any page you happen to visit drive your session.
`/_/` and the test seam answer no other origin at all.

## The token endpoint

`POST /oidc/token`, `application/x-www-form-urlencoded`. This is what the CLI
calls, and what an SDK doing client credentials will find in the discovery
document.

```
curl -X POST http://127.0.0.1:9500/oidc/token \
     -d grant_type=client_credentials -d persona=ada -d audience=billing-api
```

```json
{ "access_token": "eyJhbGciOiJSUzI1NiIs…", "token_type": "Bearer", "expires_in": 60 }
```

| Parameter | Required | Meaning |
|---|---|---|
| `grant_type` | yes | `client_credentials`, `authorization_code` or `refresh_token`. Anything else is `400 unsupported_grant_type` |
| `persona` | no | A loaded persona id. Unknown ids are a `400` naming the id |
| `audience` | no | The `aud` claim. `resource` (RFC 8707) is a synonym; `audience` wins |
| `scope` | no | Space-delimited. Becomes the `scope` claim and is echoed in the response |
| `client_id` / `client_secret` | no | Never validated. `client_id` becomes a claim when sent |
| `flaw` | no | One of `expired`, `wrong-aud`, `wrong-iss`, `bad-signature`, `alg-none`, `unknown-kid`. An unrecognized value is a `400`, not an ignored parameter |

**Client authentication is accepted in any form and checked in none.** HTTP
Basic, form parameters, or nothing at all — all four combinations mint. This is
the endpoint where every other provider would put a client registry, and it is
what lets three services on three ports share one running instance with no setup
between them.

The parameters above are the `client_credentials` grant's; `authorization_code`
takes [its own set](#the-authorization-endpoint) and `refresh_token` is below. A
`client_credentials` response carries no `id_token` — client credentials has no
user authentication event to attest to — and **never a `refresh_token`**: RFC
6749 §4.4.3 forbids it, and the reason is good, because there is no user, so
there is nothing a refresh could be on behalf of. Unknown parameters are ignored.
Errors are `400` with `{"error", "error_description"}`.

## Refresh tokens

**`offline_access` gates the refresh token exactly as `openid` gates the ID
token.** Ask for it in the authorization request and the token response carries
one; leave it out and there is no `refresh_token` key at all.

```
GET /oidc/authorize?client_id=billing-web&response_type=code
    &redirect_uri=http://localhost:5000/cb&scope=openid%20email%20offline_access
```

Auth0, Okta and Entra ID all require `offline_access`. An application that
forgets it in production gets no refresh token and breaks, and a lanyard that
handed one over unasked would hide exactly that bug. (Keycloak issues them
without it, so if that is what you are calibrated to, this is the sentence that
saves you an afternoon.)

A refresh token is **opaque and eight hours long** — an unguessable id into an
in-memory record, not a JWT. It is presented to exactly one endpoint, which has
the record, and an application that "validates" a refresh token has a bug that a
JWT-shaped one would hide. Eight hours is the one lifetime in this project not
measured in seconds, because a refresh path you cannot exercise across a lunch
break is a refresh path nobody exercises.

```
curl -X POST http://127.0.0.1:9500/oidc/token \
     -d grant_type=refresh_token -d refresh_token=$RT
```

| Parameter | Required | Meaning |
|---|---|---|
| `grant_type` | yes | `refresh_token` |
| `refresh_token` | yes | The one you were given, or the one the last refresh gave you |
| `scope` | no | May **narrow** the grant. May not widen it |
| `client_id` / `client_secret` | no | Accepted in any form, checked in none, as everywhere else |

You get a new access token — 60 seconds, minted by the same function from the
same stored persona — a new refresh token, and, when the original grant included
`openid`, a new ID token carrying the same `sub`, the same `auth_time` and **the
original `nonce`** (OIDC Core §12.2), a fresh `at_hash` over the new access
token, and no `c_hash`, because there is no code this time.

**They rotate.** Every successful refresh returns a new one and invalidates the
one you presented. That is what Auth0, Okta and Entra do for public clients, and
it is the strictly more demanding shape: an application that handles rotation
handles a static token too, and the reverse is the bug you want to find locally.

Four `invalid_grant` descriptions, told apart on purpose, because they are four
different problems:

```
that refresh token has already been exchanged; lanyard rotates refresh tokens…
that refresh token has expired; refresh tokens live eight hours…
no such refresh token; it was never issued, or lanyard was restarted since…
that refresh token has been revoked, either at /oidc/revoke or by logging out…
```

Narrowing succeeds and the response echoes the narrowed `scope`; widening is
`400 invalid_scope` **naming the offending value** (RFC 6749 §6). A refusal
costs you nothing — the refresh token you presented still works, because it is
only spent once nothing is left to refuse.

## Introspection and revocation

```
curl -X POST http://127.0.0.1:9500/oidc/introspect -d token=$TOKEN
curl -X POST http://127.0.0.1:9500/oidc/revoke     -d token=$TOKEN
```

`/oidc/introspect` (RFC 7662) always answers `200 application/json`. A live
access token comes back with `active: true` and `sub`, `client_id`, `aud`,
`scope`, `iss`, `exp`, `iat` and `jti` — only the fields the token actually
carried. **Everything else is exactly `{"active": false}` and nothing more**: a
token signed by another issuer, a string that is not a JWT, an empty `token`, a
revoked token and an expired one are one answer, because RFC 7662 §2.2 says an
inactive response reveals no other fields. Which kind of nothing it was is a
question for the request log, not for this endpoint.

`/oidc/revoke` (RFC 7009) always answers `200` with an **empty body**, including
for a token that was never issued or is not a JWT — §2.2 requires exactly that,
so a client cannot use it to find out whether a token exists.

- A **refresh token** is dropped, and the next refresh with it names revocation.
- An **access token** has its `jti` remembered until the moment the token would
  have expired anyway, and then forgotten. Access tokens stay stateless JWTs;
  this is the only way revoking one can mean anything, and the set is bounded by
  the number of unexpired tokens.
- **No cascade.** Revoking a refresh token does not revoke access tokens issued
  from it, and revoking an access token does not touch the refresh token. RFC
  7009 §2.1 says a server MAY cascade; lanyard does not track the linkage, and
  inventing one to support a MAY is how a dev tool grows a subsystem nobody asked
  for.

**`/oidc/userinfo` honours revocation** — a revoked access token gets `401` there
too. `/introspect` saying `active: false` while `/userinfo` handed over claims
would be lanyard disagreeing with itself.

**Neither endpoint authenticates the client, and RFC 7662 §2.1 says
introspection MUST.** This is a deliberate divergence, it is [the same one
`/oidc/token` already ships](#the-token-endpoint), and the mitigation is the same
one: lanyard binds to loopback, and anyone who can reach the port can mint
anything anyway. An introspection endpoint demanding a credential lanyard does
not check would be theatre with extra steps.

## The test seam

`POST /_/api/token` mints a token directly, with no browser and no redirects.
**The body is the claims. The query string is how they are minted.**

```
curl -sX POST http://127.0.0.1:9500/_/api/token \
     -H 'content-type: application/json' \
     -d '{"sub":"ada","aud":"billing-api"}'
```

```json
{ "token": "eyJhbGciOiJSUzI1NiIs…", "claims": { "sub": "ada", "…": "…" } }
```

| Query parameter | Default | Meaning |
|---|---|---|
| `persona` | none | Mint a persona's claims: `?persona=ada` |
| `ttl` | `60` | Lifetime in seconds; `exp - iat` |
| `flaw` | none | One of the six. Echoed back as `"flaw"`, and an unrecognized value is a `400`, not an ignored parameter |
| `client_id` | none | Which personas `persona=` may name. Absent behaves as a `client_id` nobody scoped to: the unscoped set |

Body claims override persona claims, and override the registered claims too —
`iss` and `exp` included — so you can mint a deliberately wrong token:

```
curl -sX POST 'http://127.0.0.1:9500/_/api/token?persona=ada' \
     -H 'content-type: application/json' -d '{"iss":"http://evil.test"}'
```

`GET /_/api/personas` lists what is loaded. **With `?client_id=`, the visible
set; without it, everything** — every persona from every source, each row
carrying the `client:` that actually applies to it and the `source` file it came
from. A `warnings` array rides on both forms, so a test can assert on a broken
project file without scraping HTML.

```
curl -s 'http://127.0.0.1:9500/_/api/personas?client_id=billing-web' | jq '[.personas[].id]'
curl -s http://127.0.0.1:9500/_/api/personas | jq .warnings
```

An unknown persona, a body that is not a JSON object, or a non-numeric `ttl`
returns `400` with `{"error", "error_description"}`.

## `lanyard doctor`

Six checks, one line each, in the order a request travels. Anything that is not
`OK` is followed by an indented **consequence sentence** and, where there is one,
the command that fixes it — a check that says "port mismatch" and stops has moved
you one step; a check that says what will be rejected has finished the job.

```
$ lanyard doctor
lanyard doctor
  Config       OK    issuer http://127.0.0.1:9500/oidc, bind 127.0.0.1:9500
  Reachable    OK    http://127.0.0.1:9500 answered in 0ms (lanyard 0.1.0)
  Issuer       OK    the running server agrees with the address I dialled
  JWKS         OK    http://127.0.0.1:9500/oidc/jwks, 1 key, kid TXntCt2b…
  Clock        OK    0.000s apart
  Signing key  OK    /home/you/.local/share/lanyard/signing-key.pem, 0600, the built-in default key — stable across a wiped data dir
```

| Check | What it does |
|---|---|
| **Config** | Resolves the environment exactly as `serve` does. `FAIL` on anything `serve` would refuse to start with |
| **Reachable** | `GET /_/health` at `LANYARD_URL`. `FAIL` — nothing is listening, and everything below it is skipped rather than guessed at |
| **Issuer** | Compares the discovery document's `issuer` against the authority `doctor` dialled, and reports every other name the server has been reached by |
| **JWKS** | Fetches `jwks_uri` **as advertised**, not as configured, and compares its `kid` to the live one |
| **Clock** | `/_/health`'s `now` against `doctor`'s, minus half the round trip. `WARN` above 2s |
| **Signing key** | The file, its mode, whether it is the built-in default, and whether its `kid` is the one being served |

**The exit-code contract**, because a container health probe depends on it:

- **`0`** — every check `OK` or `WARN`. A warning may well be deliberate:
  `LANYARD_ISSUER=http://lanyard:9500/oidc` looks like a mismatch from your shell
  and is exactly right for a container network.
- **non-zero** — any `FAIL`. Today that is a config `serve` would refuse and a
  server that is not answering.
- **`--strict`** promotes every `WARN` to a `FAIL`, for CI.
- **`--quiet`** prints nothing and only sets the exit code. This is what the
  image's `HEALTHCHECK` runs.

`doctor` needs no running server to be useful: with nothing listening, `Config`
and `Signing key` still report, `Reachable` fails naming the exact address it
tried, and the rest are skipped. It never writes anything and never edits your
configuration — it diagnoses and prints the command.

### `GET /_/health`

Liveness, and only liveness. `200` whenever the process is serving, whatever
`doctor` thinks, because an application that waits on lanyard must start even
when the issuer is misconfigured — a misconfigured issuer is exactly the state
you are trying to debug.

```
$ curl -s http://127.0.0.1:9500/_/health | jq .
{
  "hosts_seen": [],
  "issuer": "http://127.0.0.1:9500/oidc",
  "kid": "TXntCt2biz2Bj578hZocZOb2A2nQV9JfrBvFN55QWpU",
  "now": 1788710020030,
  "skew": 0,
  "status": "ok",
  "version": "0.1.0"
}
```

`now` is unix milliseconds, and it is the only reason this returns a body at all:
it is what makes clock skew measurable between two machines without either of
them consulting a third. The endpoint emits no log event — a probe every five
seconds would drown the live request log.

### The `Host` mismatch, at request time

When a request arrives with a `Host` that is not the issuer's authority, lanyard
says so **once per distinct host** for the life of the process:

```
warning: reached as "lanyard:9500" but the issuer is "http://127.0.0.1:9500/oidc"
 — a token minted here says iss=http://127.0.0.1:9500/oidc and a relying party
 that dialled lanyard:9500 will reject it. Fix with:
 LANYARD_ISSUER=http://lanyard:9500/oidc lanyard serve
```

Once per host, not once per request: discovery and JWKS get polled, and a warning
that repeats is a warning that gets filtered out. It reaches three surfaces from
one source — the event stream (so `lanyard logs --json` and `/_/log` carry it), a
band on `/_/`, and `hosts_seen` in `/_/health` so `doctor` reports it from
outside.

`localhost:9500` against an issuer of `127.0.0.1:9500` **is** a mismatch and gets
the warning. One rule, no exemptions to remember: a conforming relying party
compares `iss` as a string, and that one genuinely breaks — it is the single most
common way this fails, and in .NET it surfaces as `IDX10205`.

Health probes and event-stream connections are not counted, and neither is
`lanyard doctor` — it dials whatever address you pointed it at, on purpose, and
reports the mismatch itself.

## Running in a container

```
$ podman run --rm -p 9500:9500 ghcr.io/robap/lanyard
```

`ghcr.io/robap/lanyard` is a multi-arch manifest — `linux/amd64` and
`linux/arm64`, both built natively — tagged with every released version and
`latest`. Its binary is the one that release published, not a second compile of
it. Building the image from a checkout still works and is the same Dockerfile:

```
$ podman build --format docker -t localhost/lanyard:dev .
```

The image is `gcr.io/distroless/static-debian12:nonroot` plus one statically
linked binary — no shell, no curl, no package manager, about 10 MB. It sets
`LANYARD_BIND=0.0.0.0` and `LANYARD_DATA_DIR=/data` as image defaults, because
the bind address is a property of the deployment and the image *is* the
deployment. The native binary keeps its loopback default.

Its `HEALTHCHECK` is `["/lanyard", "doctor", "--quiet"]`. There is nothing else
in the image to run one with, which is what makes the exit-code contract above
load-bearing rather than tidy.

> **`--format docker` is not optional.** `podman build` defaults to the OCI image
> format, which has no `HEALTHCHECK`: the instruction is dropped *silently*, and
> `podman inspect --format '{{.State.Health.Status}}'` is then simply absent,
> which reads as a broken health check rather than a missing one.

### One name that resolves from both sides

This is the whole configuration, and it is the answer to almost every container
gotcha at once:

```
$ podman network create lanyard-net
$ podman run -d --name lanyard --network lanyard-net -p 9500:9500 \
    -e LANYARD_ISSUER=http://lanyard:9500/oidc ghcr.io/robap/lanyard
$ echo '127.0.0.1 lanyard' | sudo tee -a /etc/hosts
```

`http://lanyard:9500/oidc` now resolves to the same lanyard from your browser,
from your shell, and from any container on `lanyard-net` — one string, one
issuer, no mismatch from any side. `lanyard doctor` verifies it end to end.

> **The `/etc/hosts` line has a sting in the tail, and it is podman's.** Podman
> copies the host's `/etc/hosts` into every container it starts, so
> `127.0.0.1 lanyard` lands *inside* your application container too — where it
> shadows aardvark-dns and resolves `lanyard` to the container itself. The name
> then works from your browser and your shell and fails from the one place the
> network was created for, with a connection refused that looks like the IdP
> being down.
>
> Start containers that need to reach lanyard by name with **`--no-hosts`**:
>
> ```
> podman run --rm --no-hosts --network lanyard-net your-app
> ```
>
> The container gets only its network's own resolution, which is what you wanted.
> Docker does not copy the host file, so this is podman-specific.

For a lanyard running natively on the host with your application in a container,
the alternative is podman's **`host.containers.internal`** (Docker's
`host.docker.internal`; native Linux Docker also needs
`--add-host=host.docker.internal:host-gateway`). `doctor` prints whichever name
matches the runtime it detects when the issuer's host does not resolve.

### The port is inside the issuer

`-p 9500:8080` publishes the container's 8080 as your 9500, and the issuer still
says 8080. A browser arrives with `Host: localhost:9500`, gets a token that says
`iss=http://127.0.0.1:8080/oidc`, and every relying party that dialled `:9500`
rejects it. `doctor` reports it from the other side, naming both ports:

```
  Issuer       WARN  the server says 127.0.0.1:8080, I dialled localhost:9500
                     A token minted here says iss=http://127.0.0.1:8080/oidc, and a relying party that dialled localhost:9500 compares that string and rejects it.
                     The port is inside the issuer: published as 9500, but the issuer says 8080.
                     If localhost:9500 is the name everything should use: LANYARD_ISSUER=http://localhost:9500/oidc lanyard serve
```

### You do not need a volume

The default signing key is deterministic and compiled into the binary, so a fresh
container with no volume produces the same `kid` and the same public key every
time — the same one the native binary serves. No volume is declared and none is
needed. A volume is for someone who has replaced the key with their own.

If you do mount one, ownership is the thing that bites, and it bites in opposite
directions on the two runtimes. Rootless podman maps the container's `root` to
you and every other uid into a subuid range, so a directory you own appears
inside the container as `root`'s and the non-root process cannot write it:

```
$ podman run -v ./lanyard-data:/data ghcr.io/robap/lanyard
lanyard: /data is not writable by uid 65532 (this process is in a container).
  A bind-mounted data directory needs its ownership mapped. Try:
      podman run -v ./lanyard-data:/data:U …    # podman, rootless
      docker run --user $(id -u):$(id -g)  …    # docker
  Or drop the volume entirely: the default signing key is deterministic, so a
  container with no volume already produces the same kid every time.
```

`:U` makes the directory writable by chowning it to the mapped subuid — which is
not you, so you will not be able to read the key back from your shell. To have
the file end up owned by *you* at mode `0600`, run as yourself:

```
$ podman run --userns=keep-id --user $(id -u):$(id -g) \
    -v ./lanyard-data:/data ghcr.io/robap/lanyard
```

[`scripts/phase08-container.sh`](scripts/phase08-container.sh) drives all of the
above and asserts each result.

## Troubleshooting

**.NET: `Exception: Correlation failed.`** — the app is being served from an
origin that is not a secure context, so the browser dropped ASP.NET Core's
correlation cookie. .NET sets that cookie `SameSite=None`, and `SameSite=None`
*requires* `Secure`, and a `Secure` cookie is silently discarded over plain HTTP
on a host that is not `localhost`. It is not lanyard rejecting anything: the
authorization response came back fine and .NET could not find its own cookie to
match it against.

Serve your app from `http://localhost:<port>` or `http://127.0.0.1:<port>`, both
of which browsers treat as secure contexts. `http://web.localtest.me:5000` looks
equivalent and is not — which is the other reason lanyard rejects that host.

**.NET: the picker never appears, and you get
`unsupported_response_type` instead.** `AddOpenIdConnect`'s default
`ResponseType` is `id_token`, which is the implicit flow. lanyard implements the
authorization code flow only, and says so in the error:

```csharp
options.ResponseType = "code";
```

lanyard's `error_description` names that setting, but only when the requested
`response_type` actually contained `id_token` — which is the one case where it is
the diagnosis rather than a guess about which stack is calling.

**.NET: a 60-second token is still accepted six minutes later.** See
[`ClockSkew`](#read-this-before-you-use-it) above.

**A SPA's silent renew fails with
`ErrorResponse: prompt=none was sent and no lanyard_session cookie arrived with
the request…`** — in `oidc-client-ts` that arrives as
`[node-spa] renew failed: <that sentence>` from `addSilentRenewError`, and in the
network tab as a `GET /oidc/authorize?…&prompt=none` with `Sec-Fetch-Dest:
iframe`, **no `Cookie` header**, and a `302` carrying `error=login_required`.

Your app and lanyard are being dialled by two different host names. `SameSite`
ignores the port but not the host, so `http://localhost:5173` is cross-site to an
issuer on `http://127.0.0.1:9500` and the `Lax` session cookie does not travel.
Use one name on both sides — see
[Silent renew](#silent-renew-and-the-one-rule-that-decides-whether-it-works).
This is the shipped default's behaviour, not a misconfiguration you introduced.

**A silent renew times out with no network response at all.** No `postMessage`,
no error, just `silentRequestTimeoutInSeconds` elapsing. That is almost always a
`redirect_uri` lanyard **refused**: [the one rejection](#the-one-rejection)
renders a `400` page rather than redirecting, RFC 6749 §4.1.2.1 forbids sending
an error to an address just declined, and inside a hidden iframe that page is
invisible. Nothing can be done about it without making the one rejection
decorative — so **the reason is in [`lanyard logs`](#the-live-request-log)**,
which will name the host it refused. Check that your `silent_redirect_uri` is on
`localhost` or `127.0.0.1`.

**"It asks me to pick a persona every single time."** Either "always ask" is on
in the picker, or your app is sending `prompt=login`, or the cookie is not coming
back. lanyard's cookie is deliberately never `Secure`; if you have put a proxy in
front of lanyard that rewrites cookies, that is where to look.

**A relying party rejects the token with an issuer mismatch.** Set
`LANYARD_ISSUER` to the name your application actually dials. See
[Read this before you use it](#read-this-before-you-use-it) — and run
[`lanyard doctor`](#lanyard-doctor), which names both sides and prints the line
that fixes it.

**Anything else about containers, ports, names or clocks.** Run
[`lanyard doctor`](#lanyard-doctor). It is one command whose output can be pasted
into an issue, and it exists because knowing *which* of these six things went
wrong is the entire problem.

## Verifying a token

Tokens are checked by a real JWT library rather than by lanyard agreeing with
itself. `scripts/` holds a small [`jose`](https://github.com/panva/jose) harness
that resolves the JWKS over HTTP exactly as a relying party would:

```
cd scripts
npm install
node jose-verify.mjs "$TOKEN" http://127.0.0.1:9500/oidc billing-api
```

Node is a verification dependency only. lanyard itself is a single static binary
with no runtime to install.

## License

MIT
