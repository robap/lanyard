# lanyard

A local OIDC provider with nothing to configure.

`lanyard serve` starts an OpenID Connect provider on `127.0.0.1:9500` with a
working discovery document, a stable signing key and three ready-made users. No
realm to create, no client to register, no admin console to visit.
`lanyard token --as ada --aud billing-api` prints a bearer token you can paste
into a `curl`.

## Build and run

```
cargo build --release
./target/release/lanyard serve
```

```
lanyard 0.1.0
  Issuer    → http://127.0.0.1:9500/oidc
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
`jwks_uri` and `token_endpoint`; there is no `authorization_endpoint` yet,
because advertising an endpoint that returns 404 sends a client down a path that
cannot work. The document grows each phase. (.NET's `AddJwtBearer` consumes a
document with no `authorization_endpoint` without complaint — observed, not
assumed.)

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

**Scope.** This is Phase 2. There is a discovery document, a JWKS, the
`client_credentials` half of `/oidc/token`, the `token` and `env` CLI commands,
and the test seam. There is no `/authorize`, no `authorization_code` grant, no
`/userinfo`, no browser login and no web UI yet — and no deliberately-wrong
tokens (`--expired`, `--wrong-aud`, …) yet either.

## Configuration

Every setting is an environment variable read once at startup. All are optional.

| Variable | Default | Meaning |
|---|---|---|
| `LANYARD_ISSUER` | `http://127.0.0.1:{port}/oidc` | The `iss` claim and the discovery document's `issuer` |
| `LANYARD_BIND` | `127.0.0.1` | Listen address |
| `LANYARD_PORT` | `9500` | Listen port |
| `LANYARD_DATA_DIR` | `$XDG_DATA_HOME/lanyard` | Signing key lives here |
| `LANYARD_PERSONAS` | `$XDG_CONFIG_HOME/lanyard/users.yaml` | Persona file; when set, it must exist |
| `LANYARD_URL` | `http://127.0.0.1:{port}` | Where `lanyard token` and `lanyard env` reach the server |

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
client: billing-web          # optional, file-level. Reserved for a later phase.
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

`client:` is accepted at both levels, echoed back by `/_/api/personas`, and read
by nothing yet.

## Minting a token from the command line

Probably the larger half of daily use, and the shorter path to being useful:
there is no redirect dance to get right.

```
$ lanyard token --as ada --aud billing-api
eyJhbGciOiJSUzI1NiIsImtpZCI6...

$ curl -H "Authorization: Bearer $(lanyard token --as ada --aud billing-api)" \
       localhost:8080/orders
```

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

The CLI does not sign locally. It performs a real `client_credentials` grant
against `/oidc/token`, so the token arrives the way a production token arrives —
through the endpoint an SDK would use. Tokens are 60 seconds and the TTL is not
configurable.

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
| `grant_type` | yes | Must be `client_credentials`. Anything else is `400 unsupported_grant_type` |
| `persona` | no | A loaded persona id. Unknown ids are a `400` naming the id |
| `audience` | no | The `aud` claim. `resource` (RFC 8707) is a synonym; `audience` wins |
| `scope` | no | Space-delimited. Becomes the `scope` claim and is echoed in the response |
| `client_id` / `client_secret` | no | Never validated. `client_id` becomes a claim when sent |

**Client authentication is accepted in any form and checked in none.** HTTP
Basic, form parameters, or nothing at all — all four combinations mint. This is
the endpoint where every other provider would put a client registry, and it is
what lets three services on three ports share one running instance with no setup
between them.

There is no `id_token` (client credentials has no user authentication event to
attest to) and no `refresh_token`. Unknown parameters are ignored. Errors are
`400` with `{"error", "error_description"}`.

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

Body claims override persona claims, and override the registered claims too —
`iss` and `exp` included — so you can mint a deliberately wrong token:

```
curl -sX POST 'http://127.0.0.1:9500/_/api/token?persona=ada' \
     -H 'content-type: application/json' -d '{"iss":"http://evil.test"}'
```

`GET /_/api/personas` lists what is loaded. An unknown persona, a body that is
not a JSON object, or a non-numeric `ttl` returns `400` with
`{"error", "error_description"}`.

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
