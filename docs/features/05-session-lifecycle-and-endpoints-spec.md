# Session lifecycle and the remaining endpoints — spec

**Status:** done · **Roadmap:** Phase 5 · **Slug:** 05-session-lifecycle-and-endpoints

## Why

Phase 4 shipped a login and no way out of it. **There are two sessions, and
nothing today ends either one.** Measured against `spikes/dotnet-web` after
logging in as Ada:

| Cookie deleted | What happens on the next `/secure` |
|---|---|
| `lanyard_session` only | Nothing observable. Still Ada, and the browser never reaches lanyard — the RP's own cookie satisfies the request |
| The RP's cookie only | A round trip to lanyard, which still remembers Ada, so **no picker**, and you are silently signed back in as the same person |
| Both | The picker, at last |

The middle row is what `spikes/dotnet-web`'s `/logout` does today: it clears the
app's cookie and looks like it worked. `spikes/README.md` has a note admitting
this, and the two working resets it offers — tick "always ask", or restart the
process — are both the tool telling you to work around it.

- **CONCEPT §4 names `/end_session` as a reason this project exists.**
  "RP-initiated logout is a common trip-up and is missing from many mocks."
  FusionAuth is the reference for it being an ordinary thing a local provider
  offers rather than an enterprise feature. `/revoke` is a dozen lines and some
  SDKs probe for it; `/introspect` is what makes a revocation observable.
- **CONCEPT §6 asks for short lifetimes *and* an "expire this session now"
  control.** Phase 2 shipped the 60-second access token. The control that makes
  a live token stop working on purpose is the other half, and it only becomes
  buildable once revocation exists — which is this phase.
- **North star 3 — issuance is one function.** `refresh_token` is a third arm on
  `POST /oidc/token`, a third caller of `issue()`. If a refreshed access token's
  claims get assembled anywhere else, the structure was wrong.
- **North star 1 — accept everything.** `/end_session`, `/introspect` and
  `/revoke` are three more endpoints where every other IdP puts client
  authentication. lanyard checks none of it. The one rejection extends to a
  second parameter — `post_logout_redirect_uri` — and to nothing else.
- **Phase 11 has a down payment due here.** Adding a **Log out** button means
  editing all three web spikes' pages. That is the cheap moment to make those
  pages identical across stacks, which is
  [Phase 11](../../ROADMAP.md#phase-11--examples-and-the-readme)'s "one shared
  page, rendered by every stack". Doing it now costs one extra file; doing it
  later means editing every page twice.

## In scope

- **`GET`/`POST /oidc/end_session`** — RP-initiated logout. Honours
  `post_logout_redirect_uri`, `state` and `id_token_hint`. Drops the **whole**
  browser session, expires the cookie, and never renders a confirmation screen.
- **`end_session_endpoint` in the discovery document**, in the same phase the
  endpoint ships. .NET builds the logout redirect by reading that URL; without
  it, `SignOutAsync` is silently a no-op.
- **`post_logout_redirect_uri` gets the same loopback check as `redirect_uri`**,
  and the same rendered `400` when it fails.
- **The `refresh_token` grant**, gated by `offline_access` exactly as the ID
  token is gated by `openid`. Opaque tokens, rotated on every use.
- **`POST /oidc/introspect`** (RFC 7662) and **`POST /oidc/revoke`** (RFC 7009),
  both accepting client authentication in any form and checking none.
- **Revocation is real**: a revoked access token reads `active: false` at
  `/introspect` **and** `401` at `/userinfo`.
- **Three controls on `/_/`**, all plain `POST` forms, no JavaScript:
  **Log out of lanyard** (the whole browser session), **Forget** (one
  `client_id`'s selection — the per-client variant the roadmap says belongs on
  the UI and not on the protocol endpoint), and **Expire now** (revoke one
  `client_id`'s live tokens while keeping the selection).
- **Discovery grows** `end_session_endpoint`, `introspection_endpoint`,
  `revocation_endpoint`; `grant_types_supported` gains `refresh_token`;
  `scopes_supported` gains `offline_access`.
- **A Log out button in all three web spikes**, each one the SDK's own one-liner
  and no hand-built URL.
- **`spikes/shared/page.html`** — one page shell, read at runtime by
  `dotnet-web` and `php-web` and fetched by `node-spa`, so the three apps render
  the same markup and only the plumbing differs.
- **README**: the new endpoints, the logout section, the SSO-wide decision, and
  why deleting `lanyard_session` by hand does nothing visible.

## Out of scope

- **Back-channel and front-channel logout**, `sid` in the ID token,
  `check_session_iframe`, and OIDC Session Management's `postMessage` polling.
  Those exist to tell *other* applications that a logout happened. The three
  spikes are three separate browser sessions in one browser and none of them is
  watching the others; a mechanism nothing observes is a mechanism nothing can
  hold to account.
- **Persisting anything across a restart.** Refresh tokens, revocations and
  sessions all live in memory and all die with the process, exactly as codes and
  pending requests do. Restarting lanyard remains the biggest hammer.
- **Silent renew's iframe mechanics** (Phase 9). This phase makes the SPA renew
  *without a redirect* by giving it a refresh token, which is a different
  mechanism from `prompt=none` in a hidden iframe. Phase 9 still owns the iframe
  path and the `SameSite=None` problem underneath it, and this phase makes no
  claim about it.
- **The request log** (Phase 6). A failed refresh has to be diagnosable from its
  `error_description` alone.
- **Filtering personas by `client_id`** (Phase 7). Still parsed, still read by
  nothing.
- **CORS** — landed in Phase 4, in the shape acceptance criterion 2 needed. The
  new endpoints inherit it from the same layer because they are mounted under
  `/oidc`; nothing is widened, and `Access-Control-Allow-Credentials` is still
  never sent.
- **A logout confirmation screen.** OIDC RP-Initiated Logout §2 says the OP
  *should* ask the user to confirm when there is no valid `id_token_hint`.
  lanyard has no consent screen and this is the same argument: a prompt nobody
  can automate past is a prompt in the way of a test.
- **Checking client authentication on `/introspect` and `/revoke`.** RFC 7662 §2.1
  requires it. See Behavior — this is one of the places where accept-everything
  and an RFC's MUST disagree, and it is decided in favour of accept-everything
  and written down.
- **Token exchange, device code, and `token_type_hint` being obeyed as a
  requirement.** The hint is read as a hint.
- **A CI check that the shared spike page renders identically across stacks** —
  Phase 11, along with `examples/` itself. This phase ships the shared file and
  a criterion that a human ran `diff` once.
- **`flaw=` on the refresh grant or on the ID token.** Still a later phase's
  call, as Phase 4 left it.

## Behavior

### What "fully logged out" has to mean

**One click in the app, both sessions gone, no manual cookie deletion.** It is a
*browser redirect chain* rather than a back-channel call, because
`lanyard_session` lives in the browser and only a top-level navigation carries
it:

1. The user clicks **Log out** in the app.
2. The app drops its own cookie and `302`s to
   `{end_session_endpoint}?id_token_hint=…&post_logout_redirect_uri=…&state=…`.
3. lanyard drops the whole browser session, expires `lanyard_session`, and
   `302`s to the `post_logout_redirect_uri`.
4. The user lands back on the app, signed out of both. The next visit to a
   protected page shows **the picker**.

**lanyard builds almost nothing on the client side.** All three spikes already
ship the one-liner and are only waiting for the endpoint to exist:

| Spike | The call | What it needs from discovery |
|---|---|---|
| `dotnet-web` | `SignOutAsync` over the cookie **and** OIDC schemes | `end_session_endpoint`; the handler builds the URL, adds `id_token_hint` because `SaveTokens = true`, and uses its own `SignedOutCallbackPath` as the `post_logout_redirect_uri` |
| `php-web` | `$oidc->signOut($idToken, $postLogoutRedirect)` | `end_session_endpoint` |
| `node-spa` | `mgr.signoutRedirect()` | `end_session_endpoint`; `post_logout_redirect_uri` is already in its settings |

No spike hard-codes a lanyard URL for this, and criterion 4 is a `grep` proving
it.

### `/oidc/end_session`

`GET` and `POST`, because RP-Initiated Logout §2 allows both and .NET sends the
first while some SDKs send the second.

| Parameter | Handling |
|---|---|
| `post_logout_redirect_uri` | Optional. **Loopback or rendered `400`** — the one rejection, applied to a second parameter. |
| `state` | Echoed byte-for-byte on the redirect when sent; absent when not. Never invented. |
| `id_token_hint` | Optional, **never required, never a condition**. Decoded for its `aud` so the rendered page can name the application; a hint that is expired, foreign, malformed or absent changes nothing about the outcome. |
| `client_id` | Optional, ignored except for display. |
| `logout_hint`, `ui_locales` | Ignored, per Phase 2's rule for unrecognized parameters. |

The session is dropped **before** the redirect is written, and the response
carries `Set-Cookie: lanyard_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0`
— the record is what matters, and expiring the cookie is what makes the logout
readable in `curl -i` and in a network tab.

**With no `post_logout_redirect_uri`, lanyard renders its own `200` page** saying
the browser is signed out of lanyard, naming the application from the
`id_token_hint` when there was one, and linking to `/_/`. An RP that sends no
return address has to land somewhere, and landing on a blank page or a bare
`302` to `/` would be lanyard being unhelpful at exactly the moment a developer
is watching.

**Logging out never fails.** No session, an unknown session, a session from
before a restart: all of them are the same `302` or the same page. There is no
state in which a logout request produces an error the user has to read, except
the one rejection.

### Logout clears the whole browser session, not one `client_id`

lanyard's session holds a selection **per `client_id`** — that is the whole
multi-project property — so `/end_session` from `billing-web` could drop just
that selection or the entire record including `php-web`'s Mira.

**It drops everything.** Every real IdP has one SSO session and clears all of
it, and diverging from production is the thing this project exists not to do. It
also revokes every refresh token that session was issued, because "fully logged
out" that leaves a working refresh token in an app's local storage is not fully
logged out.

The cost is real and is the reason this is a decision on the record rather than
a side effect: **a logout in one app logs you out of the other two**, which is
exactly the three-app demo in `spikes/README.md`. Criterion 6 is that cost, made
visible on purpose.

The per-`client_id` variant still exists — as **Forget** on lanyard's own UI,
where it is a developer deliberately reaching for it, and not on the protocol
endpoint where an SDK would reach it by accident.

### The one rejection, applied to a second parameter

`post_logout_redirect_uri` goes through the same literal loopback check as
`redirect_uri`: `http` or `https`, host `localhost` / `*.localhost` /
`127.0.0.0/8` / `[::1]`, no DNS lookup, so `web.localtest.me` is rejected though
it resolves to loopback. Same rule, same wording, same function.

And the same rendering: a `400 text/html` page **at lanyard**, no `Location`
header, naming the offending value and stating the rule in one sentence. RFC
6749 §4.1.2.1's reasoning transfers exactly — redirecting an error to an address
you have just decided not to trust is what would make the boundary decorative.

### Refresh tokens: `offline_access` gates them, exactly as `openid` gates the ID token

A `refresh_token` comes back from the `authorization_code` grant **only when the
authorization request's scope contained `offline_access`**. Without it the token
response is what Phase 4 shipped.

This is the precedent Phase 4 set for `openid`: a request parameter having a
documented effect is not registration, and "a request without it gets a plain
OAuth 2.0 response — which is honest, and is a thing worth being able to test."
Auth0, Okta and Entra ID all require `offline_access`; an app that forgets it in
production gets no refresh token and breaks, and a lanyard that hands one over
unasked hides exactly that bug. Same argument that put scope filtering on the ID
token in Phase 4.

`offline_access` is **not** a claim filter and adds nothing to any token. It
appears in `scopes_supported` and in the granted `scope` echoed back.

**Never from `client_credentials`.** RFC 6749 §4.4.3 says a client credentials
response MUST NOT include a refresh token, and the reason is good: there is no
user, so there is nothing a refresh could be on behalf of. The client has its own
credentials and can ask again.

### The refresh token is opaque, and it rotates

An opaque, unguessable id into an in-memory record holding the persona, the
`client_id`, the granted scope, the audience, the `auth_time`, the original
`nonce`, and the browser session it belongs to. **Not a JWT** — a refresh token
is presented to exactly one endpoint, which has the record, and an app that
"validates" one has a bug that a JWT-shaped refresh token would hide.

**It rotates: every successful refresh returns a new one and invalidates the
one presented.** That is what Auth0, Okta and Entra do for public clients, and
it is the strictly more demanding shape — an app that handles rotation handles a
static token too, and the reverse is the bug you want to find locally. A
replayed refresh token gets `400 invalid_grant` with a description that says it
was already exchanged, which the `Expiring` store's existing `Spent` tombstone
already distinguishes from an unknown one.

**Eight hours**, which is a working day, and the process is the real bound
anyway. It is the one lifetime in this project that is not measured in seconds,
and the reason is that a refresh path you cannot exercise across a lunch break is
a refresh path nobody exercises. CONCEPT §6's warning is about *access* tokens
being long-lived so the refresh path never runs; this is the opposite lever.

### The refresh grant is a third caller of `issue()`

`grant_type=refresh_token`, `refresh_token`, optional `scope`. Client
authentication accepted in any form, checked in none.

- A new access token, minted by `issue()` from the stored persona with the stored
  audience and scope. 60 seconds, as always.
- A new refresh token, replacing the presented one.
- **A new ID token when the original grant included `openid`** — OIDC Core §12.2.
  Same `sub`, same `auth_time` as the original authentication, the **original
  `nonce`** (§12.2 requires it), a fresh `at_hash` over the new access token, and
  **no `c_hash`**, because there is no code this time.
- `scope` may **narrow** the grant and may not widen it. A scope that was not
  originally granted is `400 invalid_scope` naming the offending value. RFC 6749
  §6, and it is the same class of check as PKCE: cryptographically free, catches
  a real bug, and has nothing to do with registration.

Distinct `invalid_grant` descriptions, for the same reason Phase 4 wrote six of
them: unknown, expired, already exchanged (rotated), and revoked are four
different things a developer needs told apart, and Phase 6's log can only report
what this phase distinguishes.

### `/oidc/introspect`

`POST`, form-encoded, `token` required and `token_type_hint` read as a hint.
Always `200 application/json`.

For an access token: verified as lanyard's own — signature, `iss`, `exp` — and
checked against the revocation set. For a refresh token: looked up. For an ID
token: it is one of ours and introspects like an access token, which is honest.

```json
{ "active": true, "token_type": "Bearer", "scope": "openid email offline_access",
  "client_id": "billing-web", "sub": "ada", "aud": "billing-api",
  "iss": "http://127.0.0.1:9500/oidc", "exp": 1730000060, "iat": 1730000000,
  "jti": "…" }
```

**Anything else is exactly `{"active": false}` and nothing more** — RFC 7662
§2.2 is explicit that an inactive response reveals no other fields. A token
signed by another issuer, a string that is not a JWT, an empty `token`, a
revoked token and an expired token all produce the same two words. Not an error:
"I do not recognise this" *is* the answer this endpoint exists to give.

**Client authentication is accepted in any form and checked in none**, which RFC
7662 §2.1 says MUST NOT be the case. This is a deliberate divergence, it is the
same divergence `/oidc/token` already ships, and the mitigation is the same one:
loopback by default, and a README that says so in the section that already
explains that anyone who can reach the port can mint anything. An introspection
endpoint that demanded a credential lanyard does not check would be theatre with
extra steps.

### `/oidc/revoke`

`POST`, `token` required, `token_type_hint` read as a hint. **Always `200` with
an empty body**, including for a token that was never issued, is not a JWT, or
belongs to somebody else — RFC 7009 §2.2 requires exactly that, so a client
cannot use this endpoint to find out whether a token exists.

- A **refresh token** is dropped from the store. The next refresh with it is
  `invalid_grant` naming revocation.
- An **access token** has its `jti` recorded in a revocation set, kept until the
  moment the token would have expired anyway and then forgotten. Access tokens
  are stateless JWTs and this is the only way to make revoking one mean
  something; the set is bounded by the number of unexpired tokens, which for a
  single-developer tool is a number you can count.
- **No cascade.** Revoking a refresh token does not revoke access tokens issued
  from it, and revoking an access token does not touch the refresh token. RFC
  7009 §2.1 says the server MAY cascade; lanyard does not, because it does not
  track the linkage and inventing one to support a MAY is how a dev tool gets a
  subsystem nobody asked for.

**`/userinfo` honours revocation.** A revoked access token gets `401` with
`WWW-Authenticate: Bearer error="invalid_token"`, for the same reason Phase 4
made it reject an expired one: an app that reads `/userinfo` with a dead token
has a bug, and a mock that answers anyway hides it. `/introspect` saying
`active: false` while `/userinfo` hands over claims would be lanyard disagreeing
with itself.

### Three controls on `/_/`

The picker page grows a **"This browser"** section listing what the browser is
currently signed in as, one row per `client_id`, each row naming the persona and
when it was chosen. It appears whether or not a login is in progress, and it is
empty-stated when there is nothing to show.

| Control | Scope | What is observable afterwards |
|---|---|---|
| **Log out of lanyard** | The whole session: every selection, every refresh token issued to it | The next `/authorize` from any client shows the picker |
| **Forget** (per row) | One `client_id`'s selection | That client's next `/authorize` shows the picker; the other rows are untouched |
| **Expire now** (per row) | Revokes every unexpired access and refresh token this session holds for that `client_id`, and **keeps the selection** | The app's next API call gets `401` and its refresh fails — so the app's own renew path runs, rather than the picker appearing |

**Expire now** is CONCEPT §6's control, and keeping the selection is the whole
point of it: the question it answers is "what does my application do when its
token dies", not "what does the picker look like". Making the tokens dead
without making the person forgotten is the only way to ask that question without
waiting.

All three are `<form method="post">` to `/_/` endpoints. No JavaScript, same as
everything else on that page, and `SameSite=Lax` means a cross-site POST arrives
without the cookie and therefore acts on no session — the same reasoning Phase 4
recorded for `/_/pick`.

Making **Expire now** possible means the browser flow records what it issued:
per session, per `client_id`, the `jti` and `exp` of each access token and the id
of each refresh token. Bounded by the same expiry as everything else, and in
memory like everything else.

### Discovery, and one rule cutting the other way

```
end_session_endpoint, introspection_endpoint, revocation_endpoint
grant_types_supported  += "refresh_token"
scopes_supported       += "offline_access"
```

Phase 1's rule is "advertise only what exists", and this is the phase where it
cuts the other way: **the endpoint has to ship *and* be advertised together, or
the .NET one-liner is silently a no-op** — `SignOutAsync` reads the URL from the
document and simply does not build a redirect when it is absent. An endpoint that
exists and is not advertised fails exactly as confusingly as one advertised and
missing.

`introspection_endpoint_auth_methods_supported` and
`revocation_endpoint_auth_methods_supported` list the same three values
`token_endpoint_auth_methods_supported` does, and for the same reason: all three
are true, because none of them are checked.

### Why deleting `lanyard_session` by hand still does nothing

It is the first row of the table at the top of this spec, it surprises people,
and after this phase it is still true: the RP holds its own cookie, so the
browser never asks lanyard anything, so nothing lanyard forgot can matter. That
is not a lanyard bug — it is what an SSO session *is*, and an app whose logout
does not go through the provider behaves the same way against Okta.

The README says this in the logout section, in those terms, so a developer who
tries the obvious thing first finds the explanation where they are already
looking.

### One page, three stacks

`spikes/shared/page.html` — one file: the shell, the CSS, a signed-in and a
signed-out state, and substitution points for the title, the state line, and the
claim rows. `dotnet-web` and `php-web` read it at runtime (`File.ReadAllText`,
`file_get_contents`) and `node-spa` fetches it and fills it client-side. Nothing
is copied, generated, or built.

Today `dotnet-web` renders `text/plain` from a `Results.Text` and `php-web`
echoes three lines of preformatted text, so comparing what two stacks actually
did means reading past two different presentations of it. **That is the whole
reason to make them identical**: when the page is a constant, every visible
difference between .NET and PHP is a difference in the stack, which is the
question these apps exist to answer.

The claim view becomes the same table in all three: one row per claim, `name`
then `value`, in the order the stack produced them. The orders will differ, and
*that difference is a finding* rather than a defect in the page.

`dotnet-web` keeps a second, clearly-labelled **"Clear this app's cookie only"**
link alongside the real **Log out**. It is what makes criterion 6 observable, it
is what the `spikes/README.md` walkthrough uses to show per-`client_id` memory,
and it is the row of the table that used to be the only thing on offer.

## Acceptance criteria

Every run sets `LANYARD_DATA_DIR` and a throwaway `XDG_CONFIG_HOME`, as in Phases
2–4, and uses the real port 9500. Browser criteria are driven in a real browser
and leave a screenshot in `docs/decisions/evidence/` named `phase05-*`. `jose`
verification means `scripts/jose-verify.mjs` against the live JWKS URL.

Harnesses, unchanged from Phase 4: `spikes/dotnet-web/` on `http://localhost:5000`
as `billing-web`, `spikes/php-web/` on `http://localhost:5001` as `spike-php`,
`spikes/node-spa/` on `http://localhost:5173` as `node-spa`, `spikes/dotnet-api/`
on `http://localhost:5080`.

**One click, both sessions, three stacks**

- [x] 1. `spikes/dotnet-web`: signed in as Ada, click **Log out**. The browser
      goes `localhost:5000` → `127.0.0.1:9500/oidc/end_session` → back to
      `localhost:5000`, with no cookie deleted by hand at any point. Clicking
      **Log in with lanyard** then shows **the picker**, not a silent re-login as
      Ada. Screenshots of the network tab showing the chain and of the picker.
- [x] 2. The same for `spikes/php-web` via `$oidc->signOut(...)`, ending on the
      picker.
- [x] 3. The same for `spikes/node-spa` via `mgr.signoutRedirect()`, landing back
      on `localhost:5173` signed out, and a subsequent **Sign in** showing the
      picker.
- [x] 4. `end_session_endpoint` is in the discovery document and fetches;
      `grep -ri end_session spikes/dotnet-web/Program.cs spikes/php-web/index.php
      spikes/node-spa/index.html` finds **no URL** — every SDK built the redirect
      from the document.

**The one rejection, extended**

- [x] 5. `GET /oidc/end_session?post_logout_redirect_uri=https%3A%2F%2Fevil.example.com%2F`
      → `400`, `Content-Type: text/html`, **no `Location` header**, the browser
      still on `127.0.0.1:9500`, the page naming the value and stating the
      loopback rule. Same for `http://web.localtest.me:5000/`, which resolves to
      loopback and is rejected anyway. **And the session survives**: a rejected
      logout logs nobody out.

**Logout clears everything, and that is visible**

- [x] 6. Log in to `dotnet-web` as **Ada** and `php-web` as **Mira**, in one
      browser profile. Click **Log out** in `php-web` only. Then in `dotnet-web`
      click **"Clear this app's cookie only"** and visit `/secure`: **the picker
      appears**. Before this phase that same sequence signed you straight back in
      as Ada. Screenshot.
- [x] 7. `/oidc/end_session` with **no** `post_logout_redirect_uri` → `200
      text/html` at lanyard saying the browser is signed out, and
      `curl -i` shows `Set-Cookie: lanyard_session=…Max-Age=0`.
- [x] 8. `state=a b/c&d=e%2F` sent url-encoded is echoed byte-for-byte on the
      post-logout redirect; a request with no `state` produces a redirect with no
      `state` parameter at all.
- [x] 9. `/oidc/end_session` with **no** `id_token_hint`, with an expired one
      (`lanyard token --as ada --expired`), and with `id_token_hint=garbage` all
      log out identically. No confirmation screen is rendered in any of the
      three, and no response is an error.

**Refresh**

- [x] 10. A code flow with `scope=openid email offline_access` returns a
      `refresh_token`; the identical flow without `offline_access` returns a body
      with **no** `refresh_token` key.
- [x] 11. `grant_type=refresh_token` returns a new access token that `jose`
      verifies against the live JWKS, plus a new `id_token` whose `sub`,
      `auth_time` and `nonce` equal the original's and whose `at_hash` matches
      the **new** access token. `expires_in` is `60`. No `c_hash`.
- [x] 12. Rotation: the refresh token from criterion 11's response works; the one
      presented to produce it → `400 invalid_grant` whose description says it was
      already exchanged.
- [x] 13. Refreshing with `scope=openid` (narrowing) succeeds and the response's
      `scope` is the narrowed one; refreshing with `scope=openid admin`
      (widening) → `400 invalid_scope` naming `admin`.
- [x] 14. `grant_type=client_credentials` with `scope=offline_access` returns
      **no** `refresh_token` — RFC 6749 §4.4.3.
- [x] 15. `spikes/node-spa` with `automaticSilentRenew` refreshes an expired
      access token **without a redirect**: the network tab shows a
      `POST /oidc/token` with `grant_type=refresh_token`, the address bar never
      leaves `localhost:5173`, no `/authorize` request is made, and no iframe
      appears. Screenshot of the network tab.

**Introspection and revocation**

- [x] 16. `/introspect` on a live access token → `active: true` with `sub`,
      `client_id`, `aud`, `scope`, `exp` and `jti`. `POST /oidc/revoke` with the
      same token → `200`, empty body. `/introspect` again → **exactly**
      `{"active": false}`. `/userinfo` with that token → `401` carrying
      `WWW-Authenticate: Bearer`.
- [x] 17. `/revoke` on a token that was never issued, on `token=not-a-jwt`, and
      on a token signed by `tests/data/other-key.pem` → all `200`, all empty.
      `/introspect` on the foreign one → `{"active": false}`, not an error.
- [x] 18. `/revoke` on a refresh token → the next `refresh_token` grant with it
      is `400 invalid_grant` whose description names revocation.
- [x] 19. After criterion 3's logout, `/introspect` on the refresh token the SPA
      held → `{"active": false}`. Logging out revokes what the session was
      issued.

**The controls on `/_/`**

- [x] 20. `/_/` lists a **This browser** section naming `billing-web` → Ada and
      `spike-php` → Mira after criterion 6's logins, and says so plainly when
      nothing is signed in. Screenshot.
- [x] 21. **Log out of lanyard** on `/_/`: afterwards both apps' next
      `/authorize` shows the picker, and the **This browser** section is empty.
- [x] 22. **Forget** on the `billing-web` row: `billing-web`'s next `/authorize`
      shows the picker and `spike-php`'s returns a code with no picker. The
      per-`client_id` variant lives here and not on `/end_session`.
- [x] 23. **Expire now** on the `billing-web` row: the access token that browser
      holds reads `{"active": false}` at `/introspect` and `401` at `/userinfo`,
      **and** a fresh `/authorize` from `billing-web` still returns a code with no
      picker — the tokens died, the person did not.

**Discovery and CORS**

- [x] 24. `curl -s .../oidc/.well-known/openid-configuration | jq` has
      `end_session_endpoint`, `introspection_endpoint` and `revocation_endpoint`
      that all answer, `grant_types_supported` containing `refresh_token`, and
      `scopes_supported` containing `offline_access`.
- [x] 25. With `spikes/node-spa` open on `http://localhost:5173`, the browser
      console is **empty of CORS errors** across a full sign-in, a refresh and a
      sign-out — including the SPA's `fetch` of `/oidc/jwks` and its
      `POST /oidc/revoke`. Screenshot of the console. (The roadmap's criterion
      names port 3000; 5173 is the port the real SPA spike runs on.)

**One page, three stacks**

- [x] 26. `curl -s http://localhost:5000/ > a; curl -s http://localhost:5001/ > b;
      diff a b` differs **only** in the application name — same markup, same CSS,
      same buttons in the same order. Signed-in screenshots of both, side by
      side, indistinguishable apart from the persona, the port and the claim
      rows.
- [x] 27. All three spikes render the claim view as the same table markup, and
      `node-spa` fetches the same `spikes/shared/page.html` rather than carrying
      its own copy.

**Unregressed**

- [x] 28. `spikes/dotnet-api/failure-tokens.sh` still prints its seven lines and
      exits `0`. Phase 4's criteria 1, 8, 18 and 23 still pass unchanged: a
      browser login, a curl-only code flow, the remembered-selection skip, and
      the byte-identical claim diff between a browser-issued and a
      CLI-issued access token.
- [x] 29. `cargo build --release` in an environment with no `node` and no `npm`
      on `PATH` still succeeds and serves the styled picker, now with three more
      buttons on it.

## Open questions

None blocking. Nine decisions in **Behavior** go beyond what the roadmap states,
listed here because they are the ones worth disagreeing with before a plan
exists:

1. **`/end_session` clears the entire browser session, not one `client_id`.**
   The roadmap flagged this as needing a decision on the record; this is it.
   Every real IdP has one SSO session and clears all of it. The cost is that the
   three-app demo in `spikes/README.md` gets shorter after any logout, which is
   why criterion 6 makes it visible rather than letting somebody discover it.
2. **Logout also revokes the session's refresh tokens.** A "full logout" that
   leaves a working refresh token in an app's local storage is not one.
3. **`offline_access` is required for a refresh token**, matching Auth0, Okta and
   Entra, and matching Phase 4's precedent that `openid` gates the ID token. The
   counter-argument is Keycloak, which issues refresh tokens without it — and a
   developer calibrated to Keycloak will read lanyard as broken until they read
   the README.
4. **Refresh tokens rotate.** An app that only works with a static refresh token
   will break against lanyard and work against a non-rotating IdP, which is a
   false positive; the trade is accepted because the reverse failure — shipping
   an app that cannot handle rotation — is the one that costs a production
   incident.
5. **Refresh tokens live eight hours**, the only lifetime in this project not
   measured in seconds.
6. **`/introspect` and `/revoke` do not authenticate the client**, which RFC 7662
   §2.1 says they MUST. Consistent with `/oidc/token`, mitigated by loopback,
   and stated in the README rather than left to be discovered.
7. **`/introspect` returns exactly `{"active": false}` for everything it does not
   recognise** — expired, revoked, foreign, and malformed are one answer, per RFC
   7662 §2.2. Diagnosing *which* is Phase 6's log, not this endpoint's job.
8. **Revocation does not cascade** between an access token and the refresh token
   it came from, in either direction.
9. **`spikes/shared/page.html` is read at runtime rather than copied or
   templated at build time**, so there is exactly one file and no generation
   step — and so a stack that drifts from it cannot drift silently.

One thing is noted rather than decided: whether `/authorize` should eventually
accept `flaw=`, and whether a flawed ID token is worth the seventh flaw Phase 3
deferred. An `at_hash` that does not match is now producible in three places
rather than one, which makes it more attractive and no more urgent.
