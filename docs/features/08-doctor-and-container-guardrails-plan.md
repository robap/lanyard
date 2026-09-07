# `doctor` and the container guardrails — plan

**Status:** done · **Spec:** [08-doctor-and-container-guardrails-spec.md](08-doctor-and-container-guardrails-spec.md) · **Roadmap:** Phase 8

## Approach

Four independent pieces, in dependency order: **one clock**, then **one health
endpoint**, then **`doctor`** on top of both, then **the image** that ships
them.

The structural work is the clock. Five copies of `unix_now()` and a sixth read
in `events.rs` currently mean the process has six clocks that happen to agree —
and `LANYARD_CLOCK_SKEW` is the moment they stop agreeing, with a log whose
timestamps contradict the tokens it is describing. So `src/clock.rs` becomes the
single reader, a `Clock` lands on `AppState` beside the key and the config, and
it is **injected rather than global**: `issue()` and `jws::verify` take it as an
argument, exactly as `claims_at(now, …)` already takes `now`. That keeps the
in-process test harness able to spawn a skewed server and an unskewed one in the
same test binary — a `OnceLock` could not. This is north star 3's argument
(*issuance is one function*) applied to time: two places that read the clock is
the same drift, one layer down.

Everything else is additive. `/_/health` is a new module merged into the
existing `/_` nest; the `Host` sighting set is the second piece of interior
mutability on `AppState` after `Stores`, and it reuses Phase 6's event
`warnings` field and Phase 7's `/_/` warning band rather than inventing
surfaces. `doctor` is a new subcommand over `reqwest`, which is already a
dependency — **no new crate on the build path** (north star 5). The image ships
the one static binary and nothing else, which is what forces its `HEALTHCHECK`
to be `lanyard doctor --quiet` and therefore what makes `doctor`'s exit-code
contract load-bearing rather than decorative.

The spec's three open questions are planned at their recommendations: ship
`LANYARD_CLOCK_SKEW`, no `compose.yaml`, one `Host` rule with no
loopback exemption. Steps 1, and the absence of a compose step, are where that
shows.

## Files

- `src/clock.rs` — **new.** `Clock` (`Copy`, holds a signed second offset),
  `now()`, `now_millis()`, `LEEWAY`, and the `LANYARD_CLOCK_SKEW` parser.
- `src/config.rs` — `skew: i64` on `Config`, parsed and fatal on garbage the way
  `LANYARD_PORT` is.
- `src/app.rs` — `AppState` gains `clock: Clock` and `hosts: HostSightings`.
- `src/hosts.rs` — **new.** The sticky set of mismatching `Host` values, and the
  one function that decides whether an authority matches the issuer's.
- `src/health.rs` — **new.** `GET /_/health`.
- `src/doctor.rs` — **new.** The six checks, their outcome type, and the
  rendering. Pure comparison functions separated from the fetching, so the
  arithmetic is unit-testable without a server.
- `src/main.rs` — the `doctor` subcommand, `--url`, `--strict`, `--quiet`.
- `src/oidc/issue.rs` — `nbf` backdated by `LEEWAY`; clock injected.
- `src/oidc/jws.rs` — `exp` leeway, an `nbf` check, three distinct messages.
- `src/oidc/userinfo.rs`, `src/oidc/revocation.rs`, `src/oidc/authorize.rs`,
  `src/ui/mod.rs`, `src/events.rs` — their local clock reads replaced.
- `src/log_layer.rs` — publishes the first sighting of a mismatching `Host`,
  including on the two paths `emits()` excludes.
- `src/ui/mod.rs` — the warning band takes host warnings alongside the
  registry's.
- `src/banner.rs` — the `Clock →` line when skewed.
- `src/keys.rs` — the unwritable-data-dir guardrail.
- `src/lib.rs` — module declarations.
- `Dockerfile`, `.dockerignore` — **new.** Builder stage + distroless static.
- `scripts/phase08-container.sh` — **new.** Drives the whole container path so
  the criteria are repeatable rather than a paragraph of shell in a review.
- `tests/health.rs`, `tests/doctor.rs`, `tests/clock.rs` — **new.**
- `tests/cli.rs`, `tests/http.rs`, `tests/live_log.rs` — the clock signature
  change and the new warning.
- `README.md` — `doctor`, `/_/health`, the skew variable, the container section.

## Risks & unknowns

- **`podman build` defaults to the OCI image format, which has no
  `HEALTHCHECK`.** The instruction is silently dropped and
  `podman inspect .State.Health` is then absent, which reads as "the health
  check is broken". The build must be `--format docker` (or the run must pass
  `--health-cmd`). The script records the flag so nobody rediscovers this.
- **Backdating `nbf` will break existing assertions** that expect `nbf == iat` —
  in `issue.rs`'s own tests and probably in `tests/issue.rs`. Expected, and
  cheap; the risk is only that one of them is asserting it for a *reason* worth
  reading before changing.
- **`jws::verify` gaining an `nbf` check is a behavior change to Phase 5's
  endpoints.** Its module comment says it deliberately does not check `nbf`
  because lanyard never issues a future one — `LANYARD_CLOCK_SKEW` is exactly
  what makes that false, so the comment gets rewritten rather than deleted.
- **`doctor` must ignore `LANYARD_CLOCK_SKEW`.** A developer with it exported
  runs a skewed `serve` and a skewed `doctor` and is told the clocks agree,
  which is the one lie this whole phase exists to prevent. `doctor` reads the
  true clock always and says the variable is set.
- **Criterion 18's `/etc/hosts` line is the operator's to add, not
  `/implement`'s.** `127.0.0.1 lanyard` needs `sudo`, and a tool that edits a
  machine-wide name resolution file on its own behalf is not a tool this project
  ships. When the run reaches criterion 18, `/implement` **stops and asks**,
  printing the line to paste:

  ```
  echo '127.0.0.1 lanyard' | sudo tee -a /etc/hosts
  ```

  The operator adds it, says so, and the run continues. `scripts/phase08-container.sh`
  behaves the same way: it checks whether the name already resolves to loopback
  and exits with that line rather than attempting privilege. Criterion 18 also
  needs a working `spikes/dotnet-web`; if the .NET spike cannot run, the
  criterion is blocked, not silently narrowed.
- **Distroless has no shell**, so `/data` cannot be created with `RUN mkdir`. It
  is `COPY --from=builder --chown=65532:65532` of an empty directory. If that
  trick fails on this podman version, the fallback is `WORKDIR /data`, which
  buildah creates.
- **The 20 MB size ceiling in criterion 15** is an assertion about a build that
  has not happened yet. A statically linked musl binary with `rust-embed`'s
  450 KB of UI should land around 12–15 MB; if it does not, the number in the
  criterion is what is wrong, not the image — record the real figure rather than
  stripping features to reach an invented one.
- **`host.containers.internal` exists in rootless podman 4.9 via pasta or
  slirp4netns**, but is not guaranteed on every configuration. `doctor` prints
  it as a suggestion, never as a fact it has verified.

## Steps

Each box ≈ one small commit moving an observable behavior. Check only when the
outcome is real, not when the code is written.

- [x] **One clock, and a deliberately wrong one.** `src/clock.rs` holds the only
      `SystemTime` read in the crate; `Clock` lands on `AppState` and is injected
      into `issue()` and everything else that reads wall time — including
      `events.rs`, so a skewed process timestamps its log with the clock it mints
      with. `LANYARD_CLOCK_SKEW` accepts `-5m`, `+90s`, `300`, and is fatal on
      anything else, naming the variable the way `LANYARD_PORT` does. The banner
      gains a `Clock →` line **only** when skewed, naming the consequence.
      *Observable:* `LANYARD_CLOCK_SKEW=-5m lanyard serve` prints the skew line,
      and a token minted a second later has an `iat` five minutes in the past
      while `lanyard logs` timestamps that same request five minutes in the past
      too. Unset, nothing anywhere changes.
- [x] **`nbf` is backdated 5 seconds at issuance.** `claims_at` sets
      `nbf = now - LEEWAY`; `iat` and `exp` are untouched, because extending
      `exp` would silently lengthen a TTL that is 60 seconds on purpose.
      *Observable:* a fresh `lanyard token` decodes to `nbf == iat - 5` and
      `exp == iat + 60`, and the Phase 2 90-second rejection still happens.
- [x] **lanyard allows itself the same leeway, and says which clock it doubts.**
      `jws::verify` takes the clock, accepts `exp` up to `LEEWAY` past, starts
      checking `nbf` with the same leeway, and returns the three distinct
      messages from the spec's table — including naming its own skew when it is
      running skewed. *Observable:* `/oidc/userinfo` answers `200` for a token
      that expired 3 seconds ago and `401` naming the elapsed time for one that
      expired 60 seconds ago; `--expired` is still `401`; a token minted under
      `+5m` of skew and presented to an unskewed server is `401` *not valid
      yet*, naming the ~5 minute difference.
- [x] **`GET /_/health`.** New module merged into the `/_` nest: `status`,
      `version`, `issuer`, `kid`, `now` in unix milliseconds. Always `200` while
      the process serves — diagnosis belongs to `doctor`, because a health check
      that goes red on a misconfigured issuer stops the stack that was about to
      demonstrate the misconfiguration. Confirm `log_layer::emits` already
      excludes it and pin that with a test. *Observable:* `curl -si` returns the
      document; twenty probes leave the live log untouched.
- [x] **The `Host` mismatch warning, once per host.** `src/hosts.rs` holds the
      sticky set and the authority comparison; `log_layer` publishes an event on
      the **first** sighting of each distinct mismatching host, including on the
      discovery and JWKS paths it otherwise skips. The message names both
      authorities, the `iss` that will be minted, and the `LANYARD_ISSUER=` line
      that fixes it. `/_/health` reports `hosts_seen`; `/_/`'s existing warning
      band renders them beside the registry's warnings. *Observable:*
      `curl -H 'Host: lanyard:9500' …/openid-configuration` produces exactly one
      warning in `lanyard logs --json` however many times it is repeated, a
      second host produces a second, and both appear on `/_/` and in
      `/_/health`.
- [x] **`lanyard doctor`, with Config and Reachable and the exit-code
      contract.** The subcommand, `--url`, `--strict`, `--quiet`; the
      `OK`/`WARN`/`FAIL` line format with an indented consequence sentence; and
      the contract that only a `FAIL` exits non-zero, because the image's health
      probe is this command and a container must not stay unhealthy over a
      warning. *Observable:* with `serve` running, two `OK` lines and exit `0`;
      with nothing listening, `Reachable FAIL` naming the exact address tried,
      exit non-zero, and `Config` still reported.
- [x] **doctor: Issuer and JWKS.** Fetch discovery, compare its `issuer`
      authority against the one `doctor` dialled, and report the port and host
      cases with the consequence sentence. Fetch `jwks_uri` **as advertised**;
      when its host does not resolve, print the `/etc/hosts` line and the
      runtime-appropriate `host.containers.internal` / `host.docker.internal`
      suggestion. *Observable:* the `-p 9500:8080` container makes `doctor`
      `WARN` naming both ports and the `iss` that will be rejected, exit `0`,
      and `--strict` exit non-zero; an unresolvable issuer host prints the
      `/etc/hosts` line, and adding it turns both checks `OK`.
- [x] **doctor: Clock, Signing key, and the hosts seen.** Skew measured against
      `/_/health`'s `now` minus half the round trip, `WARN` above 2 seconds with
      the 60-second-TTL consequence spelled out; the key file's path, mode,
      whether it is the built-in default, and whether its `kid` is the one being
      served; and the mismatching hosts the running server has seen. `doctor`
      reads the true clock even when `LANYARD_CLOCK_SKEW` is set in its own
      environment, and says so. *Observable:* the six-line all-`OK` report, and
      against a `-5m` skewed server a `Clock WARN` naming a five-minute offset
      and what it does to a 60-second token.
- [x] **The unwritable data directory names the fix.** When `keys::load_or_create`
      cannot write and `/run/.containerenv` or `/.dockerenv` exists, the error
      names the directory, the uid, and both runtimes' remedy — and points out
      that the default key is deterministic so the volume may not be needed at
      all. *Observable:* a bind mount without `:U` exits non-zero with that
      message and no `os error 13` backtrace.
- [x] **The image.** `Dockerfile` with a musl builder stage and a
      `gcr.io/distroless/static-debian12:nonroot` final stage; `LANYARD_BIND=0.0.0.0`
      and `LANYARD_DATA_DIR=/data` as image defaults; `/data` copied in owned by
      65532; `HEALTHCHECK ["/lanyard","doctor","--quiet"]`; a `.dockerignore`
      that keeps `target/` and the spikes' vendor trees out of the build context.
      *Observable:* `podman build --format docker -t localhost/lanyard:dev .`
      from a clean checkout, an image with no shell that serves discovery on
      `-p 9500:9500`, the same `kid` on two runs with no volume, and
      `podman inspect` reporting `healthy`.
- [x] **`scripts/phase08-container.sh`.** Creates the network, runs the image,
      curls `/_/health` from a second container, runs `doctor` inside and
      outside, and drives the `-p 9500:8080` mismatch — so criteria 3, 7, 8, 15,
      16 and 18 are one command rather than a transcript. **It never calls
      `sudo`**: when `lanyard` does not already resolve to loopback it prints the
      `/etc/hosts` line and exits, because the operator owns that file.
      *Observable:* the script runs green on a machine with only podman and the
      hosts entry in place, and leaves no container or network behind.
- [x] **Docs** — `README.md` gains a `lanyard doctor` section with real output, a
      `/_/health` line, `LANYARD_CLOCK_SKEW` in the configuration table beside
      the note that it is loud on purpose, the leeway paragraph next to the
      existing five-minute-`ClockSkew` warning about .NET, and a container
      section covering the one-name-both-sides remedy, the port rule, the
      no-volume-needed rule and the ownership flags. The "Scope" paragraph moves
      to Phase 8. Spec status → done.

## Progress notes

- **Criterion 18 passed in a real browser, driven by Playwright.** The Chrome
  extension was not connected, so the login was driven with headless Chromium
  (`playwright` 1.63, out of tree in the scratchpad) rather than by hand. The
  top-level request trail was `localhost:5000/` → `/secure` →
  `lanyard:9500/oidc/authorize` → `lanyard:9500/_/` → `/_/pick` →
  `localhost:5000/signin-oidc` → `/secure` → `/`, landing authenticated as Ada
  Bell with `ada@example.test`. Then, with nothing changed between: a second
  container on `lanyard-net` minted a token whose `iss` is
  `http://lanyard:9500/oidc`, and `lanyard doctor` on the host reported all six
  checks `OK` with `hosts_seen` empty and zero `reached as` warnings in the
  container's log.

  The Playwright harness is **not** committed. It pulls a node dependency tree
  for one criterion, and `spikes/dotnet-web`'s README already documents the
  browser steps for a human. Worth revisiting in Phase 10, where CI needs a
  headless browser anyway.
- **`spikes/dotnet-web` gained a `LANYARD_AUTHORITY` override.** Its authority
  was hard-coded to `http://127.0.0.1:9500/oidc`, and criterion 18 points every
  side at one name — a side that cannot be pointed cannot be tested. The default
  is unchanged.
- **Podman copies the host's `/etc/hosts` into every container, and that breaks
  the one-name remedy from the inside.** Once `127.0.0.1 lanyard` is on the
  host, it lands in the application container too and shadows aardvark-dns, so
  `lanyard` resolves to the container itself. Containers that must reach lanyard
  by name need **`--no-hosts`**. This also means criterion 3 passes *before* the
  hosts line is added and fails after it without the flag — the script and the
  README now both carry it. Docker does not copy the host file.

- **Criterion 17's `:U` does not do what the spec assumed.** Under rootless
  podman, `-v dir:/data:U` chowns the directory to the *mapped subuid* (165531
  here), not to the invoking user — so lanyard writes the key at `0600` and you
  cannot read it back. The flag that satisfies the criterion as written is
  `--userns=keep-id --user $(id -u):$(id -g)`, which was verified: mode `0600`,
  owner `rob`, `.gitignore` beside it. `:U` is still what the guardrail message
  suggests, because the message's job is "make it writable". Both are in the
  README and in `scripts/phase08-container.sh`.
- **Criterion 15's 20 MB ceiling.** The image is **9.88 MB**. Recorded rather
  than assumed.
- **The health probe and `doctor` are not host sightings.** The image's
  `HEALTHCHECK` is `lanyard doctor --quiet`, which dials `127.0.0.1:PORT` from
  inside — a mismatch whenever the issuer names the container network. Left
  alone, every containerised lanyard would report a permanent `Host` warning
  about its own probe, and criteria 7 and 18 ask for no warning from any side.
  `/_/health` and `/_/api/events` are excluded by path; `doctor` names itself
  with a `User-Agent` (`hosts::DOCTOR_USER_AGENT`) and is excluded by that. The
  principle is the one `inspect_key` already follows: **a diagnostic must not
  change what it observes.**
- **The image does not handle `SIGTERM`.** lanyard runs as PID 1, where the
  default action for an unhandled signal is to ignore it, so `podman stop`
  always waits its full timeout and then `SIGKILL`s. Not in this phase's scope —
  `serve` has never installed a shutdown handler — but it is visible in every
  container run and worth a decision.

- **Box 9 added `src/runtime.rs`.** The plan put container detection inside
  `doctor.rs`, but `keys.rs` needs the same detection to choose which ownership
  remedy to print first — and `keys` depending on a CLI module is the wrong
  direction. `Runtime` moved to its own module; both read it, so the two
  messages cannot tell one machine to run `docker` and `podman` in one session.
- **Box 8 folded `hosts_seen` into the `Issuer` check** rather than adding a
  seventh line. It is the same fact from the other side — `doctor` dialled one
  address, the server was dialled by others — and criteria 5 and 7 both ask for
  exactly six lines.

- **Box 5, the event stream is skipped entirely.** The spec says the warning
  fires "on any request, including the two paths `emits` excludes" — meaning
  discovery and JWKS. Applied literally it also fired on `GET /_/api/events`,
  publishing an event *because a client connected to the event stream*, which is
  the exact feedback `emits` excludes that path to prevent. The stream is now
  neither sighted nor warned about, which keeps the cleaner contract: every host
  in `hosts_seen` produced exactly one warning event.
- **Box 5, the test harness now sends the issuer's `Host`.** `tests/support`
  binds an ephemeral port while the issuer stays the canonical `:9500`, so every
  existing test was suddenly a genuine mismatch and events in tests about grants
  and personas grew a Phase 8 warning. `support::client()` now defaults the
  header; `tests/hosts.rs` overrides it per request.

- **Box 3, the "about" figure is rounded.** A clock five minutes ahead, measured
  three seconds after the token was minted, produced "ahead of this one by about
  4m57s" — a sentence that undoes its own hedge. `clock::round_about` rounds to
  the nearest minute once there is a minute to round, so the message reads
  "about 5m0s" as the spec writes it. One extra pure function, tested.
- **Box 2, `--expired`'s `nbf` moved too.** The plan said only `claims_at`'s
  ordinary path backdates `nbf`; the `flaw=expired` arm rebuilds all three
  claims, and leaving it alone would have made `nbf == iat` in that one token
  and `iat - 5` in every other. The whole token shifts back, leeway and all.
  `tests/issue.rs`'s assertion was updated to match, with the reason recorded.

## Acceptance

Mirrors the spec. `/implement` is not done until every box passes by driving the
named client.

**`/_/health`**

- [x] 1. `curl -si http://127.0.0.1:9500/_/health` → `200`,
      `content-type: application/json`, `status` `"ok"`, `issuer` byte-equal to
      the banner's, `kid` equal to the one key in `/oidc/jwks`, `now` within 2s
      of `date +%s%3N`.
- [x] 2. Twenty probes of `/_/health` produce no event in `lanyard logs --json`
      or `/_/log`, and a preceding login's events are still there in order.
- [x] 3. `podman run --rm --network lanyard-net docker.io/curlimages/curl -sf
      http://lanyard:9500/_/health` exits `0` and prints the body.
- [x] 4. `podman inspect --format '{{.State.Health.Status}}' lanyard` prints
      `healthy` within 30s of `podman run`, with no shell and no curl in the
      image.

**`doctor` on a working setup**

- [x] 5. `lanyard doctor` against a default `lanyard serve` exits `0` and prints
      six `OK` lines naming the resolved issuer, the address reached, the live
      `kid`, and a skew under one second.
- [x] 6. With nothing listening, `lanyard doctor` exits non-zero, `Reachable` is
      `FAIL` naming the exact address tried, `Config` and `Signing key` still
      report, and no check reports a value it could not have measured.
- [x] 7. `podman exec lanyard /lanyard doctor` exits `0` with every check `OK`,
      including `JWKS` fetched at the advertised `http://lanyard:9500/oidc/jwks`.

**The mismatches — roadmap criterion 1**

- [x] 8. `podman run -d --name lan-8080 -p 9500:8080 -e LANYARD_PORT=8080
      localhost/lanyard:dev`, then `LANYARD_URL=http://localhost:9500 lanyard
      doctor`: `Issuer` is `WARN`, names both ports, and the consequence
      sentence carries the `iss` value that will be minted and the fact that an
      RP dialling `:9500` rejects it. Plain `doctor` exits `0`; `--strict` exits
      non-zero.
- [x] 9. `LANYARD_ISSUER=http://lanyard:9500/oidc lanyard serve` with no
      `/etc/hosts` entry: `doctor` `WARN`s that the issuer's host does not
      resolve from here and prints the `/etc/hosts` line verbatim. Add the line,
      rerun: every check `OK`.
- [x] 10. `curl -H 'Host: lanyard:9500'
      http://127.0.0.1:9500/oidc/.well-known/openid-configuration` puts exactly
      one warning in `lanyard logs --json` naming both authorities and the
      `LANYARD_ISSUER=` fix; ten repeats add none; `Host: other:9500` adds one.
      `/_/` shows a band naming both, `/_/health`'s `hosts_seen` lists both, and
      `lanyard doctor` reports them.

**The clock — roadmap criterion 3**

- [x] 11. A fresh `lanyard token --as ada --aud billing-api` decodes to
      `nbf == iat - 5` and `exp == iat + 60`; against `spikes/dotnet-api` with
      `ClockSkew = TimeSpan.Zero` it is `200` immediately and `401` ninety
      seconds later.
- [x] 12. A token 3 seconds past `exp` is `200` at `/oidc/userinfo`; one 60
      seconds past is `401` naming how long ago it expired; `--expired` is still
      `401`.
- [x] 13. `LANYARD_CLOCK_SKEW=-5m lanyard serve`: the banner prints the skew line
      with its consequence, `/_/health` reports it, `lanyard doctor` from an
      unskewed shell reports `Clock WARN` at ~5m with the 60-second-token
      consequence, and a token minted one second earlier is `401` at
      `spikes/dotnet-api` with `ClockSkew = TimeSpan.Zero`. **The `401` and the
      diagnosis are watched in the same session.**
- [x] 14. A token minted under `LANYARD_CLOCK_SKEW=+5m`, presented to an
      unskewed `serve`, is `401` at `/oidc/userinfo` saying it is *not valid
      yet* and that the issuing clock is ahead by about five minutes — not
      "expired", not bare.

**The image — roadmap criterion 2**

- [x] 15. `podman build --format docker -t localhost/lanyard:dev .` from a clean
      checkout succeeds; `podman images` shows the recorded size;
      `podman run --rm --entrypoint /bin/sh localhost/lanyard:dev -c true` fails
      for want of a shell; `podman run --rm -p 9500:9500 localhost/lanyard:dev`
      serves a discovery document `curl` fetches from the host.
- [x] 16. `podman run --rm localhost/lanyard:dev` twice with no volume: the same
      `kid` both times, and the same `kid` the native binary serves.
- [x] 17. `podman run -v $PWD/tmp-data:/data:U localhost/lanyard:dev` writes
      `tmp-data/signing-key.pem` readable by the invoking user at mode `0600`,
      plus the data dir's `.gitignore`. Without `:U`, lanyard exits non-zero
      with the ownership message naming the directory, the uid and both
      runtimes' fix — no backtrace, no bare `os error 13`.
- [x] 18. The one-name setup end to end: `podman network create lanyard-net`, the
      image run on it with `LANYARD_ISSUER=http://lanyard:9500/oidc`, and
      `127.0.0.1 lanyard` in `/etc/hosts` — **added by the operator**, who is
      asked for it at this point and not before. Then, changing nothing between:
      `spikes/dotnet-web` pointed at `http://lanyard:9500/oidc` completes a full
      browser login; a second container on `lanyard-net` mints a token against
      `http://lanyard:9500/oidc/token`; and `lanyard doctor` on the host reports
      every check `OK` with **no** host-mismatch warning from any side.
