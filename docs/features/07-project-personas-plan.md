# Personas that travel with the repo — plan

**Status:** done · **Spec:** [07-project-personas-spec.md](07-project-personas-spec.md) · **Roadmap:** Phase 7

## Approach

**One new subsystem, and five call sites that learn a parameter.**

Today `AppState.personas: Personas` is one parsed file (or the three built-ins),
immutable for the life of the process, and `Personas::get(id)` is called from
`seam.rs`, `ui/mod.rs`, `oidc/token.rs`, `oidc/authorize.rs` and
`oidc/userinfo.rs`. Phase 7 replaces that field with a **`Registry`** that merges
many sources and answers per-`client_id`, and every one of those five call sites
gains the `client_id` it already had sitting in scope.

Three shape decisions carry the phase:

**`Personas` stays what it is — one parsed file — and `Registry` holds many.**
`Origin` gains a `Project(PathBuf)` variant, which is all the precedence ladder
needs to distinguish a project file from the global one. No parser changes, no
schema changes: `lanyard.yaml` and `users.yaml` are the same format read by the
same code, which is why *"the `client:` field has been in the schema since Phase
1"* pays off here as an addition rather than a migration (CONCEPT §15).

**`resolve()` returns an `Arc<Resolved>`, guarded by a `stat`.** No `notify`, no
watcher thread — **north star 5**, and the spec's stated reason: a watcher brings
platform backends and the editor atomic-rename problem for an observable
(`edit → reload → see it`) that a modified-time comparison already delivers. The
resolve is fully synchronous and no lock crosses an `.await`, the same discipline
`Browser::of` already follows in `ui/mod.rs`.

**The visibility rule is one predicate applied at every surface, not a
per-surface rule.** A request with no `client_id` is not an exemption — it sees
exactly what an unrecognised `client_id` sees, the unscoped personas — so
`/_/api/token` and the seam need no special case. And nothing is ever *rejected*
for being unregistered: **north star 1** survives intact because the rule filters
a list, it never gates a grant.

The two CLI-facing halves fall out of that. `--client` on `lanyard token` is not
a new concept — it sets the `client_id` on a `client_credentials` grant lanyard
already accepts from anyone. And the useful refusal (*"it is defined in
`/path/lanyard.yaml` scoped to client `billing-web`"*) is written by the server
at `token.rs`'s existing `bad_request()` funnel and surfaced verbatim by the
CLI's existing `MintError::Rejected`, so it costs one lookup and no new plumbing.

## Files

- `src/registry.rs` — **new.** `Registry` (sources, the stat-guarded cache,
  `resolve()`), `Resolved` (`visible_to`, `get`, `all`, `find_anywhere`),
  `Sourced` (a persona plus its effective `client:` and its `Origin`), `Warning`
  and its `Display`/JSON. The precedence ladder lives here and nowhere else
- `src/links.rs` — **new.** The registry file: read, write, add, remove.
  Absent means empty, never an error
- `src/persona.rs` — `Origin::Project(PathBuf)`; `Personas` keeps its parser
  unchanged
- `src/config.rs` — `LinksSource` and `LANYARD_LINKS`, resolved beside
  `PersonasSource`
- `src/app.rs` — `AppState.personas: Personas` → `Registry`
- `src/ui/mod.rs` — the picker filters by the pending request's `client_id`; the
  unfiltered view; the "N hidden · Show all" line; the warning band;
  `signed_in_row` and `pick` scope their lookups
- `src/seam.rs` — `/_/api/personas` gains `?client_id=`, per-row `client` and
  `source`, and a `warnings` array; `/_/api/token` scopes its persona lookup
- `src/oidc/token.rs` — `client_credentials` scopes its lookup and writes the
  useful refusal
- `src/oidc/userinfo.rs` — the `sub` lookup is scoped by the token's `client_id`
  claim
- `src/oidc/authorize.rs` — resolves once at the top (so warnings are always
  available to attach) and scopes the remembered-selection lookup
- `src/log_detail.rs`, `src/log_layer.rs`, `src/events.rs` — `warnings` rides
  back on `LogDetail` into `Event`, and `pretty()`'s `tail` prints it
- `src/banner.rs` — one line per source, each with its persona ids, plus any
  warning
- `src/main.rs` — `Command::Link`, `Unlink`, `Links`; `--client` on `MintArgs`;
  `serve()` builds a `Registry`
- `src/client.rs` — `MintRequest.client_id`
- `web/src/lib/log.ts`, `web/src/routes/live-log.ts`, `web/dist/` — `warnings`
  on the event type and in the row
- `spikes/dotnet-web/lanyard.yaml` — **new**, `client: billing-web`
- `spikes/php-web/lanyard.yaml` — **new**, `client: spike-php`
- `spikes/node-spa/lanyard.yaml` — **new**, deliberately **no** `client:`
- `spikes/README.md` — the walkthrough now shows per-project pickers
- `tests/support/mod.rs` — `spawn_with_registry`; `spawn_with(Personas)` kept
- `tests/links.rs` — **new.** The registry file, link/unlink/links, precedence,
  warnings, live reload
- `tests/personas.rs`, `tests/web_ui.rs`, `tests/http.rs`, `tests/cli.rs` — the
  visibility rule at each surface
- `README.md` — the whole persona section, the env table, three subcommands,
  `--client`

## Risks & unknowns

- **`/_/` emits no event, by Phase 6's design.** `log_layer::emits` returns
  `false` for `/_/` on the stated grounds that "a page is not a decision", and
  there is an explicit unit test asserting it. So the spec's criterion 19 cannot
  literally pair *"reload `/_/`"* with *"a warning event appears"*. **The
  adjustment**: warnings ride back on `LogDetail` and appear on the next
  *protocol* request's event — which is the request that was about to get the
  wrong picker anyway. Criterion 19 below spells out the two observations
  separately (the band on `/_/`, the event on `/oidc/authorize`). Reopening
  `emits("/_/")` was considered and rejected: it would log every picker render
  and every reload, which is the noise Phase 6 excluded on purpose.
- **`/oidc/authorize` only resolves personas conditionally today** — inside
  `resolve(state, &selection)`, which runs only when a remembered selection
  exists. For a warning to reliably ride on an authorize event, the handler must
  resolve **unconditionally at the top**. One line, and defensible: authorize is
  the request that decides whether the picker is needed.
- **Blocking file IO inside async handlers.** `resolve()` calls `std::fs`
  behind a mutex. Deliberate: the files are tens of lines, the call is
  stat-guarded so it re-parses only on change, and lanyard is a loopback dev
  tool. `tokio::fs` would force the whole call chain async and put an `.await`
  inside the lock — strictly worse. Worth re-checking if a source count ever gets
  large, which for linked projects it will not.
- **Test harness churn.** `tests/support/mod.rs` constructs `AppState` directly
  and five test binaries compile it. Changing the field type touches all of them
  at once; keeping `spawn_with(Personas)` as a thin wrapper over
  `spawn_with_registry` keeps that to one file.
- **`deny_unknown_fields` is load-bearing and stays.** A typo in a project file
  must remain an error — but a *non-fatal* one at serve time. The two rules meet
  in one place: `lanyard link` parses with the same strictness and refuses, so
  the fatal-ness moves to the moment the developer is standing in the directory.
- **Adding `lanyard.yaml` to the spikes changes the existing walkthroughs.**
  `spikes/README.md` documents a demo where three apps show the same picker. That
  demo becomes a *better* one — three apps, three different pickers — but the
  prose has to change with it, and Phase 4's and Phase 5's browser tests must be
  re-run to confirm the flows still complete.
- **`web/dist/` needs `zero` on the machine to rebuild.** Same accepted cost as
  Phase 6: no freshness gate, `web/.zero/` gitignored. If `zero` is unavailable,
  the log page simply does not render `warnings` and every other surface still
  does — the step is separable and ordered late for that reason.
- **`Config::from_env()` requires `HOME`.** `lanyard link` needs the config
  directory and therefore inherits that. Fine, and already true of `serve`.
- **First-linked-wins is only stable if the registry file's order is.** It is a
  YAML sequence and is rewritten preserving order; `lanyard link` appends.

## Steps

Each box ≈ one small commit moving an observable behavior. Check only when the
outcome is real, not when code is written.

- [x] **`Registry` and the visibility rule, from one source** — `src/registry.rs`:
      `Sourced`, `Resolved`, `Registry` over the *existing* global source only,
      no links yet. `Resolved::visible_to(Option<&str>)` implements the one
      sentence; `Origin` gains `Project`. `AppState.personas` becomes `Registry`
      and all five call sites compile against `resolve()`. **Observable:** every
      existing test still passes, and `LANYARD_PERSONAS` pointing at a file with
      `client: billing-web` on one persona → `/_/api/personas?client_id=billing-web`
      lists it while `?client_id=node-spa` does not.
- [x] **The picker filters** — `src/ui/mod.rs`: the persona list is filtered by
      the pending request's `client_id`. **Observable:** with that same file,
      `GET /oidc/authorize?client_id=billing-web&…` → the picker shows the scoped
      persona; `client_id=node-spa` → it does not, and both logins still complete.
- [x] **The protocol surfaces scope their lookups** — `oidc/token.rs`
      (`client_credentials`), `oidc/userinfo.rs` (by the token's `client_id`
      claim), `seam.rs` (`/_/api/token?client_id=`). **Observable:**
      `curl -d grant_type=client_credentials -d client_id=node-spa -d persona=<scoped>`
      → `400`; the same with `client_id=billing-web` → a token whose `sub` is
      that persona, and `/oidc/userinfo` with it returns that persona's claims.
- [x] **The useful refusal** — `Resolved::find_anywhere`, written through
      `token.rs`'s existing `bad_request()` funnel: *no persona "dev-admin" for
      client "lanyard-cli" — it is defined in `<path>` scoped to client
      "billing-web". Retry with --client billing-web*. **Observable:**
      `lanyard token --as dev-admin` prints exactly that on stderr, stdout empty,
      exit non-zero.
- [x] **`--client` on `token` and `env`** — `MintArgs`, `MintRequest.client_id`,
      defaulting to `CLI_CLIENT_ID`. **Observable:**
      `lanyard token --as dev-admin --client billing-web --aud billing-api`
      prints a token whose `sub` is `dev-admin` and whose `client_id` claim is
      `billing-web`.
- [x] **The links registry file** — `src/links.rs` plus `LinksSource` /
      `LANYARD_LINKS` in `src/config.rs`. Read, write, add, remove; absent means
      empty for both the default and the explicit path, which is where it differs
      from `PersonasSource`. **Observable:** unit tests round-trip a file, and a
      missing one reads as zero links rather than an error.
- [x] **`lanyard link`** — validates `<dir>/lanyard.yaml` with the same parser
      and the same strictness before recording a canonicalized absolute path;
      idempotent; needs no running server; a missing file exits non-zero naming
      it and printing a copyable minimal one. **Observable:** criteria 1–4.
- [x] **`lanyard unlink` and `lanyard links`** — `unlink` takes an optional path
      so a deleted directory can still be pruned; `links` prints path, `client:`,
      persona ids, and `missing` for a directory that is gone. **Observable:**
      criteria 5–6.
- [x] **Linked projects merge into the registry** — `Registry` reads
      `links.yaml` and each `<dir>/lanyard.yaml`; built-ins and the global file
      keep their existing relationship, and **links add, never subtract**.
      **Observable:** link a project, restart `serve`, and its personas appear in
      the picker *alongside* `ada`/`mira`/`nobody`.
- [x] **Precedence and shadow warnings** — the ladder (scoped > unscoped,
      project > global, first-linked > later) and one `Warning` per shadowed id
      naming both paths. **Observable:** two projects each defining an unscoped
      `ada` → one `ada` in the picker, the first-linked one, and a warning that
      names both files.
- [x] **Live re-resolution** — the stat guard over `links.yaml` and every source
      file; `resolve()` re-parses only what changed. **Observable:** add a
      persona to a linked `lanyard.yaml` and reload `/_/` — it appears, with no
      restart and no re-`link`. `lanyard link` a new project while `serve` is
      running and reload — its personas appear.
- [x] **Warnings are warnings, not deaths** — a missing linked file or a
      broken linked file yields `Warning`s, that project contributes nothing, and
      every other source still resolves. **Observable:** `mv` a linked file
      away and reload `/_/` → `200`, the remaining personas listed, the process
      still serving; break a file with a duplicate `id` → same; fix it and reload
      → the personas return.
- [x] **The warning band and `/_/api/personas`** — a band at the top of `/_/`
      naming the path and the error; the endpoint grows `warnings`, and each row
      grows `client` and `source`. Everything escaped, as the picker already
      escapes persona-supplied strings — the surface now includes file paths and
      client labels from a cloned repo. **Observable:** criteria 19 and 21's page
      half, plus `curl /_/api/personas | jq .warnings`.
- [x] **Warnings on the event stream** — `LogDetail.warnings` → `Event.warnings`
      → `pretty()`'s tail; `/oidc/authorize` resolves unconditionally at the top
      so the warning always has a request to ride on. **Observable:** after
      `mv`ing a linked file away, a login from `spikes/dotnet-web` prints a
      stdout line carrying the warning, and
      `lanyard logs --json | jq 'select(.warnings)'` emits it.
- [x] **The unfiltered view and "N hidden"** — `/_/` with no login in progress
      lists every persona from every source, each labelled with its client and
      its source path; the filtered picker states how many it hid and links to
      the full list. **Observable:** criteria 10 and 11.
- [x] **The banner names every source** — one line per source with its persona
      ids, and a missing or broken link shown as such rather than omitted.
      **Observable:** criterion 24.
- [x] **The log page renders warnings** — `warnings` on the TS event type and in
      the row, `zero build`, `web/dist/` committed. **Observable:** `/_/log` in a
      browser shows the warning on the authorize row.
- [x] **Spike fixtures** — `spikes/dotnet-web/lanyard.yaml` (`client: billing-web`),
      `spikes/php-web/lanyard.yaml` (`client: spike-php`),
      `spikes/node-spa/lanyard.yaml` (**no** `client:`), and `spikes/README.md`
      rewritten for the three-different-pickers demo. **Observable:** the roadmap
      criteria below become drivable against real clients; Phase 4's and Phase
      5's browser flows re-run green.
- [x] **Docs** — `README.md`: a *Personas that travel with the repo* section
      (`lanyard.yaml`, `link`/`unlink`/`links`, the one-sentence visibility rule,
      the precedence ladder, links-add-never-subtract, and why a broken project
      file warns where a broken `users.yaml` is fatal); `LANYARD_LINKS` in the
      environment table; `--client` under the CLI; and the *"read by nothing
      yet"* line about `client:` deleted, because this is the phase where it
      stops being true.

## Progress notes

- **`lanyard links` leads with the registry's own path**, then a blank line,
  then the rows — and names it on an empty registry too, so "am I looking at the
  right file" is answerable when the answer is "yes, and it is empty".
  Requested on review.
- **`lanyard link` takes the persona file, not a directory, and never infers
  one** — a change from the spec's *A directory path, not a file path*, made on
  review. The spec's reason (link/unlink/links all talk about "a project") did
  not survive contact: a command that records whatever `lanyard.yaml` happens to
  be underfoot is one you have to check afterwards, and a wrong link surfaces as
  a `client_id` in a minted token rather than as an error. So the argument is
  **required** and is the file itself; `links.yaml` holds file paths; the
  filename is a convention rather than a rule; and passing a directory is a
  refusal that prints the path that would have worked. Criteria 1–6 were re-run
  in this shape.

- **Acceptance 1–24 were driven against the real clients**: the `lanyard`
  binary for 1–6 and 16–18, `curl`/`jq` against a live `lanyard serve` for
  7–15 and 19–24, `spikes/dotnet-api` (stock `AddJwtBearer`) for 16, and
  `spikes/dotnet-web` (`AddOpenIdConnect`) and `spikes/php-web`
  (`jumbojett`) driving their own full OIDC handshakes for 7–10 and 25.
  Criterion 25's silent-refresh half only exists in a browser — no Chrome
  connection was available in the session, so the developer ran that one.

- **The five call sites were rewired in box 1, not spread across boxes 1–3.**
  Changing `AppState.personas` to a `Registry` is not separable from the
  handlers that read it — they stop compiling the moment the field type
  changes. Boxes 2 and 3 kept their *observables*: the picker filter was red
  before it was green, and box 3's four tests each survived a mutation check
  against the unscoped lookup they replaced.
- **One shadow warning per *id*, not per pair.** With the built-ins present,
  two projects both defining `ada` is three colliding entries and would have
  been three warnings. The warning names every file that defines the id and
  which one won, which is what criterion 22 asks for and reads once.
- **"Show all" keeps the login rather than dropping it.** The link is
  `/_/?req=<id>&all=1`, so the unfiltered debugging view is reachable *during*
  a login. A persona the application cannot see renders as a labelled card and
  never as a button: the visibility rule filters a list and never gates a
  grant, but offering a button whose `POST` the same rule would refuse is a
  promise the next page breaks.
- **`/_/api/personas` no longer echoes a top-level `client`.** With many
  sources there is no one file-level value to echo, so the field moved onto
  every row as the *effective* client (persona-level, else file-level) beside
  the new `source`. `personas_echoes_client_at_both_levels` became
  `personas_resolves_client_onto_every_row_and_names_the_source`.

## Acceptance

Mirrors the spec's criteria. `/implement` isn't done until every box here passes
by driving the named client.

**link, unlink, links**

- [x] 1. With no `lanyard serve` running: `lanyard link <path>/lanyard.yaml`. Exit
      `0`, stdout names the absolute path recorded and the persona ids found,
      and `cat ~/.config/lanyard/links.yaml` shows that path.
- [x] 2. `lanyard link` naming a file that is not there exits non-zero, names the
      file, and prints a copyable minimal one. `links.yaml` is byte-identical to
      before. `lanyard link` with **no argument** is a usage error and records
      nothing; given a directory it refuses and prints the path that would have
      worked.
- [x] 3. `lanyard link` on a file with a duplicate `id` exits non-zero naming the
      id and the path. `links.yaml` is unchanged.
- [x] 4. `lanyard link` twice on the same file: both exit `0`, and `links.yaml`
      holds exactly one entry for it.
- [x] 5. `lanyard unlink <file>` removes its entry, including for a file that has
      since been deleted.
- [x] 6. `lanyard links` prints the registry's own path and a blank line, then one
      line per entry with its path, its
      `client:` (or `—`), and its persona ids; a file that has been `mv`ed away
      prints as `missing` on its own line rather than being omitted.

**Namespacing — roadmap criterion 1**

- [x] 7. Link `spikes/dotnet-web/lanyard.yaml` (`client: billing-web`) and
      `spikes/php-web/lanyard.yaml` (`client: spike-php`). Start a login from `spikes/dotnet-web`: the picker
      lists dotnet-web's persona and **does not list** php-web's. Start a login
      from `spikes/php-web`: the reverse.
- [x] 8. Complete criterion 7's `billing-web` login as the project's own persona.
      The .NET app lands authenticated with that persona's email claim.
- [x] 9. `spikes/node-spa/lanyard.yaml` declares **no** `client:`. Its persona
      appears in the picker for `billing-web`, for `spike-php`, and for
      `node-spa`.
- [x] 10. The filtered picker in criterion 7 states how many personas it hid and
      offers a link to the unfiltered list; following that link shows php-web's
      persona labelled with `spike-php` and with the path of its file.
- [x] 11. `curl 'http://127.0.0.1:9500/_/api/personas?client_id=billing-web' | jq
      '[.personas[].id]'` lists exactly the visible set. Without the query
      parameter the same endpoint returns every persona from every source, each
      carrying its `client` and its source path.
- [x] 12. `GET /oidc/userinfo` with an access token minted for dotnet-web's
      persona under `client_id=billing-web` returns that persona's claims — the
      `sub` lookup is scoped like the mint was, not against the unscoped set.

**Fallback — roadmap criterion 2**

- [x] 13. A fresh machine: empty `XDG_CONFIG_HOME`, no `links.yaml`, no
      `users.yaml`. `lanyard serve`, then `curl /_/api/personas` returns exactly
      `ada`, `mira`, `nobody`, and `/_/` renders all three.
- [x] 14. With criterion 7's two projects linked, `nobody` is still pickable in
      both the `billing-web` picker and the `spike-php` picker.
- [x] 15. With a global `users.yaml` present *and* projects linked, the built-ins
      are gone (the global file still replaces them) and both the global and the
      project personas are listed under their own source paths.

**The CLI**

- [x] 16. `lanyard token --as <dotnet-web's persona> --client billing-web --aud
      billing-api` prints a token whose `sub` is that persona and whose
      `client_id` claim is `billing-web`, and
      `curl -H "Authorization: Bearer $(…)"` against the Phase 2 .NET API
      returns `200`.
- [x] 17. The same `lanyard token --as …` with no `--client` exits non-zero,
      stdout is **empty**, and stderr names the persona, the file it is defined
      in, the client it is scoped to, and the `--client` flag that fixes it.
- [x] 18. `lanyard token --as <node-spa's unscoped persona>` succeeds with no
      `--client` at all.

**Staleness and live reload — roadmap criterion 3**

- [x] 19. `mv` a linked file away while `lanyard serve` is running, then
      reload `/_/`: the page renders `200` with a warning band naming the missing
      path, every other source's personas are still listed, and the picker is not
      empty. Then start a login from `spikes/dotnet-web`: that request's line on
      `serve`'s stdout carries the warning, and
      `lanyard logs --json | jq 'select(.warnings)'` emits it naming the path.
      `mv` the directory back, reload, and its personas return with no restart.
- [x] 20. Add a persona to a linked `lanyard.yaml` and reload `/_/`: it appears.
      No restart, no `lanyard link` re-run.
- [x] 21. Introduce a duplicate `id` into a linked `lanyard.yaml` while serving.
      `/_/` renders `200` with a warning band naming the file and the error, that
      project contributes no personas, every other source still lists, and the
      `serve` process is still running. Fix the file and reload: its personas
      return.
- [x] 22. Link two projects that each define an unscoped persona `ada`. The
      picker shows one `ada` — the first-linked project's — a warning names both
      file paths and the id, and completing a login mints the first-linked
      project's claims.
- [x] 23. Log in to `billing-web` as its persona, then unlink that project and
      start a second login from the same browser: the picker is shown rather than
      a silent re-login or a `500`.

**The banner**

- [x] 24. `lanyard serve` with two files linked prints one line per source — the
      global file or the built-ins, and each linked path with its persona ids. A
      registry entry whose file is missing prints on that list as missing rather
      than being omitted.

**Regression**

- [x] 25. Phase 4's and Phase 5's browser flows re-run green against the spikes
      now that each carries a `lanyard.yaml`: full login, RP-initiated logout,
      silent refresh, and the two-apps-two-personas demo in `spikes/README.md`.
      Three were driven here over HTTP against the real apps: `dotnet-web`'s
      login lands as `dev-admin@billing.test`, `php-web`'s as `qa-bot@php.test`,
      both at once in one cookie jar, and `php-web`'s RP-initiated logout signs
      both out so `dotnet-web`'s next login shows the picker. **`node-spa`'s
      silent refresh was run by the developer in a browser** — the only place
      its `automaticSilentRenew` timer exists; the exchange behind it (a public
      client PKCE `S256` login as `kiosk`, then `grant_type=refresh_token`
      returning a fresh access token for the same `sub`) was also driven here.
