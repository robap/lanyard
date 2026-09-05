# CLI token minting — spec

**Status:** done · **Roadmap:** Phase 2 · **Slug:** 02-cli-token-minting

## Why

`lanyard token --as ada --aud billing-api` prints a token on stdout, and
`curl -H "Authorization: Bearer $(…)"` against a real resource server returns
200. That is the shorter path to being useful — no redirect dance to get right —
and CONCEPT §5 expects it to be the larger half of daily use.

- **North star 3 — issuance is one function.** Phase 1 shipped `issue()` with a
  single caller, which means the "one function" claim is currently unfalsifiable.
  This phase adds the second caller (`POST /oidc/token`) and the first
  *out-of-process* one (the CLI), so equivalence becomes observable: the same
  request through the seam and through the grant must produce the same claims.
  That criterion is the point of this phase's structure, not a nicety.
- **North star 4 — real tokens.** The CLI does not sign locally and does not
  call the seam. It performs a real `client_credentials` grant against
  `/oidc/token`, so the token arrives the way a production token arrives —
  through an endpoint an SDK would use, with an OAuth-shaped response around it.
  Offline signing is [post-v1](../../ROADMAP.md) precisely because a second
  signing path would let the two drift.
- **North star 1 — accept everything.** The token endpoint is the first place a
  client presents credentials, so it is the first place that could grow a
  registry. It must not. Any `client_id`, any `client_secret`, no client
  authentication at all — all four are 200. Any `--aud`, including one no API
  expects: that is a test case, not an error (CONCEPT §5).
- **CONCEPT §6 — short lifetimes by default.** 60 seconds. Phase 1 defaulted the
  TTL; this phase is where "the TTL is real" gets *watched*, against a resource
  server that actually rejects an expired token.

## In scope

- **`POST /oidc/token`** with `grant_type=client_credentials`. New server
  surface, and the reason this phase is not purely a CLI phase.
- **Discovery grows** `token_endpoint`, `grant_types_supported`, and
  `token_endpoint_auth_methods_supported` — the document stays honest, so it
  advertises the endpoint in the phase that implements it.
- **`lanyard token --as <persona> [--aud <s>] [--scope <s>]`** → the token on
  stdout, nothing else.
- **`lanyard env --as … [--aud …] [--scope …]`** → `export BEARER_TOKEN=…`.
- **`LANYARD_URL` / `--url`** — where the CLI reaches lanyard. Deliberately *not*
  the issuer.
- **A .NET minimal API using `AddJwtBearer`** as the acceptance client, kept for
  Phase 3 to point its six failure flags at.
- **`docs/decisions/dotnet-jwt-bearer-settings.md`** — the resource-server
  counterpart to Phase 0's [dotnet-http-settings.md](../decisions/dotnet-http-settings.md),
  recording the minimal `AddJwtBearer` settings and the clock-skew finding below.

## Out of scope

- **`authorization_code`, `refresh_token`, PKCE, `/userinfo`, cookies,
  sessions.** Phases 4 and 5. `/oidc/token` ships here dispatching on
  `grant_type` with exactly one arm, so Phase 4 adds an arm rather than a
  handler.
- **The six failure flags** (`--expired`, `--wrong-aud`, `--alg-none`, …) —
  Phase 3. This spec fixes where they will attach (an extra form parameter
  alongside `persona`) and mints nothing wrong itself.
- **`--ttl`.** 60 seconds, not configurable. `--ttl 30d` for Postman collections
  is on the roadmap's *explicitly deferred, possibly forever* list.
- **`lanyard proxy`** — post-v1, and the thing that makes short TTLs bearable
  interactively. Its absence is meant to be felt.
- **Offline signing** (CLI signs from the on-disk key with no server running) —
  post-v1. `lanyard token` with no server is an error, not a fallback.
- **`id_token` from this endpoint.** `client_credentials` has no user
  authentication event, so returning one would be a lie an SDK might believe.
- **Refresh tokens, `expires_in` negotiation, token storage, keychain.** None.
- **Claim profiles** (`scp` vs `scope`, Entra shapes) and **opaque access
  tokens** — post-v1. One plain claim shape.
- **The request log** — Phase 6. An ordinary stdout line per request is fine.
- **CORS on `/oidc/token`** — Phase 5. The CLI is not a browser.
- **`examples/dotnet-api/`** as a published, CI-run example — post-v1 under
  "more examples". The API built here is an acceptance harness (see Behavior).

## Behavior

### `POST /oidc/token` — client credentials, no registration

`application/x-www-form-urlencoded`. `POST` only; `GET /oidc/token` is `405`.

| Parameter | Required | Meaning |
|---|---|---|
| `grant_type` | yes | Must be `client_credentials`. Anything else → `400 unsupported_grant_type`. |
| `persona` | no | A loaded persona id. Unknown id → `400`, naming the id. |
| `audience` | no | The `aud` claim. `resource` (RFC 8707) is accepted as a synonym; `audience` wins if both are sent. |
| `scope` | no | Space-delimited. Echoed to the `scope` claim and the response. |
| `client_id` / `client_secret` | no | Never validated. |

Client authentication is accepted in **any** form and checked in none: HTTP Basic,
form parameters, or entirely absent. All four combinations mint. That is north
star 1 arriving at the endpoint where every other IdP would put a client
registry, and it is what produces the multi-project property.

Unknown parameters are ignored, per OAuth's own rule for unrecognized request
parameters. This is the extension point: Phase 3 adds `flaw=alg-none` and friends
here, and Phase 4 adds `code`/`code_verifier`/`redirect_uri`, both without
touching the contract above.

Success is `200 application/json`, `Cache-Control: no-store`:

```json
{ "access_token": "eyJhbGciOiJSUzI1NiIs…", "token_type": "Bearer", "expires_in": 60, "scope": "orders:read" }
```

`scope` is present only when one was requested. There is no `id_token` and no
`refresh_token`. Errors are `400 application/json` in the OAuth shape already
used by the seam: `{"error": "...", "error_description": "..."}`, with
`invalid_request` for a missing or malformed `grant_type`,
`unsupported_grant_type` for a grant this phase does not implement, and
`invalid_request` naming the id for an unknown persona.

**The handler builds no claims.** It maps form parameters to an overrides map
(`aud`, `scope`, `client_id`) and calls `issue()` with the persona and the
default TTL — the same function, with the same persona→claims table, that the
seam calls. Any claim shaping that appears in this handler is the drift north
star 3 exists to prevent, and acceptance criterion 15 is what catches it.

Two claims come from the request rather than the persona:

- **`aud`** — only when asked for. A token with no `aud` is legitimate and useful:
  it is what an API that forgets to validate audience will happily accept.
- **`client_id`** — emitted when the request supplied one, matching what a real
  `client_credentials` token carries and giving Phase 6's log and Phase 7's
  namespacing something honest to key on. The CLI sends `lanyard-cli`.

`sub` remains the persona id. With no `persona`, the token carries no user
claims at all — the machine-to-machine shape.

**This endpoint mints any token for anyone who can reach the port.** So does the
seam, and the default signing key is published in this repo. Same bargain,
already stated in Phase 1: the boundary is the loopback default, and it belongs
in the README rather than in a comment.

### Discovery grows three fields

`token_endpoint` (`<issuer>/token`), `grant_types_supported`
(`["client_credentials"]`), and `token_endpoint_auth_methods_supported`
(`["none", "client_secret_basic", "client_secret_post"]` — all three are true,
because none of them are checked). `authorization_endpoint` stays absent until
Phase 4.

Phase 1 left an open question here: whether .NET's `AddJwtBearer` rejects a
discovery document without `authorization_endpoint`. **Acceptance criterion 7
answers it.** If it refuses, the missing fields get added in this phase and the
observation is recorded in the decision note — not pre-emptively, and not
silently.

### The CLI reaches lanyard at an address, not at an issuer

`LANYARD_URL`, or `--url`, defaulting to `http://127.0.0.1:9500`. The CLI then
posts to `<url>/oidc/token`.

This is deliberately a *different* knob from `LANYARD_ISSUER`. In the Docker case
CONCEPT §8 describes, the issuer is `http://lanyard:9500/oidc` — a name that
resolves inside the container network and nowhere else. A CLI that derived its
target from the issuer would be unreachable in exactly the setup the issuer
setting exists to support. `--url` is an address; `LANYARD_ISSUER` is a string in
a token.

For the same reason the CLI does **not** read the discovery document to find the
token endpoint. It is the same binary that serves it; a round trip to learn its
own route buys nothing and adds a failure mode.

### `lanyard token` — one line on stdout, or nothing

Success: the token and a trailing newline on stdout. Nothing else, ever —
no banner, no timing, no "minted for Ada". `$(…)` strips the newline and the
result goes straight into a header.

Failure: **stdout stays empty**, the message goes to stderr, exit is non-zero.
`curl -H "Authorization: Bearer $(lanyard token …)"` with a failure must send an
empty bearer, not an error message, because a 401 with a garbage token is a
better outcome than an API receiving the words "connection refused" as
credentials.

Two failures are worth wording carefully, since they are the ones every new user
hits:

- **Nothing listening** → name the URL that was tried and the fix:
  ``no lanyard at http://127.0.0.1:9500 — is `lanyard serve` running?``
- **Unknown persona** → the server's `error_description`, which names the id.

`--as` is required. A persona-less token is reachable through the endpoint and
the seam; the CLI's job is the persona case, and a required flag beats a silent
`sub`-less token that fails validation three layers away.

### `lanyard env` — the same token, shaped for `eval`

```
$ lanyard env --as ada --aud billing-api
export BEARER_TOKEN='eyJhbGciOiJSUzI1NiIs…'
```

Single-quoted. A JWT is base64url and dots, so quoting is belt-and-braces — but
`eval` is running whatever we print, and this line is the one place lanyard hands
a shell something to execute.

The variable name is fixed at `BEARER_TOKEN` (roadmap). On failure it prints
**nothing** on stdout — `eval "$(…)"` of an empty string is a no-op, so a failed
mint leaves the shell exactly as it was, with the error visible on stderr and a
non-zero status available to `$?`.

### The acceptance client: a .NET minimal API

`AddJwtBearer` with `Authority = "http://127.0.0.1:9500/oidc"`,
`RequireHttpsMetadata = false`, `Audience = "billing-api"`, one authorized
endpoint. Lives in **`spikes/dotnet-api/`**, beside the Phase 0 spikes: it exists
to be *observed against*, and Phase 3's acceptance names it directly ("Against
the Phase 2 .NET API"). `examples/dotnet-api/` is a different artifact — a
published, CI-run, deliberately boring tutorial (CONCEPT §11) — and it is post-v1.

**`ClockSkew` is the finding this phase must not miss.** .NET's
`TokenValidationParameters.ClockSkew` defaults to **five minutes**. A 60-second
lanyard token is therefore accepted by a default-configured .NET API for roughly
six minutes, and the roadmap's "the same call 90 seconds later returns 401"
criterion would quietly fail — or worse, quietly pass for the wrong reason if
somebody waits long enough.

So the harness sets `ClockSkew = TimeSpan.Zero`, and **both** behaviors get
observed and written down: 90 seconds with zero skew → 401, 90 seconds with the
default → 200. That is a genuine, non-obvious fact about the stack we care most
about, it is the resource-server half of CONCEPT §8's clock-drift discussion, and
it is README material — a developer who cannot make short-TTL rejection happen
locally will conclude lanyard's TTLs are fake.

## Acceptance criteria

Every run sets `LANYARD_DATA_DIR` and a throwaway `XDG_CONFIG_HOME` so the
operator's real dotfiles are untouched (Phase 1's note: `LANYARD_PERSONAS` cannot
point at a file that does not exist). `jose` verification means the
`scripts/jose-verify.mjs` path — against the **live JWKS URL**, not a pasted key.
The .NET harness listens on `http://127.0.0.1:5080` with `/orders` requiring
authorization.

**The token endpoint**

- [ ] 1. `curl -si -X POST http://127.0.0.1:9500/oidc/token -d grant_type=client_credentials -d persona=ada -d audience=billing-api`
      → `200`, `Cache-Control: no-store`, body with `token_type: "Bearer"`,
      `expires_in: 60`, no `id_token`, no `refresh_token`; the `access_token`
      verifies with `jose` against `{issuer, audience: 'billing-api'}` and its
      payload has `sub: "ada"` and `email: "ada@example.test"`.
- [ ] 2. The same request four ways — no client auth, `-u 'anything:whatever'`,
      `-d client_id=x -d client_secret=y`, and `-d client_id=x` alone — all
      return `200`. No registration, no rejection (north star 1).
- [ ] 3. `-d grant_type=password` → `400` with `"error":"unsupported_grant_type"`;
      omitting `grant_type` entirely → `400` with `"error":"invalid_request"`.
- [ ] 4. `-d persona=nope` → `400` whose body names `nope`, and the server is
      still serving afterwards.
- [ ] 5. `curl -s …/openid-configuration | jq -r '.token_endpoint, .grant_types_supported[]'`
      prints `http://127.0.0.1:9500/oidc/token` and `client_credentials`;
      `.authorization_endpoint` is still `null`.
- [ ] 6. `curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:9500/oidc/token`
      (GET) → `405`.

**The CLI against a real resource server**

- [ ] 7. `curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $(lanyard token --as ada --aud billing-api)" http://127.0.0.1:5080/orders`
      → `200`, against the .NET API described above. Its startup log shows it
      fetched the discovery document without complaint — which is also the
      answer to Phase 1's `authorization_endpoint` question.
- [ ] 8. The same command 90 seconds later (same token, `ClockSkew = Zero`) →
      `401`, and the response's `WWW-Authenticate` header says the token expired.
- [ ] 9. With `ClockSkew` left at its .NET default, the 90-second call returns
      `200`. Both numbers are written into
      `docs/decisions/dotnet-jwt-bearer-settings.md` along with the minimal
      `AddJwtBearer` settings, in the subtraction style of
      `dotnet-http-settings.md`.
- [ ] 10. `lanyard token --as ada --aud billing-api > /tmp/t 2>/tmp/e; echo $?`
      → `0`, `/tmp/e` is empty, and `/tmp/t` is exactly one line with three
      dot-separated base64url segments.
- [ ] 11. `eval "$(lanyard env --as ada --aud billing-api)"` in an interactive
      shell, then `echo "$BEARER_TOKEN" | cut -d. -f2 | base64 -d` shows
      `"sub":"ada"` — the variable is set **in the calling shell**.
- [ ] 12. `lanyard token --as ada --aud not-a-real-api` exits `0` and prints a
      token; that token against `/orders` from criterion 7 → `401`. Minting is
      not where the "no" happens.
- [ ] 13. `lanyard token --as ada` (no `--aud`) → a token `jose` verifies with
      `{issuer}` alone, whose payload has **no** `aud` key.
- [ ] 14. `lanyard token --as ada --aud billing-api --scope 'orders:read orders:write'`
      → the verified payload's `scope` is that exact string, and the endpoint's
      JSON response carries the same `scope`.

**One function, finally observable**

- [ ] 15. Claims from `lanyard token --as ada --aud billing-api` and from
      `curl -sX POST '…/_/api/token?persona=ada' -d '{"aud":"billing-api"}'`,
      both decoded and passed through
      `jq -S 'del(.iat,.nbf,.exp,.jti,.client_id)'`, `diff` to empty. Two
      callers, one claim set (north star 3).

**Failure modes**

- [ ] 16. With no lanyard running: `lanyard token --as ada > /tmp/t 2>/tmp/e`
      exits non-zero, `/tmp/t` is empty (0 bytes), and `/tmp/e` names
      `http://127.0.0.1:9500` and `lanyard serve`.
- [ ] 17. With lanyard running: `eval "$(lanyard env --as nope)"` leaves
      `BEARER_TOKEN` unset (`[ -z "${BEARER_TOKEN+x}" ]`), prints an error naming
      `nope` on stderr, and the shell is still usable.
- [ ] 18. Server started with `LANYARD_PORT=9600`; `LANYARD_URL=http://127.0.0.1:9600 lanyard token --as ada --aud billing-api`
      → a token. With `LANYARD_URL` unset, the same command fails per criterion
      16. The CLI's target is an address, not the issuer.
- [ ] 19. `lanyard --help` lists `serve`, `token`, and `env`; `lanyard token --help`
      lists `--as`, `--aud`, `--scope`, `--url`. `lanyard token` with no `--as`
      exits non-zero with a usage error.

## Open questions

None blocking. Five decisions in **Behavior** go beyond what the roadmap states —
listed here because these are the ones worth disagreeing with before a plan
exists, not after:

1. **`persona` is a non-standard form parameter on `/oidc/token`.** The
   alternative — overloading `client_id` as the persona — reads cleverer and
   collides head-on with Phase 7, which wants `client_id` to mean *project*.
   Extra request parameters are the boring, spec-sanctioned extension point.
2. **`client_id` is emitted as a claim** when the request supplies one, and the
   CLI supplies `lanyard-cli`. Matches real `client_credentials` tokens and gives
   Phases 6 and 7 an honest key. Costs one line; excluded from criterion 15's
   comparison.
3. **`audience` with `resource` as a synonym.** `audience` is the Auth0
   convention most developers have seen; `resource` is the standards-track one
   (RFC 8707). Accepting both is three lines and means a real SDK doing client
   credentials works whichever it sends.
4. **`--as` is required** for `token` and `env`, though the endpoint and seam
   both allow a persona-less token.
5. **The .NET harness lives in `spikes/dotnet-api/`, not `examples/`.** Phase 3
   depends on it existing, and `examples/dotnet-api/` is a post-v1 published
   artifact with different standards. If it should live somewhere more permanent
   — `harness/`, `tests/clients/` — say so now; Phase 3 inherits the path.

One thing deferred rather than open: whether `lanyard env` should support a
variable name other than `BEARER_TOKEN`. The roadmap names that variable, one
name keeps the README example short, and `--var` can be added the first time
somebody actually needs two tokens in one shell.
