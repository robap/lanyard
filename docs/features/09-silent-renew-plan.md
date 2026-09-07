# Silent renew — plan

**Status:** done · **Spec:** [09-silent-renew-spec.md](09-silent-renew-spec.md) · **Roadmap:** Phase 9

## Approach

**Measure before building.** The spec's central claim — that `SameSite` ignores
the port, so `localhost:5173` → `localhost:9500` is same-site and
`localhost:5173` → `127.0.0.1:9500` is not — is a prediction. Steps 1–4 make the
iframe path runnable and then run it in two browsers across both rows, and the
trace those steps produce is what steps 10 and 12 are allowed to write down.
Phase 0 established this order and it is the reason its decision doc is worth
reading; a `doctor` note or a README rule written before the trace would be a
guess wearing a citation.

**The refactor is one enum.** `authorize()` today computes `remembered` and
`always_ask`, filters into `usable`, and lands every miss on one
`redirect_error` with one sentence. Replacing `Option<Selection>` with a
`Resolution` enum that names *why* it missed gives all six descriptions from one
place, costs nothing on the happy path, and leaves the interactive branch alone
— it still falls through to the picker for every variant, because "show the
picker" is the right answer to all six when somebody can click. Only
`prompt=none`, which cannot ask, has to tell them apart. This is CONCEPT §6's
"why did my login fail" answered in the tool's own output, one phase after
Phase 6 built the surfaces to answer it on: `redirect_error` already calls
`attach`, so a better sentence reaches the RP's query string, stdout, the SSE
stream and `logs --json` with no new plumbing.

**`id_token_hint` gets its own verifier.** `jws::verify` enforces `exp`, and a
hint is *expected* to be expired — it is the token from the session being
renewed. So a separate `hint::subject_of()` checks the signature and `iss` and
reads `sub`, and nothing else. No `aud`, no `azp`, no expiry (spec, question 2).

**Nothing renders and no cookie changes.** `src/session.rs`'s
`the_cookie_is_never_secure_and_never_expires` test stays exactly as it is —
north star 4 says a renew that works through a `Secure` cookie the browser
drops over HTTP is worse than one that fails honestly.

## Files

- `src/oidc/authorize.rs` — `Resolution` enum replacing the `usable` filter; six
  `login_required` descriptions; `id_token_hint` check on the `prompt=none` arm.
- `src/oidc/hint.rs` — **new.** `subject_of(key, issuer, token) -> Result<String,
  String>`: signature + `iss` + `sub`, deliberately no `exp`.
- `src/oidc/mod.rs` — declare `hint`.
- `src/doctor.rs` — one note on `issuer_check` when the issuer host is
  `127.0.0.1` (step 12, gated on step 4's trace).
- `tests/browser_flow.rs` — the six descriptions; `id_token_hint`; the
  frameability assertion; `auth_time` preserved across a renew.
- `tests/live_log.rs` — the six descriptions arriving on `logs --json`.
- `spikes/node-spa/index.html` — `?renew=iframe` toggle, `silent_redirect_uri`.
- `spikes/node-spa/silent-callback.html` — **new.** `signinSilentCallback()`.
- `spikes/node-spa/README.md` — how to run both paths.
- `docs/decisions/silent-renew-over-http.md` — **new.** The measurement, the
  verdict on HTTPS.
- `docs/decisions/https-priority.md` — *Revisit when* → *Revisited*.
- `README.md` — troubleshooting entries; a silent-renew section under Sessions.

## Risks & unknowns

- **The `localhost` ≠ `127.0.0.1` prediction could be wrong in either
  direction.** If the cross-site row *works*, some browser leniency is holding it
  up and the decision doc has to say which — that is the "works only through
  browser leniency that will not hold in production" case CONCEPT §9 names, and
  it argues for HTTPS more strongly than a clean failure would. If the same-site
  row *fails*, HTTPS moves ahead of Phases 10–11 and this plan's steps 5–9 are
  still correct but the phase's conclusion inverts. **Steps 10 and 12 are
  written after step 4, not before.**
- **`localhost` may resolve to `::1` while `DEFAULT_BIND` is `127.0.0.1`.**
  Step 1 will hit connection-refused on a machine where that happens, and the
  workaround is `LANYARD_BIND=::1` or `::`. Worth noting in the decision doc,
  because it is the concrete cost of leaving the default issuer alone
  (spec, question 1) and the thing a future default-issuer phase has to fix
  first.
- **The `403`-shaped failure has no observable at all.** A `prompt=none` whose
  `redirect_uri` fails the loopback check renders a `400` inside a hidden iframe:
  no postMessage, no network error, just `silentRequestTimeoutInSeconds`
  elapsing. Nothing can fix this without breaking the one rejection, so step 14
  documents it and points at `lanyard logs`.
- **Six descriptions are six chances to leak a persona id to an RP that should
  not have one.** They already get the `client_id` back and lanyard has no
  authorization model — but the sentences go in the query string of a URL that
  ends up in browser history, so cause (e) names the persona id and cause (f)
  does not name the *hinted* subject, only that it did not match.
- **Two browsers, by hand.** Criteria 1–2 cannot be automated here; the Rust
  tests keep the wiring honest between runs, exactly as `tests/browser_flow.rs`
  already says of itself.

## Steps

Each box ≈ one small commit moving an observable behavior. Check only when the
outcome is real.

**Make the iframe path runnable — nothing in `src/` yet**

- [x] 1. Add `spikes/node-spa/silent-callback.html` — a page that calls
      `oidc.UserManager.prototype.signinSilentCallback` and nothing else, and
      point `silent_redirect_uri` at it. Observable: loading
      `http://localhost:5173/silent-callback.html?error=login_required&state=x`
      directly renders a blank page and logs no exception, where loading `/`
      with the same query string tries a redirect-callback and fails.
- [x] 2. Add the `?renew=iframe` toggle to `index.html` — it drops
      `offline_access` from `scope` (spec, question 3). Observable: with the
      toggle, `mgr.getUser()` after a login has `refresh_token === null`; without
      it, a refresh token is present. Both render the shared page identically.
- [x] 3. Update `spikes/node-spa/README.md` with both run recipes and what each
      one is evidence of.

**Measure — this is the phase**

- [x] 4. Run the matrix and save the traces. `LANYARD_ISSUER=http://localhost:9500/oidc`
      versus the stock `127.0.0.1`, app on `http://localhost:5173`, Chrome for
      Testing and Firefox, `?renew=iframe`. For each cell capture the iframe
      `/authorize` request's `Cookie` header, the `Location` it got back, and
      the console line. Save them under `docs/decisions/evidence/` beside
      Phase 0's. **Acceptance criteria 1 and 2.**

**Six causes, one enum**

- [x] 5. Replace the `usable` filter in `authorize()` with a `Resolution` enum
      — `Usable(Persona, u64)` plus `NoCookie`, `NoSelection`, `AlwaysAsk`,
      `TooOld { max_age, age }`, `PersonaGone { persona_id }`. The interactive
      branch treats every non-`Usable` variant identically (picker), so nothing
      changes there. Observable: `cargo test` green, `tests/browser_flow.rs`
      untouched.
- [x] 6. Give each variant its own `login_required` description on the
      `prompt=none` arm. Observable: five `curl -i` calls, five different
      `error_description` values, `NoCookie`'s naming that no cookie arrived and
      that a cross-site iframe is the usual reason. **Acceptance criterion 4
      (a)–(e).**
- [x] 7. Assert the same five reach the log: `lanyard logs --json | jq -r
      .error_description`. `redirect_error` already calls `attach`, so this is a
      test in `tests/live_log.rs`, not new code. **Acceptance criterion 5.**

**`id_token_hint`**

- [x] 8. Add `src/oidc/hint.rs::subject_of` — signature, `iss`, `sub`, and
      **no `exp` check**. Observable: a unit test where a token 10 minutes past
      its `exp` yields its `sub`, and one signed by another key is refused.
- [x] 9. Wire it into the `prompt=none` arm: a hint whose `sub` differs from the
      remembered selection returns `login_required` rather than a code for the
      wrong person; an unverifiable hint is `invalid_request`. Observable: two
      `curl` calls against a session logged in as Ada — a hint for Ada goes
      through, a hint for Mira comes back `login_required` with a sixth distinct
      description that does *not* echo the hinted subject. **Acceptance
      criterion 4 (f).**

**Pin what must not regress**

- [x] 10. Frameability: assert no `X-Frame-Options` and no
      `Content-Security-Policy` on a `prompt=none` success `302`, a
      `login_required` `302`, and the rendered `400` for a non-loopback
      `redirect_uri`. A test that exists to fail the day somebody hardens the
      headers, in the shape `session.rs`'s cookie test already established.
      **Acceptance criterion 10.**
- [x] 11. `auth_time` survives a renew: a `prompt=none` code exchanged for an ID
      token carries the same `sub` and the same `auth_time` as the original
      login's, and a different `iat`. `resolve()` already returns
      `selection.auth_time`, so this pins a property rather than building one —
      an `auth_time` that advanced would let an RP's `max_age` pass forever
      without anybody authenticating. **Acceptance criterion 8.**

**Write down what step 4 found**

- [x] 12. `docs/decisions/silent-renew-over-http.md` — the traces, the site
      table with measurements replacing predictions, and **in the first line**
      whether HTTPS moves ahead of Phases 10–11. Update
      `docs/decisions/https-priority.md`'s *Revisit when* section to point at it
      and stop calling the question open. **Acceptance criterion 3.**
- [x] 13. `doctor`: one note on `issuer_check` when the issuer host is
      `127.0.0.1`, naming the consequence for a hidden-iframe renew (spec,
      question 4). **Only if step 4 confirmed the diagnosis** — if the
      cross-site row worked, this box is checked by deleting itself and saying
      so here. Observable: `lanyard doctor` with the stock issuer prints the
      note; with `LANYARD_ISSUER=http://localhost:9500/oidc` it does not.
- [x] 14. Docs — `README.md`: a silent-renew subsection under **Sessions**
      covering the same-name rule and what `prompt=none` returns; two
      troubleshooting entries — the verbatim `oidc-client-ts` renew error with
      the cross-site diagnosis (Phase 0's obligation #2 format, next to
      "Correlation failed"), and *a silent renew that times out with no network
      response was probably a refused `redirect_uri` rendered invisibly in the
      iframe — check `lanyard logs`*. Note `id_token_hint` under **The
      authorization endpoint**. **Acceptance criteria 14 and 15.**

## Acceptance

Mirrors the spec. Not done until each passes by driving the named client.

**The measurement**

- [x] 1. Same-name, Chrome **and** Firefox: iframe `/authorize?…prompt=none`
      carries `Cookie: lanyard_session=…`, gets a `code`, `POST /oidc/token`
      follows, no picker, address bar unchanged, console prints
      `[node-spa] access token renewed`.
- [x] 2. Cross-name, both browsers: the `Cookie` header, the `Location` and the
      console line recorded verbatim, and the outcome reproducible by changing
      only `authority`.
- [x] 3. `docs/decisions/silent-renew-over-http.md` states the HTTPS verdict in
      its first line, citing 1 and 2; `https-priority.md` links to it.

**The six causes**

- [x] 4. Six `curl -i` calls → six different `error_description` values, all
      `error=login_required`, (a)'s naming that no cookie arrived.
- [x] 5. `lanyard logs --json | jq -r .error_description` prints the same six.
- [x] 6. No response body contains picker markup and none is a `200`.

**The renewed token is a real token**

- [x] 7. The renewed access token verifies with `scripts/jose-verify.mjs` and
      its `exp` is later than the pre-renew token's.
- [x] 8. Renew vs. original ID token: same `sub`, same `auth_time`, different
      `iat`.
- [x] 9. After `mgr.signoutRedirect()`, the next renew's iframe gets
      `error=login_required` and the SPA shows the signed-out page.

**Frameability**

- [x] 10. No `X-Frame-Options` and no `Content-Security-Policy` on the
      `prompt=none` `302`, the `login_required` `302`, or the rendered `400`.

**Nothing earlier regressed**

- [x] 11. `node-spa` without the toggle still renews over
      `grant_type=refresh_token`, no `/authorize`, no iframe.
- [x] 12. `the_cookie_is_never_secure_and_never_expires` passes; `cargo test`
      green.
- [x] 13. `http://localhost:5173/` still completes a full redirect login and
      renders the shared page once.

**Written down**

- [x] 14. README troubleshooting carries the same-name rule and the verbatim
      `oidc-client-ts` error.
- [x] 15. README explains the invisible `400` and points at `lanyard logs`.

## Progress notes

- **Step 1's observable, corrected.** The plan's contrast used
  `?error=login_required&state=x` on both pages, but `index.html` branches on
  `code=`, not on any query string — so that URL renders the signed-out page
  rather than attempting a redirect callback. The trap is real and the check is
  the same shape with `?code=abc&state=x`: `silent-callback.html` is blank and
  silent, `/` prints `FAILED: Error: No matching state found in storage`.
- **Step 2 gained a line the plan did not name.** The toggle has to survive the
  round trip through lanyard, so it rides on `redirect_uri` and
  `post_logout_redirect_uri` as well (`http://localhost:5173/?renew=iframe`) and
  is re-applied by the `history.replaceState` after the callback. Without that,
  the page that came back from the picker rebuilt its manager in refresh-token
  mode. lanyard needed no change for it: the one rejection is about the host.
- **Two spike fixes found while making the path runnable**, both in
  `spikes/node-spa/index.html` and neither in the plan:
  `.replace('{{APP}}', …)` filled the shared page's `<title>` — which this SPA
  throws away — and left the `<h1>` reading `{{APP}}` literally; it is
  `replaceAll` now. And `addSilentRenewError` logged the error *object*, which
  Firefox renders as the bare word `Error`; it logs `err.message` now, which is
  `error_description` as lanyard wrote it, and both browser families print the
  same readable line.
- **Step 4 ran under Playwright** driving Google Chrome for Testing
  149.0.7827.55 and Mozilla Firefox 152.0.4 — the same two builds Phase 0's
  evidence names. Playwright is not a dependency of anything in the repository;
  it was resolved out of the machine's npm cache and the harness lives in a
  scratch directory. The renew was never provoked from the console: each cell
  logs in and waits for `accessTokenExpiringNotificationTimeInSeconds` to fire
  on its own, which is what criterion 1 asks for.
- **The site table holds, in both browser families, in both directions.**
  Trace: `docs/decisions/evidence/phase09-1-matrix.txt`.
- **`localhost` resolves to `::1` only on this machine** (`getent hosts
  localhost` → `::1`) while `DEFAULT_BIND` is `127.0.0.1`, and the predicted
  connection-refused did **not** happen: both browsers and `curl` fall back to
  the A record. Worth recording in the decision doc as *survived by fallback*
  rather than *not a problem*.
- **Steps 5 and 6 landed in one edit.** The `Resolution` enum and its six
  descriptions are the same change viewed twice — the enum has no reason to exist
  except to carry them — so splitting the commit would have meant a variant set
  nothing reads. Step 5's stated observable (`cargo test` green,
  `tests/browser_flow.rs` untouched) held on its own: no existing test asserts on
  the sentence, only on `error=login_required`. Step 6 was then TDD'd properly —
  the new tests were run against `git show HEAD:src/oidc/authorize.rs` first and
  fail there on the assertion (`two causes must not share a sentence`, and
  `name the persona: prompt=none was sent and this browser has no usable
  selection…`), not on a compile error.
- **Cause (a) names three reasons, not one.** Acceptance criterion 9 turned up
  the gap: after `signoutRedirect()` the cookie is genuinely gone, so the *no
  cookie* sentence fires — and leading a developer who just logged out with "the
  usual reason is a hidden iframe" is the wrong diagnosis. It now names the
  logout and the restart too, which is what the spec's own table row said all
  along ("third-party iframe, cleared jar, or a restart").
- **Step 13's `doctor` note shipped**, because step 4 confirmed the diagnosis
  rather than refuting it. It is attached as a consequence line without changing
  the check's level, so the shipped default does not print a `WARN` at every
  developer who is not building a SPA — and `--strict` still exits zero.
  `tests/doctor.rs`'s "one heading and six checks, no more" assertion counted
  *lines*; it counts level columns now, because a note is an explanation of a
  check rather than a check.
- **Criterion 9 needed a hand-driven iframe.** `oidc-client-ts` stops its own
  renew timer on `removeUser`, so after a logout no iframe fires by itself and
  there is nothing to observe. The measurement drives the identical navigation
  the library would have — a real hidden iframe at the same silent-renew URL, in
  the same browser, with the same cookie jar — and gets `login_required`.
- **The counterfactual was measured rather than argued.** The decision doc's
  claim that `SameSite=None; Secure` would not have needed a CA was, as first
  written, a prediction wearing a citation — exactly what this plan's *Approach*
  warns against. It was run: on a throwaway patch to `src/session.rs`, reverted
  immediately, the cross-site row renews in both browsers over plain HTTP. That
  makes the reason for keeping `Lax` north star 4 rather than "it wouldn't help",
  which is the stronger and the honest argument.
- **Every acceptance run is reproducible but nothing was added to the
  repository.** The Playwright harness lives in the session scratch directory and
  resolves Playwright out of the machine's npm cache; `Cargo.toml`,
  `scripts/package.json` and `spikes/node-spa/vendor/` are untouched.
