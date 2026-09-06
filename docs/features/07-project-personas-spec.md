# Personas that travel with the repo — spec

**Status:** done · **Roadmap:** Phase 7 · **Slug:** 07-project-personas

## Why

**The `client:` field has been in the persona schema since Phase 1 and nothing
reads it.** `src/persona.rs` says so out loud:

> Parsed, validated, echoed back — and read by nothing until Phase 7. Carrying it
> from day one is what turns Phase 7 into an addition rather than a migration.

The README says the same thing to users — "`client:` is accepted at both levels,
echoed back by `/_/api/personas`, and read by nothing yet." This is the phase
that collects that debt, and it is the last genuinely open design question in
CONCEPT (§15).

The problem it closes is a consequence of north star 2. **A machine-wide
singleton is not sitting in any project directory.** Today there is exactly one
persona list on the machine — `~/.config/lanyard/users.yaml`, or the three
built-ins — and it is the same list for every application on every port. Two
things follow, and both get worse the more the tool is used:

| | What happens today |
|---|---|
| Personas do not travel | A repo that needs `dev-admin`, `billing-readonly` and `locked-out-user` has no way to ship them. Every developer who clones it hand-edits one global file, and the README has to explain that instead of the tool doing it. |
| The list only grows | Ten projects' worth of personas land in one file, and every picker on the machine shows all forty. The picker is the product; a scrolling list of strangers is the product getting worse with use. |

CONCEPT §15 lays out three options and states a lean:

> **Current lean:** linking plus client-id namespacing, with a global file as
> fallback so the zero-config path still works on a fresh machine.

This spec takes that lean and makes it concrete. **The two halves solve
different problems and are independent**, which is the shape decision worth
stating first:

- **`lanyard link`** makes personas travel. A project commits `lanyard.yaml`; a
  developer runs one command after cloning; the personas are there.
- **`client:`** makes the picker clean. It is *opt-in*, and a project that never
  sets it still gets everything link gives it.

Conflating them would force every project into namespacing to get travelling
personas, and namespacing has a cost — see *The CLI problem* below.

### Which north stars it serves

- **North star 1 — accept everything.** The visibility rule added here is not
  registration, and it must not be mistaken for it. lanyard still accepts any
  `client_id` from anyone; a `client_id` no persona names simply sees the
  unscoped personas and logs in fine. Nothing is ever *rejected* for being
  unregistered. The rule filters a list a human reads; it does not gate a grant.
- **North star 2 — one instance, every project.** This is the phase where "one
  instance" stops being a tax on the tenth project. The singleton stays a
  singleton and each project gets its own view of it.
- **North star 5 — single static binary.** Notably: **no filesystem watcher.**
  See *How the daemon notices* — the observable requirement is "edit the file,
  reload the page, see the change", and a `stat` on read delivers that without
  `notify`, its platform quirks, or its dependency tree.

## In scope

- **`lanyard link`** — records the current directory (or an argument path) in a
  machine-wide registry, after validating that its `lanyard.yaml` parses.
  Requires no running `lanyard serve`.
- **`lanyard unlink`** — removes it, including when the directory is gone.
- **`lanyard links`** — the registry's own path, a blank line, then one line per
  link: path, state, persona ids. This is how a stale link becomes actionable
  rather than merely warned about, and how "which registry did it read" is
  answerable when `LANYARD_LINKS` has moved it.
- **A project file, `lanyard.yaml`, in the same schema as `users.yaml`.** Same
  parser, same `deny_unknown_fields`, same `client:` at both levels. Committed to
  the repo — that is the entire point.
- **The visibility rule**, one sentence, applied at every surface that has a
  `client_id`: `/oidc/authorize`, `/oidc/token`, `/oidc/userinfo`, `/_/`,
  `/_/api/personas`, `/_/api/token`.
- **Merge and precedence** across built-ins, the global file, and every linked
  project, with shadowed ids warned rather than silently resolved.
- **Live re-resolution.** Editing, breaking, removing or restoring a linked
  project's file is reflected on the next page load with no restart.
- **Warnings that are not fatal**, surfaced in three places from one source: the
  startup banner, the Phase 6 event stream, and a band on `/_/`.
- **`--client` on `lanyard token` and `lanyard env`**, so a scoped persona is
  reachable from the CLI at all.
- **`LANYARD_LINKS`** to point the registry somewhere else, so tests never touch
  a developer's real config directory.

## Out of scope

- **Client registration in any costume.** No client list, no per-client
  settings, no "unknown client" error. A `client:` value is a label a persona
  wears, not a record lanyard keeps. (North star 1, and CONCEPT §4's "Out".)
- **Deriving the CLI's `client_id` from the working directory.** Considered and
  deferred — see *The CLI problem* and open question 1.
- **`client:` as a list.** One string per persona in this phase. Accepting a
  sequence later is additive in YAML — every file written under this spec still
  parses — so deferring it costs no migration, unlike deferring the field itself
  would have. Open question 2.
- **A filesystem watcher.** See *How the daemon notices*.
- **Persisting anything else across restarts.** Sessions, codes and pending
  requests still die with the process. The links registry is the only new file,
  and it holds paths, not state.
- **Auto-discovery of `lanyard.yaml` by walking up from anywhere.** The roadmap
  chose registration; a daemon that scanned the filesystem for persona files
  would be both slower and more surprising.
- **A UI for linking.** `/_/` displays links and their state; it does not add or
  remove them. Linking is a thing you do in a project directory, which is a
  place a browser is not.

## Behavior

### The visibility rule

One sentence, and everything else in this section is a consequence of it:

> **A persona is visible to a request if it declares no `client:`, or if its
> `client:` equals that request's `client_id`.**

`client:` may be declared at the file level, at the persona level, or both. A
persona-level value wins; a file-level value applies to every persona in that
file that does not declare its own. There is no third level and no inheritance
beyond that.

Every surface either carries a `client_id` or is defined not to have one, so the
rule needs no per-surface special cases:

| Surface | Its `client_id` | Consequence |
|---|---|---|
| `GET /oidc/authorize` | the query parameter | The picker shows that application's people |
| `POST /oidc/token`, `client_credentials` | the form parameter | `lanyard token` mints from the same set the picker shows |
| `POST /oidc/token`, `authorization_code` / `refresh_token` | — | The persona is already in the code or refresh record; no lookup, no filtering |
| `GET /oidc/userinfo` | the `client_id` claim on the presented access token | The `sub`→persona lookup is scoped the same way the token was minted |
| `GET /_/api/personas` | the `client_id` query parameter, optional | With it, the visible set; without it, **everything**, each row labelled |
| `POST /_/api/token` | the `client_id` query parameter, optional | Absent behaves as a `client_id` nobody scoped to: the unscoped set |
| `GET /_/` | from the pending request, when a login is in progress | Filtered during a login, unfiltered otherwise |

The absent-`client_id` cases are not an exemption. A request with no `client_id`
sees exactly what a request with an unrecognised `client_id` sees — the unscoped
personas — because that is what the one sentence already says.

### Where the personas come from

Three sources, merged on every resolve:

1. **The built-ins** — `ada`, `mira`, `nobody`. Unscoped.
2. **The global file** — `~/.config/lanyard/users.yaml`, or `LANYARD_PERSONAS`.
   Unchanged from today, including the rule that its presence **replaces** the
   built-ins entirely and that a malformed one is fatal at startup.
3. **Every linked project's `<dir>/lanyard.yaml`**, in registry order.

**Links add; they never subtract.** A linked project does not suppress the
built-ins the way the global file does. That is deliberate: `nobody` is the
flagship persona — "the user who breaks applications and the one nobody
remembers to create" — and a developer who links a project should not lose it.
A project that wants a picker showing only its own people uses `client:`, which
hides the *other* projects' forty strangers; three built-ins remaining is a
feature, not clutter.

Consequence worth stating plainly, because it is what makes the roadmap's third
criterion achievable at all: **the picker can never be emptied by a link going
bad.** Whatever happens to a project file, sources 1 or 2 are still there.

### Precedence, when two sources use the same id

An id collision only matters between two personas *both visible to the same
request*. When that happens, the ladder is:

1. A client-scoped persona beats an unscoped one.
2. Among equals, a project file beats the global file.
3. Among project files, **the one linked first wins**.

First-linked rather than last-linked, so that linking a new project can never
silently steal an id out from under a project that already had it.

Every shadowing emits one warning naming both file paths and the id. It is a
warning and not an error because the resolution is deterministic and the login
still works — but it is exactly the kind of thing that produces "why did I get
the wrong claims", so it must not be silent.

### Malformed and missing files: fatal at link time, a warning at serve time

Phase 1 established that a malformed `users.yaml` is fatal:

> There is no fallback to the defaults: a persona that silently failed to load
> shows up three redirects later as a name missing from the picker.

That rule is right and stays, but it cannot be extended to project files.
**A machine-wide daemon must not die because one of ten projects has a typo** —
that would make linking a project an act of sabotage against the other nine.

The fatal-ness moves rather than disappearing:

| When | Behavior |
|---|---|
| `lanyard link` | **Fatal.** Parses the file first; a bad one exits non-zero naming the offending key and the path, and records nothing. The developer is standing in the directory, which is the moment the error is cheapest. |
| `lanyard serve`, global file | **Fatal at startup**, unchanged. |
| `lanyard serve`, linked project file broken *later* | **Warning.** That project contributes no personas; every other source still resolves; the process keeps serving. |
| `lanyard serve`, linked file gone | **Warning**, same handling. The entry stays in the registry — a file can be temporarily absent — and `lanyard unlink` is what prunes it. |

Warnings reach the developer in three places, from one source:

- **The startup banner** — one line per source, so "why is Ada not there" is
  still answerable from the first eight lines of output.
- **The Phase 6 event stream** — as they occur, so `lanyard logs --json` and
  `/_/log` see a project file break in real time.
- **A band at the top of `/_/`** — naming the path and the error, because the
  picker is where the missing person is noticed.

`/_/api/personas` carries the same warnings as an array, so a test can assert on
them without scraping HTML.

### How the daemon notices — a `stat`, not a watcher

The observable requirement is: **edit a linked project's `lanyard.yaml`, reload
`/_/`, see the change.** Two implementations satisfy it.

A filesystem watcher (`notify`) is the obvious one and the wrong one here. It
brings a dependency tree onto a single-static-binary project, platform-specific
backends, and the atomic-rename problem — most editors save by writing a temp
file and renaming over the original, which silently breaks a naive watch on the
original inode. It would also need its own thread and its own error path for
"the watch died".

Instead: **resolve on demand, guarded by a `stat`.** A resolve happens at most
once per request that needs personas; it compares each source file's modified
time and length against what was last parsed and re-parses only what changed.
The files are tens of lines. A missing directory is discovered the same way, at
the same moment, with no separate code path — which is precisely the "warning,
not a crash" case.

This does mean the persona set stops being an immutable field on `AppState` read
by five call sites. That structural consequence is real and belongs to `/plan`;
what this spec fixes is the observable behavior, which is that no restart is
ever required to pick up a persona change.

### The links registry

`~/.config/lanyard/links.yaml`, or wherever `LANYARD_LINKS` points:

```yaml
links:
  - /home/dev/code/billing/lanyard.yaml
  - /home/dev/code/ops-console/dev.yaml
```

Decisions, each with its reason:

- **The config directory, not the data directory.** The data directory is
  documented as "signing key lives here", carries its own `.gitignore`, and is
  designed to be deletable — deleting it regenerates the same key deterministically.
  Losing every link because someone cleared a cache directory would be a bad
  surprise. Links sit next to `users.yaml`, which is the other file that answers
  "where do personas come from".
- **A directory path, not a file path.** The file is always `lanyard.yaml` in
  that directory. Storing the directory keeps `lanyard link` / `unlink` / `links`
  talking about the same thing the developer types: a project.
- **Absolute and canonicalized at link time.** A relative path in a registry read
  by a daemon with a different working directory means nothing.
- **Ordered.** The order is the precedence tie-break, so it is a list and it is
  preserved.
- **Hand-editable, and absent means empty.** An absent registry is the
  zero-config state, not an error — for `LANYARD_LINKS` too, which is where it
  differs from `LANYARD_PERSONAS`. Pointing at a persona file that is not there
  leaves you with no personas and an empty picker, which is why that case is
  fatal; having no links is the normal condition of a fresh machine.

### `lanyard link`, `unlink`, `links`

```
$ lanyard link ~/code/billing/lanyard.yaml
linked /home/dev/code/billing/lanyard.yaml — 3 personas for client billing-web
  dev-admin, billing-readonly, locked-out
```

- **The file is named, never discovered.** The argument is required.
- **Validates before recording.** The file must exist and parse.
- **Idempotent.** Linking an already-linked file succeeds and does not
  duplicate the entry.
- **Needs no running server.** It writes a file. A running `lanyard serve` picks
  the change up on its next resolve; a not-yet-running one reads it at startup.
- A file that is not there exits non-zero, names it, and prints a minimal one to
  copy — the error is also the documentation.

```
$ lanyard unlink ~/code/billing/lanyard.yaml
unlinked /home/dev/code/billing/lanyard.yaml
```

Unlinking a path that no longer exists on disk still removes the entry — a link
to a project you deleted is the one you most want to prune.

```
$ lanyard links
/home/dev/.config/lanyard/links.yaml

/home/dev/code/billing/lanyard.yaml    billing-web  dev-admin, billing-readonly, locked-out
/home/dev/code/ops-console/dev.yaml    —            ops-bot
/home/dev/code/old-thing/lanyard.yaml  missing      (no such file)
```

### The CLI problem

`lanyard token --as ada` sends `client_id=lanyard-cli`. Under the visibility
rule, a persona scoped to `billing-web` is invisible to it. CLI minting is
"probably the larger half of daily use", so that cliff has to be addressed
rather than discovered.

Two things address it, and the first is the important one:

**Namespacing is opt-in.** A project that links its `lanyard.yaml` without
declaring `client:` gets travelling personas that work everywhere, CLI included.
Most projects will start there and reach for `client:` only when their picker
gets crowded.

**`--client` for the projects that do scope.** It is not a new concept — it sets
the `client_id` on a grant lanyard already accepts from anyone:

```
$ lanyard token --as dev-admin --client billing-web --aud billing-api
```

And the failure without it must be the useful one, not `no such persona`:

```
$ lanyard token --as dev-admin
lanyard: no persona "dev-admin" for client "lanyard-cli" — it is defined in
  /home/dev/code/billing/lanyard.yaml scoped to client "billing-web".
  Retry with --client billing-web
```

That message is the difference between this feature being usable and being
maddening, and it is an acceptance criterion for that reason.

Deriving the client from the working directory — `cd billing && lanyard token
--as dev-admin` just working — is the obvious next step and is deliberately not
in this phase. It makes the `client_id` claim in a minted token depend on
invisible context, on a command whose whole contract is that stdout carries the
token and nothing else. Open question 1.

### What the picker looks like

**With a login in progress**, the list is filtered to the visible set. It has to
be — the clean-picker-per-project property is half of why the phase exists. But
filtering silently would recreate "why is Ada not there" one layer down, so the
page says what it hid:

> 4 other personas are scoped to other applications. [Show all]

**With no login in progress** — the banner's `UI →` line — the page lists every
persona from every source, each row labelled with the client it is scoped to and
the file it came from. This is the debugging view, and it is the one a developer
opens when the filtered view surprised them.

A persona id and a `client_id` are both developer-supplied strings that reach
this page, and both go through the existing escaping. That is not new, but the
surface area grew: this phase renders file paths and client labels that came out
of a project file somebody cloned.

### Sessions and a persona that goes away

A browser selection is `(session, client_id) → persona_id`, and `/oidc/authorize`
already handles the id no longer resolving:

> A selection that no longer names anybody falls through to the picker rather
> than failing the login.

Editing a project file, unlinking it, or scoping a persona to a different client
all land in that existing path. Nothing new is required, and the criteria assert
it still holds rather than assuming it.

## Acceptance criteria

Each names a client and an operation, or an assertion against a file or a
rendered page.

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
      id and the path. `links.yaml` is unchanged — a broken file is never
      recorded.
- [x] 4. `lanyard link` twice on the same file: both exit `0`, and `links.yaml`
      holds exactly one entry for it.
- [x] 5. `lanyard unlink <file>` removes its entry, including for a file that has
      since been deleted.
- [x] 6. `lanyard links` prints the registry's own path and a blank line, then one
      line per entry with its path, its
      `client:` (or `—`), and its persona ids; a file that has been `mv`ed away
      prints as `missing` on its own line rather than being omitted.

**Namespacing — roadmap criterion 1**

- [x] 7. Link project A (`client: billing-web`, persona `dev-admin`) and project
      B (`client: spike-php`, persona `qa-bot`). Start a login from
      `spikes/dotnet-web` (`client_id=billing-web`): the picker lists `dev-admin`
      and **does not list** `qa-bot`. Start a login from `spikes/php-web`
      (`client_id=spike-php`): the reverse.
- [x] 8. Complete criterion 7's `billing-web` login as `dev-admin`. The .NET app
      lands authenticated with `dev-admin`'s email claim.
- [x] 9. Link project C with a `lanyard.yaml` declaring **no** `client:`. Its
      persona appears in the picker for `billing-web`, for `spike-php`, and for
      `node-spa`.
- [x] 10. The filtered picker in criterion 7 states how many personas it hid and
      offers a link to the unfiltered list; following that link shows `qa-bot`
      labelled with `spike-php` and with the path of project B's file.
- [x] 11. `curl 'http://127.0.0.1:9500/_/api/personas?client_id=billing-web' | jq
      '[.personas[].id]'` lists exactly the visible set. Without the query
      parameter the same endpoint returns every persona from every source, each
      carrying its `client` and its source path.
- [x] 12. `GET /oidc/userinfo` with an access token minted for `dev-admin` under
      `client_id=billing-web` returns `dev-admin`'s claims — the `sub` lookup is
      scoped like the mint was, not against the unscoped set.

**Fallback — roadmap criterion 2**

- [x] 13. A fresh machine: empty `XDG_CONFIG_HOME`, no `links.yaml`, no
      `users.yaml`. `lanyard serve`, then `curl /_/api/personas` returns exactly
      `ada`, `mira`, `nobody`, and `/_/` renders all three.
- [x] 14. With criterion 7's two projects linked, `nobody` is still pickable in
      both the `billing-web` picker and the `spike-php` picker. Links add and
      never subtract.
- [x] 15. With a global `users.yaml` present *and* projects linked, the
      built-ins are gone (the global file still replaces them) and both the
      global and the project personas are listed under their own source paths.

**The CLI**

- [x] 16. `lanyard token --as dev-admin --client billing-web --aud billing-api`
      prints a token whose `sub` is `dev-admin` and whose `client_id` claim is
      `billing-web`, and `curl -H "Authorization: Bearer $(…)"` against the
      Phase 2 .NET API returns `200`.
- [x] 17. `lanyard token --as dev-admin` with no `--client` exits non-zero,
      stdout is **empty**, and stderr names the persona, the file it is defined
      in, the client it is scoped to, and the `--client` flag that fixes it.
- [x] 18. `lanyard token --as <persona from criterion 9's unscoped project>`
      succeeds with no `--client` at all.

**Staleness and live reload — roadmap criterion 3**

- [x] 19. `mv` a linked file away while `lanyard serve` is running, then
      reload `/_/`: the page renders `200` with a warning band naming the missing
      path, every other source's personas are still listed, and the picker is not
      empty. `lanyard logs --json` carries one warning event naming the path.
      `mv` it back, reload, and its personas return with no restart.
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
- [x] 23. Log in to `billing-web` as `dev-admin`, then unlink project A and start
      a second login from the same browser: the picker is shown rather than a
      silent re-login or a `500`.

**The banner**

- [x] 24. `lanyard serve` with two files linked prints one line per source — the
      global file or the built-ins, and each linked path with its persona ids. A
      registry entry whose file is missing prints on that list as missing rather
      than being omitted.

## Open questions

Neither blocks the criteria above; both have a recommendation, and both change
work only inside their own bullet.

1. **Should `lanyard token` derive `--client` from the working directory?**
   `cd billing && lanyard token --as dev-admin` working with no flag is
   genuinely nicer, and it is what "personas travel with the repo" implies for
   the CLI half. The implementation is small: walk up from the working directory
   for a `lanyard.yaml` and read its `client:`.

   **Recommendation: not in this phase.** It makes the `client_id` claim of a
   minted token depend on where the shell happened to be, on a command whose
   contract is that stdout is the token and nothing else — there is no line
   available to say "using client billing-web from ./lanyard.yaml". `--client`
   is one flag and never surprises. If you want the inference, say so and it
   goes in the criteria as its own numbered item; it does not change anything
   else in the spec.

2. **Should `client:` accept a list — `client: [billing-web, billing-spa]`?**
   The real case is one project with a server and a SPA front end that share
   personas under two `client_id`s, which is exactly `spikes/dotnet-web` plus
   `spikes/node-spa`.

   **Recommendation: string only, revisit post-v1.** Unlike the field itself —
   which had to land in Phase 1 because adding it later would have been a
   migration — widening a scalar to accept a sequence is additive: every file
   written under this spec keeps parsing unchanged. The two-client project can
   link and leave `client:` off today. Say so if you would rather have it now.
