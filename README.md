# lanyard

A local OIDC provider with nothing to configure.

`lanyard serve` starts an OpenID Connect provider on `127.0.0.1:9500` with a
working discovery document, a stable signing key and three ready-made users. No
realm to create, no client to register, no admin console to visit.

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

`POST /_/api/token` compounds this: it mints any claims anyone asks for, with no
authentication at all. That is the entire point of a test seam, and it is why
the default bind is loopback. `LANYARD_BIND=0.0.0.0` turns that off, so do it
only on a network you control.

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

**The discovery document lists only the endpoints that exist.** Today `jwks_uri`
is the only endpoint in it; there is no `authorization_endpoint` yet, because
advertising an endpoint that returns 404 sends a client down a path that cannot
work. The document grows each phase.

**Scope.** This is Phase 1. There is a discovery document, a JWKS, and the test
seam below. There is no `/authorize`, no `/oidc/token`, no browser login, no web
UI and no `lanyard token` CLI command yet.

## Configuration

Every setting is an environment variable read once at startup. All are optional.

| Variable | Default | Meaning |
|---|---|---|
| `LANYARD_ISSUER` | `http://127.0.0.1:{port}/oidc` | The `iss` claim and the discovery document's `issuer` |
| `LANYARD_BIND` | `127.0.0.1` | Listen address |
| `LANYARD_PORT` | `9500` | Listen port |
| `LANYARD_DATA_DIR` | `$XDG_DATA_HOME/lanyard` | Signing key lives here |
| `LANYARD_PERSONAS` | `$XDG_CONFIG_HOME/lanyard/users.yaml` | Persona file; when set, it must exist |

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
