# Silent renew — spec

**Status:** done · **Roadmap:** Phase 9 · **Slug:** `09-silent-renew`

## Why

**The hidden iframe is the one login path lanyard has never actually run.**

Two earlier phases touched its edges and both were careful to say they had not
done it:

- **Phase 4 shipped `prompt=none`** — but as a *refusal to render*, not as a
  claim that renew works. Its spec says so in the out-of-scope list: "lanyard's
  `Lax` cookie is not sent on a third-party iframe navigation, so an actual SPA
  renew will get `login_required`. That is Phase 9's problem and its evidence."
- **Phase 5 shipped a renew with no redirect** — over `grant_type=refresh_token`,
  which the `node-spa` README is equally careful about: "That is a different
  mechanism from `prompt=none` in a hidden iframe, which is Phase 9's subject and
  is untouched by this."

So `/authorize?prompt=none` has been exercised by `curl` and by
`tests/browser_flow.rs`, and never once by a browser that put it in an `<iframe>`.
That gap is the entire phase, and it matters because CONCEPT §6 picked this
feature for a specific reason:

> SPAs do this in a hidden iframe. It is a common source of production breakage
> and nearly impossible to test against a mock that does not implement it.

A provider that returns the right thing to `curl` and the wrong thing to an
iframe is worse than one that does not implement `prompt=none` at all: it looks
like coverage.

### This is the promised revisit point for HTTPS

`docs/decisions/https-priority.md` put HTTPS post-v1 and named exactly one thing
that could move it:

> **Revisit when** Phase 9 (silent renew, `prompt=none`). A hidden iframe makes
> lanyard's session cookie third-party, which needs `SameSite=None`, which needs
> `Secure` — and unlike the correlation cookie, that one is **lanyard's** cookie
> on **lanyard's** origin, so the localhost exception may not save it. That is
> the test that could move HTTPS forward, and it was explicitly out of scope
> here.

The roadmap says the same thing from the other side: *"If Phase 0 showed this
needs TLS to work at all, the HTTPS phase moves ahead of this one."* Phase 0
deliberately did not look. **This phase owes a written answer**, and the answer
is a deliverable of equal weight to the code.

### Which north stars it serves

- **North star 1 — accept everything.** A silent renew must work with no
  registered `silent_redirect_uri` and no `post_logout` allowlist entry. The
  loopback check is the only thing the iframe's `redirect_uri` has to pass, same
  as every other one.
- **North star 4 — real tokens.** A renew that "works" through browser leniency
  that production will not extend is precisely the class of bug this project
  exists to surface early. Either it works for a reason we can name, or it
  fails and we say so in the README.
- **North star 2 — one instance, every project.** The failure this phase is
  hunting is a *naming* failure — which of lanyard's several addresses the app
  dialled — and that is the shape every problem in a machine-wide singleton
  takes.

## What is actually unknown

Not "does silent renew work over HTTP". The honest question is narrower, and
splitting it is most of the work.

`SameSite` compares **scheme + registrable domain, and never the port.** For a
host with no registrable domain the host itself is the site. So of the two
loopback names lanyard answers to:

| App origin | lanyard's issuer origin | Same site? | Does `Lax` ride the iframe navigation? |
|---|---|---|---|
| `http://localhost:5173` | `http://localhost:9500` | yes — one host, port ignored | expected yes |
| `http://localhost:5173` | `http://127.0.0.1:9500` | no — two different hosts | expected no |
| `http://127.0.0.1:5173` | `http://127.0.0.1:9500` | yes | expected yes |
| `http://web.localtest.me:5173` | `http://localhost:9500` | no | expected no |

Every "expected" in that table is a prediction, not a measurement, and this spec
does not get to assert any of them. **Phase 0's method is the standard**: it
predicted .NET's `SameSite=None` correlation cookie, confirmed it verbatim in a
network trace, and then reproduced the failure on demand by moving only the app's
origin. Phase 9 does the same for lanyard's own cookie.

If the table holds, the answer to "does silent renew work over plain HTTP" is
neither yes nor no. It is **"it depends which name you dialled"** — which is a
rule that fits in one README line and one `doctor` note, and is a much better
outcome than a CA.

**And the current default is the failing row.** `src/config.rs` defaults the
issuer to `http://127.0.0.1:9500/oidc`, while every web spike is reached at
`http://localhost:<port>`. Out of the box, today, the app and lanyard are
cross-site. That is not a detail of the test setup; it is the shipped default,
and see open question 1.

## In scope

- **Run the iframe renew for real**, in Chrome for Testing and Firefox, across
  the host matrix above, and write down what each cell does.
- **`node-spa` gains the iframe path** — a `silent-callback.html` and a way to
  force `signinSilent()` down the iframe branch — *without* losing Phase 5's
  refresh-token evidence, which the same spike is the acceptance client for.
- **`login_required` says which of four things happened.** Today one generic
  sentence covers four causes, and the one a developer most needs to tell apart
  — *the browser sent no cookie at all* — is the `SameSite` symptom itself.
- **lanyard stays frameable**, asserted rather than assumed: no
  `X-Frame-Options`, no `Content-Security-Policy: frame-ancestors` on any
  `/authorize` response, including the rendered `400`.
- **A renew is a renew, not a re-login**: fresh `iat`/`exp`, same `sub`, same
  `auth_time`.
- **`id_token_hint` on `prompt=none`** — honoured or explicitly ignored, decided
  here rather than left silent (open question 2).
- **`docs/decisions/silent-renew-over-http.md`**, and
  `docs/decisions/https-priority.md`'s *Revisit when* section updated to point
  at it.
- **README troubleshooting entry** with the verbatim console error, in the shape
  Phase 0's obligation #2 established for "Correlation failed".

## Out of scope

- **HTTPS and `lanyard trust`.** This phase produces the *decision* about
  whether they move up the roadmap. It does not produce a CA. If the decision is
  "move it up", that is a re-ordering of the roadmap and a separate spec.
- **OIDC Session Management** — `check_session_iframe`, `session_state`,
  `monitorSession`, the OP status iframe. `oidc-client-ts` can be asked to poll
  it and will be asked not to. That is a different specification from
  RP-Initiated Logout and it is not on the roadmap.
- **Front-channel and back-channel logout.** Same reasoning.
- **Flipping the cookie to `SameSite=None; Secure`.** It cannot work over HTTP
  by construction — the browser drops it — and `src/session.rs` has a test whose
  stated purpose is to fail the day somebody hardens the cookie. That test stays.

  > **Corrected by the measurement.** "It cannot work over HTTP by construction"
  > is false, and was worth finding out: `http://localhost` and
  > `http://127.0.0.1` are *secure contexts*, so both browsers store and send a
  > `Secure` cookie set over plain HTTP there, and the cross-site row renews.
  > Flipping the cookie would have worked. It stays out of scope on north star 4
  > instead — a cookie claiming `Secure` while lanyard serves `http://` is one
  > that silently stops being sent the day the deployment changes. See
  > `docs/decisions/silent-renew-over-http.md` and its counterfactual trace.
- **Making `prompt=none` create a session it did not have.** A renew that mints
  a persona nobody picked is not a renew.
- **`prompt=consent`.** Still no consent screen; still registration in a
  different costume.
- **Changing the picker.** Nothing in this phase renders HTML.

## Behavior

### The renew, step by step

`oidc-client-ts` v3.4.0 with `automaticSilentRenew` and **no refresh token**
(that is: no `offline_access` in scope) takes the iframe branch:

1. The access token nears expiry; `SilentRenewService` calls `signinSilent()`.
2. It creates a hidden `<iframe>` and navigates it to
   `{authorization_endpoint}?response_type=code&prompt=none&redirect_uri={silent_redirect_uri}&code_challenge=…`.
3. **lanyard reads `lanyard_session` off that request, or does not.** This is the
   whole phase in one line.
4. On a hit: a `302` to the `silent_redirect_uri` carrying `code` and `state`.
   On a miss: a `302` carrying `error=login_required` and `error_description`.
5. The callback page — same origin as the SPA — calls `signinSilentCallback()`,
   which `postMessage`s the URL to the parent frame.
6. The parent exchanges the code at `POST /oidc/token` **cross-origin**, which
   works because Phase 4 already shipped CORS on that endpoint.
7. The address bar never moves and nothing is visible.

Two library defaults matter and are confirmed in the vendored bundle:
`silent_redirect_uri` **defaults to `redirect_uri`**, and
`includeIdTokenInSilentRenew` **defaults to `false`** (so no `id_token_hint` is
sent unless asked for).

The first default is a trap for this spike specifically: `redirect_uri` is
`http://localhost:5173/`, whose `index.html` runs `signinRedirectCallback()` when
it sees `code=` in the query string. Loaded inside the renew iframe it would try
to complete a *redirect* login and fail. Hence a dedicated callback page — which
is also what every real SPA ships.

### What `login_required` should say

`/authorize` refuses a `prompt=none` for four distinct reasons and currently
describes all four identically:

```
prompt=none was sent and this browser has no usable selection for this client_id
```

That sentence is true in all four cases and useful in none, because the
developer's actual question is *"did my cookie arrive?"* Four causes, four
descriptions:

| Cause | What it means | What the description must name |
|---|---|---|
| No `lanyard_session` cookie on the request at all | The browser did not send it — third-party iframe, cleared jar, or a restart | **that no cookie arrived**, and that a cross-site iframe is the usual reason |
| Cookie present, no selection for this `client_id` | Never logged in to *this* app, or logged out of it | the `client_id` it looked under |
| Cookie present, selection present, `always_ask` on | The picker toggle is on | that the toggle is the cause and where it lives |
| Cookie present, selection present, older than `max_age` | The RP asked for a fresher authentication than exists | the `max_age` and the selection's age |

The first row is the one that pays for this phase. "No cookie arrived" is the
`SameSite` diagnosis, delivered by the tool at the moment of failure instead of
by a developer's third hour in the network tab.

These descriptions reach two places for free — the RP's own error handler reads
`error_description` off its query string, and Phase 6's log already attaches it
(`redirect_error` calls `attach` with both `error` and `error_description`). No
new plumbing; the strings get better and the log gets better with them.

**The error code stays `login_required` in all four cases.** It is what OIDC
Core §3.1.2.6 specifies and what `oidc-client-ts` branches on. Only the human
half changes.

### The picker still never renders

Unchanged from Phase 4 and re-asserted here, because this is the phase where a
regression would be invisible: `prompt=none` returns a code or a redirect, never
markup. A picker inside a hidden iframe is a login screen nobody can click.

### lanyard must stay frameable

There is no `X-Frame-Options` and no `Content-Security-Policy` anywhere in `src/`
today, so the iframe path works by omission. That is fine and it should stay
fine — but "by omission" is exactly what a future security-hygiene commit
removes without noticing, and the symptom would be a silent renew that stopped
working in one browser family first. It gets an assertion.

### One case stays broken, and gets documented instead of fixed

A `prompt=none` whose `redirect_uri` fails the loopback check renders a `400`
(RFC 6749 §4.1.2.1 — no error may be sent to an address just refused). Inside a
hidden iframe that page is invisible, and the SPA sees only
`silentRequestTimeoutInSeconds` elapsing. **Nothing can be done about this
without breaking the one rejection**, so the README says it: *a silent renew that
times out with no network response was probably refused, and the reason is in
`lanyard logs`.* The live log is the answer here, which is what Phase 6 was for.

### A renew is not a re-authentication

The token from a renew must carry a **fresh `iat`/`exp`**, the **same `sub`**,
and the **same `auth_time`** as the login it renews. `auth_time` is the time the
human actually picked a person; a renew that advances it would let an RP's
`max_age` check pass forever without anybody ever authenticating again.

`authorize::resolve` already returns `selection.auth_time` rather than `now`, so
this is a property to pin with a criterion, not one to build.

## Acceptance criteria

Each names a client and an operation, or a file. Criteria 1–3 are the phase; the
rest keep it honest.

**The measurement**

- [x] 1. **Same-name, Chrome.** `node-spa` in iframe mode, app at
      `http://localhost:5173`, lanyard started with
      `LANYARD_ISSUER=http://localhost:9500/oidc`. After the access token nears
      expiry, DevTools Network shows `GET /oidc/authorize?…prompt=none…` with
      `Sec-Fetch-Dest: iframe` **and a `Cookie: lanyard_session=…` header**, then
      a `302` carrying `code=`, then `POST /oidc/token`. No `/authorize` in the
      top frame, no picker on screen, address bar unchanged, and the console
      prints `[node-spa] access token renewed`. Repeated in **Firefox**.
- [x] 2. **Cross-name, both browsers.** The same run with the stock issuer
      `http://127.0.0.1:9500/oidc` and the app still on `http://localhost:5173`.
      Whatever happens is recorded verbatim in the decision doc: the iframe
      request's `Cookie` header (present or absent), the `Location` it gets back,
      and the console line `oidc-client-ts` emits. If it fails, the failure is
      reproducible on demand by changing only `authority`.
- [x] 3. **`docs/decisions/silent-renew-over-http.md` exists**, cites criteria 1
      and 2's traces, and states in its first line whether HTTPS moves ahead of
      Phases 10–11 or stays post-v1. `docs/decisions/https-priority.md`'s
      *Revisit when* section links to it and no longer describes the question as
      open.

**The four causes**

- [x] 4. **Six** `curl -i 'http://…/oidc/authorize?…&prompt=none'` calls — (a) no
      cookie, (b) a cookie whose only selection is under a different
      `client_id`, (c) a cookie with a selection and `always_ask` on, (d) a
      cookie with a selection plus `max_age=1` and a selection older than that,
      (e) a cookie whose selection names a persona removed from `users.yaml`,
      (f) an `id_token_hint` whose `sub` is somebody other than the remembered
      selection — each `302` to the `redirect_uri` with `error=login_required`
      and **six different `error_description` values**, (a)'s naming that no
      cookie arrived.
- [x] 5. `lanyard logs --json | jq -r .error_description` over those same six
      requests prints the same six distinct sentences.
- [x] 6. None of the six responses' bodies contains picker markup
      (`grep -c 'name="persona"'` → 0), and none is a `200`.

**The renewed token is a real token**

- [x] 7. The access token from criterion 1's renew verifies against the live
      JWKS with `scripts/jose-verify.mjs`, and its `exp` is later than the
      pre-renew token's.
- [x] 8. The ID token from the renew and the ID token from the original login,
      decoded side by side, have the **same `sub` and the same `auth_time`** and
      **different `iat`**.
- [x] 9. After `mgr.signoutRedirect()` completes, the next automatic renew's
      iframe gets `error=login_required` and the SPA shows the signed-out page —
      a logout is not silently undone by a renew.

**Frameability**

- [x] 10. `curl -is` of a `prompt=none` success `302`, a `login_required` `302`,
      and the rendered `400` for a non-loopback `redirect_uri` shows no
      `X-Frame-Options` and no `Content-Security-Policy` header on any of the
      three.

**Nothing earlier regressed**

- [x] 11. `node-spa` in its default (refresh-token) mode still renews with a
      `POST /oidc/token` carrying `grant_type=refresh_token` and **no**
      `/authorize` and **no** iframe — Phase 5's criterion, re-run.
- [x] 12. `src/session.rs`'s `the_cookie_is_never_secure_and_never_expires` test
      still passes, and `cargo test` is green.
- [x] 13. Loading `http://localhost:5173/` normally still completes a full
      redirect login and renders the shared page once — the new callback page
      did not change the top-frame path.

**Written down**

- [x] 14. The README's troubleshooting section contains the same-name rule and
      the verbatim `oidc-client-ts` silent-renew error, in the format Phase 0's
      obligation #2 set for "Correlation failed".
- [x] 15. The README says that a silent renew which times out with no response
      was probably a refused `redirect_uri` rendered invisibly in the iframe, and
      points at `lanyard logs`.

## Open questions

**All four leans were confirmed on 2026-09-06.** Recorded here as answered
rather than deleted, because the reasoning is what the plan cites.

1. **Should the default issuer become `http://localhost:9500/oidc`?**
   This phase is what surfaces the question: the shipped default is `127.0.0.1`,
   every spike is dialled at `localhost`, and if the site table holds then the
   default configuration is the one that cannot silently renew. Changing it is
   one line in `src/config.rs` and a long tail everywhere else — the banner,
   `doctor`, Phase 8's container criteria, three spikes, every doc that quotes a
   URL. It also has a real hazard: `localhost` resolves to `::1` first on many
   systems while `DEFAULT_BIND` is `127.0.0.1`, so the default bind would have to
   cover both or a client gets connection-refused for a *new* reason.
   **Resolved: leave it.** Measure first (criteria 1–2), then decide with the
   trace in hand. Criterion 1 sets `LANYARD_ISSUER` explicitly and the decision
   doc records what the stock default costs. If it should change, it changes in
   its own phase with its own criteria — after `DEFAULT_BIND` covers `::1` —
   not as a rider here.

2. **`id_token_hint` on `prompt=none`: honour or ignore?**
   OIDC Core says the OP should return `login_required` when the hint names a
   subject other than the current session's. `oidc-client-ts` does not send one
   by default. Honouring it is maybe twenty lines — verify the signature, compare
   `sub` to the remembered selection — and it makes a real mismatch observable
   instead of returning a token for the wrong person.
   **Resolved: honour it, `sub` comparison only** — no `azp`, no `aud`
   cross-check, and **no expiry check**, because a hint is expected to be
   expired. That last point means it cannot reuse `jws::verify`, which enforces
   `exp`. A mismatch returns `login_required`, which makes it the sixth cause in
   the table above. It is the one behavioural addition in this phase.

3. **How does `node-spa` expose both renew paths?**
   The same spike is Phase 5's acceptance client for the refresh-token path and
   Phase 9's for the iframe path, and the two differ only by whether
   `offline_access` is in `scope`. Options: a `?renew=iframe` query toggle in the
   one page; a second page `silent.html` beside it; or a checkbox on the shared
   page.
   **Resolved: the query toggle.** One file, both paths, and the difference
   between them is legible in one `if`. `spikes/shared/page.html` is deliberately
   byte-identical across three stacks, so a visible control there is off the
   table and the toggle is URL-only.

4. **Should `doctor` gain a silent-renew note?**
   It knows the resolved issuer, so it can say: *"the issuer host is `127.0.0.1`;
   a browser app served from `localhost` is cross-site to it, and a hidden-iframe
   silent renew will not receive lanyard's session cookie."* It cannot know the
   app's origin, so it is a note rather than a check — which is arguably the
   right register for it, and Phase 8 established that `doctor` is where
   this class of naming problem gets explained.
   **Resolved: yes, one line, gated on criterion 2** confirming the diagnosis.
   If criterion 2 shows the cross-site case works after all, the note is a lie
   and does not ship.

5. **Does anything need to happen for a SPA served from `file://` or a
   non-loopback host?** Neither is on the roadmap and neither is in the spikes.
   **Resolved: no.** A non-loopback `redirect_uri` is the one rejection, and
   `file://` has an opaque origin that cannot receive a redirect at all.

## Addendum — a fifth cause, found while planning

The table in *What `login_required` should say* lists four causes. Reading
`authorize.rs` against it turns up a fifth that reaches the same
`redirect_error` call: a cookie is present, a selection is present and fresh,
**but `resolve()` returns `None`** — the persona it names has been deleted from
`users.yaml`, or Phase 7 has since scoped it to a different `client_id`. An
interactive login falls through to the picker there, which is right; a
`prompt=none` cannot, and today it borrows the "no usable selection" sentence
that the other four also borrow.

It gets its own description for the same reason as the rest: *"the selection
this browser holds names persona `ada`, which no longer exists for client_id
`billing-web`"* is a fixable sentence, and the generic one is not. Honouring
`id_token_hint` (question 2) adds a sixth. **Criterion 4 therefore covers six
descriptions, not four.**
