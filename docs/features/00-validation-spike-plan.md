# Validation spike — plan

**Status:** done · **Spec:** [00-validation-spike-spec.md](00-validation-spike-spec.md) · **Roadmap:** Phase 0

**Outcome: HTTPS is post-v1.** See [https-priority.md](../decisions/https-priority.md).

## Approach

No product code — the deliverable is committed markdown under `docs/decisions/`
and `docs/notes/`, backed by two throwaway apps that were watched logging in.
The instrument is `oidc-provider-mock`, run via `uvx` so nothing is installed
system-wide and there is no venv to clean up.

Three design choices worth naming:

- **Seed the mock with a persona rather than typing a subject into its custom-user
  box.** The acceptance criterion is that the app renders an `email` claim, and a
  bare `sub` produces no email. `--user-claims` gives us `ada@example.test` to
  click. This also exercises the same shape lanyard's own picker will have.
- **Order the .NET leg as *get it working → take it apart*.** The subtraction
  experiments are worthless until there is a known-good baseline to subtract
  from, and the baseline is what proves the login at all.
- **The HTTPS decision box comes after both language legs, not after .NET.**
  It is the phase's real output and it should not be written while half the
  evidence is outstanding. Serves the roadmap's stated purpose for Phase 0:
  "HTTPS moves into or out of the v1 list" — one decision, made once.

The mock provider was probed while planning, so these are observed, not assumed:
issuer is `http://localhost:9400`, authorize is `/oauth2/authorize`, jwks is
`/jwks`, and `--require-registration` defaults to **false** — an arbitrary
`client_id=spike` renders the identity picker with no registration step. That
last one matters: it means the instrument shares lanyard's "accept everything"
property (north star 1), so the spike is measuring runtime/browser behavior and
not fighting the mock.

## Files

- `.gitignore` — add `spikes/`
- `spikes/dotnet-web/` — new, gitignored. `dotnet new web` + OpenIdConnect
- `spikes/php-web/` — new, gitignored. `composer require jumbojett/openid-connect-php`
- `docs/decisions/dotnet-http-settings.md` — new
- `docs/decisions/php-spike.md` — new
- `docs/decisions/https-priority.md` — new; the phase's output
- `docs/decisions/name-check.md` — new
- `docs/decisions/crates-io-reservation.md` — new
- `docs/notes/navikt-mock-oauth2-server.md` — new
- `README.md` — likely untouched; see the docs box

## Risks & unknowns

- **`sudo` is required for `php8.3-curl`.** `/implement` cannot type a password.
  Stop and ask the user to run it with the `! ` prefix rather than attempting it.
- **The issuer is `http://localhost:9400`, not `127.0.0.1`.** .NET compares
  `iss` against the discovery document's `issuer` by exact string (CONCEPT §10).
  Setting `Authority` to `http://127.0.0.1:9400` will fail *for a reason
  unrelated to HTTP-vs-HTTPS* and could easily be misread as evidence for HTTPS.
  Keep `localhost` on both sides, and if this trips anyway, it belongs in the
  navikt/Docker notes as a live demonstration of CONCEPT §8 — not in the HTTPS
  decision.
- **NuGet and Packagist need network.** Both spikes restore from the internet on
  first run.
- **The flatpak Chromium's network namespace** — confirmed shared by default, but
  verify `http://localhost:5000` loads there before blaming the login.
- **A browser that "works" may be leniency, not correctness.** Chromium and
  Firefox disagree on default `SameSite` enforcement; the spec requires both, and
  the stricter one decides. Do not stop at the first green.
- **Scope drift risk:** the mock's authorize page is a persona picker that looks
  a lot like the one we intend to build. The spec puts borrowing its design out
  of scope. Note it and move on.

## Steps

Each box ≈ one small commit moving an observable behavior. Check only when the
outcome is real, not when code is written.

- [x] Ignore the spike tree — add `spikes/` to `.gitignore`; create
      `spikes/dotnet-web/` and touch a file in it; `git status` stays clean
- [x] Run the instrument seeded with a persona — `uvx oidc-provider-mock
      --user-claims '{"sub":"ada","email":"ada@example.test","name":"Ada Bell"}'`;
      `curl .../.well-known/openid-configuration` shows
      `"issuer":"http://localhost:9400"`, and loading `/oauth2/authorize?client_id=spike&…`
      in a browser renders Ada as a clickable identity
- [x] Stand up the .NET spike — `dotnet new web` + `AddOpenIdConnect` +
      `AddCookie`, `Authority=http://localhost:9400`,
      `RequireHttpsMetadata=false`, one `[Authorize]` page echoing
      `User.FindFirst("email")`. `dotnet run --urls http://localhost:5000` starts
      clean
- [x] **Complete the .NET login in a Chromium-family browser** — driven via
      Selenium (Chrome MCP unavailable); browser redirects out, Ada is chosen,
      lands back on `/signin-oidc` and the page shows `ada@example.test`.
      Screenshot kept in `docs/decisions/evidence/`
- [x] **Complete the same login in Firefox 152** — same observable outcome, or a
      recorded divergence. Record browser + version for both
- [x] Subtract `RequireHttpsMetadata` — remove it, retry the login, capture the
      exact failure text verbatim (this is the criterion CONCEPT §10 predicts)
- [x] Subtract everything else, one at a time — for each remaining setting,
      remove → retry → record the failure, or drop it from the minimal set
      because nothing broke. A setting with no recorded failure is not minimal
- [x] **Measure the secure-context boundary** — added mid-flight. The login only
      survives because `Secure` cookies are accepted on `http://localhost`; move
      the app's own origin off localhost, change nothing else, and record what
      happens. This is what makes the HTTPS box a decision rather than a guess
- [x] Write `docs/decisions/dotnet-http-settings.md` — .NET 10.0.110, both
      browsers + versions, the minimal set, and the observed failure behind each
      entry. Committed
- [x] Unblock PHP — `php8.3-curl` installed (**needs the user's `sudo`**);
      `php -m` now lists `curl`
- [x] Stand up the PHP spike — `composer require jumbojett/openid-connect-php`
      resolves; a single `index.php` calling `authenticate()` against
      `http://localhost:9400`, served by `php -S localhost:5001`
- [x] **Complete the PHP login in a browser** — Ada chosen, page prints
      `ada@example.test` via `requestUserInfo('email')`
- [x] Write `docs/decisions/php-spike.md` — PHP 8.3.6, the extension that had to
      be installed, and whether anything beyond defaults was needed for an
      `http://` issuer (e.g. `setHttpUpgradeInsecureRequests(false)`). Committed
- [x] **Write `docs/decisions/https-priority.md`** — "v1" or "post-v1" as the
      first line, citing the specific observation from the two legs above. This
      is the box the rest of the roadmap is waiting on
- [x] Read navikt/mock-oauth2-server's README end to end → write
      `docs/notes/navikt-mock-oauth2-server.md` with ≥3 concrete Docker
      networking behaviors, each marked copy / differ / ignore
- [x] Name check → `docs/decisions/name-check.md` with the exact GitHub and web
      queries run, top results per query, and a one-line verdict
- [x] `lanyard-cli` availability → `docs/decisions/crates-io-reservation.md` with
      the observed check output and a reserve-or-skip decision
- [x] Tear down and verify — spike apps deleted or left gitignored; `git status`
      shows only `docs/` and `.gitignore`; no `Cargo.toml`, no server source
- [x] Docs — confirm no `README.md` change is needed. Phase 0 adds no
      user-facing surface and there is no binary to install yet; a README
      promising `lanyard serve` would be false until Phase 1

## Progress notes

- **The Chrome MCP was unavailable this session** (extension not connected), so
  both browser legs were driven with Selenium against the machine's real
  installed browsers instead. This turned out better than the planned approach:
  Selenium drives Firefox too, so the two-browser requirement is met by one
  harness, and each run leaves screenshots. The Chromium-family browser on this
  machine is `/opt/google/chrome`, which self-reports as **Google Chrome for
  Testing 149.0.7827.55** — not the Chromium 150 flatpak the spec listed. The
  flatpak was not needed and its network namespace was never tested.
- **Added a box: "Measure the secure-context boundary."** The subtraction pass
  showed .NET marks its correlation/nonce cookies `secure; samesite=none`
  unprompted — CONCEPT §10 confirmed — yet the login passed anyway. Writing the
  HTTPS decision off "it worked" would have missed *why* it worked. Moving only
  the app's origin off `localhost` reproduces "Correlation failed" on demand and
  identifies the actual variable. The plan had no box for this and the decision
  would have been weaker without it.
- **`ResponseType` was expected to be inert and was not.** .NET's default is
  `id_token` (implicit), which CONCEPT §4 puts out of lanyard's scope. Recorded
  in the .NET decision doc as a lanyard compatibility item, not just a spike note.
- No divergence between Chrome and Firefox. The spec anticipated a `SameSite`
  disagreement deciding the phase; it did not arise, because .NET marks the
  cookies explicitly rather than leaving them unmarked.
- **The PHP leg got the same two extra experiments**, for the same reason: the
  spec asks whether `setHttpUpgradeInsecureRequests(false)` was needed, and the
  only honest way to answer is to remove it and re-run (it was not needed). The
  non-localhost origin test was then re-run on PHP to check whether .NET's
  `Correlation failed` wall is a general property or a .NET one. It is .NET's —
  PHP completed the login on a plain hostname origin. That contrast is what lets
  the HTTPS decision be narrow instead of hedged.
- **The flatpak Chromium was never used**, so its network namespace was never
  tested. The risk the plan flagged did not come up; noted in case a later phase
  assumes that check was done.
- **`sudo` for `php8.3-curl` went to the user as planned** and was the only
  blocker on the PHP leg — `composer require` resolved first try afterwards.

## Acceptance

Mirrors the spec. Not done until every box passes by driving the named client.

- [x] A throwaway .NET 10 web app using `AddOpenIdConnect` against
      `oidc-provider-mock` over `http://` completes a full interactive browser
      login and renders the chosen user's `email` claim — watched in a real
      browser, not an HTTP client following redirects
- [x] The same login completed in **both** a Chromium-family browser and Firefox,
      with browser and version recorded for each; any divergence recorded, and
      the stricter result drives the HTTPS decision
- [x] With `RequireHttpsMetadata = false` removed, the app fails and the exact
      error text is in `docs/decisions/dotnet-http-settings.md`
- [x] `docs/decisions/dotnet-http-settings.md` lists the minimal setting set, the
      SDK version, and an observed failure for every setting listed
- [x] A PHP app served by `php -S` using `jumbojett/openid-connect-php` completes
      `authenticate()` and prints the chosen user's email in the browser
- [x] `docs/decisions/php-spike.md` records PHP version, extensions installed,
      and whether non-default settings were needed for an `http://` issuer
- [x] `docs/decisions/https-priority.md` states "v1" or "post-v1" in its first
      line, citing the .NET observation
- [x] `docs/notes/navikt-mock-oauth2-server.md` names ≥3 concrete Docker
      networking behaviors, each with a copy / differ / ignore note
- [x] `docs/decisions/name-check.md` lists the exact queries, top results, and a
      verdict
- [x] `docs/decisions/crates-io-reservation.md` records the observed availability
      check and states reserve or skip
- [x] `git status` is clean of spike code — `spikes/` gitignored, no `Cargo.toml`
      or server source added
