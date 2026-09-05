# CLI token minting — plan

**Spec:** [02-cli-token-minting-spec.md](02-cli-token-minting-spec.md) · **Roadmap:** Phase 2 · **Status:** done

## Approach

Two halves that meet in the middle. Server-side, `POST /oidc/token` becomes the
**second caller** of `oidc::issue` — a handler that parses a form, maps three
parameters into an overrides map, and calls the function Phase 1 built. It builds
no claims itself; if it does, north star 3 is already lost and step 13's
equivalence test is what fails. Client-side, `lanyard token` and `lanyard env`
are thin wrappers over one `client::mint()` that performs a real
`client_credentials` grant over HTTP, because a CLI that signed locally would be
a second issuance path within one release (north star 4, and offline signing is
post-v1 for exactly this reason).

Three shapes are settled deliberately:

- **The form is parsed by hand** (`serde_urlencoded` on raw `Bytes`), not via
  axum's `Form` extractor, for the same reason `seam::parse_claims` hand-rolls
  its JSON: every 4xx on this endpoint must be ours and carry the OAuth
  `{"error","error_description"}` shape, rather than axum's 415/422 for a missing
  content-type.
- **The grant is dispatched on `grant_type` with one arm.** Phase 4 adds
  `authorization_code` as a second arm; Phase 3 adds a `flaw` parameter that
  never touches this dispatch.
- **The CLI's target is `LANYARD_URL`, resolved separately from `Config`.**
  `Config::resolve` needs `HOME` and would fail a CLI invocation that has no
  business knowing where the data dir is. `client::resolve_url` is a four-line
  function of the environment plus `--url`, and it is *not* the issuer —
  CONCEPT §8's Docker issuer (`http://lanyard:9500/oidc`) resolves nowhere useful
  from a shell.

`reqwest` graduates from dev-dependency to dependency, `default-features = false`
so no TLS stack is linked (north star 5 stays intact; `LANYARD_URL` is http-only
until HTTPS lands post-v1).

## Files

- `Cargo.toml` — `reqwest` moves to `[dependencies]` (`default-features = false`,
  `features = ["json"]`); add `serde_urlencoded = "0.7"`
- `src/oidc/token.rs` — **new.** The `/oidc/token` handler: form parsing, grant
  dispatch, client-auth extraction, the OAuth response and error shapes
- `src/oidc/routes.rs` — mount `POST /token`; discovery grows `token_endpoint`,
  `grant_types_supported`, `token_endpoint_auth_methods_supported`
- `src/oidc/mod.rs` — declare `token`
- `src/config.rs` — `Config::token_endpoint()` beside `jwks_uri()`
- `src/client.rs` — **new.** `resolve_url()`, `MintRequest`, `mint()`, and the
  error type whose `Display` is what the user reads on stderr
- `src/lib.rs` — declare `client`
- `src/main.rs` — `token` and `env` subcommands; the stdout/stderr discipline
- `tests/http.rs` — token-endpoint cases, discovery fields, and the seam↔grant
  equivalence test
- `tests/cli.rs` — **new.** Drives the real binary (`CARGO_BIN_EXE_lanyard`)
  against a router on port 0: exact stdout, empty stdout on failure, exit codes
- `spikes/dotnet-api/` — **new.** The `AddJwtBearer` acceptance client
- `docs/decisions/dotnet-jwt-bearer-settings.md` — **new.** Minimal settings and
  the clock-skew finding
- `README.md` — the CLI section, the token endpoint, `LANYARD_URL`, the clock
  skew sharp edge, and a corrected scope paragraph

## Risks & unknowns

- **.NET's `ClockSkew` defaults to five minutes.** The single most likely way
  this phase produces a false pass: criterion 8 waits 90 seconds and gets a 200,
  and somebody "fixes" lanyard's TTL. The harness must expose skew as a runtime
  knob (`CLOCK_SKEW=zero|default`) so both numbers are observed from one build,
  in the subtraction style of Phase 0's `DROP=` harness.
- **`AddJwtBearer` may reject the discovery document** for lacking
  `authorization_endpoint` — Phase 1 left this open. If it does, add the field in
  this phase pointing at `<issuer>/authorize` and record the observation in the
  decision note. Do not work around it by setting `MetadataAddress` or by
  hand-configuring `TokenValidationParameters`: the point of criterion 7 is that
  a stock RP consumes our document.
- **`aud` is a JSON string here, and RPs differ on string-vs-array.** .NET
  accepts both. Keep the string; if any client objects, that is a finding, not a
  quiet change.
- **`Form` vs raw bytes.** If the hand-parse route drifts from what curl sends
  (`-d` implies `application/x-www-form-urlencoded`), the symptom is a confusing
  400. Test the no-content-type case explicitly.
- **`reqwest` without TLS means `LANYARD_URL=https://…` fails at runtime**, with
  a reqwest error rather than a lanyard one. Acceptable while HTTPS is post-v1;
  the error should still name the URL.
- **Nothing may print to stdout on the CLI paths except the token.** A stray
  `println!` — a banner, a warning, a debug line — silently poisons
  `$(lanyard token …)` and every criterion still passes except a real curl.
- **`spikes/dotnet-api/` outlives this phase.** Phase 3 names it. `bin/` and
  `obj/` must be gitignored the way the Phase 0 spikes are.
- **Acceptance takes 90+ seconds of wall clock** by construction. Do not
  substitute a shorter wait and a shorter TTL — the 60-second default being real
  is the thing under test.

## Steps

Each box ≈ one small commit moving an observable behavior. Check only when the
outcome is real, not when code is written.

- [x] **`POST /oidc/token`, client credentials** — new `oidc::token` module,
      mounted at `/oidc/token`. Hand-parse the urlencoded body, dispatch on
      `grant_type`, and for `client_credentials` call `oidc::issue` with the
      `persona` and `DEFAULT_TTL`. `curl -X POST …/oidc/token -d grant_type=client_credentials -d persona=ada`
      returns `{"access_token":…,"token_type":"Bearer","expires_in":60}` and
      `scripts/jose-verify.mjs` verifies it against the live JWKS with
      `sub: "ada"`. **North star 3** — the handler maps parameters to an overrides
      map and calls the one function; no claim is constructed in this file
- [x] **Any client authentication, or none** — form `client_id`/`client_secret`,
      HTTP Basic, or nothing at all, all mint. `client_id` (form first, then the
      Basic username) is emitted as a claim when present. All four curl shapes
      return 200. **North star 1** — this is the endpoint where every other IdP
      would put a registry
- [x] **`audience`, `resource`, `scope`, and the response envelope** —
      `audience` (or `resource`; `audience` wins) becomes the `aud` claim, `scope`
      becomes the `scope` claim and is echoed in the response body, and the
      response carries `Cache-Control: no-store`. No `aud` key at all when none
      was asked for; no `id_token` and no `refresh_token` ever
- [x] **The endpoint's error shapes** — `grant_type=password` → `400`
      `unsupported_grant_type`; missing or empty `grant_type` → `400`
      `invalid_request`; `persona=nope` → `400` naming `nope`; a body that is not
      a form, whatever its content-type, → `400` in the same
      `{"error","error_description"}` shape; `GET /oidc/token` → `405`. Unknown
      parameters are ignored, which is where Phase 3's `flaw=` will land
- [x] **Discovery advertises the endpoint** — `token_endpoint`,
      `grant_types_supported: ["client_credentials"]`,
      `token_endpoint_auth_methods_supported: ["none","client_secret_basic","client_secret_post"]`.
      `authorization_endpoint` stays absent — the document still lists only what
      exists
- [x] **`client::resolve_url`** — `--url`, then `LANYARD_URL`, then
      `http://127.0.0.1:{LANYARD_PORT or 9500}`; trailing slash trimmed. Unit
      tests over an injected env lookup, the same pattern as `Config::resolve`.
      Deliberately independent of `LANYARD_ISSUER`: an address is not a string in
      a token (CONCEPT §8)
- [x] **`client::mint`** — posts the `client_credentials` form with
      `client_id=lanyard-cli`, returns the `access_token`. Its error type
      distinguishes *nothing listening* (names the URL and suggests
      `lanyard serve`), *the server said 400* (surfaces `error_description`
      verbatim, so the unknown-persona id reaches the user), and *unexpected
      status*
- [x] **`lanyard token --as … [--aud …] [--scope …] [--url …]`** — the token and
      a newline on stdout, nothing else, exit 0. `--as` is required; clap's usage
      error covers its absence. `lanyard token --as ada --aud billing-api > t 2> e`
      leaves `e` empty and `t` a single three-segment line
- [x] **Failure never reaches stdout** — with no server running, `lanyard token`
      exits non-zero, writes 0 bytes to stdout, and names the URL and
      `lanyard serve` on stderr. Same discipline for an unknown persona. This is
      what keeps `curl -H "Bearer $(lanyard token …)"` from sending an error
      message as a credential
- [x] **`lanyard env`** — prints `export BEARER_TOKEN='<token>'`, single-quoted,
      and prints nothing at all on stdout when minting fails, so `eval "$(…)"` of
      a failure is a no-op that leaves the shell untouched.
      `eval "$(lanyard env --as ada --aud billing-api)"` sets the variable in the
      calling shell
- [x] **Tests** — `tests/http.rs` gains the endpoint cases above (happy path,
      four auth shapes, grant errors, unknown persona, discovery fields, scope
      and audience mapping); new `tests/cli.rs` spawns the router on port 0 and
      runs the real binary with `LANYARD_URL` pointed at it, asserting exact
      stdout, empty stdout on failure, and exit codes. `cargo test` green
- [x] **The equivalence test** — one test mints through `/_/api/token?persona=ada`
      with `{"aud":"billing-api"}` and through the grant with the same inputs, and
      asserts the claim maps are equal after removing `iat`, `nbf`, `exp`, `jti`
      and `client_id`. **North star 3, finally falsifiable** — Phase 1 could only
      assert this in prose
- [x] **The .NET acceptance harness** — `spikes/dotnet-api/`: `AddJwtBearer` with
      `Authority = http://127.0.0.1:9500/oidc`, `RequireHttpsMetadata = false`,
      `Audience = billing-api`, one `/orders` endpoint with
      `RequireAuthorization()`, served on `http://127.0.0.1:5080`. A `DROP=`
      subtraction knob as in Phase 0, plus `CLOCK_SKEW=zero|default`.
      `bin/`/`obj/` gitignored. `curl -H "Authorization: Bearer $(lanyard token --as ada --aud billing-api)" …/orders`
      → 200, and no token → 401
- [x] **The clock-skew observation, written down** — with `CLOCK_SKEW=zero` the
      90-second call is 401; with the .NET default it is 200. Both, plus the
      minimal-settings subtraction table and whether `AddJwtBearer` accepted a
      discovery document without `authorization_endpoint`, go into
      `docs/decisions/dotnet-jwt-bearer-settings.md` in the style of
      [dotnet-http-settings.md](../decisions/dotnet-http-settings.md)
- [x] **Docs** — `README.md` gains: `lanyard token` / `lanyard env` above the test
      seam (the CLI is the larger half of daily use, so it goes first);
      `POST /oidc/token` with its parameter table; `LANYARD_URL` in the
      configuration table with a sentence on why it is not `LANYARD_ISSUER`; and
      two sharp edges — **any `aud` mints, so rejection happens at the API, not
      here**, and **a 60-second token stays valid for ~6 minutes against a stock
      .NET API** because `ClockSkew` defaults to five minutes, with the one-line
      fix. Correct the Phase 1 scope paragraph, which currently promises no
      `/oidc/token` and no `lanyard token`

## Acceptance

Mirrors the spec. Every run sets `LANYARD_DATA_DIR` and a throwaway
`XDG_CONFIG_HOME`; `jose` means `scripts/jose-verify.mjs` against the live JWKS;
the .NET harness listens on `http://127.0.0.1:5080`.

**The token endpoint**

- [x] 1. `curl -si -X POST …/oidc/token -d grant_type=client_credentials -d persona=ada -d audience=billing-api`
      → `200`, `Cache-Control: no-store`, `token_type: "Bearer"`, `expires_in: 60`,
      no `id_token`, no `refresh_token`; the `access_token` verifies with `jose`
      for `{issuer, audience: 'billing-api'}` with `sub: "ada"` and
      `email: "ada@example.test"`
- [x] 2. The same request with no client auth, with `-u 'anything:whatever'`,
      with `-d client_id=x -d client_secret=y`, and with `-d client_id=x` alone —
      all four `200`
- [x] 3. `-d grant_type=password` → `400` `"unsupported_grant_type"`; no
      `grant_type` → `400` `"invalid_request"`
- [x] 4. `-d persona=nope` → `400` naming `nope`, and the server still serves
- [x] 5. `curl -s …/openid-configuration | jq -r '.token_endpoint, .grant_types_supported[]'`
      → `http://127.0.0.1:9500/oidc/token` and `client_credentials`;
      `.authorization_endpoint` is `null`
- [x] 6. `GET /oidc/token` → `405`

**The CLI against a real resource server**

- [x] 7. `curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $(lanyard token --as ada --aud billing-api)" http://127.0.0.1:5080/orders`
      → `200`; the harness's startup log shows it consumed the discovery document
      without complaint
- [x] 8. The same token 90 seconds later, `CLOCK_SKEW=zero` → `401`, and
      `WWW-Authenticate` says the token expired
- [x] 9. The same, with .NET's default `ClockSkew` → `200`. Both recorded in
      `docs/decisions/dotnet-jwt-bearer-settings.md`
- [x] 10. `lanyard token --as ada --aud billing-api > /tmp/t 2>/tmp/e; echo $?` →
      `0`, `/tmp/e` empty, `/tmp/t` one line of three dot-separated base64url
      segments
- [x] 11. `eval "$(lanyard env --as ada --aud billing-api)"`, then
      `echo "$BEARER_TOKEN" | cut -d. -f2 | base64 -d` shows `"sub":"ada"`
- [x] 12. `lanyard token --as ada --aud not-a-real-api` exits `0` and prints a
      token; that token against `/orders` → `401`
- [x] 13. `lanyard token --as ada` → a token `jose` verifies with `{issuer}`
      alone, with no `aud` key in the payload
- [x] 14. `--scope 'orders:read orders:write'` → the verified payload's `scope`
      is that exact string and the endpoint response's `scope` matches

**One function, finally observable**

- [x] 15. Claims from `lanyard token --as ada --aud billing-api` and from
      `curl -sX POST '…/_/api/token?persona=ada' -d '{"aud":"billing-api"}'`,
      decoded and passed through `jq -S 'del(.iat,.nbf,.exp,.jti,.client_id)'`,
      `diff` to empty

**Failure modes**

- [x] 16. No lanyard running: `lanyard token --as ada > /tmp/t 2>/tmp/e` exits
      non-zero, `/tmp/t` is 0 bytes, `/tmp/e` names `http://127.0.0.1:9500` and
      `lanyard serve`
- [x] 17. `eval "$(lanyard env --as nope)"` leaves `BEARER_TOKEN` unset
      (`[ -z "${BEARER_TOKEN+x}" ]`), errors on stderr naming `nope`, shell still
      usable
- [x] 18. Server on `LANYARD_PORT=9600`:
      `LANYARD_URL=http://127.0.0.1:9600 lanyard token --as ada --aud billing-api`
      → a token; without `LANYARD_URL` the same command fails per 16
- [x] 19. `lanyard --help` lists `serve`, `token`, `env`; `lanyard token --help`
      lists `--as`, `--aud`, `--scope`, `--url`; `lanyard token` with no `--as`
      exits non-zero with a usage error

## Progress notes

- **The content-type header is not consulted, and a missing one is a `200`, not
  a `400`.** The box read "arrives with no content-type → 400"; the Approach's
  stated intent was that every 4xx be *ours* rather than axum's 415. Ignoring
  the header satisfies that intent and is the only reading consistent with north
  star 1 — rejecting a well-formed body over its label would be the single
  registration-shaped "no" on the endpoint whose point is that it has none. The
  spec's Behavior section never mentions content-type. Tested both ways in
  `the_body_is_read_without_consulting_the_content_type`.
- **"A body that is not valid urlencoded" turned out to be unreachable, so
  `Form::parse` is infallible.** Measured, not assumed: `serde_urlencoded`
  decoding into `Vec<(String, String)>` is lossy on invalid UTF-8 and lenient
  about stray `%` escapes, so no byte string fails to parse. A body that is not
  a form arrives as unknown parameters and falls out as the ordinary
  `invalid_request` for a missing `grant_type` — the same shape the box asked
  for, reached by the honest route rather than a dead error arm.
- **`reqwest` needed its `form` feature**, not just `json`, for the CLI's grant
  post and the tests' form posts. Still `default-features = false`, so no TLS
  stack is linked.
- **The message for "nothing listening" reads `nothing listening at <url>` rather
  than the spec's illustrative `no lanyard at <url>`.** `main` prefixes every
  error with `lanyard: `, and "lanyard: no lanyard at …" stutters. Both things
  the criterion requires — the URL that was tried and `lanyard serve` — are
  present either way.
- **`spikes/dotnet-api/` is committed; the Phase 0 spikes stay ignored.**
  `.gitignore` had `spikes/` wholesale, which would have left Phase 3 pointing at
  a path that does not exist in a fresh checkout. Now `spikes/*` with a negation
  for `spikes/dotnet-api/`, and `bin/`/`obj/` under it still ignored.
- **`main.rs` reads 0% in `cargo llvm-cov`.** `tests/cli.rs` drives the real
  binary as a subprocess, so its coverage is not attributed to the parent run.
  The behavior is covered — exact stdout, empty stdout on failure, exit codes —
  just not counted.
