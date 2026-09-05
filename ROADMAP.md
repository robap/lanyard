# lanyard — roadmap

Build order for the design in [CONCEPT.md](CONCEPT.md). Each phase has a goal, a
scope, and acceptance criteria naming a concrete observer — a real client doing a
real operation, never "the code does X".

Phases are numbered so `docs/features/` sorts into build order: phase N becomes
`docs/features/NN-<slug>-spec.md` via `/refine`, then `NN-<slug>-plan.md` via
`/plan`. A phase is not done until its acceptance criteria have been watched to
pass against the named client.

---

## North stars

Every scope decision below is a tiebreak against these. If a phase starts
growing configuration, it has drifted.

1. **Accept everything.** No client registration, no redirect allowlist, no
   audience registration. Localhost-only redirects is the entire security
   boundary.
2. **One instance, every project.** Machine-wide singleton on one fixed port.
3. **Issuance is one function.** Browser flow, `/oidc/token`, CLI, and test seam
   are all thin callers of it. This is the structural decision that everything
   else leans on.
4. **Real tokens — including deliberately wrong ones.** RS256 from a real JWKS
   endpoint. Producing bad tokens is a first-class feature, not a config hack.
5. **Single static binary.** Nothing on the build path that needs Node, a JVM,
   or a package manager at runtime.

## Non-goals for v1

SAML, LDAP, WS-Federation, CAS. Implicit and ROPC grants. Multi-tenancy.
Anything that requires an admin console.

---

## Phase 0 — Validation spike

**Goal:** answer the questions that change the shape of later phases, before
writing product code. Cheap, and two of them gate priorities.

- Point a throwaway .NET app at `oidc-provider-mock` (pipx) or Soluto
  `oidc-server-mock` over plain HTTP. Does `RequireHttpsMetadata = false` alone
  get a login working, or does the correlation/nonce cookie
  `SameSite=None` problem bite immediately? (CONCEPT §10)
- Same for PHP with `jumbojett/openid-connect-php`.
- Read navikt/mock-oauth2-server's README end to end, specifically its Docker
  networking notes.
- Search GitHub and the web for existing developer tools named `lanyard`.
- Decide whether to reserve `lanyard-cli` on crates.io.

**Acceptance**

- [ ] A .NET web app completes an interactive login against an existing HTTP-only
      mock provider, and the exact set of settings required is written down.
- [ ] A PHP app completes the same, or the blocker is recorded.
- [ ] `docs/decisions/https-priority.md` states whether HTTPS is v1 or post-v1,
      citing what the spike showed.
- [ ] Name check recorded — no established dev tool with the same name and
      audience, or a rename decision.

**Output:** HTTPS moves into or out of the v1 list below. Nothing else in this
roadmap depends on the answer.

---

## Phase 1 — Skeleton, keys, and the issuance core

**Goal:** `lanyard serve` starts, advertises itself correctly, and can mint a
signed token with no browser involved. This phase decides the internal structure
for the whole project, so the test seam ships here — not later. (CONCEPT §6)

- `lanyard serve` as a plain foreground process. One fixed port (9500), fail
  loudly on conflict, never fall back. `LANYARD_BIND` env var; loopback default.
- Startup banner prints the resolved issuer and the UI URL.
- Signing keypair: deterministic default dev key (stable `kid` and public key on
  a fresh install with no data dir), persisted to the data dir when generated.
  Data dir carries its own `.gitignore`.
- `GET /oidc/.well-known/openid-configuration`, `GET /oidc/jwks`.
- The persona type: stable id, email, display name, roles, arbitrary attributes.
  No `sub`, no `claims` key — persona→claims mapping lives in the OIDC module.
- **The persona file schema carries a `client:` field from day one**, even though
  nothing reads it until Phase 7. Adding it later is a migration. (CONCEPT §15)
- Personas loaded from a global `~/.config/lanyard/users.yaml`, with built-in
  defaults if absent — including the **no-roles / empty-claims persona**.
- `POST /_/api/token` — post claims, get a signed token.
- Routing settled: `/oidc/...`, `/_/...`, root left free.

**Acceptance**

- [ ] `curl http://127.0.0.1:9500/oidc/.well-known/openid-configuration` returns
      a document whose `issuer` exactly matches the banner and whose `jwks_uri`
      fetches.
- [ ] `POST /_/api/token` with `{"sub":"ada","aud":"billing-api"}` returns a
      token that node `jose` `jwtVerify` accepts against the live JWKS URL.
- [ ] Deleting the data dir and restarting produces the same `kid` and the same
      public key.
- [ ] `serve` twice → second process exits non-zero naming the port conflict.

---

## Phase 2 — CLI token minting

**Goal:** the shorter path to being useful, and probably the larger half of
daily use. (CONCEPT §5)

- `lanyard token --as <persona> --aud <audience> --scope '<scopes>'` → a token
  on stdout, nothing else.
- Implemented over the real `client_credentials` grant against `/oidc/token`, so
  the token arrives the way a production token arrives.
- `lanyard env --as … --aud …` → `export BEARER_TOKEN=…`.
- Any `aud` string accepted, including one no API expects. That is a test case,
  not an error.
- 60-second access tokens by default.

**Acceptance**

- [ ] `curl -H "Authorization: Bearer $(lanyard token --as ada --aud billing-api)"`
      against a .NET minimal API using `AddJwtBearer` with
      `Authority=http://127.0.0.1:9500/oidc` returns 200.
- [ ] The same call 90 seconds later returns 401 — short TTLs are real.
- [ ] `eval "$(lanyard env --as ada --aud billing-api)"` sets `BEARER_TOKEN` in
      the calling shell.
- [ ] `lanyard token --as ada --aud not-a-real-api` succeeds and prints a token.

---

## Phase 3 — Deliberate failure tokens

**Goal:** the differentiator, and it is cheap once issuance is one function.
This block goes in the README above the login screenshot. (CONCEPT §6)

- `--expired`, `--wrong-aud`, `--wrong-iss`, `--bad-signature`, `--alg-none`,
  `--unknown-kid`.
- Same overrides reachable through `POST /_/api/token`, so the seam and the CLI
  stay one code path.

**Acceptance**

- [ ] Against the Phase 2 .NET API: good → 200, and each of the six flags → 401.
      Six lines of shell, run as one script.
- [ ] `--alg-none` produces a genuinely `alg: none` token, not an RS256 token
      with a rewritten header.
- [ ] Node `jose` reports a distinct, correct error for each of the six.

---

## Phase 4 — Browser flow and the persona picker

**Goal:** redirect in, click a person, redirect out. (CONCEPT §5)

- `GET /oidc/authorize` — authorization code with PKCE, and with client secret.
  Any `client_id`, any `client_secret`, any `redirect_uri` resolving to
  `localhost`/`127.0.0.1`/`::1`. Non-localhost redirect is the one rejection.
- Persona picker UI at `/_/`: a list of people, no password field, plus a
  "mint one now" panel for arbitrary claims.
- Code exchange at `/oidc/token`, `GET /oidc/userinfo`.
- ID token emits `nonce`, `at_hash`, `c_hash` correctly.
- Session cookie `SameSite=Lax`, `Secure` never set in HTTP mode.
- Sessions keyed per `client_id` so project two does not inherit project one's
  persona. Plus an "always ask" toggle.
- Embedded UI assets — no Node on the build path.

**Acceptance**

- [ ] A .NET web app (`AddOpenIdConnect`, `RequireHttpsMetadata=false`) completes
      a full login: redirect, pick "Ada Bell", land back authenticated with an
      email claim.
- [ ] A `node-spa` using `oidc-client-ts` (public client, PKCE) completes login
      with no client secret and no registration step.
- [ ] `jumbojett/openid-connect-php` completes `authenticate()`.
- [ ] Two apps on different ports with different `client_id`s log in as
      different personas in the same browser session, neither disturbing the
      other.
- [ ] A `redirect_uri` of `https://evil.example.com/cb` is rejected with an
      error the RP can read.

---

## Phase 5 — Session lifecycle and the remaining endpoints

**Goal:** the endpoints SDKs probe for, and the ones missing from most mocks.

- `refresh_token` grant.
- `GET|POST /oidc/end_session` — RP-initiated logout, honouring
  `post_logout_redirect_uri` and `id_token_hint`.
- `POST /oidc/introspect`, `POST /oidc/revoke`.
- ~~CORS on `/oidc/token` and `/oidc/jwks` for any localhost origin.~~ **Landed
  in Phase 4**, in the minimum shape acceptance criterion 2 needed — that
  criterion asks a browser SPA to complete a login, which is impossible
  cross-origin without it. Phase 4's spec records the change.
- **"Log out of lanyard" as a visible control on `/_/`**, not only as a protocol
  endpoint. Today the picker can be told to *always ask* but the browser's
  selection cannot be dropped at all, and there is no `/end_session` yet either:
  the only ways to clear it are closing the browser or restarting the process.
- "Expire this session now" control in the UI.

**There are two sessions, and Phase 4 shipped no way to end either one.**
Measured against `spikes/dotnet-web` after logging in as Ada:

| Cookie deleted | What happens on the next `/secure` |
|---|---|
| `lanyard_session` only | Nothing observable. Still Ada, and the browser never reaches lanyard — the RP's own cookie satisfies the request |
| The RP's cookie only | A round trip to lanyard, which still remembers Ada, so **no picker** and you are silently signed back in as the same person |
| Both | The picker, at last |

The middle row is what `spikes/dotnet-web`'s `/logout` does today: it clears the
app's cookie and looks like it worked.

### What "fully logged out" has to mean

**One click in the app, both sessions gone, no manual cookie deletion.** This is
RP-initiated logout, and it is a *browser redirect chain* rather than a
back-channel call — `lanyard_session` lives in the browser, so only a top-level
navigation carries it:

1. The user clicks **Log out** in the app.
2. The app drops its own cookie and `302`s to
   `{end_session_endpoint}?id_token_hint=…&post_logout_redirect_uri=…&state=…`.
3. lanyard drops `lanyard_session` and `302`s to the `post_logout_redirect_uri`.
4. The user lands back on the app, signed out of both. The next visit to a
   protected page shows **the picker**.

FusionAuth is the reference for this being an ordinary thing a local provider
offers rather than an enterprise feature: its OAuth logout endpoint takes the
same `post_logout_redirect_uri` shape, and an application can be configured for
how far the logout reaches.

**lanyard has to build almost nothing on the client side.** All three spikes
already ship the one-liner and are only waiting for the endpoint to exist:

| Spike | The call |
|---|---|
| `dotnet-web` | `SignOutAsync` over both the cookie and OIDC schemes — `AddOpenIdConnect` builds the redirect itself |
| `php-web` | `$oidc->signOut($idToken, $postLogoutRedirect)` |
| `node-spa` | `mgr.signoutRedirect()` |

Four decisions this forces, none of them obvious:

- **`end_session_endpoint` must appear in discovery**, or .NET will not build the
  redirect at all — it reads the URL from the document. That is the
  advertise-only-what-exists rule cutting the other way for once: the endpoint
  has to ship *and* be advertised in the same phase, or the .NET one-liner is
  silently a no-op.
- **`post_logout_redirect_uri` gets the same loopback check as `redirect_uri`.**
  The one rejection now applies to a second parameter, for the same reason, and
  the rejection is rendered rather than redirected for the same reason again.
- **Does logging out of one app log you out of all of them?** lanyard's session
  holds a selection *per `client_id`* — that is the whole multi-project property
  — so `/end_session` from `billing-web` could drop just that selection or the
  entire browser session including php-web's Mira. Every real IdP has one SSO
  session and clears all of it, and diverging from production is the thing this
  project exists not to do; **clear everything** is the default to beat. But it
  means a logout in one app visibly logs you out of the other two, which is
  exactly the demo in `spikes/README.md`, so it needs to be a decision on the
  record rather than a side effect. A per-`client_id` variant, if it is wanted,
  belongs on lanyard's own UI and not on the protocol endpoint.
- **lanyard never shows a logout confirmation screen.** OIDC RP-Initiated Logout
  says the OP *should* ask the user to confirm when there is no valid
  `id_token_hint`. lanyard has no consent screen and this is the same argument:
  a prompt nobody can automate past is a prompt in the way of a test.

Adding the button means editing every web spike's page, which is the cheap
moment to also make those pages **identical across stacks** — see
[Phase 11](#phase-11--examples-and-the-readme). Doing it here costs one extra
file; doing it later means editing every page twice.

**Acceptance**

- [ ] **One click, both sessions, in all three web spikes.** Click **Log out**;
      the browser goes app → `/oidc/end_session` → back to the app; the next
      visit to the protected page shows **the persona picker**, not a silent
      re-login as the same person. No cookie is deleted by hand at any point,
      and the redirect chain is visible in the network tab.
- [ ] `end_session_endpoint` is in the discovery document and .NET builds the
      redirect from it with no URL hard-coded in the app.
- [ ] `post_logout_redirect_uri=https://evil.example.com/` is refused the same
      way a bad `redirect_uri` is: `400` rendered at lanyard, no `Location`.
- [ ] Whichever answer the per-`client_id` question gets, it is observable:
      log in to two apps as two people, log out of one, and the other's state is
      whatever the decision says it should be — recorded in the spec, not
      discovered in a browser.
- [ ] Clearing `lanyard_session` by hand and revisiting a protected page still
      does nothing visible, and the README says why rather than leaving it as a
      surprise.
- [ ] The `oidc-client-ts` SPA refreshes an expired access token without a
      redirect.
- [ ] A browser SPA on `http://localhost:3000` fetches `/oidc/jwks` with no CORS
      error in the console.
- [ ] `/introspect` reports `active: false` for a token after `/revoke`.

---

## Phase 6 — Live request log

**Goal:** "why did my login fail" answered in the tool's own output, across
every project on the machine at once. (CONCEPT §6)

- Every `/authorize` and `/token` with decoded parameters and resulting claims.
- Three surfaces from one event stream: human-readable stdout, SSE for the UI,
  ndjson for scripting.
- Decoded claims viewer in the UI.

**Acceptance**

- [ ] A failed login (bad PKCE verifier) shows, in the UI log, which parameter
      mismatched — without reading server source.
- [ ] Two different projects logging in concurrently appear interleaved in one
      log, each labelled with its `client_id`.
- [ ] `lanyard logs --json | jq .client_id` works.

---

## Phase 7 — Personas that travel with the repo

**Goal:** close the one genuinely open design question. (CONCEPT §15)

Current lean: linking plus client-id namespacing, with the global file as
fallback so the zero-config path still works on a fresh machine. The `client:`
field reserved in Phase 1 is what makes this additive rather than a migration.

- `lanyard link` from a project records its path; the daemon watches and merges
  each registered project's `lanyard.yaml`.
- A project file declaring `client: billing-web` shows its personas only for
  that `client_id`.
- `lanyard unlink`, plus sane behavior when a linked directory disappears.

**Acceptance**

- [ ] `lanyard link` in project A, then loading `/authorize?client_id=billing-web`
      shows A's personas and not project B's.
- [ ] A fresh machine with no links still shows the built-in defaults.
- [ ] Deleting a linked directory produces a warning, not a crash or an empty
      picker.

---

## Phase 8 — `doctor` and the Docker guardrails

**Goal:** collapse the whole Docker gotcha category into one command whose
output can be pasted into an issue. Nothing in the competitive set has one.
(CONCEPT §7, §8)

- `lanyard doctor`: resolved issuer, self-reachability at that address, internal
  vs external port comparison, clock skew against the host, signing key
  persistence.
- Warn at request time when an inbound `Host` does not match the configured
  issuer, saying explicitly that the minted token will be rejected.
- `nbf`/`exp` leeway of a few seconds, and "expired, but the clock looks skewed"
  as a specific diagnosis rather than a generic 401.
- `GET /_/health` for `depends_on: condition: service_healthy`.
- Container image: distroless static, amd64 + arm64.

**Acceptance**

- [ ] With `-p 9500:8080`, `doctor` reports the port mismatch and names the
      consequence.
- [ ] From inside a Compose container, `curl http://lanyard:9500/_/health`
      returns 200 and `doctor` run in that container passes.
- [ ] Skewing the container clock by 5 minutes produces the skew diagnosis, not
      a bare 401.

---

## Phase 9 — Silent renew

**Goal:** the SPA hidden-iframe path, which is a common source of production
breakage and untestable against most mocks. (CONCEPT §6)

- `prompt=none` handled correctly; `login_required` when there is no session.
- Document what does and does not work over plain HTTP, and why (`SameSite=None`
  requires `Secure`). If Phase 0 showed this needs TLS to work at all, the HTTPS
  phase moves ahead of this one.

**Acceptance**

- [ ] The `oidc-client-ts` SPA renews silently in a hidden iframe with no visible
      redirect.
- [ ] With no session, `prompt=none` returns `login_required` to the iframe
      rather than rendering the picker.

---

## Phase 10 — Distribution

**Goal:** installable by someone who has never heard of Rust. (CONCEPT §7, §14)

- `cargo-dist` in GitHub Actions → release binaries for macOS arm64/x86_64,
  Linux arm64/x86_64, Windows.
- Homebrew tap `robap/tap/lanyard` — which gives `brew services start lanyard`
  for free.
- Container image on GHCR.
- curl installer for the README one-liner.
- Crate `lanyard-cli`, binary `lanyard`.

**Acceptance**

- [ ] `brew install robap/tap/lanyard && lanyard serve` works on a machine with
      no Rust toolchain.
- [ ] The Linux release binary runs on a distro older than the build host.
- [ ] `docker run ghcr.io/robap/lanyard` serves discovery on both amd64 and
      arm64.

---

## Phase 11 — Examples and the README

**Goal:** a directory listing that answers "does it work with my stack" without
prose, and a README whose first line lets a stranger finish "oh, this is for
when I…". (CONCEPT §11, §13)

- `examples/curl/` first — zero dependencies, proves the tool without committing
  anyone to a framework.
- `examples/dotnet-web/` second — the one most likely to need
  `RequireHttpsMetadata = false`, and we will have debugged it anyway.
- **One shared page, rendered by every stack.** Every web example — and every web
  spike it grows out of — serves byte-identical HTML and CSS: the same landing
  page, the same "sign in" and "log out" buttons in the same places, the same
  claim table. Only the server-side plumbing differs.

  Today `spikes/dotnet-web` renders `text/plain` from a `Results.Text` and
  `spikes/php-web` echoes three lines from a `header('Content-Type: text/plain')`,
  so comparing what two stacks actually did means reading past two different
  presentations of it. **That is the whole reason to make them identical**: when
  the page is a constant, every visible difference between .NET and PHP is a
  difference in the stack, which is the question these apps exist to answer.
  It also makes the Phase 11 screenshot and GIF reusable across examples instead
  of one-per-framework.

  Cheapest shape that gets it: one `shared/` directory holding the page as a
  template with a couple of substitution points (the claim rows, the signed-in
  state), copied or symlinked into each example, with a CI check that the
  rendered output matches across stacks. `node-spa` fills the same template
  client-side rather than server-side — the markup can still match, and where it
  cannot, that difference is itself worth seeing.

  **Start this in Phase 5**, which has to touch every one of these pages anyway
  to add the logout button. A down payment already exists: `dotnet-web` and
  `php-web` share a landing page that is identical apart from one explanatory
  sentence, added when both spikes stopped auto-redirecting. The claim views are
  still two different formats, and that is the part left to do.
- Each ships a `test.sh` exercising the failure tokens: good → 200,
  expired → 401, wrong audience → 401, no token → 401.
- CI matrix, one job per example. That makes `examples/` an executable
  compatibility promise rather than documentation that rots.
- README: tagline, then the persona picker screenshot directly beneath it (light
  mode, cropped tight, `<picture>` for both schemes), then the failure-token
  block, then two-command install. Implementation language appears only under
  installation.
- A GIF: clicking "Ada Bell", decoded claims appearing in the live log.

**Acceptance**

- [ ] `git clone && cd examples/curl && ./test.sh` passes against a lanyard the
      operator installed 60 seconds earlier, with no configuration between.
- [ ] `examples/dotnet-web` runs with two commands and completes a login.
- [ ] Two web examples on different stacks, screenshotted side by side, are
      indistinguishable apart from the persona and the port.
- [ ] Both example jobs green in CI.

**→ v1.0**

---

## Post-v1

Ordered by expected value, not by effort. Phase 0's findings can promote HTTPS
above everything here.

- **HTTPS and `lanyard trust`.** Local CA (not a self-signed leaf), trust store
  installation per platform, NSS databases for Firefox/Chromium on Linux,
  detection of an existing mkcert CA. Serve HTTP on 9500 and HTTPS on 9501
  simultaneously once a CA exists. `trust` prints the exact env var line per
  detected runtime (`NODE_EXTRA_CA_CERTS`, `REQUESTS_CA_BUNDLE`, `keytool`), and
  `doctor` verifies a real handshake. The CA private key is a real MITM
  capability — 0600, config dir only, said plainly in the README. (CONCEPT §9)
- **Claim profiles.** Auth0, Cognito, Okta, Entra ID shapes from the same
  persona. **Entra first** — `tid`, `oid` as the stable id distinct from `sub`,
  `roles`, `scp` as a space-delimited string, `preferred_username` over `email`,
  and the v1.0/v2.0 endpoint distinction. (CONCEPT §6)
- **Opaque access token mode.** Half of all application code validates the wrong
  token; returning an opaque access token is a good bug-finder.
- **`lanyard service install|status|logs|uninstall`.** launchd agent, systemd
  *user* unit, Windows scheduled task. A thin wrapper around `serve` — no
  forking, no pidfiles, no daemon mode. (CONCEPT §7)
- **`lanyard proxy --port 8081 --target localhost:8080 --as ada`.** Attaches a
  fresh bearer to every request, which is what lets short TTLs stay short
  without making interactive exploration irritating.
- **Device code grant**, if CLI apps come up.
- **Offline signing** — the CLI signs directly from the on-disk keypair with no
  server running.
- **Socket activation** — launchd `Sockets`, systemd socket units. The elegant
  version of always-on; ship boring first.
- **More examples**: `dotnet-api`, `php-web`, `node-api`, `node-spa`,
  `python-api`, `go-api`. Empty slots are a good "help wanted" signal.
- **Extract the shared crate** with cubby — bind-address handling, health
  endpoint, self-gitignoring data dir, `doctor` scaffolding, the
  stdout/SSE/ndjson event stream, embedded UI, conformance runner. A library's
  consumers *are* Rust developers, so that one belongs on crates.io.
  (CONCEPT §16)

## Explicitly deferred, possibly forever

- SAML 2.0. Not a variant of what we are building — a separate stack with XML
  signatures and metadata exchange. The protocol-neutral persona model is the
  door we left open; walking through it is where an outside contributor could
  add the most value.
- LDAP, WS-Federation, CAS.
- Implicit and ROPC grants, unless something we own needs them.
- `--ttl 30d` for Postman collections. If it ever goes in, make the log line
  loud.
