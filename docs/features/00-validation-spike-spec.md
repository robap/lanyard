# Validation spike — spec

**Status:** done · **Roadmap:** Phase 0 · **Slug:** 00-validation-spike

## Why

Two questions change the shape of later phases, and both are cheap to answer
against software that already exists. Answering them after writing product code
means rewriting it.

1. **Does plain HTTP actually work for our stack?** CONCEPT §9 and §10 assert
   that .NET's `RequireHttpsMetadata = true` default blocks discovery over
   `http://`, and that .NET's correlation/nonce cookies may need `SameSite=None`
   — which needs `Secure`, which needs HTTPS. If the second problem bites, HTTPS
   and `lanyard trust` are v1 work, not post-v1, and Phase 9 (silent renew) is
   blocked behind them. If it does not, HTTPS stays post-v1 and the roadmap is
   unchanged.
2. **Is the name clear?** Registry availability is already verified (CONCEPT
   §14); an established dev tool with the same name and audience is not, and it
   is the only thing that would force a rename. Renaming after distribution
   (Phase 10) is expensive; renaming now is free.

Serves north star 5 indirectly and the whole roadmap's ordering directly: this
phase produces **decisions**, not product code.

There is also a standing instruction in CONCEPT §12 to read
navikt/mock-oauth2-server's README end to end before building — its Docker
networking notes cover the same ground as CONCEPT §8, and reading them first is
cheaper than rediscovering them.

## In scope

- A throwaway .NET web app doing an interactive OIDC login against an existing
  HTTP-only mock provider, with the exact minimal settings required recorded.
- The same for PHP (`jumbojett/openid-connect-php`), or a recorded blocker.
- Notes from reading navikt/mock-oauth2-server's README, specifically Docker
  networking.
- A name search across GitHub and the wider web, with the verdict recorded.
- A decision on reserving `lanyard-cli` on crates.io.
- Committed output under `docs/decisions/` and `docs/notes/`.

## Out of scope

- **Any lanyard product code.** No Rust crate, no `Cargo.toml`, no server. This
  phase's only artifacts are throwaway spike apps and committed markdown.
- Implementing HTTPS or `lanyard trust`. This phase decides *when*, not *how*.
- Testing silent renew / `prompt=none` end to end (Phase 9). We only record
  whether the `SameSite`/`Secure` constraint appears in the ordinary login path.
- Deciding where personas come from (CONCEPT §15, Phase 7). Unrelated open
  question; do not fold it in.
- Evaluating mock providers as competitors or borrowing their design. They are
  instruments here, nothing more.

## Behavior

**Mock provider under test.** Docker is not installed on this machine, so the
Soluto `oidc-server-mock` container path is unavailable and
[`oidc-provider-mock`](https://pypi.org/project/oidc-provider-mock/) (Python,
serves plain HTTP, accepts any client by default) is the instrument. Install it
into a throwaway venv or via `uvx`/`pipx`; do not install anything system-wide.
If it turns out not to accept an arbitrary `client_id` without registration,
that is itself a finding worth recording — it is the property lanyard is built
around.

**Spike apps are throwaway.** They live under `spikes/dotnet-web/` and
`spikes/php-web/` and are gitignored. Only the findings under `docs/` are
committed. The spike apps exist to be watched working and then deleted; the
Phase 11 `examples/` directory is the durable version and is deliberately not
seeded from these (CONCEPT §11).

**Both logins are driven through a real browser, and through more than one.**
Available: Firefox 152.0.4 (native), Chromium 150.0.7871.128 (flatpak), and a
Google Chrome install under `/opt/google/chrome` — the one the Chrome MCP
drives. Driving the login with browser automation is fine and preferable, since
it leaves a screenshot as evidence; what is *not* acceptable is an HTTP client
walking the redirect chain, because a cookie jar is the thing under test and
curl does not have the one that matters.

Run the .NET login in **both a Chromium-family browser and Firefox**, because
they do not agree about the cookie behavior this phase exists to measure.
Chromium enforces `SameSite=Lax` as the default for cookies that set no
`SameSite` attribute; Firefox does not enforce Lax-by-default. A .NET
correlation or nonce cookie that emerges unmarked can therefore pass in one and
fail in the other, and a single-browser "it works" would send the HTTPS decision
the wrong way. If the two disagree, that disagreement *is* the finding — record
both, and let the stricter browser decide the answer.

(Chromium here is a flatpak; flatpak apps share the host network namespace by
default, so `http://localhost:<port>` should reach the spike apps. Confirm it
early rather than mid-login.)

**The .NET question is answered by subtraction.** Start from a login that works,
then remove one setting at a time and watch what breaks. A settings list nobody
tried to shrink is not evidence of a minimum. Specifically we need to know:

- Is `RequireHttpsMetadata = false` sufficient on its own?
- Does anything have to be said about `CookieSecurePolicy`,
  `MinimumSameSitePolicy`, or the OIDC handler's correlation/nonce cookies?
- If "Correlation failed" appears, record the exact message and what fixed it.
  CONCEPT §10 predicts this specific error; confirming or refuting it is the
  point of the exercise.

The .NET version on this machine is 10.0.110 — record it, since the cookie
defaults are the sort of thing that shifts between majors.

**PHP is runnable.** PHP 8.3.6 (CLI) and Composer 2.10.3 are installed, and
`php -S` is enough of a web server for the spike — no nginx or Apache needed.
One gap remains: `php -m` shows `json` and `openssl` but **not `curl`**, and
`jumbojett/openid-connect-php` declares `ext-curl` as a hard requirement, so
`composer require` will refuse outright. Installing the distro's
`php8.3-curl` package is the expected fix and should be the first step of the
PHP leg. Record the PHP version and extension set alongside the .NET one — the
point of both legs is what a real runtime does, and "which runtime" is part of
the finding.

CONCEPT §10 expects PHP to be relaxed about `http://` and not to block us. Now
that the toolchain is present, the roadmap's "or the blocker is recorded"
escape applies only to a blocker discovered *while running the spike*, not to a
missing toolchain.

**The name check needs to be reproducible.** Record the actual queries run and
the top results, not a conclusion. "No conflict" with no search terms attached
cannot be re-checked in six months when someone asks.

**The HTTPS decision is the phase's real output.** It must name what was
observed, not what was reasoned. "Login completed with only
`RequireHttpsMetadata = false`; correlation cookies were `SameSite=Lax` by
default and no `Secure` was required → HTTPS stays post-v1" is a decision.
"HTTPS is probably fine to defer" is not.

## Acceptance criteria

- [x] A throwaway .NET 10 web app using `AddOpenIdConnect` against
      `oidc-provider-mock` over `http://` completes a full interactive browser
      login: the browser is redirected out, a user is chosen, it lands back on
      the app's callback, and the app renders that user's `email` (or `name`)
      claim on the page. Watched in a real browser — hand-driven or automated,
      but not an HTTP client following redirects.
- [x] The same .NET login is completed in **both** a Chromium-family browser and
      Firefox, with the browser and version recorded for each. If one succeeds
      and the other fails, both outcomes and the failing browser's error text are
      recorded, and the stricter result drives the HTTPS decision.
- [x] With `RequireHttpsMetadata = false` removed and everything else unchanged,
      the same app fails, and the exact error text is pasted into
      `docs/decisions/dotnet-http-settings.md`.
- [x] `docs/decisions/dotnet-http-settings.md` lists the minimal setting set,
      the .NET SDK version used, and — for each setting listed — the observed
      failure when it is removed. A setting with no recorded failure is not in
      the minimal set.
- [x] A throwaway PHP app served by `php -S`, using
      `jumbojett/openid-connect-php`, completes `authenticate()` against the same
      mock and prints the chosen user's email in the browser.
- [x] `docs/decisions/php-spike.md` records the PHP version, the extensions that
      had to be installed to get `composer require` to resolve, and whether
      anything beyond default settings was needed to accept an `http://` issuer
      (e.g. `setHttpUpgradeInsecureRequests(false)`) — each entry observed, not
      assumed. If the spike is blocked by something discovered while running it,
      that file records the failing command instead.
- [x] `docs/decisions/https-priority.md` exists, states "v1" or "post-v1" in its
      first line, and cites the specific .NET observation above as the reason.
- [x] `docs/notes/navikt-mock-oauth2-server.md` exists and names at least three
      concrete Docker-networking behaviors from that README — issuer/host
      handling, port handling, container-vs-browser addressing — each with a
      one-line note on whether lanyard should copy, differ, or ignore.
- [x] `docs/decisions/name-check.md` exists, lists the exact search queries run
      (GitHub and web), the top results for each, and a one-line verdict: no
      conflict, or rename.
- [x] `docs/decisions/crates-io-reservation.md` exists, records the observed
      output of a `lanyard-cli` availability check, and states reserve or skip.
- [x] `git status` is clean of spike code — `spikes/` is gitignored and no
      `Cargo.toml` or server source has been added to the repo.

## Open questions

- ~~**Install PHP and Composer, or defer the PHP leg?**~~ Resolved — both are
  installed (PHP 8.3.6, Composer 2.10.3), so the PHP leg is in scope as a real
  login, not a recorded blocker. Still needs `php8.3-curl`, which the plan
  should install as its first PHP step.
- ~~**Is a browser available for the interactive login?**~~ Resolved — Firefox
  152 (native), Chromium 150 (flatpak), and Google Chrome under
  `/opt/google/chrome`, plus a Chrome MCP for automation. This turned into a
  scope *addition* rather than a constraint: both browser families get exercised,
  because they disagree about default `SameSite` enforcement.

None blocking. The spec is ready to plan against.
