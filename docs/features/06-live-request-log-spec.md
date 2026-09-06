# Live request log — spec

**Status:** done · **Roadmap:** Phase 6 · **Slug:** 06-live-request-log

## Why

**lanyard already knows why your login failed, and it throws the answer away.**

Phases 4 and 5 wrote ten distinct `invalid_grant` descriptions on purpose, each
naming exactly one thing that went wrong:

```
the code_verifier does not match the S256 code_challenge this code was issued against
that authorization code has already been exchanged; codes are single-use
that authorization code has expired; codes live 60 seconds, which is the
  redirect and the exchange and nothing else
no such authorization code; it was never issued, or lanyard was restarted since it was
```

Every one of those sentences is written to a `400` returned on a **back-channel**
call — the token exchange the developer never sees. .NET surfaces it as an
`OpenIdConnectProtocolException`; `oidc-client-ts` throws `Error: invalid_grant`;
`jumbojett` throws on the string alone. The sentence lanyard wrote is, in
practice, invisible. The developer sees a login that bounced and has no route
back to the reason.

Five phases deferred to this one and said so in their specs:

| Phase | What it deferred, in its own words |
|---|---|
| 1 | "The live request log, SSE, ndjson — Phase 6. Ordinary per-request stdout logging is fine and deliberately unspecified here." |
| 2 | "The request log — Phase 6. An ordinary stdout line per request is fine." |
| 3 | "**Logging the flaw.** Nothing prints a per-request line today… The flag is explicit at the call site; the server stays quiet." |
| 4 | "Phase 6's 'which parameter mismatched' is what the PKCE and code failures below are written to make possible." |
| 5 | "A failed refresh has to be diagnosable from its `error_description` alone." |

Phase 6 collects that debt. **Nothing prints a per-request line today** — the
startup banner is lanyard's entire output — so this phase introduces the event
stream from nothing.

Which north stars it serves:

- **North star 2 — one instance, every project.** This is why the log is worth
  more here than in a per-project mock. CONCEPT §6: "Because lanyard is a
  machine-wide singleton, this becomes a view across every project being worked
  on at once — more useful than a per-project log, not less." Three services on
  three ports produce one interleaved stream, and the `client_id` on every line
  is what makes that readable rather than noise.
- **North star 3 — issuance is one function.** The same argument applies to
  observation: one event type, emitted at one place, feeding three surfaces. If
  stdout, SSE and ndjson can disagree about what happened, the structure is
  wrong.
- **North star 5 — single static binary.** The log page is the first thing
  lanyard has wanted JavaScript for, and the framework that answers it —
  [zero](https://github.com/robap/zero) — is a cargo-installed Rust binary with
  no `node_modules`. It clears the bar the north star sets, which is the only
  reason a frontend framework can arrive at all.

### cubby has already built this

CONCEPT §16: "lanyard is the second instance of a pattern cubby established:
emulate a service locally as a single zero-config binary with a debugger UI,
deterministic fixtures, and executable conformance against real SDKs." The
roadmap's post-v1 list names the thing to extract: "the stdout/SSE/ndjson event
stream, embedded UI".

cubby ships that stream today — `src/events.rs`, `src/api/events.rs`,
`src/embed.rs`, `web/src/routes/live-log.ts`. **This phase follows it rather
than reinventing it**, because the closer the two implementations stay, the
smaller the eventual extraction is. Every place this spec diverges from cubby is
called out and justified; everywhere else, cubby's shape is the shape.

## In scope

- **One event, one emit point.** Every request to `/oidc/*` and to the test seam
  produces exactly one structured event: what was asked, what was decided, what
  came out. `client_id` on every event, always.
- **Decoded parameters, not a raw query string.** `scope` split into a list,
  `code_challenge` next to its `method`, `redirect_uri`, `response_type`,
  `response_mode`, `nonce`, `prompt`, `grant_type`, and — on a failure — the
  specific comparison that failed.
- **Decoded claims.** For a successful issuance, the JWT header and payload of
  every token minted. For a login, the persona selected.
- **The flaw, named.** A token minted with `--expired`, `--wrong-aud`,
  `--wrong-iss`, `--bad-signature`, `--alg-none` or `--unknown-kid` says so in
  its event. Phase 3's deferred item, paid here.
- **An in-process event bus**: a `tokio::sync::broadcast` channel plus a
  1000-event ring buffer. Nothing persisted; the log resets on restart.
- **Three surfaces from one stream:**
  - **stdout** — one aligned, human-readable line per event, always on, no flag.
  - **`GET /_/api/events`** — `text/event-stream`, honouring `Last-Event-ID`.
  - **`GET /_/api/events?format=ndjson`** — the same objects, one JSON per line.
- **`POST /_/api/events/clear`** — drain the ring and tell live streams to empty.
- **`lanyard logs [--json]`** — a fourth subcommand alongside `serve`, `token`
  and `env`. Connects to the running singleton over HTTP.
- **`GET /_/log` — the live log page, and the only JavaScript lanyard ships.**
  A zero app: a row per event, newest first, a `client_id` filter, pause/resume,
  clear, and click-to-expand into a decoded claims panel.
- **zero's design system adopted for every `/_/` page**, replacing the
  hand-written `src/ui/lanyard.css`. The server-rendered pages gain zero's
  tokens, layout primitives and typography and gain **no JavaScript**.
- **The build path, cubby's exactly**: the zero app's source in `web/`,
  `zero build` writes `web/dist/`, that directory is **committed**, and
  `rust-embed` compiles it in. `zero` is never on lanyard's build path.
- **Banner** gains a `Log → …/_/log` line; `/_/` links to it and back.
- **README**: the log section, the three surfaces, and the sentence that says
  the log prints secrets.

## Out of scope

- **Persistence.** No log file, no rotation, no `--log-file`, no replay across a
  restart. stdout is the durable surface — `lanyard serve > lanyard.log` is the
  whole story — and the ring is a convenience for the UI, not an archive.
- **Log levels, `RUST_LOG`, `tracing` subscribers, OpenTelemetry.** One event
  type at one verbosity. A level selector is configuration, and configuration
  growing is the roadmap's stated signal that a phase has drifted.
- **Redaction, masking, or a "safe mode".** See Behavior — a decision, not an
  omission.
- **Migrating the picker to zero.** The login path completes with JavaScript
  disabled (Phase 4, criterion 26) and this phase does not touch that. zero
  arrives for the log page and for CSS, and stops there.
- **Redesigning any existing page.** The restyle swaps the stylesheet, not the
  information architecture: same pages, same content, same forms, same copy.
- **Extracting the shared crate with cubby.** Post-v1. This phase makes the
  extraction cheaper by keeping the shapes aligned; it does not perform it.
- **Filtering personas by `client_id`** (Phase 7). Still parsed, read by nothing.
- **`lanyard doctor`, `/_/health`, the `Host`-mismatch warning** (Phase 8). The
  log is where doctor's findings will surface, and that is Phase 8's problem.
- **A request id propagated to the RP.** The event id is lanyard's own and
  appears in no response header.

## Behavior

### The event

One shape, whatever produced it. Field names follow cubby's `Event` wherever the
two overlap (`id`, `ts`, `method`, `status`, `duration_ms`) so the eventual
shared crate is a merge and not a translation:

```json
{
  "id": 412,
  "ts": 1757080931418,
  "client_id": "billing-web",
  "endpoint": "/oidc/token",
  "method": "POST",
  "status": 400,
  "duration_ms": 3,
  "grant_type": "authorization_code",
  "error": "invalid_grant",
  "error_description": "the code_verifier does not match the S256 code_challenge this code was issued against",
  "request": {
    "code": "8Xk2…",
    "redirect_uri": "http://localhost:5001/signin-oidc",
    "code_verifier": "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
  },
  "detail": {
    "pkce": {
      "method": "S256",
      "verifier_presented":  "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
      "challenge_computed":  "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
      "challenge_recorded":  "K2-ltc83acc4h0c9w6ESC_rEMTJ3F50BXVuGJSstw-cM"
    }
  },
  "issued": null,
  "flaw": null
}
```

`id` is monotonic from process start and is the SSE frame id. `ts` is Unix
milliseconds. `detail` is the endpoint-specific payload and is where "which
parameter mismatched" lives — three PKCE values side by side are the entire
diagnosis, and the developer reads which two were meant to be equal without
being told.

On success, `issued` carries the decoded tokens:

```json
"issued": {
  "access_token": { "header": {"alg":"RS256","kid":"lanyard-dev-1","typ":"JWT"},
                    "payload": {"iss":"…","sub":"ada","aud":"billing-api","exp":1757080991} },
  "id_token":     { "header": {…},
                    "payload": {"nonce":"n-0S6","at_hash":"…","email":"ada@example.test"} }
}
```

Endpoints that emit: `/oidc/authorize`, `/oidc/token` (all four grants),
`/oidc/userinfo`, `/oidc/introspect`, `/oidc/revoke`, `/oidc/end_session`, and
`POST /_/api/token`. `/oidc/jwks` and the discovery document do not — they are
static, and an SDK's poll loop on discovery would drown everything else.

**One human moment gets an event too: the pick.** `POST /_/pick` is what turns
"the picker appeared" into "you are Ada", and "why am I signed in as the wrong
person" is a question the log has to answer. It is the only `/_/` route that
emits.

### Three surfaces, one stream

**stdout** — one aligned line, always on:

```
14:02:09  billing-web  GET  /oidc/authorize  302  ada  scope=openid,email,profile  pkce=S256
14:02:11  billing-web  POST /oidc/token      400  invalid_grant  the code_verifier does not match the S256 code_challenge this code was issued against
14:02:14  lanyard-cli  POST /oidc/token      200  client_credentials  sub=ada aud=billing-api exp=+60s
14:02:19  lanyard-cli  POST /oidc/token      200  client_credentials  sub=ada aud=billing-api  flaw=alg-none
```

The failure line carries the whole `error_description`. That is the point of the
surface: a developer who never opens the UI still gets the sentence.

**`GET /_/api/events`** — one endpoint, two encodings, exactly as cubby does it.
SSE by default; `?format=ndjson` switches to `application/x-ndjson`. Both replay
the ring buffer first, then stream live. `Last-Event-ID: 411` resumes at 412 and
replays nothing before it. Both carry `Cache-Control: no-cache` and
`X-Accel-Buffering: no`.

This lives under `/_/api/` beside the Phase 1 test seam `POST /_/api/token`, so
lanyard's own namespace stays one namespace — and it is the same path cubby
serves it on.

### `lanyard logs`

Connects to the running singleton over HTTP, exactly as `lanyard token` performs
a real `client_credentials` grant rather than signing locally. `--json` reads
`?format=ndjson`; bare reads the SSE stream and prints the stdout format, so
`lanyard logs` in a second terminal shows what `lanyard serve` is showing in the
first.

No server running is an error, not an empty stream. The README's existing stance
applies: a tool that exits `0` having printed nothing is a tool that lies about
having looked.

### A stalled reader must not stall a login

An SSE client that connects and stops reading is ordinary — a backgrounded
browser tab, a `curl` into a full pipe. The fan-out is therefore **lossy at the
subscriber and never at the source**. A consumer that lags the broadcast channel
receives a **synthetic `dropped` marker naming how many events it missed**
(cubby's `RecvError::Lagged(n)` handling) rather than silently losing them or
stalling the server. The stdout line and the ring buffer are written regardless
of who is listening.

Telling the reader it missed events matters more than it sounds: a log with a
silent hole in it is worse than no log, because the developer concludes the
request never happened.

### Which parameter mismatched — the worked example

This is roadmap acceptance criterion 1, spelled out because it is what the phase
is for.

1. A browser completes `/oidc/authorize` and lands back at the RP with a `code`.
   lanyard recorded `code_challenge=K2-ltc83…`, `method=S256`.
2. The RP — or a `curl` standing in for one — exchanges the code with a
   `code_verifier` that is one character off.
3. `challenge.verify(verifier)` fails and `src/oidc/token.rs` returns
   `400 invalid_grant` with the sentence naming the code_verifier.
4. **The event carries all three values.** The expanded row stacks them, and the
   recorded and computed challenges differ visibly at character 3.

The developer learns their verifier is wrong — not that "login failed" — and
reads it off a page rather than out of `src/`.

### Two projects at once

`spikes/dotnet-web` on 5001 and `spikes/php-web` on 8080 log in as two people at
once. One stream, in time order:

```
14:07:02  billing-web   GET  /oidc/authorize  302  ada
14:07:03  php-demo      GET  /oidc/authorize  302  mira
14:07:03  billing-web   POST /oidc/token      200  authorization_code  sub=ada
14:07:04  php-demo      POST /oidc/token      200  authorization_code  sub=mira
```

The `client_id` column makes this readable, and it is the same field Phase 7 will
namespace personas on — carried since Phase 1 for exactly this reason.

### Nothing is redacted, and that is deliberate

The log prints authorization codes, refresh tokens, `code_verifier`s, and any
`client_secret` a client sends. It prints the full decoded claims of every token
minted.

This follows from north star 1. lanyard accepts every `client_secret` without
looking at it — the value carries no authority here, and starring it out would
teach a developer that lanyard checked something it did not. The tokens are
60-second tokens signed by a key whose private half is published in this repo.
**The log is exactly as sensitive as the tokens `lanyard token` already prints to
stdout**, and the README says so in those words rather than leaving someone to
discover it when they paste a terminal into an issue.

*(cubby's equivalent captures no headers or signatures, because in an S3 emulator
a SigV4 signature is a credential-derived secret with no other purpose. lanyard's
parameters are the diagnosis itself — a `code_verifier` you cannot see is a PKCE
failure you cannot diagnose — so this is a deliberate divergence, not drift.)*

### Where JavaScript is allowed, and where it is not

| Surface | Rendering | Script |
|---|---|---|
| `/_/` picker, mint panel, session controls | Server-rendered HTML, `POST` forms | none |
| The rejection page | Server-rendered HTML | none |
| `/oidc/authorize` → `form_post` response mode | Server-rendered HTML | the auto-submit with its `<noscript>` button — unchanged from Phase 4 |
| **`/_/log`** | **zero app** | **one `<script type="module">`** |

Phase 4's criterion 26 — the whole login flow in a browser with JavaScript
disabled — stands unchanged and is re-run in this phase's acceptance list.
Phase 5's three session controls stay plain `POST` forms.

`src/ui/html.rs`'s test currently asserts **"no script tag anywhere"**. It
narrows to "no script tag on a server-rendered page", and `/_/log` gets its own
test asserting exactly one module script pointing at a fingerprinted asset.

With JavaScript off, `/_/log` renders a `<noscript>` line saying so and naming
`lanyard logs` as the surface that works without it. It does not degrade to a
static snapshot — a page that silently stops updating is worse than one that says
it needs a thing.

**This is the divergence from cubby that matters.** cubby mounts a whole SPA at
`/_/` and has no server-rendered UI at all. lanyard keeps `/_/` server-rendered
and mounts the zero app at `/_/log` alone, because lanyard's `/_/` is *in the
login path* — an `/authorize` with no session redirects a real browser to it —
and that path is committed to working without JavaScript. cubby's UI is a
debugger beside the product; lanyard's picker is part of the product.

### zero on the build path — cubby's arrangement, unchanged

`web/` holds the zero app's source. `zero build` writes `web/dist/`. **That
directory is committed** and `rust-embed` compiles it into the binary
(`src/embed.rs`, `#[folder = "web/dist/"]`).

So: **a machine with a Rust toolchain and nothing else builds lanyard.** `zero`
is needed only to *modify* the UI — edit `web/src`, run `zero build`, commit the
regenerated `web/dist/`. This is what keeps Phase 10's `cargo install
lanyard-cli` and `brew install` promises intact, and `cargo publish`'s
verification build is itself a check that the committed UI and the package
include-list are correct.

`rust-embed` reads from disk in debug builds and embeds in release, so the
development loop is `zero build && cargo run` with no second process and no
proxy.

**There is no CI gate on UI freshness — regenerate before committing.** cubby
made this call explicitly and lanyard matches it. A byte-identical rebuild check
would have to pin CI to the same `zero` version, or a routine `zero update` fails
the build as though the bundle were stale; a gate that goes red on a framework
bump gets disabled within a month. What is actually load-bearing is checked
instead: `cargo publish`'s verification build compiles the packaged crate with
`web/dist/` embedded and no `zero` present, which catches a broken embed or a
wrong `include` list — the failures that break a release. A stale bundle is a
cosmetic wrong-version UI, and the convention carries it.

**`web/.zero/` is gitignored**, following cubby again and following from the
decision above: the only reason to commit the framework files was to make a
rebuild reproducible enough to gate on, and there is no gate. A contributor
modifying the UI runs `cargo install zero --locked`, `zero update`, `zero build`,
and commits the regenerated `web/dist/`. `web/dist/` is the committed artifact;
`.zero/` is regenerable scaffolding.

**`zero dev`'s proxy is not used, and could not be.**
`crates/zero-dev/src/proxy.rs:239` reads the upstream response with
`upstream_resp.bytes().await` — whole-body buffering — so a proxied
`/_/api/events` would never flush a frame, and the one feature under development
is the one the dev server cannot show. cubby's `zero.toml` leaves `proxy`
commented out for its own reasons; lanyard has a specific one. The consequence is
a happy one: everything stays same-origin, so **no CORS work is needed on the
event endpoints at all.**

### The design system, and the one thing it costs

zero ships thirteen semantic `--color-*` tokens, six layout primitives (`stack`,
`cluster`, `split`, `flank`, `grid`, `frame`), forty-four utilities, twelve
typography classes, and light/dark on `prefers-color-scheme` with a
`[data-theme]` override. `src/ui/lanyard.css` is 173 hand-written lines
reimplementing a worse subset of that.

The pages are restyled onto it and `lanyard.css` is deleted. Same markup
structure, same forms, same copy — new tokens and primitives. Doing it in this
phase costs one pass over three pages; doing it after zero has landed for the log
page means shipping two visual languages and reconciling them later.

**This is Phase 6's work, not Phase 11's.** It was the obvious thing to defer —
Phase 11 already owns "one shared page, rendered by every stack" — and it is
staying here anyway, because the alternative is a release where `/_/log` renders
in zero's design system and the picker beside it renders in a hand-written
imitation of one. Criteria 24 and 25 are therefore not optional trim.

**The mount-prefix problem is already solved.** zero emits root-absolute asset
refs — `/assets/…` and `/.zero/fonts/…` — with no base-path config, and Phase 1
settled that lanyard's root stays free. cubby's answer (`src/embed.rs`) is to
rewrite those two prefixes **as the asset is served**, not at build time:

```rust
body.replace("/assets/", "/_/assets/")
    .replace("/.zero/", "/_/.zero/")
```

Applied only to `.html` and `.css` (the JS bundle carries none), it is a single
non-doubling pass, it leaves `web/dist/` in zero's native form so a bare
`zero build` still works for a UI developer, and it cannot be bypassed. lanyard
takes it verbatim. Geist ships, root stays free, nothing is rewritten at build
time.

**The cost: the picker gains a `<link>` and loses its inlined `<style>`.**
Phase 4 inlined the CSS on two grounds — one round trip, and no cache to be
stale. The design system is 36 KB, too much to repeat in every render, and zero's
output is *fingerprinted*, so "no cache to be stale" is now satisfied by the
filename. The round trip is on loopback. **This reverses a Phase 4 decision and
is recorded here rather than discovered in a diff**; the `!out.contains("<link")`
assertion in `src/ui/html.rs` goes with it. What does not change: nothing is
fetched from a CDN, and the binary is still the whole website.

The server-rendered pages learn the hashed stylesheet name the way zero's own
documentation prescribes for a backend — read the embedded `manifest.json` once
at startup, cache the `styles/app.scss` → `assets/app.<hash>.css` mapping, and
emit the `<link>` from it. No hash is ever written by hand.

### Testing the zero app

cubby's pattern, which works around a real gap: zero's test runner lists
`EventSource` as **out of scope** in its in-memory web platform ("reach for them
inside a test and stub them yourself"). So:

- Pure logic — the filter predicate, the capped ring append, the relative-time
  label — lives in `web/src/lib/log.ts` and is unit-tested directly, with no DOM.
- The screen test stubs `globalThis.EventSource` with a controllable fake,
  dispatches canned events, and asserts the rendered rows: empty state, a frame
  becomes a row, click-to-expand, the filter, pause buffering.

## Acceptance criteria

Each names a client and an operation, or an assertion against a file or a
rendered page.

**The stream and its three surfaces**

- [x] 1. `lanyard serve`, then a complete login from `spikes/dotnet-web`. Server
      stdout carries exactly two new lines — one `GET /oidc/authorize`, one
      `POST /oidc/token` — each labelled with that app's `client_id`, the
      authorize line naming the persona picked and the token line naming the
      granted scopes.
- [x] 2. `curl -N 'http://127.0.0.1:9500/_/api/events?format=ndjson'` in one
      terminal while `lanyard token --as ada --aud billing-api` runs in another →
      one line of JSON appears, and
      `jq -e '.client_id == "lanyard-cli" and .grant_type == "client_credentials"'`
      exits `0`.
- [x] 3. `lanyard logs --json | jq .client_id` prints a client id per event as
      each happens. *(roadmap criterion 3)*
- [x] 4. `lanyard logs` with no flag prints, for the same events, the same lines
      `lanyard serve` printed on its own stdout.
- [x] 5. `curl -N -H 'Accept: text/event-stream' http://127.0.0.1:9500/_/api/events`
      emits frames carrying `id:` and `data:`. Reconnecting with
      `Last-Event-ID: <n>` delivers events after `n` and replays none before it.
- [x] 6. Three logins, *then* start `lanyard logs`: the three already-finished
      logins print before the stream goes live.
- [x] 7. `lanyard logs` with no lanyard running exits non-zero naming the
      connection failure. It does not exit `0` having printed nothing.
- [x] 8. An `/_/api/events` reader connected and then `SIGSTOP`ped: 500
      sequential `lanyard token` mints complete in the same wall time as with no
      reader attached, all 500 appear on stdout, and the stalled reader —
      once resumed — receives a `dropped` marker naming how many it missed
      rather than a silent hole.
- [x] 9. `POST /_/api/events/clear` returns `204`; a subsequent reconnect replays
      nothing, and an already-open `/_/log` tab empties its own view.

**Which parameter mismatched**

- [x] 10. Complete `/oidc/authorize` in a browser to get a real `code`, then
      exchange it with `curl` using a `code_verifier` one character off. `/_/log`
      shows that `/oidc/token` event as failed, and expanding the row stacks the
      verifier presented, the S256 computed from it, and the `code_challenge`
      recorded at `/authorize`. The mismatched pair is visibly the one that
      differs, and nothing under `src/` was opened. *(roadmap criterion 1)*
- [x] 11. Each of Phase 4's six and Phase 5's four `invalid_grant` descriptions
      appears verbatim in the log event for the request that produced it. Ten
      shell lines, run as one script, exit `0`.
- [x] 12. A `redirect_uri` of `https://evil.example.com/cb` appears as a failed
      `/oidc/authorize` event naming the host that failed the loopback check —
      even though nothing was redirected and the RP was never contacted.

**Two projects at once**

- [x] 13. `spikes/dotnet-web` and `spikes/php-web` log in as two different
      personas concurrently. One `/_/log` shows both in time order, each row
      labelled with its own `client_id`. Filtering to one `client_id` hides the
      other's rows and only those. *(roadmap criterion 2)*

**Decoded claims**

- [x] 14. After a login with `scope=openid email profile`, expanding the
      `/oidc/token` event shows the ID token's decoded header and payload: `alg`
      is `RS256`, `kid` matches the banner's `Signing → kid` line, and the
      payload matches what `scripts/jose-verify.mjs` decodes from the same token.
- [x] 15. `lanyard token --as ada --aud billing-api --alg-none` produces an event
      whose `flaw` is `alg-none` and whose expanded view shows a header with
      `"alg": "none"`. Phase 3's deferred item, paid.
- [x] 16. `exp` renders as both the epoch integer and a human time, and a token
      minted with `--expired` reads as already expired at the moment it was
      issued.

**The line between JavaScript and no JavaScript**

- [x] 17. `curl -s http://127.0.0.1:9500/_/ | grep -c '<script'` → `0`.
      `curl -s http://127.0.0.1:9500/_/log | grep -c '<script'` → `1`, and its
      `src` is a hashed file under `/_/assets/`.
- [x] 18. **Phase 4 criterion 26, repeated unchanged**: the whole login flow in a
      browser with JavaScript disabled, completing to the same rendered email
      claim. In that same browser, `/_/log` renders a `<noscript>` line naming
      `lanyard logs` as the surface that works without it.
- [x] 19. Phase 5's three controls — **Log out of lanyard**, **Forget**,
      **Expire now** — still work in that JavaScript-disabled browser.

**The build path**

- [x] 20. On a machine with no `node`, no `npm` and **no `zero`** on `PATH`,
      `cargo build --release` produces a binary that serves a styled `/_/` and a
      working `/_/log`.
- [x] 21. `cargo publish --dry-run --locked` succeeds: the packaged crate embeds
      `web/dist/` and compiles without `zero`, so Phase 10's crates.io path is
      proven now rather than at release time.
- [x] 22. Loading `/_/` and `/_/log` in a browser, the network tab shows requests
      to `127.0.0.1:9500` and to no other host — no CDN, no Google Fonts.
- [x] 23. Every asset reference resolves under the mount: `/_/log`'s HTML names
      `/_/assets/…`, the served stylesheet names `/_/.zero/fonts/…`, and
      `curl -i http://127.0.0.1:9500/.zero/fonts/Geist-VariableFont_wght.woff2`
      is a `404`. Root stays free, per Phase 1's routing decision.

**The design system**

- [x] 24. `/_/`, the rejection page and `/_/log` all render from one stylesheet
      whose href came from the embedded `manifest.json`. The picker's persona
      rows use zero's layout primitives and the thirteen `--color-*` tokens,
      `src/ui/lanyard.css` is deleted, and setting `<html data-theme="dark">`
      flips all three pages.
- [x] 25. Phase 4's and Phase 5's browser flows re-run green against the
      restyled pages, and fresh light/dark screenshots of the picker replace
      `docs/decisions/evidence/phase04-1-picker.png`.

**Discoverability**

- [x] 26. The banner gains a `Log → http://127.0.0.1:9500/_/log` line, and that
      URL returns `200` — Phase 1's rule that the banner never prints a URL that
      404s.
- [x] 27. `/_/` links to `/_/log`, and `/_/log` links back.

## Open questions

**None.** Everything that was open is resolved above and recorded where it
belongs rather than left in this section:

| Was open | Where it landed |
|---|---|
| How far zero goes into the UI | `/_/log` only — *Where JavaScript is allowed, and where it is not* |
| How the bundle reaches the binary | Committed `web/dist/` + `rust-embed`, cubby's arrangement — *zero on the build path* |
| zero's root-absolute asset and font URLs | Serve-time prefix rewrite, cubby's `src/embed.rs` verbatim — *The design system* |
| A CI gate on UI freshness | No gate; convention plus `cargo publish`'s verification build — *zero on the build path* |
| Whether `web/.zero/` is committed | Gitignored, following from the no-gate decision — *zero on the build path* |
| Whether the restyle is Phase 6 or Phase 11 | Phase 6; criteria 24 and 25 are not optional — *The design system* |
| `zero dev`'s buffering proxy | No upstream issue. This phase never uses the proxy: `rust-embed` serves `web/dist/` from disk in debug builds, so the loop is same-origin — *zero on the build path* |
