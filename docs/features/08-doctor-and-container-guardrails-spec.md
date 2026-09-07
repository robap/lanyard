# `doctor` and the container guardrails — spec

**Status:** done · **Roadmap:** Phase 8 · **Slug:** `08-doctor-and-container-guardrails`

## Why

Every phase so far has assumed lanyard and the application are on the same
machine, reachable at the same name, on the same clock. **This is the phase
where that stops being true**, and CONCEPT §8 is a list of six ways it goes
wrong once a container is involved. Each one produces a failure that reads as
something else:

| The gotcha | What the developer sees |
|---|---|
| Issuer is one string, there are two addresses | `IDX10205: Issuer validation failed` — or a login that loops |
| `-p 9500:8080` — the port is inside the issuer | The same, for a reason one character wide |
| `localhost` inside a container is the container | Connection refused, which reads as "the IdP is down" |
| `http://lanyard` is not a secure context | "It asks me to pick a persona every single time" |
| Clock drift after a laptop sleeps | A bare `401` on a token minted four seconds ago |
| Ephemeral signing keys | Saved tokens stop working, cached JWKS fails, no visible cause |

None of these is hard once you know which one it is. **Knowing which one it is
is the entire problem**, and it is why this phase is one command rather than six
README paragraphs. From CONCEPT §6:

> Roughly ten minutes of work per check, and it collapses the entire Docker
> gotcha category into one command whose output can be pasted into an issue.
> Nothing in the competitive set has one.

### Which north stars it serves

- **North star 2 — one instance, every project.** A machine-wide singleton is
  reachable by more than one address almost by definition: the browser dials
  `localhost`, a container dials a service name, and the issuer is one string.
  `doctor` is what makes that arrangement debuggable instead of mystifying.
- **North star 5 — single static binary.** The image ships that binary and
  nothing else — no shell, no curl, no package manager. That constraint is what
  forces the container's health probe to be lanyard itself, which turns out to
  be a better design than a `curl` in a `HEALTHCHECK` anyway.
- **North star 4 — real tokens, including deliberately wrong ones.** A
  deliberately skewed clock belongs on that list. See *A skewed clock is the
  seventh flaw*.

### Podman, not Docker

**The development machine runs rootless podman 4.9 with netavark, and no
compose provider.** That changes the observable commands and two decisions, and
it is worth stating in the first section rather than being discovered halfway
down:

- Container-to-container name resolution comes from a user-defined network
  (`podman network create`) and aardvark-dns, not from Compose. The criteria
  below are written against `podman network` + `podman run`, which needs
  nothing installed that is not already here.
- Podman's name for the host is **`host.containers.internal`**, not
  `host.docker.internal`. CONCEPT §8 names only the Docker spelling.
- Rootless podman maps the container's `root` to the invoking user, so bind
  mount ownership behaves the *opposite* way from rootful Docker. See
  *Rootless, and who owns the signing key*.

The prose in CONCEPT §8 is Docker-flavoured because that is what it was written
against; the *problems* are the container runtime's, not Docker's, and every one
of them reproduces under podman. Where the fix differs by runtime, the tool
prints both.

## In scope

- **`lanyard doctor`** — six checks, one line each, an exit-code contract, and a
  consequence sentence for anything that is not OK. Works with the server
  running or not.
- **`GET /_/health`** — liveness, always `200` while the process serves, with the
  handful of facts `doctor` needs to compare two sides.
- **A request-time warning when the inbound `Host` does not match the issuer**,
  once per distinct host, surfaced in the log, on `/_/`, and in `doctor`.
- **`nbf` backdated a few seconds at issuance**, and the same few seconds of
  leeway on `exp` **and `nbf`** when lanyard verifies one of its own tokens.
- **A skew-aware rejection message** — "not valid yet, and the clock that issued
  it is ~5 minutes ahead of this one" rather than a bare `401`.
- **`LANYARD_CLOCK_SKEW`** — a deliberately wrong clock, so the diagnosis above
  is reproducible on a laptop that has no VM to drift.
- **A container image**: distroless static, no shell, `LANYARD_BIND=0.0.0.0` and
  `LANYARD_DATA_DIR=/data` as image defaults, a non-root uid, and a
  `HEALTHCHECK` that runs the binary.
- **A guardrail on an unwritable data directory** — the ownership fix, named,
  instead of a permission-denied panic.
- **README section** on running in a container: the one-name-both-sides remedy,
  the port rule, the volume rule, and what `doctor` says about each.

## Out of scope

- **arm64.** The build host has no musl target, no cross toolchain, and no qemu
  binfmt, so an arm64 image cannot be produced *or* run here today. The
  Dockerfile takes the binary as a build argument so a second architecture is a
  second `COPY` and nothing else, and the roadmap already puts the observable
  — "`docker run ghcr.io/robap/lanyard` serves discovery on both amd64 and
  arm64" — in **Phase 10**, where CI has both runners. Building an artifact here
  that nobody can execute here would be shipping untested documentation.
- **Publishing the image anywhere.** GHCR is Phase 10. This phase builds
  `localhost/lanyard:dev`.
- **HTTPS, and any TLS check in `doctor`.** Post-v1, per
  `docs/decisions/https-priority.md`. `doctor` gains a TLS handshake check when
  there is a handshake to make.
- **`doctor --json`.** The output is meant to be pasted into an issue, and
  human-readable text is what gets pasted. Add it when something parses it.
- **Deriving the issuer from the `Host` header**, now or ever. CONCEPT §8 is
  explicit, and this phase is the one that *reports* the mismatch precisely so
  that nobody is tempted to paper over it.
- **Fixing anything automatically.** `doctor` diagnoses and prints the command;
  it does not edit `/etc/hosts`, restart the server, or rewrite config.
- **A second clock source.** No NTP, no external time service. Skew is measured
  between two lanyard processes' clocks — `doctor`'s and the server's — which is
  the only comparison that needs no network beyond the one already in use.

## Behavior

### `GET /_/health` is liveness, and only liveness

```
$ curl -s http://127.0.0.1:9500/_/health | jq .
{
  "status": "ok",
  "version": "0.1.0",
  "issuer": "http://127.0.0.1:9500/oidc",
  "kid": "TXntCt2biz2Bj578hZocZOb2A2nQV9JfrBvFN55QWpU",
  "now": 1757116800123,
  "hosts_seen": ["lanyard:9500"]
}
```

Four decisions:

- **It is `200` whenever the process is serving, whatever `doctor` thinks.** The
  whole point of a health endpoint is `depends_on: condition: service_healthy`,
  and an application that waits for lanyard must start even when the issuer is
  misconfigured — because a misconfigured issuer is exactly the state the
  developer is trying to debug. A health check that goes red on a *diagnosis*
  turns "your token will be rejected" into "your stack will not come up", which
  is strictly less debuggable. Diagnosis lives in `doctor`; liveness lives here.
- **`now` is unix milliseconds, and it is the only reason this returns a body at
  all.** It is what makes skew measurable between two machines without either of
  them consulting a third.
- **It emits no log event.** `log_layer::emits` already excludes everything
  under `/_/` that is not a decision, and a probe every five seconds would
  drown the live request log — the one surface Phase 6 exists to keep readable.
- **`hosts_seen`** is the sticky set from *The `Host` mismatch* below, so
  `doctor` can report it from outside the process.

### `lanyard doctor`

```
$ lanyard doctor
lanyard doctor
  Config       OK    issuer http://127.0.0.1:9500/oidc, bind 127.0.0.1:9500
  Reachable    OK    http://127.0.0.1:9500 answered in 2ms (lanyard 0.1.0)
  Issuer       OK    the running server agrees with the address I dialled
  JWKS         OK    http://127.0.0.1:9500/oidc/jwks, 1 key, kid TXntCt2b…
  Clock        OK    0.001s apart
  Signing key  OK    /home/you/.local/share/lanyard/signing-key.pem, 0600,
                     the built-in default key — stable across a wiped data dir
```

Six checks, in the order a request travels. Each prints `OK`, `WARN` or `FAIL`,
and anything that is not `OK` is followed by an indented **consequence
sentence** and, where there is one, **the command that fixes it**. That pairing
is the feature: a check that says "port mismatch" and stops has moved the
developer one step, and a check that says what will be rejected has finished the
job.

| Check | What it does | Not-OK looks like |
|---|---|---|
| **Config** | Resolves the environment exactly as `serve` does — issuer, bind, port, data dir, persona sources | `FAIL` on anything `serve` would refuse to start with |
| **Reachable** | `GET /_/health` at `LANYARD_URL` | `FAIL` — nothing is listening, and everything below is skipped |
| **Issuer** | Compares the `issuer` in the discovery document against the authority `doctor` dialled | `WARN` — host and/or port differ |
| **JWKS** | Fetches `jwks_uri` **as advertised**, not as configured, and compares its `kid` to `/_/health`'s | `WARN` — the name in the issuer does not resolve from here, or resolves somewhere else |
| **Clock** | `/_/health`'s `now` against `doctor`'s, minus half the round trip | `WARN` above 2s |
| **Signing key** | The file, its mode, whether it is the built-in default, and whether its `kid` is the one being served | `WARN` — a custom key that a container will lose, or a `kid` that does not match the live JWKS |

**The exit-code contract**, because a `HEALTHCHECK` depends on it:

- **`0`** — every check `OK` or `WARN`. A warning is a thing that *may* be
  deliberate: `LANYARD_ISSUER=http://lanyard:9500/oidc` looks like a mismatch
  from the host shell and is exactly right for the container network.
- **non-zero** — any `FAIL`. Today that is a config the server would refuse and
  a server that is not answering.
- **`--strict`** promotes every `WARN` to a `FAIL`, for CI.
- **`--quiet`** prints nothing and only sets the exit code. This is what the
  image's `HEALTHCHECK` runs, and it is why the default contract has to be
  liveness-shaped: a container that never goes healthy because its issuer is
  unusual would block every service that waits on it.

**`doctor` needs no running server to be useful.** With nothing listening,
Config and Signing key still report, Reachable `FAIL`s naming the address it
tried, and the rest are skipped rather than guessed at. `LANYARD_URL` points it
somewhere else, the same variable `lanyard token` and `lanyard logs` already
use — it is an address, not the issuer, and this phase is the one where that
distinction earns its keep twice over.

### The `Host` mismatch, at request time

> **When a request's `Host` is not the issuer's authority, lanyard says so once
> per distinct host, and names the consequence.**

```
warning: reached as "lanyard:9500" but the issuer is
  "http://127.0.0.1:9500/oidc" — a token minted here says
  iss=http://127.0.0.1:9500/oidc and a relying party that dialled
  lanyard:9500 will reject it. Fix with:
      LANYARD_ISSUER=http://lanyard:9500/oidc lanyard serve
```

- **Once per distinct `Host` value, for the life of the process.** Not once per
  request: discovery and JWKS are polled, a health probe runs every few seconds,
  and a warning that repeats is a warning that gets filtered out. The set is
  small and bounded by how many names actually reach the machine.
- **It fires on any request**, including the two paths `log_layer::emits`
  excludes. Discovery is the *first* request of every flow and the one where the
  mismatch is cheapest to catch, so the first sighting publishes an event even
  though a discovery request normally publishes none. The once-per-host rule is
  what makes that safe.
- **No severity tiers.** `localhost:9500` against an issuer of `127.0.0.1:9500`
  is a mismatch and gets the warning, because a conforming RP compares `iss` as
  a string and this one genuinely breaks. One rule, no exceptions to remember —
  and once per host, so the common harmless case costs one line, once.
- **Three surfaces, one source**, the shape Phase 6 established: the event
  stream (so `lanyard logs --json` and `/_/log` carry it), a band on `/_/`
  naming every host seen, and `hosts_seen` in `/_/health` so `doctor` reports it
  from outside.

This one check covers two of CONCEPT §8's gotchas. `-p 9500:8080` is not a
separate detector: the container listens on 8080, the issuer says 8080, the
browser arrives with `Host: localhost:9500`, and the mismatch is the port. From
the other side, `doctor` dialled `:9500` and the discovery document says
`:8080`, which is the same fact reported to the person rather than to the log.

### The clock

Three separate things, easily confused, so they are named separately.

**1. `nbf` is backdated 5 seconds at issuance.** A relying party whose clock is
a second or two behind must not reject a token that is a second old. Only `nbf`
moves — `exp` is untouched, because extending it would silently lengthen a TTL
that is 60 seconds on purpose, and `iat` is untouched because it is a statement
of fact.

This is deliberately small enough not to disturb Phase 2's criterion that *"the
same call 90 seconds later returns 401"*: 60 seconds of life plus 5 of leeway is
65, and 90 is still comfortably past it.

**2. lanyard allows itself the same 5 seconds when verifying its own tokens.**
`jws::verify` — the code behind `/userinfo`, `/introspect`, `/revoke` and
`id_token_hint` — accepts a token up to 5 seconds past `exp`, and starts
checking `nbf` with the same leeway. It did not check `nbf` before, on the
grounds that lanyard never issues a future one; `LANYARD_CLOCK_SKEW` makes that
false, and a token that is not valid yet is precisely the skew symptom worth
naming.

`--expired` is unaffected: it backdates `exp` by an hour, which no leeway
reaches. That is not an accident of the number — it is why the flaw was defined
as an hour rather than a second.

**3. A rejection says which clock it doubts.** Three messages instead of one:

| State of the token | `401` says |
|---|---|
| `exp` more than 5s past | `the token expired 4m12s ago` |
| `nbf` more than 5s ahead | `the token is not valid yet — the clock that issued it is ahead of this one by about 5m. Check the clocks on both sides; run lanyard doctor` |
| `exp` past **and** this process is running skewed | `the token expired 4m12s ago (this process's clock is skewed by -5m0s via LANYARD_CLOCK_SKEW)` |

### A skewed clock is the seventh flaw

`LANYARD_CLOCK_SKEW=-5m` offsets every clock read in the process.

The honest reason it exists: **on native Linux there is no way to skew a
container's clock.** `CLOCK_REALTIME` is not virtualized by time namespaces, and
a rootless container shares the host kernel's clock exactly. The scenario
CONCEPT §8 describes — a Docker Desktop or `podman machine` VM lagging after the
laptop sleeps — cannot be reproduced on this machine at all, which would leave
the skew diagnosis as the one part of this phase nobody can watch work.

It is also, on its own terms, the same idea as the six failure flags:

> Testing that an API *accepts* a good token is the easy half and everyone does
> it; testing that it correctly *rejects* the bad ones is the half that never
> gets written, because producing those tokens is annoying enough that people
> skip it.

A skewed provider is a bad token generator that no config file can produce, and
"my API's `ClockSkew` is five minutes so it accepts a token that died four
minutes ago" is a real production bug that is currently untestable anywhere.

**It is loud.** The banner gains a line, `/_/health` reports it, `doctor`
reports it as a `WARN` even when the two clocks agree by construction, and every
expiry rejection names it. A dev tool may lie about the time; it may not do so
quietly.

```
lanyard 0.1.0
  Issuer    → http://127.0.0.1:9500/oidc
  …
  Clock     → skewed -5m0s (LANYARD_CLOCK_SKEW) — tokens minted here are
              already expired by 4m for anything on a correct clock
```

### The image

```
FROM gcr.io/distroless/static-debian12:nonroot
```

- **Distroless static, no shell.** The binary is statically linked against musl
  and is the only executable in the image. That is a real constraint and it
  shapes the next decision.
- **`HEALTHCHECK` is `["/lanyard", "doctor", "--quiet"]`.** There is no `curl`
  and no `sh` to run one, which is what makes the exit-code contract above
  load-bearing rather than tidy: the probe must be `0` for a lanyard that is
  serving but oddly configured, or a container never reports healthy and
  everything waiting on it never starts.
- **`LANYARD_BIND=0.0.0.0` as an image default**, so `podman run -p 9500:9500`
  works with no flags, while the native binary keeps its loopback default. The
  bind address is a property of the deployment, and the image is the deployment.
  CONCEPT §7 asks for exactly this.
- **`LANYARD_DATA_DIR=/data`**, created in the image owned by the non-root uid.
  Not `$XDG_DATA_HOME`, which in a distroless image resolves under a home
  directory nobody thinks about.
- **No volume is needed and none is declared.** The deterministic default key
  means a fresh container with no volume produces the same `kid` and the same
  public key every time — the property Phase 1 built and the answer to CONCEPT
  §8's ephemeral-keys gotcha. A volume is for someone who has replaced the key
  with their own.
- **A builder stage compiles the binary**, so `podman build .` works from a
  clean checkout with nothing installed on the host — which is criterion 15, and
  the only version of "the image builds" worth asserting. The builder stage is
  also what keeps musl and its target off the development machine's toolchain.
  Phase 10, which will have binaries from `cargo-dist` before it has an image,
  can add a `BINARY` build argument that skips the stage; that is an addition to
  this Dockerfile, not a rewrite of it.

### Rootless, and who owns the signing key

Rootless podman maps the container's `root` to the invoking user and every other
uid into a subuid range. A bind-mounted directory owned by you appears inside
the container as owned by `root`, and a process running as uid 65532 cannot
write to it. Under rootful Docker the same mount behaves the other way around.

Since no volume is needed for the ordinary case, this only bites the person
mounting one — and for them, the guardrail is the error message:

```
lanyard: /data is not writable by uid 65532 (this process is in a container).
  A bind-mounted data directory needs its ownership mapped. Try:
      podman run -v ./lanyard-data:/data:U   …    # podman, rootless
      docker run --user $(id -u):$(id -g)    …    # docker
  Or drop the volume entirely: the default signing key is deterministic, so a
  container with no volume already produces the same kid every time.
```

"This process is in a container" is `/run/.containerenv` or `/.dockerenv`
existing — the same detection `doctor` uses to choose which of the two commands
to print first. Without it, the message is a bare `Permission denied (os error
13)` on a path the developer did not choose, which is the class of failure this
whole phase exists to delete.

### One name that resolves from both sides

CONCEPT §8's remedy, now demonstrable and documented:

```
$ podman network create lanyard-net
$ podman run -d --name lanyard --network lanyard-net -p 9500:9500 \
    -e LANYARD_ISSUER=http://lanyard:9500/oidc localhost/lanyard:dev
$ echo '127.0.0.1 lanyard' | sudo tee -a /etc/hosts
```

`http://lanyard:9500/oidc` now resolves to the same lanyard from the host
browser, from the host shell, and from any container on `lanyard-net` — one
string, one issuer, no mismatch warning from any side. That is the whole
configuration, and `doctor` verifies it end to end.

The alternative, for a lanyard running natively on the host with the application
in a container, is podman's `host.containers.internal` (Docker's
`host.docker.internal`, and native Linux Docker needs
`--add-host=host.docker.internal:host-gateway`). `doctor` prints whichever name
matches the runtime it detects when the issuer's host does not resolve.

## Acceptance criteria

Each names a client and an operation, or an assertion against a file or a
rendered page. `lanyard` below is the release binary; `localhost/lanyard:dev` is
the image built by criterion 13.

**`/_/health`**

- [x] 1. `curl -si http://127.0.0.1:9500/_/health` → `200`,
      `content-type: application/json`, `status` is `"ok"`, `issuer` is byte-equal
      to the banner's `Issuer →` line, `kid` equals the one key in
      `/oidc/jwks`, and `now` is within 2s of `date +%s%3N`.
- [x] 2. Hit `/_/health` 20 times, then `lanyard logs --json` and `/_/log`: not
      one event for any of them, and the events from a preceding login are
      still there in order.
- [x] 3. From a second container on the same podman network:
      `podman run --rm --network lanyard-net docker.io/curlimages/curl -sf
      http://lanyard:9500/_/health` exits `0` and prints the body.
- [x] 4. `podman inspect --format '{{.State.Health.Status}}' lanyard` prints
      `healthy` within 30s of `podman run`, with no shell and no curl in the
      image.

**`doctor` on a working setup**

- [x] 5. `lanyard serve` on defaults; in another shell `lanyard doctor` exits
      `0` and prints six lines, one per check, all `OK`, naming the resolved
      issuer, the address it reached, the live `kid`, and a measured skew under
      one second.
- [x] 6. With nothing listening, `lanyard doctor` exits non-zero, `Reachable`
      is `FAIL` naming the exact address it tried, `Config` and `Signing key`
      still report, and no check reports a value it could not have measured.
- [x] 7. `lanyard doctor` inside the running container
      (`podman exec lanyard /lanyard doctor`) exits `0` with every check `OK`,
      including `JWKS` fetched at the advertised `http://lanyard:9500/oidc/jwks`.

**The mismatches — roadmap criterion 1**

- [x] 8. `podman run -d --name lan-8080 -p 9500:8080 -e LANYARD_PORT=8080
      localhost/lanyard:dev`, then on the host
      `LANYARD_URL=http://localhost:9500 lanyard doctor`: the `Issuer` check is
      `WARN`, names both ports, and the consequence sentence contains the `iss`
      value that will be minted and the fact that an RP dialling `:9500` will
      reject it. Plain `doctor` exits `0`; `doctor --strict` exits non-zero.
- [x] 9. `LANYARD_ISSUER=http://lanyard:9500/oidc lanyard serve` with no
      `/etc/hosts` entry: `lanyard doctor` `WARN`s that the issuer's host does
      not resolve from here and prints the `/etc/hosts` line to add verbatim.
      Add the line, rerun: every check `OK`.
- [x] 10. With the default issuer, `curl -H 'Host: lanyard:9500'
      http://127.0.0.1:9500/oidc/.well-known/openid-configuration`:
      `lanyard logs --json` gains exactly one event carrying a warning that
      names both authorities and the `LANYARD_ISSUER=` line that fixes it.
      Repeat the same curl ten times — still exactly one such warning. A curl
      with `Host: other:9500` produces a second. `/_/` shows a band naming both
      hosts; `/_/health`'s `hosts_seen` lists both; `lanyard doctor` reports
      them.

**The clock — roadmap criterion 3**

- [x] 11. Decode a fresh `lanyard token --as ada --aud billing-api`: `nbf` is
      exactly 5 seconds before `iat`, and `exp` is exactly 60 seconds after it.
      Against `spikes/dotnet-api` with `ClockSkew = TimeSpan.Zero`, that token
      returns `200` immediately and `401` ninety seconds later — Phase 2's
      criterion still holds.
- [x] 12. A token whose `exp` passed 3 seconds ago is still accepted at
      `/oidc/userinfo` (`200`); one whose `exp` passed 60 seconds ago is `401`
      and the body names how long ago it expired.
      `lanyard token --as ada --expired` is still `401`.
- [x] 13. `LANYARD_CLOCK_SKEW=-5m lanyard serve`: the banner prints the skew
      line with the consequence; `/_/health` reports the skew; `lanyard doctor`
      from an unskewed shell reports `Clock WARN` with an offset of 5m and a
      consequence sentence saying a 60-second token is already ~4 minutes
      expired for anything on a correct clock; and
      `curl -H "Authorization: Bearer $(lanyard token --as ada --aud
      billing-api)"` against `spikes/dotnet-api` with `ClockSkew = TimeSpan.Zero`
      returns `401` on a token minted one second earlier. **The `401` and the
      diagnosis are watched in the same session**: that pairing is the criterion.
- [x] 14. Mint a token under `LANYARD_CLOCK_SKEW=+5m`, restart `lanyard serve`
      with no skew, present that token to `/oidc/userinfo`: `401` whose body
      says the token is *not valid yet* and that the issuing clock is ahead by
      about 5 minutes — not "expired", and not a bare `401`.

**The image — roadmap criterion 2**

- [x] 15. `podman build -t localhost/lanyard:dev .` from a clean checkout
      succeeds; `podman images` shows under 20 MB;
      `podman run --rm --entrypoint /bin/sh localhost/lanyard:dev -c true` fails
      because there is no shell; and `podman run --rm -p 9500:9500
      localhost/lanyard:dev` serves a discovery document that `curl` fetches
      from the host.
- [x] 16. `podman run --rm localhost/lanyard:dev` twice, no volume either time:
      `/oidc/jwks` reports the same `kid` both times, and it is the same `kid`
      the native binary serves.
- [x] 17. `podman run -v $PWD/tmp-data:/data:U localhost/lanyard:dev` writes
      `tmp-data/signing-key.pem` readable by the invoking user at mode `0600`,
      plus the data dir's own `.gitignore`. Dropping `:U` makes lanyard exit
      non-zero with the message from *Rootless, and who owns the signing key* —
      naming the directory, the uid, and both runtimes' fix — and no stack trace
      or bare `os error 13`.
- [x] 18. The full one-name setup: `podman network create lanyard-net`, run the
      image on it with `LANYARD_ISSUER=http://lanyard:9500/oidc` and
      `127.0.0.1 lanyard` in `/etc/hosts`. Then, without changing anything
      between: `spikes/dotnet-web` pointed at `http://lanyard:9500/oidc`
      completes a full browser login and lands authenticated; a second container
      on `lanyard-net` mints a token with `curl` against
      `http://lanyard:9500/oidc/token`; and `lanyard doctor` on the host reports
      every check `OK` with **no** host-mismatch warning from any side.

## Open questions

Three. None blocks the criteria above; each has a recommendation and changes
work only inside its own bullet.

1. **Ship `LANYARD_CLOCK_SKEW` at all?** It is scope the roadmap did not ask
   for, and it is a knob that makes lanyard lie about the time.

   **Recommendation: yes, ship it.** Criterion 13 is otherwise unobservable on
   this machine — `CLOCK_REALTIME` is not virtualized by Linux time namespaces,
   so a rootless container's clock *is* the host's and cannot be moved. The two
   alternatives are worse: `libfaketime` is an apt package plus an `LD_PRELOAD`
   whose interception of `clock_gettime` through the vDSO is unreliable and
   would break entirely once the binary is statically linked for the image; and
   a stub HTTP server that reports a false `now` tests `doctor`'s arithmetic
   without ever proving that a skewed provider produces a `401`. The knob also
   stands on its own as a seventh deliberate flaw, and it is what lets CI test
   the diagnosis. Say the word and criteria 13 and 14 are rewritten against
   `faketime` instead, with a recorded caveat that they may not be reproducible.

2. **A `compose.yaml` in the repo?** Docker Compose users want one, and
   `depends_on: condition: service_healthy` is the reason `/_/health` has this
   shape. But there is no compose provider on this machine, so a committed
   compose file would be documentation nobody has run — the thing this project's
   method exists to avoid.

   **Recommendation: not in this phase.** The podman-native path in criteria 3
   and 18 proves everything the compose file would, and Phase 10 or 11 is where
   a CI job with a real compose provider can add and *run* it. If you would
   rather have it now, `pip install --user podman-compose` is a rootless
   one-liner and it becomes a nineteenth criterion — nothing else in the spec
   changes.

3. **Does the `Host` warning fire on `localhost` vs `127.0.0.1`?** As written,
   yes: one rule, no severity tiers, and a strict RP really does reject that
   token. The cost is that a developer who types `localhost` in the browser with
   the default issuer sees a warning that is technically correct and, for a
   browser-only flow where nothing compares `iss`, practically harmless.

   **Recommendation: keep the one rule.** Once per host means it costs exactly
   one line for the life of the process, and the message names the one-line fix.
   The alternative — a loopback-equivalence exemption — is a special case that
   has to be explained in the README, and it hides the mismatch that produces
   `IDX10205` in .NET, which is the single most common way this fails.
