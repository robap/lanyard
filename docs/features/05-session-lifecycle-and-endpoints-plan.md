# Session lifecycle and the remaining endpoints — plan

**Status:** done · **Spec:** [05-session-lifecycle-and-endpoints-spec.md](05-session-lifecycle-and-endpoints-spec.md) · **Roadmap:** Phase 5

## Approach

Four endpoints, one new store, three buttons, and one shared HTML file. Nothing
here is a new subsystem: `/oidc/end_session` is `authorize.rs`'s loopback check
and rendered-`400` machinery pointed at a second parameter; `refresh_token` is a
**third arm** on the `grant_type` match and a **third caller** of `issue()`;
`/introspect` and `/revoke` are `jws::verify` plus one set of `jti`s. The whole
phase adds one dependency-free module of storage and touches `issue()` not at
all.

Five shapes are settled deliberately:

- **The refresh token is opaque and lives in `Expiring`, which already does
  everything it needs.** Server-generated unguessable id, TTL sweep on insert,
  and a `Spent` tombstone that is *exactly* rotation's error case — a replayed
  refresh token gets "already exchanged" rather than "unknown" for free. Rotation
  is `take()` then `insert()`. **North star 3**: the record holds a whole
  `Persona`, so the refresh grant reaches `issue()` with the same arguments the
  code grant did, and criterion 11 is what fails if it does not.
- **Revocation of an access token is a set of `jti`s, not a token store.** Access
  tokens stay stateless JWTs — Phase 2's contract is untouched — and the set is
  bounded by unexpired tokens because each entry is forgotten at the moment its
  token would have expired anyway. `/introspect` and `/userinfo` both consult it,
  so lanyard cannot disagree with itself about whether a token is alive.
- **Refresh tokens carry their `session_id`, and the session record carries the
  access tokens' `jti`s.** That is the minimum bookkeeping that makes **Log out**
  revoke what a session holds and **Expire now** revoke what one `client_id`
  holds. Revoking refresh tokens is a `retain` over the refresh store keyed on
  `session_id`, which cannot go stale under rotation the way a list of ids would.
- **`end_session` reuses `redirect_uri::check` verbatim.** One rejection, one
  function, one sentence of prose — extending the boundary to a second parameter
  must not mean a second implementation of it (**north star 1**).
- **`spikes/shared/page.html` is read at runtime, not copied or generated.** One
  file, no build step, and a stack that drifts from it cannot drift silently
  (spec, open question 9). It is Phase 11's down payment, taken here because this
  phase has to edit all three pages anyway.

The discovery document grows in the same commit as the endpoints it names,
because this is the phase where "advertise only what exists" cuts the other way:
.NET reads `end_session_endpoint` out of the document, and without it
`SignOutAsync` builds no redirect and fails silently.

## Files

- `src/store.rs` — `RefreshRecord` + `RefreshTokens` (8 h, rotating);
  `Revoked` (a `jti` → deadline set, swept on insert); `Session` grows
  `issued: Vec<Issuance>`; `Sessions` grows `remove`, `forget`, `record_issuance`,
  `issuances_for`; `Expiring` grows `retain` and `insert_at`
- `src/oidc/revocation.rs` — **new.** Resolve a presented token to
  `Presented::{Access, Refresh, Unrecognized}`, and the two revoke paths. Shared
  by `/introspect` and `/revoke` so they cannot answer differently
- `src/oidc/introspect.rs` — **new.** `POST /oidc/introspect`, always `200`,
  exactly `{"active": false}` for everything unrecognized
- `src/oidc/revoke.rs` — **new.** `POST /oidc/revoke`, always `200` and empty
- `src/oidc/end_session.rs` — **new.** `GET`/`POST`, the loopback check on
  `post_logout_redirect_uri`, the rendered `400`, the session drop, the
  `Max-Age=0` cookie, the redirect or the signed-out page
- `src/oidc/token.rs` — the **third arm**: `grant_type=refresh_token`. The code
  arm gains a `refresh_token` in the response when `offline_access` was granted,
  and both arms record what they issued against the session
- `src/oidc/code.rs` — `CodeRecord` gains `session_id: Option<String>`
- `src/oidc/authorize.rs` — `complete()` takes the `session_id` so the code
  record can carry it
- `src/oidc/scope.rs` — `Scopes::covers(&Scopes)` for the narrowing check, and
  `Scopes::to_raw()` for the echoed `scope`
- `src/oidc/userinfo.rs` — one revocation check after `jws::verify`
- `src/oidc/routes.rs` — mount three endpoints; discovery gains
  `end_session_endpoint`, `introspection_endpoint`, `revocation_endpoint`,
  `refresh_token`, `offline_access`, and the two auth-methods lists
- `src/config.rs` — `end_session_endpoint()`, `introspection_endpoint()`,
  `revocation_endpoint()`
- `src/session.rs` — `clear_cookie()` next to `set_cookie()`
- `src/ui/mod.rs` — the **This browser** panel and `POST /_/logout`,
  `/_/forget`, `/_/expire`
- `src/ui/lanyard.css` — the panel's rows and the destructive-button variant
- `tests/http.rs` — the must-be-absent discovery assertion inverts; new coverage
  for introspect/revoke/refresh/end_session
- `tests/browser_flow.rs` — logout, rotation, and the per-`client_id` controls
- `spikes/shared/page.html` — **new.** The one page shell, with substitution
  points for the title, the state line, the buttons and the claim rows
- `spikes/dotnet-web/Program.cs` — renders the shared page; **Log out** through
  `SignOutAsync` over both schemes; a second **Clear this app's cookie only** link
- `spikes/php-web/index.php` — renders the shared page; **Log out** through
  `$oidc->signOut()`
- `spikes/node-spa/index.html` — fetches the shared page; **Sign out** through
  `mgr.signoutRedirect()`; `offline_access` and `automaticSilentRenew`
- `spikes/README.md` — the "there is no full log-out yet" note comes out
- `README.md` — the three endpoints, refresh tokens, the logout section, and why
  deleting `lanyard_session` by hand does nothing

## Risks & unknowns

- **`end_session_endpoint` must land in discovery in the same commit as the
  endpoint.** .NET reads the URL from the document and silently builds no
  redirect when it is absent — the failure mode is a **Log out** button that
  appears to do nothing. If criterion 1 shows no navigation at all, check the
  document before checking the code.
- **.NET's `post_logout_redirect_uri` is its `SignedOutCallbackPath`**
  (`/signout-callback-oidc` by default), not the app root, and it only sends
  `id_token_hint` because `SaveTokens = true` is already set. Do not add
  configuration to the spike to make this work; if it needs any, that is a
  finding worth writing down rather than a line to paste.
- **`jumbojett`'s `signOut()` redirects and exits**, and it requires
  `end_session_endpoint` in the discovery document or it throws. Its
  `post_logout_redirect_uri` is whatever the second argument is — pass
  `http://localhost:5001/`, which is loopback and therefore accepted.
- **`oidc-client-ts` will not use a refresh token it was never given.** For
  criterion 15 the SPA needs `offline_access` in its scope **and**
  `automaticSilentRenew: true`; with a 60-second access token the default
  `accessTokenExpiringNotificationTimeInSeconds` (60) fires immediately or never,
  so set it to something like 10. If a renew still goes through an iframe rather
  than `POST /oidc/token`, that is Phase 9's path and the criterion has not
  passed.
- **Holding a `std::sync::Mutex` across an `.await` deadlocks the multi-thread
  runtime.** Two new stores and a `retain` over one of them do not change that
  rule: take the lock, do the work, drop it before awaiting anything.
- **`Expiring::insert` chooses its own id**, which is right for a refresh token
  and wrong for the revoked-`jti` set, whose key is the token's. `Revoked` is a
  separate type keyed by the caller; do not force one into the other.
- **A revoked `jti` deadline comes from the token's own `exp`**, which is unix
  seconds, while the stores hold `Instant`s. Convert by difference from `now` and
  saturate — a token whose `exp` is already past need not be recorded at all.
- **Rotation makes a recorded refresh-token id stale immediately.** Revoke by
  scanning the refresh store for a matching `session_id`, never by replaying a
  list of ids the session recorded.
- **Criterion 6 costs the `spikes/README.md` demo.** Logging out of one app logs
  you out of all three, on purpose (spec, open question 1). The walkthrough has
  to be re-ordered so the logout step comes last, or it will contradict itself.
- **`tests/http.rs` asserts these three endpoints are *absent* from discovery.**
  That assertion is correct until this phase and must be inverted, not deleted —
  it becomes the check that all three are present and fetch.
- **Phase 4 must not regress.** `client_credentials` still returns exactly
  `["access_token", "expires_in", "token_type"]` (criterion 14 and an existing
  test both say so), and criterion 23's byte-identical claim diff between a
  browser-issued and a CLI-issued access token still has to hold.

## Steps

Each box ≈ one small commit moving an observable behavior. Check only when the
outcome is real, not when code is written.

- [x] **The two new stores, alone** — `src/store.rs` gains `RefreshTokens`
      (`Expiring<RefreshRecord>` at 8 h, holding the `Persona`, `client_id`,
      granted `Scopes`, raw scope, audience, `auth_time`, original `nonce` and
      `session_id`) and `Revoked` (a caller-keyed `jti` → deadline set, swept on
      insert, each entry forgotten once its token would have expired anyway).
      `Expiring` gains `retain`. Unit tests: rotation reads as `Spent` not
      `Unknown`, a revoked `jti` stops being remembered after its deadline, and
      `retain` by `session_id` removes only that session's. Nothing routes to
      them yet
- [x] **`POST /oidc/introspect`** — new `src/oidc/introspect.rs` over a new
      `src/oidc/revocation.rs` resolver. A live access token → `active: true`
      with `sub`, `client_id`, `aud`, `scope`, `exp`, `iat`, `jti` and
      `token_type: "Bearer"`. Everything else — expired, foreign, malformed,
      absent, or a refresh token nobody issued — is **exactly**
      `{"active": false}` and nothing more (RFC 7662 §2.2). Client authentication
      accepted in any form and checked in none, as on `/oidc/token`. Criteria 16
      (first half) and 17 (the foreign-token half). Not yet advertised
- [x] **`POST /oidc/revoke` makes it flip** — new `src/oidc/revoke.rs`. Always
      `200` with an empty body, including for a token that was never issued or is
      not a JWT (RFC 7009 §2.2 — a client must not be able to probe existence
      here). An access token records its `jti` until its own `exp`; a refresh
      token is dropped. **No cascade** in either direction. `/userinfo` gains the
      same check and answers `401 invalid_token` for a revoked token, because
      `/introspect` saying `active: false` while `/userinfo` hands over claims
      would be lanyard disagreeing with itself. Criteria 16 (rest) and 17
- [x] **The code grant issues a refresh token when `offline_access` was granted**
      — `CodeRecord` gains `session_id`, `authorize::complete()` takes it, and
      `token.rs`'s code arm inserts a `RefreshRecord` and adds `refresh_token` to
      the response **only when the authorization request's scope contained
      `offline_access`**. Same precedent as `openid` gating the ID token: a
      request without it gets a plain OAuth 2.0 response, which is honest and is
      what Auth0, Okta and Entra do. Criterion 10, and criterion 14 —
      `client_credentials` still returns exactly three keys, RFC 6749 §4.4.3
- [x] **`refresh_token` as the third arm, and the third caller of `issue()`** —
      `grant_type=refresh_token` mints a new access token from the stored persona
      with the stored audience and scope, rotates the refresh token, and returns a
      new ID token when the original grant included `openid` — same `sub`, same
      `auth_time`, the **original `nonce`** (OIDC Core §12.2), a fresh `at_hash`
      over the new access token, and **no `c_hash`**. Four distinct
      `invalid_grant` descriptions: unknown, expired, already exchanged, revoked.
      Criteria 11, 12, 18. **North star 3** — if a refreshed token's claims get
      assembled anywhere but `issue()`, the structure was wrong
- [x] **Scope may narrow and may not widen** — `Scopes::covers`, and a `scope`
      parameter on the refresh grant that is intersected with the granted set. A
      scope that was never granted → `400 invalid_scope` naming it (RFC 6749 §6).
      Same class of check as PKCE: free, catches a real bug, nothing to do with
      registration. Criterion 13
- [x] **`GET`/`POST /oidc/end_session`** — new `src/oidc/end_session.rs`.
      `post_logout_redirect_uri` goes through `redirect_uri::check` **verbatim**;
      a failure renders the same `400 text/html` at lanyard with no `Location`,
      naming the value and the rule, **and logs nobody out**. Otherwise: drop the
      whole session record, revoke every refresh token carrying that
      `session_id`, emit `Set-Cookie: lanyard_session=; …; Max-Age=0`, and either
      `302` to the return address with `state` echoed byte-for-byte or render a
      `200` signed-out page naming the application from the `id_token_hint`.
      `id_token_hint` is **read, never required** — absent, expired, foreign and
      garbage all log out identically. **No confirmation screen, ever** (spec,
      out of scope): a prompt nobody can automate past is a prompt in the way of
      a test. Criteria 5, 7, 8, 9
- [x] **Discovery grows, and the absence test inverts** — `end_session_endpoint`,
      `introspection_endpoint`, `revocation_endpoint`,
      `grant_types_supported` += `refresh_token`, `scopes_supported` +=
      `offline_access`, and the two `*_endpoint_auth_methods_supported` lists
      carrying the same three values as the token endpoint, because all three are
      true and none are checked. `tests/http.rs`'s must-be-absent loop becomes a
      must-be-present-and-fetch loop. Criterion 24. **This is the phase where
      "advertise only what exists" cuts the other way** — .NET reads the URL out
      of the document, so the endpoint and its advertisement ship together or the
      one-liner is silently a no-op
- [x] **The This browser panel** — `/_/` lists one row per `client_id` the
      session is signed in for, naming the persona and when it was chosen, with an
      honest empty state. Rendered on both versions of the page (with a `req` and
      without), escaped like everything else on it. Criterion 20
- [x] **Log out of lanyard, Forget, Expire now** — three `<form method="post">`
      controls, no JavaScript, each redirecting back to `/_/`. **Log out** drops
      the whole session and expires the cookie; **Forget** drops one
      `client_id`'s selection and leaves the others; **Expire now** revokes that
      `client_id`'s unexpired access `jti`s and refresh tokens and **keeps the
      selection**, so the app's own renew path runs rather than the picker
      appearing — that asymmetry is the whole point of the control (CONCEPT §6).
      `SameSite=Lax` is why none of them needs a CSRF token, the same reasoning
      Phase 4 recorded for `/_/pick`. Criteria 21, 22, 23
- [x] **One page, three stacks** — new `spikes/shared/page.html`: the shell, the
      CSS, the signed-in and signed-out states, and substitution points for the
      title, the state line, the buttons and the claim rows. `dotnet-web` and
      `php-web` read it at runtime; `node-spa` fetches it and fills it
      client-side. The claim view becomes the same table in all three, one row per
      claim. Criteria 26 and 27. When the page is a constant, every visible
      difference between two stacks is a difference in the stack — which is the
      question these apps exist to answer
- [x] **.NET logs out** — `spikes/dotnet-web` gains **Log out** as
      `SignOutAsync` over the cookie **and** OIDC schemes, with no lanyard URL
      anywhere in the file, plus a second clearly-labelled **Clear this app's
      cookie only** link. Criterion 1 (the redirect chain, then the picker),
      criterion 4 (`grep` finds no `end_session` in any spike), criterion 6 (log
      out of `php-web`, clear .NET's own cookie, and the picker appears where
      Ada used to be returned silently). Screenshots into
      `docs/decisions/evidence/`
- [x] **PHP logs out** — `spikes/php-web` gains **Log out** as
      `$oidc->signOut($idToken, 'http://localhost:5001/')`, which needs the ID
      token kept in its session. Criterion 2, with a screenshot
- [x] **The SPA logs out and refreshes without a redirect** — `spikes/node-spa`
      gains `offline_access`, `automaticSilentRenew`, a lowered
      `accessTokenExpiringNotificationTimeInSeconds`, and **Sign out** as
      `mgr.signoutRedirect()`. Criterion 3, criterion 15 (a `POST /oidc/token`
      with `grant_type=refresh_token`, no `/authorize`, no iframe, the address bar
      never leaving `localhost:5173`), criterion 19 (its refresh token reads
      `{"active": false}` after the logout) and criterion 25 (a console with no
      CORS errors across sign-in, refresh and sign-out). Screenshots of the
      network tab and the console
- [x] **Unregressed** — criterion 28: `spikes/dotnet-api/failure-tokens.sh` still
      prints seven lines and exits `0`, and Phase 4's criteria 1, 8, 18 and 23
      still pass — a browser login, the curl-only code flow, the
      remembered-selection skip, and the byte-identical claim diff between a
      browser-issued and a CLI-issued access token. Criterion 29: `cargo build
      --release` with no `node` and no `npm` on `PATH`. Run this **before** the
      docs box, not after
- [x] **Docs** — `README.md` gains a **Logging out** section carrying the
      two-sessions table, the redirect chain, the decision that logout clears the
      whole browser session, and **why deleting `lanyard_session` by hand still
      does nothing visible** — in the place a developer who tried the obvious
      thing first will already be looking. Plus: refresh tokens and the
      `offline_access` gate, rotation, the eight-hour lifetime,
      `/oidc/introspect` and `/oidc/revoke` including the unauthenticated-client
      divergence from RFC 7662 §2.1, the three controls on `/_/`, and the updated
      **Scope** paragraph. `spikes/README.md` loses its "there is no full log-out
      yet" note and re-orders the walkthrough so the logout step comes last

## Acceptance

Mirrors the spec. Every box passes by driving the named client, in a real
browser where the spec says so, leaving a screenshot in
`docs/decisions/evidence/` named `phase05-*`.

- [x] 1. `spikes/dotnet-web` **Log out** → app → `/oidc/end_session` → app, no
      cookie deleted by hand, and the next login shows **the picker**
- [x] 2. `spikes/php-web` `$oidc->signOut(...)` → the same, ending on the picker
- [x] 3. `spikes/node-spa` `mgr.signoutRedirect()` → back on `localhost:5173`
      signed out; **Sign in** shows the picker
- [x] 4. `end_session_endpoint` is in discovery and fetches; `grep -ri end_session`
      over the three spikes finds **no URL**
- [x] 5. `post_logout_redirect_uri=https://evil.example.com/` → `400 text/html`,
      **no `Location`**, naming the value and the rule; same for
      `http://web.localtest.me:5000/`; and the session survives
- [x] 6. Ada in `dotnet-web`, Mira in `php-web`; **Log out** in `php-web`; then
      **Clear this app's cookie only** in `dotnet-web` and `/secure` → **the
      picker**
- [x] 7. `/oidc/end_session` with no `post_logout_redirect_uri` → `200 text/html`
      at lanyard, and `curl -i` shows `lanyard_session=…Max-Age=0`
- [x] 8. `state=a b/c&d=e%2F` echoed byte-for-byte; no `state` sent → no `state`
      parameter at all
- [x] 9. No `id_token_hint`, an expired one, and `garbage` all log out
      identically, with no confirmation screen and no error
- [x] 10. `offline_access` → a `refresh_token`; without it → no `refresh_token`
      key at all
- [x] 11. The refresh grant returns an access token `jose` verifies and an
      `id_token` whose `sub`, `auth_time` and `nonce` match the original and
      whose `at_hash` matches the **new** access token; `expires_in: 60`; no
      `c_hash`
- [x] 12. Rotation: the new refresh token works, the presented one →
      `400 invalid_grant` saying it was already exchanged
- [x] 13. Narrowing succeeds and echoes the narrowed `scope`; widening →
      `400 invalid_scope` naming the offending scope
- [x] 14. `client_credentials` with `scope=offline_access` → **no**
      `refresh_token`
- [x] 15. `spikes/node-spa` refreshes without a redirect: `POST /oidc/token` with
      `grant_type=refresh_token`, no `/authorize`, no iframe
- [x] 16. `/introspect` `active: true` → `/revoke` → **exactly**
      `{"active": false}` → `/userinfo` `401`
- [x] 17. `/revoke` on an unissued token, `not-a-jwt`, and one signed by
      `tests/data/other-key.pem` → all `200` and empty; `/introspect` on the
      foreign one → `{"active": false}`
- [x] 18. `/revoke` on a refresh token → the next refresh is `400 invalid_grant`
      naming revocation
- [x] 19. After criterion 3's logout, the SPA's refresh token → `{"active": false}`
- [x] 20. `/_/` lists **This browser** with `billing-web` → Ada and `spike-php` →
      Mira, and an honest empty state
- [x] 21. **Log out of lanyard** → both apps' next `/authorize` shows the picker
      and the panel is empty
- [x] 22. **Forget** on `billing-web` → its next `/authorize` shows the picker,
      `spike-php`'s returns a code
- [x] 23. **Expire now** on `billing-web` → its token is `{"active": false}` and
      `401` at `/userinfo`, **and** a fresh `/authorize` still returns a code with
      no picker
- [x] 24. Discovery has all three endpoints answering, `refresh_token` in
      `grant_types_supported`, `offline_access` in `scopes_supported`
- [x] 25. `spikes/node-spa`'s console is empty of CORS errors across sign-in,
      refresh and sign-out
- [x] 26. `curl` of `localhost:5000/` and `localhost:5001/` differs **only** in
      the application name; signed-in screenshots side by side are
      indistinguishable apart from persona, port and claim rows
- [x] 27. All three spikes render the claim view as the same table markup, and
      `node-spa` fetches `spikes/shared/page.html` rather than carrying a copy
- [x] 28. `failure-tokens.sh` prints seven lines and exits `0`; Phase 4's
      criteria 1, 8, 18 and 23 still pass
- [x] 29. `cargo build --release` with no `node` and no `npm` on `PATH` serves the
      styled picker, now with three more buttons on it

## Progress notes

- **`Scopes::covers` became `Scopes::missing_from`.** The plan named a predicate;
  the refusal has to *name the offending scope* (criterion 13), and a `bool`
  cannot. `missing_from` returns the first ungranted scope, so the check and the
  `error_description` read the same value.
- **The refresh arm peeks before it takes.** The plan had `take()` then
  `insert()`. Taking first means a `400 invalid_scope` has already spent the
  token the caller still holds, so the arm now peeks, finishes every refusal, and
  takes only at the moment of rotation. A rejected narrowing costs nothing.
- **Revoked refresh tokens are recorded in `Revoked` as well as dropped.**
  Criterion 18 needs "revoked" told apart from "unknown", and `Expiring`'s `Spent`
  tombstone already means "already exchanged". So `Revoked` holds refresh-token
  ids alongside access `jti`s — one set, two kinds of id, and the refresh arm
  consults it before the store.
- **`redirect_uri::RULE` became `redirect_uri::rule(parameter)` and `check` took a
  parameter name.** Pointing the one rejection at `post_logout_redirect_uri` and
  having it say "redirect_uri" would have been a rule the reader must translate.
  Still one function, told which parameter it is guarding.
- **The rendered `400` moved to `ui::html::rejected_page`.** Two callers, one
  page: `/authorize` and `/end_session` differ only in the closing sentence they
  pass in.
- **The ID token is assembled by one helper on both grants** (`Grant` + `id_token`
  in `token.rs`), with `c_hash` present only when there is a code. The plan left
  the code arm's assembly in place; two copies of the same claim set is the drift
  north star 3 exists to prevent.
- **`php-web` gained "Clear this app's cookie only" too.** The plan gave it to
  `dotnet-web` alone, but criterion 26 asks that the two signed-in pages be
  indistinguishable apart from persona, port and claim rows — an extra button on
  one of them is a visible difference. It is meaningful for PHP as well: it has
  its own session cookie.
- **The two apps' routes were unified** (`/`, `/secure`, `/logout`,
  `/logout-local`) so the shared page's `href`s are constants rather than a
  substitution point. `node-spa` intercepts the same three links.
  `spikes/node-spa/shared` is a symlink to `../shared`, so the SPA fetches the one
  file rather than a copy.
- **The network-tab evidence is a captured request log, not a screenshot.** This
  machine has no X screenshot tool and Playwright cannot photograph browser
  chrome, so criteria 1, 2 and 15 leave `phase05-*-chain.txt` /
  `phase05-8-spa-renew-requests.txt` — every request the browser actually made, in
  order. The page screenshots are real. Browser criteria were driven with headless
  Chromium (the only build available here), not a headed window.
