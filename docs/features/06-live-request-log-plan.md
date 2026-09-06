# Live request log — plan

**Status:** done · **Spec:** [06-live-request-log-spec.md](06-live-request-log-spec.md) · **Roadmap:** Phase 6

## Approach

Three things that happen to ship together: an **event bus**, a **frontend**, and
a **stylesheet**. Only the first is a new subsystem.

The bus is cubby's, deliberately: a `tokio::sync::broadcast` channel plus a
1000-event ring, `pretty()` for stdout, one endpoint serving both encodings.
Post-v1 wants this extracted into a crate shared with cubby, so every name that
can match cubby's does — `Event`, `EventDraft`, `BusSignal`, `EventBus`,
`subscribe(after)`, `pretty` — and the divergences are the ones lanyard's domain
forces.

**One emit point, reached two ways.** An axum middleware wraps `/oidc/*` and the
`/_/api` seam: it times the request, reads the final status, and publishes
exactly one event. What only a handler knows — `client_id`, `grant_type`, the
decoded parameters, the PKCE triple, the claims that came out — rides back on the
**response extensions** as a `LogDetail`, which the middleware pops and folds in.
This is cubby's two-part capture (`http.rs` wrapper + `access_log.rs` hook joined
through request extensions) in axum's idiom, and it is what **north star 3**
demands of observation: if two places could build an event, stdout and SSE could
disagree about what happened.

The choke points do most of the work for free. `token.rs`'s `bad_request()` is
the single funnel every `invalid_grant` passes through, so attaching
`error`/`error_description` *there* means **no error can be returned without
being logged** — one edit, ten criteria's worth of coverage.

The frontend is a zero app at `/_/log` and nowhere else. `/_/` stays
server-rendered HTML with no script tag, because `/authorize` redirects a real
browser to it and Phase 4 criterion 26 committed that path to working without
JavaScript. `web/dist/` is committed and embedded with `rust-embed`, so `zero` is
never on lanyard's build path (**north star 5**), and the root-absolute asset
refs zero emits are rewritten to the `/_/` mount **as they are served** —
cubby's `src/embed.rs` verbatim — which keeps root free (Phase 1's routing
decision) without touching `zero build`'s output.

The stylesheet is the phase's one reversal, recorded in the spec: the picker
gains a `<link>` to a fingerprinted asset and loses its inlined `<style>`,
because a 36 KB design system is too much to repeat per render and a content hash
already answers "no cache to be stale".

## Files

- `src/events.rs` — **new.** `Event`, `EventDraft`, `BusSignal`, `EventBus`
  (broadcast + 1000-event ring, `publish`, `subscribe(after)`, `clear`), and
  `pretty()` for the stdout line. No HTTP, no axum
- `src/api/mod.rs`, `src/api/events.rs` — **new.** `GET /_/api/events` (SSE and
  `?format=ndjson` from one handler) and `POST /_/api/events/clear`. Mirrors
  cubby's path so the eventual extraction is a merge
- `src/log_layer.rs` — **new.** The middleware: times the request, publishes one
  event, prints one line. Owns the exclusion list
- `src/log_detail.rs` — **new.** `LogDetail`, the struct handlers attach to
  response extensions, plus the `attach` helper the choke points call
- `src/embed.rs` — **new.** `rust-embed` over `web/dist/`, the `/_/` mount-prefix
  rewrite, content types, and the `manifest.json` lookup the server-rendered
  pages read their stylesheet href from
- `src/app.rs` — `AppState` gains `events: EventBus`; router gains the log
  layer, `/_/log`, `/_/assets/*`, `/_/.zero/*`
- `src/oidc/token.rs` — attach `LogDetail` in all four arms and in
  `bad_request()`; the PKCE triple at the verifier-mismatch site
- `src/oidc/authorize.rs` — attach on success, on the loopback rejection, and on
  every `error=` redirect
- `src/oidc/userinfo.rs`, `introspect.rs`, `revoke.rs`, `end_session.rs` —
  attach `client_id` and outcome
- `src/seam.rs` — `POST /_/api/token` attaches its flaw and claims
- `src/ui/mod.rs` — `POST /_/pick` attaches the persona chosen; picker gains the
  `/_/log` link; pages restyled
- `src/ui/html.rs` — `page()` emits `<link>` from the manifest instead of an
  inlined `<style>`; the no-`<link>` assertion goes, the no-`<script>` one stays
- `src/ui/lanyard.css` — **deleted**
- `src/main.rs` — `Command::Logs`; `serve()` constructs the bus
- `src/banner.rs` — the `Log →` line
- `web/` — **new.** The zero app: `src/app.ts`, `src/routes/live-log.ts`,
  `src/lib/log.ts`, `src/lib/format.ts`, `styles/app.scss`, plus `.test.ts`
  siblings
- `web/dist/` — **new, committed.** `zero build`'s output
- `zero.toml` — **new.** `[project] root = "web"`, `[build] out = "web/dist"`
- `.gitignore` — `web/.zero/`
- `Cargo.toml` — `rust-embed`, `async-stream`; `reqwest` gains `"stream"`
- `tests/live_log.rs` — **new.** The bus, the endpoints, the exclusion list
- `tests/cli.rs` — `lanyard logs` cases
- `README.md` — the log section

## Risks & unknowns

- **The events endpoint must not log itself.** `GET /_/api/events` held open
  would publish an event on connect, which every connected client then receives,
  and a second client's connect feeds the first. The middleware's exclusion list
  is load-bearing, not cosmetic: `/oidc/.well-known/…`, `/oidc/jwks`,
  `/_/api/events`, `/_/log`, `/_/assets/*`, `/_/.zero/*` and `/_/` itself emit
  nothing. Discovery and JWKS are excluded for the spec's stated reason — an
  SDK's poll loop would drown everything else. **Test this explicitly**: connect
  two SSE clients, assert neither sees an event.
- **`reqwest` cannot stream today.** Probed: `Response::bytes_stream()` does not
  exist under the current `default-features = false, features = ["json","form"]`.
  `lanyard logs` needs `"stream"` added. `tokio::sync::broadcast` *is* already
  available transitively — probed and compiles — so no `tokio` feature change.
- **Three new dependencies** — `rust-embed`, `async-stream`, and reqwest's
  `stream`. All pure Rust; north star 5 is about the *build path*, not the
  dependency count, and `zero` stays off it. Still worth one look at what
  `rust-embed` pulls in before committing.
- **Restyling touches the login path.** The picker's form field names and the
  flow must not change — Phase 4's and Phase 5's browser tests are the guard, and
  they are re-run before the phase closes. Four existing assertions bear on
  markup and need review as a group: `tests/browser_flow.rs:95` ("no external
  script"), `:335` (`<noscript>` on the form-post page), `:2681` ("no script
  anywhere on this page"), and `src/ui/html.rs`'s `!out.contains("<link")`. Only
  the last should change.
- **`cargo publish` must package `web/dist/`.** It is committed so cargo includes
  it, but `web/src`, `web/.zero` and `web/node_modules`-shaped noise should not
  ship in the crate. If an `include`/`exclude` list is needed in `Cargo.toml`,
  criterion 21 is what catches its absence — run it early, not at release.
- **Binary growth ≈ 450 KB** (91 KB JS + 36 KB CSS + 291 KB Geist woff2, measured
  from cubby's `dist/`). Acceptable for a single binary; worth stating in the
  README rather than being a surprise.
- **`zero`'s version is whatever is current.** `web/.zero/` is gitignored per the
  spec, so a contributor's `zero update` may produce a different bundle than the
  committed one. That is the accepted cost of having no freshness gate; it is not
  a bug to fix here.
- **axum SSE vs raw ndjson from one handler.** `axum::response::sse::Sse` wants a
  `Stream<Item = Result<Event, _>>`; ndjson wants raw bytes. One handler builds
  one `async_stream` of already-encoded frames and picks the content type, rather
  than two code paths that could drift in what they emit.

## Steps

Each box ≈ one small commit moving an observable behavior. Check only when the
outcome is real, not when code is written.

- [x] **The bus, alone** — `src/events.rs`: `Event` (`id`, `ts`, `client_id`,
      `endpoint`, `method`, `status`, `duration_ms`, `grant_type`, `error`,
      `error_description`, `request`, `detail`, `issued`, `flaw`), `EventDraft`,
      `BusSignal::{Event, Clear}`, and `EventBus` over a `broadcast` channel plus
      a 1000-entry `VecDeque`. `subscribe(after)` returns `(backlog, rx)`.
      `pretty()` renders the aligned stdout line. Unit tests: the ring caps at
      1000 and drops oldest, `subscribe(Some(n))` replays strictly after `n`,
      `clear()` empties the ring, `pretty()` puts the whole
      `error_description` on the line. Names match cubby's where they can.
      Nothing routes to it yet
- [x] **One emit point, and lanyard starts talking** — `AppState` gains
      `events: EventBus`; `src/log_layer.rs` wraps `/oidc/*` and `/_/api/*`,
      times the request, publishes one event and prints one `pretty()` line.
      **The exclusion list ships in this commit**, with a test that two open SSE
      clients see no events from each other's connections. Observable:
      `lanyard serve`, then `lanyard token --as ada --aud billing-api` prints one
      aligned line naming method, path, status and duration
- [x] **Errors cannot escape unlogged** — `src/log_detail.rs` and the response
      extension; `token.rs`'s `bad_request()` attaches `error` and
      `error_description` at the one funnel every `invalid_grant` passes
      through. Observable: an unknown code exchanged at `/oidc/token` prints the
      whole "no such authorization code; it was never issued, or lanyard was
      restarted since it was" on stdout
- [x] **`/oidc/token` enriched** — all four arms attach `client_id`,
      `grant_type`, the decoded form, the `flaw` when one was asked for, and
      `issued` carrying the decoded header and payload of every token minted.
      Observable: criterion 1's token line names the granted scopes;
      `--alg-none` prints `flaw=alg-none` (criterion 15's first half)
- [x] **The PKCE triple** — the verifier-mismatch site attaches
      `detail.pkce.{method, verifier_presented, challenge_computed,
      challenge_recorded}`. Observable: exchange a real code with a wrong
      verifier, and all three values are in the ndjson event. This is criterion
      10's payload; the UI only has to render it
- [x] **`/oidc/authorize` enriched** — `client_id`, `response_type`,
      `response_mode`, `scope` as a list, `redirect_uri`, `code_challenge` and
      method, `nonce`, `prompt`, and the persona when the session already had
      one. The loopback rejection attaches the host that failed the check.
      Criterion 12 — a rejection the RP never sees is still in the log
- [x] **The rest of the protocol surface** — `/oidc/userinfo`,
      `/oidc/introspect`, `/oidc/revoke`, `/oidc/end_session` attach their
      `client_id` and outcome. Observable: a logout and a revoke each print a
      line naming what happened
- [x] **The two `/_/` events** — `POST /_/api/token` attaches its flaw and
      claims; `POST /_/pick` attaches the persona chosen. Observable: clicking
      "Ada Bell" in the picker prints a line naming her. This is the only `/_/`
      route that emits, per the spec
- [x] **`GET /_/api/events` streams SSE** — `src/api/events.rs`. Ring replayed
      first, then live; `Last-Event-ID` resumes strictly after the given id;
      `Cache-Control: no-cache` and `X-Accel-Buffering: no`. Criterion 5
- [x] **`?format=ndjson` from the same handler** — one stream of encoded frames,
      two content types, no second code path. Criterion 2
- [x] **A lagged subscriber is told, not silently starved** —
      `RecvError::Lagged(n)` becomes a synthetic `dropped` marker naming the
      count. Criterion 8. A log with an invisible hole is worse than no log,
      because the developer concludes the request never happened
- [x] **`POST /_/api/events/clear`** — drains the ring and broadcasts
      `BusSignal::Clear`, so the clear survives an `EventSource` reconnect
      replay and reaches every open tab. `204`. Criterion 9
- [x] **`lanyard logs [--json]`** — a fourth subcommand; `reqwest` gains
      `"stream"`. Bare reads the SSE stream and prints `pretty()`; `--json` reads
      `?format=ndjson`. No server running exits non-zero naming the connection
      failure, never `0` with empty output — the README's existing stance.
      Criteria 3, 4, 6, 7
- [x] **`web/` exists and `zero build` produces `web/dist/`** — `zero init`,
      `zero.toml` (`root = "web"`, `out = "web/dist"`), `.gitignore` for
      `web/.zero/`, and the committed first bundle. Nothing serves it yet.
      Observable: `zero build` writes `web/dist/{index.html,manifest.json,assets/}`
      and `git status` shows them tracked
- [x] **The binary serves the bundle under `/_/`** — `src/embed.rs`:
      `rust-embed` over `web/dist/`, cubby's `/assets/` → `/_/assets/` and
      `/.zero/` → `/_/.zero/` rewrite applied to `.html` and `.css` only, content
      types, immutable cache on assets and `no-cache` on the index. Routes for
      `/_/log`, `/_/assets/*`, `/_/.zero/*`. Criteria 17 and 23 — including that
      `/.zero/fonts/…` at root is a `404`
- [x] **`web/src/lib/log.ts` — the pure logic, tested without a DOM** — the
      `client_id` filter predicate, the capped ring append, the relative-time
      label. `zero test` covers them directly. Extracted first because the screen
      test cannot reach them once they are tangled in a view
- [x] **The live log screen** — `web/src/routes/live-log.ts`: `EventSource` on
      `/_/api/events`, rows newest-first, the `client_id` filter, pause/resume,
      clear, and a `<noscript>` line naming `lanyard logs`. Tested with a
      stubbed `globalThis.EventSource` — zero's test platform lists `EventSource`
      as out of scope, so stubbing is the documented route. Criteria 13 and 18
- [x] **Click a row: the decoded claims viewer** — the expanded panel renders
      `issued`'s header and payload, `exp` as both epoch integer and human time,
      and — for a failure — `detail.pkce`'s three values stacked. Criteria 10,
      14, 15, 16. This is the box the phase exists for
- [x] **The stylesheet moves to the manifest** — `src/embed.rs` reads the
      embedded `manifest.json` once at startup and caches
      `styles/app.scss` → `assets/app.<hash>.css`; `html::page()` emits a
      `<link>` built from it instead of inlining `lanyard.css`. The
      `!out.contains("<link")` assertion goes; the no-`<script>` one stays and
      narrows to server-rendered pages. No hash is ever written by hand
- [x] **Restyle onto zero's design system, and delete `lanyard.css`** — the
      picker, mint panel, session controls and rejection page move to the
      thirteen `--color-*` tokens and the layout primitives. Same markup
      structure, same form field names, same copy. `<html data-theme="dark">`
      flips all three pages. Criterion 24. **Re-run Phase 4's and Phase 5's
      browser flows here, not at the end** — this is the box that can break the
      login path
- [x] **Banner and links** — `Log → …/_/log` in the banner, `/_/` links to the
      log and back. The banner must not print a URL that 404s (Phase 1), so this
      lands after the page exists, not before. Criteria 26, 27
- [x] **The build path, proven** — `cargo build --release` with no `node`, no
      `npm` and no `zero` on `PATH`; `cargo publish --dry-run --locked` compiles
      the packaged crate with `web/dist/` embedded, adding an `include`/`exclude`
      list to `Cargo.toml` if the package is wrong. Criteria 20, 21 — run now, so
      Phase 10 inherits a proven path rather than a hopeful one
- [x] **Fresh evidence** — light and dark picker screenshots replace
      `docs/decisions/evidence/phase04-1-picker.png`, plus a shot of the log
      showing two `client_id`s interleaved and one showing the PKCE triple.
      Criterion 25
- [x] **Docs** — `README.md` gains the live-log section: the three surfaces,
      `lanyard logs`, the `/_/log` page, **the sentence saying the log prints
      authorization codes, refresh tokens and any `client_secret` sent**, and the
      contributor note that `web/dist/` is a committed artifact regenerated with
      `zero build` — there is no CI gate on it

## Acceptance

Mirrors the spec's acceptance criteria. `/implement` is not done until every box
here passes by driving the named client.

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
      each happens.
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
      reader attached, all 500 appear on stdout, and the stalled reader — once
      resumed — receives a `dropped` marker naming how many it missed rather than
      a silent hole.
- [x] 9. `POST /_/api/events/clear` returns `204`; a subsequent reconnect replays
      nothing, and an already-open `/_/log` tab empties its own view.

**Which parameter mismatched**

- [x] 10. Complete `/oidc/authorize` in a browser to get a real `code`, then
      exchange it with `curl` using a `code_verifier` one character off. `/_/log`
      shows that `/oidc/token` event as failed, and expanding the row stacks the
      verifier presented, the S256 computed from it, and the `code_challenge`
      recorded at `/authorize`. The mismatched pair is visibly the one that
      differs, and nothing under `src/` was opened.
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
      other's rows and only those.

**Decoded claims**

- [x] 14. After a login with `scope=openid email profile`, expanding the
      `/oidc/token` event shows the ID token's decoded header and payload: `alg`
      is `RS256`, `kid` matches the banner's `Signing → kid` line, and the
      payload matches what `scripts/jose-verify.mjs` decodes from the same token.
- [x] 15. `lanyard token --as ada --aud billing-api --alg-none` produces an event
      whose `flaw` is `alg-none` and whose expanded view shows a header with
      `"alg": "none"`.
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
      `web/dist/` and compiles without `zero`.
- [x] 22. Loading `/_/` and `/_/log` in a browser, the network tab shows requests
      to `127.0.0.1:9500` and to no other host — no CDN, no Google Fonts.
- [x] 23. Every asset reference resolves under the mount: `/_/log`'s HTML names
      `/_/assets/…`, the served stylesheet names `/_/.zero/fonts/…`, and
      `curl -i http://127.0.0.1:9500/.zero/fonts/Geist-VariableFont_wght.woff2`
      is a `404`.

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
      URL returns `200`.
- [x] 27. `/_/` links to `/_/log`, and `/_/log` links back.

## Progress notes

Divergences from the plan as written, and why.

- **The two-SSE-clients test moved from box 2 to box 9.** Box 2 asked for it
  before `GET /_/api/events` existed. What box 2 could prove — and does, in
  `log_layer::emits` and `tests/live_log.rs` — is that every excluded path emits
  nothing; the two-open-readers assertion landed with the endpoint.

- **`BusSignal::Event` carries an `Arc<Event>`, not an inline `Event`.** Clippy's
  `large_enum_variant` fires on lanyard's event, which carries decoded claims
  where cubby's carries a bucket and a key. `Arc` rather than clippy's suggested
  `Box` because a broadcast channel clones the signal once per subscriber: two
  open tabs and a `lanyard logs` should cost three refcount bumps, not three deep
  copies. The one name that had to diverge from cubby, and the reason is
  lanyard's payload.

- **`pretty()` prints UTC.** `std::time` has no local offset and a timezone crate
  was not among the three dependencies the plan budgeted. Named in the README
  rather than left to be discovered.

- **`granted_scope()` was added after criterion 1 was first driven.** An RP
  exchanging an authorization code sends `code` and a verifier and no `scope`, so
  reading the form alone left the token line silent on exactly the thing criterion
  1 asks it to name. It now falls back to the `scope` claim on the token that came
  out. Two unit tests pin both sources and which one wins.

- **Refusals after a code record is taken are labelled with its `client_id`.**
  Driving criterion 10 showed the failed row with an em-dash in the CLIENT column:
  the exchange form names no client and only the success path was reading the
  record. `invalid_grant_for` fixes the three post-record refusals — "`client_id`
  on every event, always" is what makes the log readable with three projects
  running.

- **The status pills and the rejection card had a contrast bug.** The restyle
  paired `--color-*-soft` backgrounds with `--color-*-fg` foregrounds; `-fg` is
  the foreground for text on the *solid* colour — white in light, near-black in
  dark — so it was unreadable in both themes. Caught by looking at the first
  screenshot, not by a test. On a soft background the solid token is the readable
  one; amber takes its `-hover` step, being the one hue legible against neither of
  its own soft backgrounds.

- **Criterion 1 holds on the second login, not the first.** The spec requires
  `POST /_/pick` to emit, so a login that shows the picker produces three lines
  and the persona is named on the pick line. A login against a remembered
  selection produces exactly the two the criterion describes, with `ada` on the
  authorize line and the granted scopes on the token line. Both are recorded in
  the acceptance run; the criterion's wording only fits the second.

- **Criterion 11 is nine shell lines plus one test.** A refresh token lives eight
  hours, so its expiry sentence cannot be produced by a shell script.
  `scripts/phase06-invalid-grant-log.sh` drives the other nine (waiting the 61
  seconds an authorization code needs) and exits `0`; all ten, including that
  one, are pinned by
  `tests/live_log.rs::every_invalid_grant_description_reaches_the_log_verbatim`,
  which builds stores whose codes and refresh tokens die on issue.

- **`Cargo.toml` gained an `exclude` list.** The first `cargo package --list`
  carried 179 files — `docs/`, `spikes/`, the zero sources and `.claude/` were
  three quarters of it. `exclude` rather than `include`, so a new source file
  ships by default and dropping one is the deliberate act. 179 files / 3.7 MiB →
  67 files / 1.1 MiB, with `web/dist/` intact.

- **`rust-embed` pulls `mime_guess`, `walkdir` and a second `sha2` major.** All
  pure Rust, all small, nothing on the build path. Looked at before committing, as
  the plan asked.

- **Box 18's tests were written after its code.** The claims viewer was
  implemented in the same pass as box 17's screen, so its assertions were written
  against working code rather than red-first. They were then held to the same
  standard the loop exists to enforce: `zero mutate` on the route surfaced eight
  real gaps — the pause label, the `open` class, the method and status classes,
  the batch latch — and each one became an assertion. The two behavioural mutants
  still surviving are equivalent (a skip and a latch, both perf-only) and are
  commented as such in the source.
