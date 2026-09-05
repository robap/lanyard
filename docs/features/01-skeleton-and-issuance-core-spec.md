# Skeleton, keys, and the issuance core — spec

**Status:** done · **Roadmap:** Phase 1 · **Slug:** 01-skeleton-and-issuance-core

## Why

`lanyard serve` starts, says truthfully where it is, and mints a signed token
that a real JWT library accepts against a real JWKS endpoint — with no browser,
no client registration, and no configuration.

This is the first phase with product code, and it is the phase that decides the
internal structure everything else leans on:

- **North star 3 — issuance is one function.** The browser flow (Phase 4), the
  CLI (Phase 2), `/oidc/token`, and the test seam must all be thin callers of a
  single persona+overrides→signed-token function. CONCEPT §6 says to build the
  test seam **in week one, not as a later addition**, precisely because it is
  what forces that shape: if the seam is the first caller, the browser flow
  arrives as a second caller rather than as the original home of the claim
  logic. Claim-building in three places that drift apart is the failure mode
  this phase exists to prevent.
- **North star 4 — real tokens.** RS256 from a real JWKS endpoint, verified by
  node `jose` rather than by our own code agreeing with itself.
- **North star 2 — one instance, every project.** One fixed port, one issuer
  string resolved once at startup, printed. CONCEPT §8's central rule — *do not
  derive the issuer per-request from the `Host` header* — is a structural
  decision, not a later hardening step, and the Phase 0 notes on
  navikt/mock-oauth2-server ([notes](../notes/navikt-mock-oauth2-server.md) §1)
  record what happens to a tool that gets it wrong: two README sections exist
  only to work around it.
- **CONCEPT §15 / Phase 7 insurance.** The persona file carries a `client:`
  field from day one. Nothing reads it until Phase 7; adding it later is a
  migration.

Phase 0 settled the surrounding facts: HTTPS is
[post-v1](../decisions/https-priority.md), the name stands
([name-check.md](../decisions/name-check.md)), and crates.io is
[not a channel](../decisions/crates-io-reservation.md) — so the crate/binary
split (`lanyard-cli` → `lanyard`) is free to set up now and costs a rename later.

## In scope

- **`lanyard serve`** — a plain foreground process. No forking, no pidfile, no
  daemon mode (CONCEPT §7).
- **Port 9500, fixed.** Fail loudly on conflict, never fall back to another port.
- **`LANYARD_BIND`** — bind address, defaulting to loopback (`127.0.0.1`).
- **`LANYARD_ISSUER`** — the one issuer string, resolved once at startup,
  defaulting to `http://127.0.0.1:9500/oidc`. Never derived from `Host`.
- **Startup banner** printing the resolved issuer, the listen address, the data
  dir and signing-key `kid`, and where personas came from.
- **Signing key.** RS256. A deterministic default dev key — same `kid` and same
  public key on every fresh install with no data dir — persisted to the data dir
  on first run, and read from there afterwards. A user-supplied key in that file
  is used instead.
- **Data dir**, defaulting to `$XDG_DATA_HOME/lanyard` (`~/.local/share/lanyard`),
  overridable via `LANYARD_DATA_DIR`, carrying its own `.gitignore`.
- **`GET /oidc/.well-known/openid-configuration`** and **`GET /oidc/jwks`**.
- **The persona type** — stable id, email, display name, roles, arbitrary
  attributes. No `sub`, no `preferred_username`, no `claims` key (CONCEPT §3).
- **`client:` in the persona file schema**, parsed and validated, read by nothing.
- **Personas loaded from `~/.config/lanyard/users.yaml`**, with built-in defaults
  when the file is absent — including the **no-roles / no-claims persona**.
- **`POST /_/api/token`** — the test seam. Post claims, get a signed token.
- **`GET /_/api/personas`** — the JSON seam the Phase 4 picker will read. In
  scope here because it is the only way to *observe* that persona loading works
  at all in this phase; without it, personas are unobservable code.
- **Routing settled** — `/oidc/…`, `/_/…`, root left free (404).
- **Crate `lanyard-cli`, binary `lanyard`** (CONCEPT §14).

## Out of scope

- **Any `/oidc` endpoint beyond discovery and JWKS.** No `/authorize`, no
  `/token`, no `/userinfo`. Those are Phases 4 and 5, and the discovery document
  does not advertise them until they exist.
- **The CLI's `token` / `env` subcommands** — Phase 2. `serve` is the only
  subcommand this phase ships.
- **The deliberate failure flags** (`--expired`, `--alg-none`, …) — Phase 3.
  This spec fixes the seam's *extension point* for them and nothing more.
- **Any HTML UI at `/_/`** — Phase 4. The banner must not print a URL that 404s.
- **The live request log, SSE, ndjson** — Phase 6. Ordinary per-request stdout
  logging is fine and deliberately unspecified here.
- **Persona namespacing, `lanyard link`, project `lanyard.yaml`** — Phase 7. The
  `client:` field is carried, never consulted.
- **`lanyard doctor`, `/_/health`, the `Host`-mismatch warning, clock-skew
  leeway, the container image** — Phase 8.
- **CORS** — Phase 5.
- **HTTPS, `lanyard trust`** — post-v1, per
  [https-priority.md](../decisions/https-priority.md).
- **Claim profiles** (Entra/Auth0/Cognito shapes) and **opaque access tokens** —
  post-v1. This phase emits one plain claim shape.
- **Service install** (`launchd`/`systemd` units) — post-v1. `serve` is a
  foreground process someone runs in a terminal.
- **Refresh tokens, sessions, cookies.** Nothing in this phase sets a cookie.

## Behavior

### The issuer is `http://127.0.0.1:9500/oidc`

The roadmap puts discovery at `/oidc/.well-known/openid-configuration`, and OIDC
Discovery 1.0 defines that path as *issuer* + `/.well-known/openid-configuration`.
Those two facts together fix the issuer at `http://127.0.0.1:9500/oidc` — path
segment included. This is consistent with the roadmap's own Phase 2 criterion,
which configures .NET with `Authority=http://127.0.0.1:9500/oidc`.

It differs from the illustrative banner in CONCEPT §13, which shows
`Issuer → http://127.0.0.1:9500`. That snippet predates the routing decision in
CONCEPT §3; the `/oidc` mount is the deliberate choice and the banner example is
the casualty. **CONCEPT §13's banner should be corrected** rather than the
routing changed — .NET compares `iss` against the discovery document's `issuer`
by exact string ([dotnet-http-settings.md](../decisions/dotnet-http-settings.md)),
so a discovery document served somewhere other than issuer + `/.well-known/…` is
a trap laid for the exact stack we care most about.

Host `127.0.0.1` rather than `localhost` is deliberate and matches the roadmap's
acceptance criteria. `localhost` may resolve to `::1`, and the default bind is
`127.0.0.1` — an issuer nothing is listening on is the worst possible default.

Consequence to state plainly in the README later: a browser that reaches lanyard
at `http://localhost:9500` still gets a discovery document saying `127.0.0.1`.
That is the intended behavior — one issuer, resolved once — and it is why
`LANYARD_ISSUER` exists. Phase 8 adds the warning when an inbound `Host` and the
configured issuer disagree; this phase only guarantees the issuer does not move.

### Discovery advertises only what exists

The document is honest, not aspirational. Phase 1 emits `issuer`, `jwks_uri`,
`response_types_supported`, `subject_types_supported`, and
`id_token_signing_alg_values_supported` (`["RS256"]`). `authorization_endpoint`,
`token_endpoint`, `userinfo_endpoint`, `end_session_endpoint`,
`introspection_endpoint`, and `revocation_endpoint` appear in the phase that
implements them.

This makes the document intentionally non-conforming until Phase 4 —
`authorization_endpoint` is required by OIDC Discovery 1.0. That is the right
trade: a document listing endpoints that 404 sends a client down a path that
fails later and further away. Phase 2's only discovery consumer is .NET's
`AddJwtBearer`, which needs `issuer` and `jwks_uri`. If it turns out to reject a
document missing `authorization_endpoint`, that is a Phase 2 finding and the
fields get added then, with the observation recorded.

`GET /oidc/jwks` returns a JWKS with exactly one RSA public key —
`kty`/`use`/`alg`/`kid`/`n`/`e`, no private material — served `Cache-Control:
no-store`, because Phase 3's `--unknown-kid` and any future rotation are both
made confusing by a cached JWKS.

### The signing key is a known constant, on purpose

CONCEPT §8 asks for two things at once: persist the keypair, *and* have a fresh
install with no data dir produce the same `kid` and public key every time.

The default key is therefore a **fixed RSA-2048 private key compiled into the
binary** — not a key generated from a seeded RNG. Seeded generation is
deterministic only as long as the RSA keygen implementation never changes its
candidate search, which is not a promise any crate makes across versions; a
"stable" key that silently moves on a dependency bump is worse than no promise
at all.

On startup:

1. If `<data-dir>/signing-key.pem` exists, load it. It is the authority.
2. If it does not, write the built-in default key there (PKCS#8 PEM, mode
   `0600`), creating the data dir and its `.gitignore` first, then load it.

One load path, one file, and dropping in your own RSA key just works. `kid` is
the RFC 7638 JWK thumbprint of the key, so a replaced key gets a correct `kid`
with nothing to configure.

**The default key is publicly known.** Anyone with the binary can forge a token
lanyard's JWKS will validate. That is correct for a development IdP and it is the
same bargain as accepting any `client_id`, but it belongs in the README beside
"never expose this to a network you do not trust", not left implicit. The same
goes for the seam below: `POST /_/api/token` mints a token for any claims anyone
asks for, which is exactly why the default bind is loopback.

If the file exists but is not a readable RSA private key, `serve` exits non-zero
naming the file — it does not silently fall back to the default, because a token
signed by a different key than the operator thinks is the single most confusing
failure this tool can produce.

### Port and bind

Port 9500. On `EADDRINUSE`, print the address that is taken and exit non-zero;
never try 9501. A machine-wide service that sometimes lands elsewhere defeats its
own purpose, because the issuer string is baked into every project's config
(CONCEPT §7).

`LANYARD_BIND` sets the bind *host* only (`0.0.0.0` in the container image, per
[navikt notes](../notes/navikt-mock-oauth2-server.md) §4 — copy the knob, keep
loopback as the default). It does not affect the issuer.

`LANYARD_PORT` exists but is documented as "you almost certainly should not set
this". It moves the listen port and, when `LANYARD_ISSUER` is unset, the default
issuer moves with it. It exists so Phase 8 can reproduce the `-p 9500:8080`
misconfiguration that `doctor` is supposed to diagnose.

### The test seam: `POST /_/api/token`

**The body is the claims. The query string is how they are minted.** That split
is the whole design, and it is what keeps Phase 3 additive:

```
POST /_/api/token?persona=ada&ttl=300
Content-Type: application/json

{"aud": "billing-api", "scope": "orders:read orders:write"}
```

- **Body** — a JSON object of claims, merged *over* the persona's claims. May be
  empty or absent. Standard claims are overridable, including `iss`, `exp`, and
  `iat`; that is deliberate, and it is what makes Phase 3's `--expired` and
  `--wrong-iss` thin sugar over this endpoint rather than a separate code path.
- **Query** — `persona` (optional; the persona id), `ttl` (optional; seconds,
  default 60). Phase 3 adds `flaw=alg-none`, `flaw=bad-signature`,
  `flaw=unknown-kid` here — the ones that are not expressible as claims.
- With no `persona`, the body alone is the claim set. The roadmap's
  `{"sub":"ada","aud":"billing-api"}` works verbatim.

Response `200 application/json`:

```json
{ "token": "eyJhbGciOiJSUzI1NiIs…", "claims": { "…the exact payload signed…" } }
```

Echoing the signed claims costs nothing and turns the seam into something you can
debug with `curl` alone, which is the point of a seam.

Errors are `400 application/json` with `{"error": "...", "error_description": "..."}`:
unknown persona (naming the id), malformed JSON, a body that is not an object, a
non-numeric `ttl`.

### Persona → claims

The persona type is protocol-neutral (CONCEPT §3). The mapping happens at issue
time, inside the OIDC module:

| Persona field | Claim |
|---|---|
| `id` | `sub`, and `preferred_username` |
| `email` | `email`, plus `email_verified: true` — both omitted when absent |
| `name` | `name` — omitted when absent |
| `roles` | `roles` (array) — omitted when empty |
| `attributes` | merged in as top-level claims |

Always added, before body overrides: `iss` (the resolved issuer), `iat`, `nbf`
(= `iat`), `exp` (= `iat` + ttl), `jti`. `aud` is emitted only if asked for —
the seam mints exactly what you request, and an `aud` string that matches no real
API is a test case, not an error (CONCEPT §5).

Access tokens are **60 seconds by default**. Long-lived dev tokens mean the
refresh path never runs locally (CONCEPT §6). The roadmap's acceptance for that
lifetime being *real* lives in Phase 2; this phase only has to default it.

### Personas: file, schema, and defaults

`~/.config/lanyard/users.yaml` (honouring `$XDG_CONFIG_HOME`), overridable with
`LANYARD_PERSONAS`:

```yaml
client: billing-web          # optional, file-level. Reserved for Phase 7.
personas:
  - id: ada
    name: Ada Bell
    email: ada@example.test
    roles: [admin, user]
    attributes:
      department: platform
  - id: ops
    name: Ops Bot
    client: ops-console      # optional, per-persona; overrides the file-level one
```

`client:` is accepted at **both** levels and echoed back by `/_/api/personas`, so
that whichever shape Phase 7 wants, it is already there. The roadmap says the
field must exist from day one; it does not say at which level, and supporting
both now is cheaper than guessing and migrating.

**A malformed file is fatal.** Unknown keys, a duplicate `id`, a missing `id` →
`serve` exits non-zero naming the offending key and the file path. Falling back
to defaults with a warning is worse: you believe your personas loaded, and the
symptom appears three redirects later as a persona that is not on the picker.
`attributes` is the escape hatch for anything the schema does not name, so
rejecting unknown keys costs nobody anything.

When the file is absent, three built-in personas ship:

| id | name | email | roles |
|---|---|---|---|
| `ada` | Ada Bell | `ada@example.test` | `admin`, `user` |
| `mira` | Mira Okonkwo | `mira@example.test` | `user` |
| `nobody` | Nobody | — | — |

`nobody` is the important one — CONCEPT §3 calls it out specifically. A token for
`nobody` carries `sub` and the registered claims and nothing else: no `email`, no
`name`, no `roles`. That is the user that breaks applications and the one nobody
remembers to create.

The banner says which of the two sources was used and, when it is the file, its
path. "Why is Ada not there" should be answerable from the first eight lines of
output.

### One function, not yet observable

North star 3 says the browser flow, `/oidc/token`, the CLI, and the seam are all
thin callers of one issuance function. In this phase there is exactly one caller,
so no acceptance criterion below can prove it — the observable version is Phase 2
and Phase 3, where the CLI and the seam must produce equivalent tokens for
equivalent input. Stating it here anyway because the plan has to honour it now:
the structural cost of getting this wrong is not paid until Phase 4, and by then
it is expensive.

## Acceptance criteria

Data dir is `$LANYARD_DATA_DIR`; personas file is `$LANYARD_PERSONAS`. Both are
set to throwaway paths so these can be run without touching the operator's real
`~/.config`.

**Serving and the issuer**

- [x] `lanyard serve` prints a banner, and
      `curl -s http://127.0.0.1:9500/oidc/.well-known/openid-configuration | jq -r .issuer`
      prints `http://127.0.0.1:9500/oidc` — byte-identical to the issuer line in
      the banner.
- [x] `curl -s "$(curl -s …/openid-configuration | jq -r .jwks_uri)"` returns 200
      with exactly one key, `"kty":"RSA"`, `"alg":"RS256"`, a `kid`, and no `d`
      or other private field anywhere in the body.
- [x] `curl -s -H 'Host: lanyard:9500' http://127.0.0.1:9500/oidc/.well-known/openid-configuration | jq -r .issuer`
      still prints `http://127.0.0.1:9500/oidc`. The issuer does not follow `Host`.
- [x] `LANYARD_ISSUER=http://lanyard:9500/oidc lanyard serve` → the banner and the
      discovery document both show that string, and `jwks_uri` is
      `http://lanyard:9500/oidc/jwks`.
- [x] `curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:9500/` → `404`.
      Root is free.
- [x] Starting `lanyard serve` while another is running: the second exits
      non-zero (`echo $?`), and its stderr names `127.0.0.1:9500`. Nothing ends
      up listening on 9501 (`ss -ltn | grep 9501` is empty).
- [x] Default run: `ss -ltn | grep 9500` shows `127.0.0.1:9500`. With
      `LANYARD_BIND=0.0.0.0`, the same command shows `0.0.0.0:9500`.

**Keys**

- [x] `rm -rf "$LANYARD_DATA_DIR"`, restart, refetch `/oidc/jwks` → the `kid` and
      `n` are byte-identical to the values captured before the deletion
      (`diff` of the two saved JSON bodies is empty).
- [x] After a first run: `<data-dir>/signing-key.pem` exists,
      `stat -c %a` on it prints `600`, and `cat <data-dir>/.gitignore` shows the
      data dir ignoring itself.
- [x] Replacing `signing-key.pem` with `printf 'not a key\n'` and restarting →
      `serve` exits non-zero and stderr names the file path. `/oidc/jwks` is not
      served by a process that fell back to the default key.

**The seam**

- [x] `curl -sX POST http://127.0.0.1:9500/_/api/token -H 'content-type: application/json' -d '{"sub":"ada","aud":"billing-api"}'`
      returns a token that a node script accepts:
      `jwtVerify(token, createRemoteJWKSet(new URL(jwks_uri)), {issuer, audience:'billing-api'})`
      resolves without throwing, and the returned payload's `sub` is `ada`.
      Verified against the **live JWKS URL over HTTP**, not a locally pasted key.
- [x] The same verified payload has `exp - iat === 60`. With `?ttl=300`, it is
      `300`.
- [x] `curl -sX POST '…/_/api/token?persona=ada'` → a token that `jose` verifies,
      whose payload has `sub: "ada"`, `email: "ada@example.test"`,
      `email_verified: true`, and `roles` containing `admin`.
- [x] `curl -sX POST '…/_/api/token?persona=nobody'` → a token that `jose`
      verifies, whose payload has `sub: "nobody"` and **no** `email`, `name`, or
      `roles` keys at all.
- [x] `curl -sX POST '…/_/api/token?persona=ada' -d '{"email":"other@example.test"}'`
      → the verified payload's `email` is `other@example.test`. Body claims win
      over persona claims.
- [x] `curl -sX POST '…/_/api/token?persona=nope'` → HTTP 400, and the response
      body names `nope`.
- [x] `curl -sX POST '…/_/api/token' -d 'not json'` → HTTP 400 with a JSON
      `error` / `error_description` body, and the server is still serving
      afterwards.

**Personas**

- [x] With no personas file: `curl -s …/_/api/personas` lists exactly `ada`,
      `mira`, and `nobody`; `nobody` has no `email` and an empty `roles`. The
      banner says the built-in defaults were used.
- [x] With a personas file containing one persona: `/_/api/personas` returns that
      one and none of the built-ins, and the banner prints the file's path.
- [x] A personas file using `client:` at the file level and `client:` on one
      persona loads cleanly, and `/_/api/personas` echoes both back. Nothing
      filters on it — every persona is still listed.
- [x] A personas file with `rolez: [admin]` on a persona → `serve` exits non-zero
      and stderr names both `rolez` and the file path.
- [x] A personas file with two personas sharing `id: ada` → `serve` exits
      non-zero and stderr names `ada`.

**Shape of the artifact**

- [x] `cargo build --release` produces a binary named `lanyard` from a crate
      named `lanyard-cli`; `./target/release/lanyard --help` lists `serve`.
- [x] `ldd` on the release binary shows no dependency on Node, a JVM, or a
      runtime package manager (north star 5). Dynamic libc is fine at this stage.

## Open questions

None blocking. Four decisions are made in **Behavior** above that go slightly
beyond what the roadmap states — flagged here because they are the ones worth
disagreeing with before a plan is written, not after:

1. **The issuer carries the `/oidc` path segment**, which means CONCEPT §13's
   example banner (`Issuer → http://127.0.0.1:9500`) is wrong and should be
   corrected. Everything else follows from the roadmap's own Phase 2 criterion.
2. **The default signing key is a constant compiled into the binary**, not a
   seeded generation — and it is therefore publicly known. Deliberate, and it
   needs a README line rather than a code comment.
3. **`GET /_/api/personas` is added** beyond the roadmap's Phase 1 list, because
   persona loading is otherwise unobservable until Phase 4. It is the same JSON
   seam the picker will consume.
4. **`client:` is accepted at both file and persona level.** The roadmap requires
   the field from day one but does not say where; supporting both costs a few
   lines now and removes the migration risk the bullet exists to prevent.

Two things are deferred rather than open: **macOS config paths** (this spec uses
XDG everywhere, because the roadmap names `~/.config/lanyard/users.yaml`
explicitly; revisit at Phase 10 when macOS is actually built for), and whether
discovery must list `authorization_endpoint` before Phase 4 — which Phase 2's
.NET client will answer by either accepting the document or not.
