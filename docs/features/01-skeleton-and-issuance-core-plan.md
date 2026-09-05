# Skeleton, keys, and the issuance core — plan

**Spec:** [01-skeleton-and-issuance-core-spec.md](01-skeleton-and-issuance-core-spec.md) · **Roadmap:** Phase 1 · **Status:** done

## Approach

Green field — there is no `Cargo.toml` in the repo yet. This plan stands up the
crate, the HTTP surface, the signing key, the persona model, and the one
issuance function, in that order, so that something is runnable after step 1 and
every later box is checked by curling a running server.

Four choices that are not obvious, each with the north star it serves:

- **Hand-write the JWS serialization; use `rsa` only for the signature.**
  Not a JWT library. Phase 3 requires a genuine `alg: none` token and a
  deliberately broken signature, and every JWT library correctly makes both
  impossible — reaching for one now means a second, divergent code path in two
  phases, which is exactly the drift **north star 3 (issuance is one function)**
  exists to prevent. A JWS is three base64url segments; the cryptography stays in
  `rsa`'s `Pkcs1v15Sign`. It also gives us the raw modulus and exponent that
  `/oidc/jwks` needs anyway.
- **Pure-Rust crypto, no OpenSSL.** `rsa` + `sha2`, not `josekit`/`openssl`.
  **North star 5 (single static binary)** — a native OpenSSL dependency is the
  thing that makes Phase 10's five-target cross-compile matrix painful.
- **Resolve config paths by hand from `XDG_*`/`HOME`, no `directories` crate.**
  The spec pins `~/.config/lanyard/users.yaml` and `~/.local/share/lanyard`;
  `directories` would return `~/Library/Application Support/…` on macOS and
  quietly contradict the spec. Six lines of our own, revisited at Phase 10 when
  macOS is actually built for.
- **The router is built by a function that does not own the listener.** `serve`
  binds 9500 and hands the router to axum; tests bind port 0. Without this, no
  integration test can run in parallel, because the whole point of **north star 2
  (one instance)** is that there is exactly one port.

Structure follows **north star 3** literally: `oidc::issue()` takes an optional
persona plus a claim override map plus a ttl and returns `(token, claims)`.
`/_/api/token` is its only caller in this phase. Phase 2's CLI, Phase 3's failure
flags, and Phase 4's browser flow are later callers — none of them may build
claims themselves.

## Files

- `Cargo.toml` — new. `[package] name = "lanyard-cli"`, `[[bin]] name = "lanyard"`
  (CONCEPT §14, [crates-io-reservation.md](../decisions/crates-io-reservation.md))
- `.gitignore` — no change needed (`target` already ignored); add `scripts/node_modules/`
- `src/main.rs` — new. `clap` derive, one subcommand: `serve`
- `src/config.rs` — new. Resolve issuer / bind / port / data dir / personas path
  from env, once, at startup
- `src/banner.rs` — new. The startup block
- `src/keys.rs` — new. Load-or-write the signing key, RFC 7638 thumbprint `kid`,
  public JWK export
- `src/default-dev-key.pem` — new, **committed and deliberately public**. The
  fixed RSA-2048 PKCS#8 key, `include_str!`d
- `src/persona.rs` — new. The `Persona` type, the YAML file schema, the loader,
  the three built-in defaults
- `src/oidc/mod.rs` — new
- `src/oidc/jws.rs` — new. base64url segments + RS256 signing
- `src/oidc/issue.rs` — new. **The one function.** persona → claims mapping lives
  here, per CONCEPT §3
- `src/oidc/routes.rs` — new. Discovery + JWKS
- `src/seam.rs` — new. `POST /_/api/token`, `GET /_/api/personas`
- `src/app.rs` — new. Router assembly, independent of the listener
- `tests/personas.rs`, `tests/issue.rs`, `tests/http.rs` — new
- `scripts/jose-verify.mjs`, `scripts/package.json` — new. The `jose` harness the
  acceptance criteria drive; reused by Phases 2, 3 and 5
- `README.md` — expand from its current two lines

## Risks & unknowns

- **`rsa` and `sha2` are version-coupled.** Stable `rsa` is 0.9.10 and it depends
  on `sha2 ^0.10.6`; `cargo add sha2` today resolves **0.11.0**, whose `digest`
  traits `rsa 0.9` will not accept. Pin `sha2 = "0.10"`. (`rsa 0.10` exists only
  as `0.10.0-rc.18` — do not use a release candidate for the signing path.)
- **`cargo audit` will flag `rsa`** — RUSTSEC-2023-0071, the Marvin timing
  side-channel. It applies to *decryption*, we only sign, and the default key is
  published in this repo on purpose. Record the decision in a comment beside the
  dependency so it is not re-litigated every phase.
- **`serde_yaml` is archived and unmaintained.** Use a maintained fork —
  `serde_yaml_ng` (0.10) or `serde_norway` (0.9.42) — and confirm during
  implementation that it supports `#[serde(deny_unknown_fields)]` and reports the
  offending key by name. The spec's "unknown key is fatal and names `rolez`"
  criterion depends on it; if the chosen crate cannot name the key, switch crates
  rather than weakening the criterion.
- **`deny_unknown_fields` does not work with `flatten`.** The tempting design —
  flatten unrecognised persona keys into `attributes` — silently swallows typos
  and makes the `rolez` criterion unsatisfiable. `attributes` must be an explicit
  map field.
- **Sign the exact bytes you emit.** The JWS signing input is the literal ASCII
  of `header.payload` as it appears in the token. Re-serializing the payload to
  sign it will produce a token that verifies locally and fails in `jose` — a
  slow, confusing bug. Build the two segments once, sign that string, append.
- **The RFC 7638 thumbprint is a hand-built string,** not `serde_json` output:
  exactly `{"e":"…","kty":"RSA","n":"…"}`, lexicographic keys, no whitespace.
  Getting it wrong still yields a *stable* `kid`, so the determinism criterion
  passes while the value is non-standard — check it against a second
  implementation (`jose`'s `calculateJwkThumbprint`) rather than against itself.
- **base64url everywhere is unpadded** (`URL_SAFE_NO_PAD`) — token segments, JWK
  `n`/`e`, and the thumbprint. One padded value breaks verification.
- **Acceptance runs must not touch the operator's real dotfiles.** Every command
  in the Acceptance section sets `LANYARD_DATA_DIR` and `LANYARD_PERSONAS` to
  throwaway paths. A run that writes a signing key into `~/.local/share/lanyard`
  and then `rm -rf`s it is a bad afternoon.
- **Port 9500 is global to the machine.** Integration tests must use the router
  builder on port 0; only the port-conflict criterion actually takes 9500, and it
  must clean up after itself.
- **Phase 2 may reject the deliberately incomplete discovery document.** The spec
  advertises only implemented endpoints, so `authorization_endpoint` is absent
  until Phase 4. If .NET's `AddJwtBearer` refuses it, that is a Phase 2 finding —
  add the fields then, with the observation recorded. Do not pre-emptively
  advertise endpoints that 404.
- **`stat -c %a` is GNU coreutils.** Fine on this machine; the criterion needs
  rewording if it is ever run on macOS.

## Steps

Each box ≈ one small commit moving an observable behavior. Check only when the
outcome is real, not when code is written.

- [x] **Crate skeleton** — `Cargo.toml` with package `lanyard-cli` and bin
      `lanyard`; `clap` with a `serve` subcommand; axum router built by
      `app::router()` and served on 9500. `cargo run -- --help` lists `serve`,
      and `curl -o /dev/null -w '%{http_code}' http://127.0.0.1:9500/` prints
      `404` — root stays free per the spec's routing rule
- [x] **Config resolution + banner** — `config.rs` reads `LANYARD_ISSUER`,
      `LANYARD_BIND`, `LANYARD_PORT`, `LANYARD_DATA_DIR`, `LANYARD_PERSONAS`
      once at startup; issuer defaults to `http://127.0.0.1:{port}/oidc`.
      `serve` prints the issuer and listen address; `LANYARD_ISSUER=…` and
      `LANYARD_BIND=0.0.0.0` both visibly change what is printed, and
      `ss -ltn` agrees with the listen line
- [x] **Fail loudly on a taken port** — `EADDRINUSE` prints the address and exits
      non-zero. Two `serve`s: the second dies, `echo $?` is non-zero, stderr says
      `127.0.0.1:9500`, and nothing is listening on 9501. **North star 2** — a
      singleton that wanders is not a singleton
- [x] **Generate and commit the default dev key** —
      `openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048` into
      `src/default-dev-key.pem`, `include_str!`d. Committed on purpose, with a
      header comment saying it is public and forgeable
- [x] **Data dir bootstrap** — on startup, create the data dir if absent, write
      `.gitignore` (`*`) if absent, write `signing-key.pem` at mode `0600` from
      the built-in constant if absent, then load from the file in all cases.
      One load path. After a first run the three paths exist and
      `stat -c %a signing-key.pem` prints `600`
- [x] **`kid` from the RFC 7638 thumbprint** — computed from the loaded key;
      banner prints the data dir and the `kid`. Cross-check the value against
      `jose`'s `calculateJwkThumbprint` before checking this box
- [x] **`GET /oidc/jwks`** — one RSA public key, `kty`/`use`/`alg`/`kid`/`n`/`e`,
      unpadded base64url, `Cache-Control: no-store`, and no private field
      anywhere in the body. `rm -rf` the data dir, restart, refetch → `kid` and
      `n` are byte-identical (**north star 4**: a rotating key silently breaks
      every cached JWKS)
- [x] **A bad key file is fatal** — `signing-key.pem` containing garbage, or a
      non-RSA key, makes `serve` exit non-zero naming the file. It must never
      fall back to the default key: a token signed by a key the operator does not
      expect is the most confusing failure this tool can produce
- [x] **`GET /oidc/.well-known/openid-configuration`** — `issuer`, `jwks_uri`,
      `response_types_supported`, `subject_types_supported`,
      `id_token_signing_alg_values_supported: ["RS256"]`, and nothing that 404s.
      `issuer` is byte-identical to the banner, `jwks_uri` fetches, and sending
      `-H 'Host: lanyard:9500'` does not change either — CONCEPT §8 and
      [navikt notes §1](../notes/navikt-mock-oauth2-server.md), the mistake we
      are deliberately not copying
- [x] **The `jose` harness** — `scripts/jose-verify.mjs` takes a token, an issuer
      and an optional audience, verifies against the live JWKS URL with
      `createRemoteJWKSet`, and prints the payload as JSON or the error name.
      `scripts/package.json` declares `jose`; `scripts/node_modules/` is
      gitignored. Node is a *verification* dependency, never a build dependency —
      **north star 5** is untouched
- [x] **JWS signer** — `oidc::jws` builds `base64url(header).base64url(payload)`,
      signs *that exact string* with `Pkcs1v15Sign::new::<Sha256>()`, appends the
      signature. Header is `{"alg":"RS256","typ":"JWT","kid":…}`
- [x] **The one issuance function** — `oidc::issue(persona, overrides, ttl)`
      returns `(token, claims)`. Adds `iss`, `iat`, `nbf`, `jti`, `exp = iat+ttl`;
      overrides win over everything, including `iss` and `exp` (this is what makes
      Phase 3 sugar rather than a second code path). **North star 3** — nothing
      outside this function may build claims
- [x] **`POST /_/api/token`, claims-body form** — body is the claim object.
      `-d '{"sub":"ada","aud":"billing-api"}'` returns
      `{"token":…,"claims":{…}}`, and `scripts/jose-verify.mjs` resolves it
      against the live JWKS URL with `sub: "ada"`
- [x] **`?ttl=` and the 60-second default** — verified payload has
      `exp - iat === 60`, and `300` with `?ttl=300`. Query is *how* it is minted,
      body is *what* is in it; Phase 3's `?flaw=` slots in here with no change to
      the body contract
- [x] **Persona type + built-in defaults + `GET /_/api/personas`** — `ada`,
      `mira`, `nobody`; the response echoes `id`/`name`/`email`/`roles`/
      `attributes`/`client`; the banner says the built-ins were used. `nobody` has
      no email and no roles — CONCEPT §3's "the user that breaks applications"
- [x] **Persona → claims mapping** — `?persona=ada` mints `sub`,
      `preferred_username`, `email`, `email_verified`, `name`, `roles`, plus
      attributes as top-level claims; `?persona=nobody` mints `sub` and the
      registered claims and *nothing else*; body claims override persona claims.
      The mapping lives in `oidc::issue`, never on the `Persona` type — CONCEPT §3
      keeps the persona model protocol-neutral
- [x] **`users.yaml` loading** — `personas:` list with optional file-level and
      per-persona `client:`; `LANYARD_PERSONAS` points at it; the banner prints
      the path. A file with one persona replaces the built-ins entirely, and
      `/_/api/personas` echoes both `client:` levels back. Nothing filters on it —
      that is Phase 7, and this field exists now only so Phase 7 is additive
      (CONCEPT §15)
- [x] **A malformed `users.yaml` is fatal** — unknown key, duplicate `id`, or
      missing `id` → `serve` exits non-zero naming the offending key or id *and*
      the file path. No fallback to defaults: a persona that silently did not load
      shows up three redirects later as a name missing from the picker
- [x] **Seam error responses** — unknown persona, non-JSON body, non-object body,
      bad `ttl` → `400` with `{"error","error_description"}`, the unknown persona
      id quoted back, and the server still serving afterwards
- [x] **Tests** — `tests/personas.rs` (schema errors, duplicate ids, built-in
      defaults), `tests/issue.rs` (claim mapping, override precedence, thumbprint
      stability against a known-good value), `tests/http.rs` (router on port 0:
      discovery shape, JWKS shape, seam happy path and 400s). `cargo test` green
- [x] **Release build** — `cargo build --release` produces `target/release/lanyard`;
      `ldd` shows no Node, JVM, or package-manager runtime linked
- [x] **Docs** — expand `README.md` past its two lines: build + `./lanyard serve`,
      the banner, the env vars, `users.yaml` format, and `POST /_/api/token`. Four
      sharp edges must be written down, not left implicit:
      (1) **the default signing key is public and forgeable** — never expose
      lanyard to an untrusted network, and `POST /_/api/token` mints anything for
      anyone, which is why the default bind is loopback;
      (2) **the issuer is `http://127.0.0.1:9500/oidc`**, path segment included,
      and it does not follow `Host` — reaching lanyard at `localhost` still yields
      a document saying `127.0.0.1`, which is what `LANYARD_ISSUER` is for;
      (3) the discovery document lists only implemented endpoints and grows each
      phase;
      (4) scope — this is Phase 1: there is no `/authorize`, no `/token`, no CLI
      `token` command yet. Do not promise Phase 2's surface
- [x] **Correct CONCEPT §13's example banner** — it shows
      `Issuer → http://127.0.0.1:9500`, which the `/oidc` mount makes wrong. One
      line, and leaving it makes the concept doc contradict the shipped tool

## Acceptance

Mirrors the spec. Not done until every box passes by driving the named client.
Every run sets `LANYARD_DATA_DIR` and `LANYARD_PERSONAS` to throwaway paths.

**Serving and the issuer**

- [x] `lanyard serve` prints a banner, and
      `curl -s http://127.0.0.1:9500/oidc/.well-known/openid-configuration | jq -r .issuer`
      prints `http://127.0.0.1:9500/oidc` — byte-identical to the banner's issuer line
- [x] Fetching the document's own `jwks_uri` returns 200 with exactly one key,
      `"kty":"RSA"`, `"alg":"RS256"`, a `kid`, and no `d` or other private field
- [x] `curl -s -H 'Host: lanyard:9500' …/openid-configuration | jq -r .issuer`
      still prints `http://127.0.0.1:9500/oidc`
- [x] `LANYARD_ISSUER=http://lanyard:9500/oidc lanyard serve` → banner and
      discovery both show that string; `jwks_uri` is `http://lanyard:9500/oidc/jwks`
- [x] `curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:9500/` → `404`
- [x] Second `lanyard serve` exits non-zero, stderr names `127.0.0.1:9500`, and
      `ss -ltn | grep 9501` is empty
- [x] `ss -ltn` shows `127.0.0.1:9500` by default and `0.0.0.0:9500` under
      `LANYARD_BIND=0.0.0.0`

**Keys**

- [x] `rm -rf "$LANYARD_DATA_DIR"`, restart, refetch `/oidc/jwks` → `diff` of the
      saved JSON bodies is empty (same `kid`, same `n`)
- [x] `<data-dir>/signing-key.pem` exists, `stat -c %a` prints `600`, and
      `<data-dir>/.gitignore` ignores the directory's contents
- [x] `printf 'not a key\n' > signing-key.pem` and restart → exits non-zero,
      stderr names the file path, and nothing is serving on 9500

**The seam**

- [x] `curl -sX POST …/_/api/token -H 'content-type: application/json' -d '{"sub":"ada","aud":"billing-api"}'`
      → a token that `scripts/jose-verify.mjs` accepts via
      `jwtVerify(token, createRemoteJWKSet(new URL(jwks_uri)), {issuer, audience:'billing-api'})`
      against the **live JWKS URL over HTTP**, with payload `sub` of `ada`
- [x] That verified payload has `exp - iat === 60`; with `?ttl=300` it is `300`
- [x] `?persona=ada` → verified payload with `sub: "ada"`,
      `email: "ada@example.test"`, `email_verified: true`, `roles` containing `admin`
- [x] `?persona=nobody` → verified payload with `sub: "nobody"` and **no**
      `email`, `name`, or `roles` keys at all
- [x] `?persona=ada` with body `{"email":"other@example.test"}` → verified payload
      `email` is `other@example.test`
- [x] `?persona=nope` → HTTP 400 whose body names `nope`
- [x] `-d 'not json'` → HTTP 400 with `error` / `error_description`, and the
      server still answers the next request

**Personas**

- [x] No personas file → `/_/api/personas` lists exactly `ada`, `mira`, `nobody`;
      `nobody` has no `email` and empty `roles`; the banner says built-in defaults
- [x] A file with one persona → that one and none of the built-ins; the banner
      prints the file path
- [x] A file using `client:` at both file and persona level loads cleanly and
      `/_/api/personas` echoes both back, with every persona still listed
- [x] A file with `rolez: [admin]` on a persona → `serve` exits non-zero, stderr
      names both `rolez` and the file path
- [x] A file with two personas sharing `id: ada` → `serve` exits non-zero, stderr
      names `ada`

**Shape of the artifact**

- [x] `cargo build --release` produces a binary named `lanyard` from a crate named
      `lanyard-cli`; `./target/release/lanyard --help` lists `serve`
- [x] `ldd` on the release binary shows no Node, JVM, or package-manager runtime

## Progress notes

- **`preferred_username` now travels with `name`, not with `sub`.** The persona →
  claims mapping minted `preferred_username` for every persona including
  `nobody`, which contradicted the spec's "a token for `nobody` carries `sub` and
  the registered claims and nothing else" and quietly defeated the point of that
  persona: an app rendering `preferred_username` would not break on `nobody`.
  Both are `profile`-scope claims, so they are now emitted together and `nobody`
  carries `sub` plus the registered claims only. `tests/issue.rs` was updated to
  pin the narrower key set.
- **`LANYARD_PERSONAS` cannot be set to a throwaway path that does not exist.**
  The Acceptance preamble says every run sets it; an explicit file that is absent
  is fatal by design, so the built-in-persona runs set a throwaway
  `XDG_CONFIG_HOME` instead and leave `LANYARD_PERSONAS` unset.
