# Distribution — spec

**Status:** draft · **Roadmap:** Phase 10 · **Slug:** `10-distribution`

## Why

Nine phases have produced a tool whose install instructions are still
`cargo build --release`. That line is in the README today, at the top, and it
excludes the entire audience this project was written for:

> **crates.io is not needed.** `cargo install` reaches Rust developers, who are
> a small slice of the audience for a local OIDC provider — our own stack is PHP
> and .NET. Nobody installs an S3 emulator via cargo either. (CONCEPT §7)

A PHP developer who reads the pitch, wants to try it, and hits "install a Rust
toolchain" has already left. **This phase's whole job is to delete that
sentence** and replace it with a line someone can paste.

### Which north stars it serves

- **North star 5 — single static binary.** This is the phase where "single
  static binary" stops being a property of the source tree and becomes a
  property of a file someone downloads. It is also where the word *static* gets
  tested for real: the roadmap's Linux criterion is "runs on a distro older than
  the build host", which a default `cargo build` on this machine does not
  satisfy.
- **North star 2 — one instance, every project.** A machine-wide singleton has
  to be installed machine-wide, by the machine's own package manager, and start
  the way other machine-wide things start.
- **North star 1 — accept everything.** Untouched, and worth saying: nothing in
  this phase adds configuration. The install command has no flags.

### What already exists, and what does not

Worth stating plainly, because half of this phase is discovering that the
starting point is further back than the roadmap bullet implies:

| | Today |
|---|---|
| GitHub repo | `robap/lanyard`, **public**, `origin/main` at `a9cfb95` |
| Tags / releases | **none** — this phase cuts the first |
| CI | **none at all.** No `.github/` directory exists |
| `Cargo.toml` | `name = "lanyard-cli"`, `[[bin]] name = "lanyard"`, version `0.1.0`, `exclude` list already tuned for packaging |
| `Dockerfile` | Phase 8's — builds from source, amd64 only, distroless, `HEALTHCHECK` runs `doctor --quiet` |
| Homebrew tap | `robap/homebrew-tap` **does not exist** (`404`) |
| Windows support | **the crate does not compile on Windows.** See below |
| `cargo fmt --check`, `cargo clippy --all-targets -D warnings` | both clean as of writing |

### The finding: Windows is a port, and this phase does not do it

The roadmap lists Windows in one word among five platforms. It is not one word.
`std::os::unix` is imported unconditionally at five sites:

```
src/keys.rs:117   OpenOptionsExt   — create the signing key at mode 0600
src/keys.rs:203   MetadataExt      — current uid, via /proc/self
src/keys.rs:403   PermissionsExt   — tests
src/keys.rs:443   PermissionsExt   — tests
src/doctor.rs:664 PermissionsExt   — doctor's Signing key check reads the mode
```

Plus `Config::resolve` derives every path from `$HOME` with XDG fallbacks and
**errors out when `HOME` is unset**, which on Windows it normally is. So
"Windows" means: `cfg`-gate five call sites, decide what a data directory is on
Windows, decide what `doctor` prints where there is no file mode, and gate the
tests that assert `0600`. Perhaps sixty lines and four decisions — small, but
not free, and it is a *portability* change landing inside a *distribution*
phase because that is where the roadmap put it.

**It is not done here, because Windows is already reachable twice over.** See
*Windows without a Windows binary* below. The short version: WSL2 runs the Linux
musl binary with the full browser flow intact, Docker Desktop runs the published
image, and criterion 10 watches the first of those work on a real Windows
machine. A fifth target is the thing to add *after* a pipeline has cut its first
release, not during — and unlike Phase 1's `client:` field, this one genuinely is
additive later: five `cfg` gates and a target-list entry, no migration.

## In scope

- **A release pipeline** driven by `dist` (formerly `cargo-dist`, currently
  0.32.0, actively maintained): a GitHub Actions workflow triggered by a version
  tag, producing archives, checksums, and a `dist-manifest.json` on a GitHub
  release.
- **Four platform artifacts**: macOS arm64 and x86_64, Linux arm64 and x86_64.
  **The Linux artifacts are statically linked musl builds**, not glibc — that is
  the roadmap's older-distro criterion, and it is a build configuration
  decision, not a side effect.
- **Homebrew tap** `robap/homebrew-tap`, so `brew install robap/tap/lanyard`
  works on macOS and on Homebrew-on-Linux.
- **A curl installer**, generated and hosted on the release, for the README
  one-liner.
- **A README section that states the Windows story out loud** — WSL or the
  container, the one-line command for each, and the `localhost` caveat below.
  A Windows reader must not have to infer it from an absent download.
- **The container image on GHCR**, `ghcr.io/robap/lanyard`, as a **multi-arch
  manifest covering amd64 and arm64**, tagged with the version and `latest`,
  built from the release's own binaries rather than compiled a second time.
- **A CI workflow** — `fmt`, `clippy -D warnings`, `cargo test --locked` on
  Linux and macOS, on every push and PR. This phase introduces `.github/`, and
  a release pipeline that publishes binaries no job has tested is not worth
  having.
- **`cargo package --locked` in CI**, which is the check `Cargo.toml`'s `exclude`
  comment currently promises and nothing performs — that `web/dist/` is in the
  package and `docs/`, `spikes/`, `web/src/` are not. It runs a verification
  build; it does not publish.
- **A `CHANGELOG.md`**, because `dist` lifts the matching version's section into
  the GitHub release body, and a release page with no notes is a worse artifact
  than the binaries deserve.
- **`[profile.dist]`** — the release-artifact profile (thin LTO, stripped), so
  the downloaded binary is the small one.
- **README install section**: `brew`, `curl | sh`, `docker run`, and the
  from-source path demoted below them. The container section switches from
  `localhost/lanyard:dev` to the published image.

## Out of scope

- **Publishing to crates.io.** `docs/decisions/crates-io-reservation.md` decided
  this and nothing since has changed it: `cargo install` reaches the smallest
  slice of the audience, and publishing means maintaining a published version
  forever. The *names* — crate `lanyard-cli`, binary `lanyard` — are already in
  `Cargo.toml` and are the only part of that roadmap bullet with work in it.
- **The README rewrite.** Tagline, screenshot, `<picture>` for both colour
  schemes, the failure-token block above the fold — that is **Phase 11**, which
  also has the examples the screenshots come from. This phase changes the install
  instructions and nothing else about the document's shape.
- **Code signing and notarization.** An unsigned macOS binary installed by
  `brew` or by `curl | sh` is not quarantined — Gatekeeper's quarantine attribute
  comes from browsers, not from the terminal. Signing costs a paid Apple
  developer account and buys nothing until someone downloads a `.dmg` from a
  webpage, which is not a channel here. Same for Windows Authenticode and its
  SmartScreen reputation problem.
- **`lanyard service install`.** CONCEPT §7's launchd/systemd/scheduled-task
  wrapper is post-v1 and is a separate feature with its own observable
  behaviour. See open question 2 — it is also the honest answer to
  `brew services`.
- **`compose.yaml` in the repo.** Phase 8 deferred it to "Phase 10 or 11, where
  CI has a real compose provider". It belongs with **Phase 11**'s examples, next
  to the applications it would start alongside.
- **A native Windows binary**, and with it the Windows port, the PowerShell
  installer and a Windows CI job. Windows is reached by WSL and by the container
  — see below — and shipping a fifth platform that nobody here can complete a
  login on is shipping untested documentation, which is the same argument Phase
  8 used to keep arm64 out of its own scope. Additive later: five `cfg` gates,
  one path decision, one target-list entry.
- **Windows arm64, and Linux x86 32-bit.** Downstream of the above, and a `dist`
  target list entry away once anyone asks.
- **Any package manager with a review queue** — homebrew-core, winget, apt, AUR,
  nixpkgs. Personal tap only. CONCEPT §7 is explicit that homebrew-core has
  notability requirements we will not meet at launch, and every one of these has
  a human in the loop between a tag and an install.

## Behavior

### A tag is the entire release interface

```
$ git tag v0.1.0 && git push origin v0.1.0
```

Everything downstream is a consequence of that one push: the archives, the
checksums, the release page, the installers, the Homebrew formula, and the
container manifest. **There is no release button, no local build step, and no
artifact that anybody uploads by hand.** A release that requires a person to
remember a step is a release that is eventually cut wrong, and the whole
argument for `dist` — "it will save a weekend of release plumbing" (CONCEPT §7)
— is really the argument for that property rather than for the tool.

The tag must match the version in `Cargo.toml`; `dist` refuses otherwise, which
is the right refusal. Since `/_/health`, the startup banner and `doctor` all
print `CARGO_PKG_VERSION`, **the version a user sees in the banner is the tag
they installed** — which is the one property that makes a bug report legible.

### Linux is static, or it is not distribution

A default `cargo build --release` on this machine links against the host's
glibc and is a `x86_64-unknown-linux-gnu` binary that fails on anything older
with `GLIBC_2.xx not found`. That failure is invisible until it happens on
somebody else's laptop, and the roadmap's criterion — *"the Linux release binary
runs on a distro older than the build host"* — exists precisely to catch it.

So both Linux targets are musl:

```
x86_64-unknown-linux-musl
aarch64-unknown-linux-musl
```

**Nothing in the dependency tree resists this.** `reqwest` is already
`default-features = false` with no TLS stack (Phase 2's decision, made for
binary size), `rsa` and `sha2` are pure Rust, and Phase 8's Dockerfile already
builds a musl binary in an Alpine stage — the container image is a musl build
that has been running since Phase 8. The only new thing is doing it on a runner
for two architectures.

The observable consequence is stronger than "it works on old Debian": `file`
says `statically linked`, `ldd` says `not a dynamic executable`, and the
question "which distros does this support" stops having an answer that needs
maintaining.

### Windows without a Windows binary

**The full browser flow works with lanyard in WSL2 and the application on
Windows**, and it is worth tracing rather than asserting, because "run it in
WSL" usually means "accept something degraded" and here it does not:

| Step | Where it goes |
|---|---|
| .NET app's discovery, JWKS, and code exchange | Windows → `127.0.0.1:9500` → WSL localhost forwarding → lanyard |
| Browser redirect to `/oidc/authorize`, persona picker | The Windows browser, same forwarded port |
| Redirect back to `http://localhost:5000/signin-oidc` | Never leaves Windows — the app is there |

The issuer is `http://127.0.0.1:9500/oidc` from both sides, so nothing
string-compares wrong, and Phase 8's `Host` warning stays quiet. Every one of
CONCEPT §8's gotchas is absent because there is only one address.

**The second path needs no WSL at all.** Docker Desktop or Podman Desktop and
`docker run -p 9500:9500 ghcr.io/robap/lanyard` is the same image criterion 14
proves, and Windows is the platform where "it lives in a container that must
stay running" is least objectionable, because a .NET shop on Windows very likely
has one running already.

Two things this costs, and both are README lines rather than code:

- **`localhost` versus `127.0.0.1`.** Phase 8 deliberately kept one `Host` rule
  with no loopback-equivalence exemption. A WSL user who sets
  `LANYARD_BIND=0.0.0.0` and points an app at `localhost:9500` gets a mismatch
  warning that is technically correct and, in that context, confusing. The
  README says: use `127.0.0.1` on both sides.
- **`LANYARD_BIND`.** Default loopback is usually enough — WSL2's
  `localhostForwarding` reaches a listener bound to `127.0.0.1` inside the VM —
  but mirrored networking mode and some corporate VPN configurations behave
  differently, so `LANYARD_BIND=0.0.0.0` is the documented fallback.

**And it gets watched.** There is a Windows machine with WSL available to this
project, so the paragraph above is an acceptance criterion (criterion 10) rather
than a claim: `spikes/dotnet-web` runs on Windows, lanyard runs in WSL, and
somebody completes a login. That is the same standard every other phase has been
held to, and it is the reason to recommend WSL rather than merely permit it.

**What is still not watched** is a native Windows binary, and the criteria are
written so that stays visible: nothing below asserts anything about `.exe`
behaviour, `%USERPROFILE%` paths, or a file mode Windows does not have.

### Homebrew, and the claim that is not free

The roadmap says the tap gives `brew services start lanyard` "for free". **It
does not.** `brew services` requires the formula to carry a `service do` block
or install a locatable service file; a formula without one fails with
`Formula 'lanyard' has not implemented #plist, #service or installed a locatable
service file`. `dist`'s generated formula installs the binary and defines a
`test do` block — there is no service stanza, and the formula is regenerated on
every release, so hand-editing the tap is a change that survives exactly one
release.

Three ways out, and the third is the one CONCEPT already chose:

| | Cost |
|---|---|
| Post-process the formula in the release workflow | A `sed` against a generated file, re-broken by any `dist` template change |
| Hand-maintain the tap formula | Loses the checksum automation that is the reason to use a tap generator at all |
| **Ship no service block; `lanyard serve` runs in the foreground** | The roadmap loses a sentence it should not have had |

**Recommendation: the third.** CONCEPT §7 already says the always-on story is
`lanyard service install` — "a launchd agent in `~/Library/LaunchAgents`, a
systemd **user** unit on Linux, a scheduled task on Windows… perhaps 200 lines"
— written by us, working identically on all three platforms, not just for people
who installed via `brew`. `brew services` was a shortcut to a feature that is
already designed. Correct the roadmap line rather than build a workaround for
it. (Open question 2.)

Two things the tap does give, and they are the ones that mattered:
`brew install robap/tap/lanyard`, and `brew upgrade`.

### The image: two architectures, and no second compile

Phase 8 built `localhost/lanyard:dev` from a Dockerfile that compiles the
binary in an Alpine builder stage, and said so in its own spec:

> Phase 10, which will have binaries from `cargo-dist` before it has an image,
> can add a `BINARY` build argument that skips the stage; that is an addition to
> this Dockerfile, not a rewrite of it.

That is what happens. The release already produced a static musl binary for each
architecture; the image stage becomes a `COPY` of one of them into
`gcr.io/distroless/static-debian12:nonroot`. Everything else in that Dockerfile
— `LANYARD_BIND=0.0.0.0`, `LANYARD_DATA_DIR=/data`, the non-root uid, the
`HEALTHCHECK` that runs `doctor --quiet` because there is no shell to run a
`curl` — is unchanged and already proven.

**Both architectures are built natively.** `ubuntu-24.04-arm` runners are
generally available and free for public repositories, and `robap/lanyard` is
public. No qemu, no `binfmt_misc`, no emulated build — which also means the
arm64 image is *tested* on arm64 hardware rather than assumed. That is the
answer to Phase 8's stated out-of-scope: *"the build host has no musl target, no
cross toolchain and no qemu binfmt, so an arm64 image cannot be produced or run
here today."* It can be produced and run on a runner.

Tags: `ghcr.io/robap/lanyard:0.1.0` and `:latest`, one manifest list, two
digests underneath.

### CI arrives in this phase, not in Phase 11

Phase 11 owns "CI matrix, one job per example". But `.github/` does not exist,
and this phase is about to add a workflow that takes a git tag and publishes
binaries to the internet under the maintainer's name. **Shipping that before
anything runs the test suite is the wrong order**, and the suite is ready for
it: `tests/support` binds ephemeral ports, needs no network and no fixtures
beyond `tests/data`, so `cargo test --locked` is hermetic on any runner.

`ci.yml`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test --locked`, on `ubuntu-latest` and `macos-latest` — the two platforms
this phase ships binaries for, and no others, because a green job on a platform
nobody downloads is a job that only ever costs time. Both lint gates are clean
today, so they start green and stay meaningful. Phase 11 adds jobs to this file
rather than inventing one.

### The one-time setup a person has to do

Three things cannot be done from a workflow and are prerequisites, not steps:

1. **Create `robap/homebrew-tap`** — an empty public repo. `dist` manages its
   contents from then on. The name is fixed by Homebrew's convention: the repo
   is `homebrew-tap`, the install line is `brew install robap/tap/lanyard`.
2. **Create a GitHub PAT with `repo` scope**, and add it to `robap/lanyard` as
   the `HOMEBREW_TAP_TOKEN` secret. The release workflow needs write access to a
   second repository, which `GITHUB_TOKEN` does not have.
3. **Decide the tap and package are public and permanent-ish.** A pushed tag
   creates a public GitHub release; the tap formula and the GHCR package are
   public artifacts under a personal account. Deleting a release after people
   have an installer pointed at `/latest/download/` breaks them. This is the
   first genuinely outward-facing thing lanyard has done.

### Which version to cut

`v1.0.0` is the roadmap's marker for the end of **Phase 11**, so this phase does
not cut it. `Cargo.toml` says `0.1.0` and nothing has been released, so
`v0.1.0` is the first public version.

**Cut `v0.1.0-rc.1` first.** `dist` marks a prerelease tag as a GitHub
prerelease and, by default, does not publish prereleases to the tap — so the rc
shakes out the workflow, the runners, the musl targets and the GHCR push without
putting a broken formula in front of anyone. Then `v0.1.0` for real, and that is
the release every criterion below is observed against.

### What the README says afterwards

The `## Build and run` section (README:49) becomes `## Install`:

```
brew install robap/tap/lanyard

curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/robap/lanyard/releases/latest/download/lanyard-installer.sh | sh

docker run -p 9500:9500 ghcr.io/robap/lanyard
```

…followed by the banner block that is already there, then a short **Windows**
paragraph naming WSL and the container with the `127.0.0.1` caveat, and then
`cargo build --release` demoted to a "from source" note. Per CONCEPT §13, **the
implementation language appears only under installation** — the top of the
README does not say Rust, and after this phase it does not have to.

The Windows paragraph is load-bearing, not a footnote. A Windows developer who
scans an install section listing macOS and Linux and finds nothing else
concludes the tool does not run on their machine, which is false twice over.
Saying "run it in WSL, or run the container, here is the line" costs three
sentences and is the difference between a deliberate scope decision and an
apparent gap.

`## Running in a container` (README:1236) swaps `localhost/lanyard:dev` for
`ghcr.io/robap/lanyard` throughout. Every other section is Phase 11's problem.

## Acceptance criteria

`v0.1.0` below is the real release cut by criterion 1. Criteria marked **(CI)**
are observed in a workflow log — the job either goes green with the asserted
output or it goes red, and the log is the artifact.

**The release**

- [ ] 1. Pushing `v0.1.0` runs the release workflow to green, and
      `https://github.com/robap/lanyard/releases/tag/v0.1.0` lists archives for
      `aarch64-apple-darwin`, `x86_64-apple-darwin`,
      `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl`, plus
      `lanyard-installer.sh`, `dist-manifest.json` and a checksum for every
      archive — and **no** `x86_64-pc-windows-msvc` archive and no
      `lanyard-installer.ps1`, so the omission is deliberate rather than a
      target that silently failed to build.
- [ ] 2. Every archive downloads and its recorded checksum matches
      `sha256sum` locally. The release body is the `0.1.0` section of
      `CHANGELOG.md`.
- [ ] 3. A prerelease tag (`v0.1.0-rc.1`) produced a GitHub **prerelease**, and
      `brew install robap/tap/lanyard` at that moment did not offer it.

**Linux**

- [ ] 4. In `podman run --rm -it docker.io/library/debian:12` with no Rust
      toolchain (`command -v cargo` prints nothing), the `curl … | sh` installer
      line from the README puts `lanyard` on `PATH`; `lanyard serve` prints the
      banner and `curl` fetches
      `/oidc/.well-known/openid-configuration` with the issuer the banner named.
- [ ] 5. **The older-distro criterion.** `file` on the x86_64 archive's binary
      says `statically linked` and `ldd` says `not a dynamic executable`; the
      same binary, mounted into `docker.io/library/debian:10` (glibc 2.28,
      older than this build host's), runs `lanyard serve` and answers discovery
      to a `curl` from the host. No `GLIBC_` error anywhere.
- [ ] 6. **(CI)** On an `ubuntu-24.04-arm` runner: the aarch64 archive's binary
      is `ELF 64-bit … ARM aarch64 … statically linked` per `file`, runs
      `lanyard serve`, answers discovery, and mints a token with
      `lanyard token --as ada --aud billing-api`.

**Homebrew**

- [ ] 7. `podman run --rm -it docker.io/homebrew/brew`, then
      `brew install robap/tap/lanyard`: installs without compiling anything,
      `command -v cargo` prints nothing, `lanyard --version` prints `0.1.0`,
      `lanyard serve` answers discovery.
- [ ] 8. **(CI)** On `macos-latest`: `brew install robap/tap/lanyard`,
      `lanyard serve &`, discovery returns `200`, and a token from
      `lanyard token --as ada --aud billing-api` is accepted by
      `scripts/jose-verify.mjs` against the live JWKS URL.
- [ ] 9. Whichever way open question 2 lands, it is observable: either
      `brew services start lanyard` leaves a lanyard answering discovery, or
      the README says in as many words that it does not work and names what to
      run instead — and the command in the README is the one that was run.

**Windows**

- [ ] 10. **On a Windows machine, watched by a person.** Install lanyard inside
      WSL2 by the README's `curl … | sh` line; run `spikes/dotnet-web` on
      **Windows** with `Authority = http://127.0.0.1:9500/oidc`; complete a full
      browser login — redirect out, pick "Ada Bell", land back authenticated
      with an email claim. Then `lanyard doctor` inside WSL reports every check
      `OK` with no `Host` mismatch warning from either side. This is Phase 4's
      criterion 1 re-run across the WSL boundary, and it is what makes the
      README's Windows paragraph an observation rather than a claim.
- [ ] 11. The README's `## Install` section names both Windows paths — WSL and
      the container — with a runnable line for each and the `127.0.0.1`-not-
      `localhost` caveat, and a reader who searches the README for "Windows"
      lands on it. The WSL line is the one criterion 10 was run from, verbatim.

**The image**

- [ ] 12. `podman pull ghcr.io/robap/lanyard:0.1.0` on this machine (amd64) with
      no login; `podman run -p 9500:9500` answers discovery from the host;
      `podman inspect --format '{{.State.Health.Status}}'` prints `healthy`
      within 30s; `podman images` shows under 20 MB; `--entrypoint /bin/sh`
      still fails because there is no shell.
- [ ] 13. `podman manifest inspect ghcr.io/robap/lanyard:0.1.0` lists both
      `linux/amd64` and `linux/arm64`; **(CI)** an `ubuntu-24.04-arm` job pulls
      the same tag, gets the arm64 digest, runs it, and gets `200` on discovery.
- [ ] 14. `:latest` and `:0.1.0` resolve to the same manifest digest, and the
      `kid` served by the pulled image is the same one the native binary and
      Phase 8's local image serve.

**CI and packaging**

- [ ] 15. **(CI)** `ci.yml` runs `cargo fmt --check`,
      `cargo clippy --all-targets -- -D warnings` and `cargo test --locked`
      green on `ubuntu-latest` and `macos-latest` for a push to `main`; a
      branch with one deliberate formatting error goes red on the `fmt` step
      and is reverted.
- [ ] 16. **(CI)** `cargo package --locked` succeeds, and the resulting file
      list contains `web/dist/assets/*.js` and `web/dist/index.html` and
      contains no path under `docs/`, `spikes/`, `web/src/` or `.claude/` —
      the check `Cargo.toml`'s `exclude` comment promises, performed without
      publishing to crates.io.

**README**

- [ ] 17. A reader following the new `## Install` section from a clean machine
      reaches a running lanyard by one of three copy-pasted lines, and no line
      above it mentions a Rust toolchain. Each of the three was run verbatim to
      produce criteria 4, 7 and 12 — not retyped afterwards.

## Open questions

Three. One reopens a decision this spec settles; two change a line of config.

**Settled, and recorded here because the roadmap says otherwise:** *no native
Windows binary in v1.* The roadmap lists Windows among five platforms; this spec
ships four and reaches Windows through WSL and the container image. The reasons
are in *The finding* and *Windows without a Windows binary* above. The roadmap
line needs the same correction as the `brew services` one.

1. **There is a Windows machine after all — does that reopen the native
   binary?**

   Partly, and it is worth being straight about which argument it removes. One
   of the reasons above was "a native `.exe` would be the only platform nobody
   here can dogfood". That reason is gone: the same machine that verifies
   criterion 10 could verify a native build.

   **Recommendation: still no, and criterion 10 is why.** The remaining reasons
   did not depend on the observer. Two paths to Windows already ship and one of
   them is now watched end to end; the port is five `cfg` gates plus a path
   decision that is purely additive whenever anyone wants it; and this is a
   distribution phase that is already introducing CI, a tap, a multi-arch
   manifest and the first public release. A fifth target is the thing to add
   *after* the pipeline has cut a release, not during.

   What changes: criterion 10 is a real observation instead of a documentation
   assertion, which is a strictly better outcome — the path the README
   recommends is the path somebody completed a login on. If the WSL experience
   turns out to be worse in practice than the trace above predicts, that is the
   signal to reopen this, and it will be visible while running criterion 10
   rather than after a release.

2. **`brew services`, or `lanyard service install`?**

   **Recommendation: drop `brew services` and fix the roadmap line.** The
   formula `dist` generates has no service block and is regenerated every
   release, so the alternatives are a `sed` against a generated file or a
   hand-maintained tap. CONCEPT §7 already committed to `lanyard service
   install` covering launchd, systemd user units and Windows scheduled tasks —
   one feature, all three platforms, everyone who installed by any channel.
   Criterion 9 is written to be observable either way.

3. **Reconsider crates.io now that there is a release pipeline?**

   **Recommendation: no, and keep the decision doc as it stands.**
   `cargo publish` is one line in the workflow, but it is an obligation from
   then on: a published version that must be yanked or superseded, and a crate
   page that is the wrong front door for a PHP developer. Criterion 16 gives
   the packaging check the `exclude` comment asks for without taking on that
   obligation. Revisit if the shared crate from CONCEPT §16 is ever extracted —
   that one's consumers really are Rust developers.
