# Browser flow and the persona picker — plan

**Status:** done · **Spec:** [04-browser-flow-and-persona-picker-spec.md](04-browser-flow-and-persona-picker-spec.md) · **Roadmap:** Phase 4

## Approach

`/oidc/authorize` validates and **stores**; `/_/` renders and **chooses**;
`/oidc/token` **exchanges**. Three handlers over three in-memory stores (pending
requests, authorization codes, sessions), all living in one `src/store.rs` behind
a `Mutex` on `AppState`, all dying with the process. The `/oidc` ↔ `/_/` line
CONCEPT §3 drew stays intact because the picker is reached by a redirect rather
than rendered at the protocol endpoint (spec, open question 8).

Four shapes are settled deliberately:

- **`issue()` grows exactly one parameter, not a second code path.** The ID
  token's registered claims — `aud`, `auth_time`, `nonce`, `at_hash`, `c_hash` —
  go in through the `overrides` map, which Phase 1 already documents as the
  channel that wins over everything. The only thing overrides *cannot* do is
  remove a claim, so scope filtering is the one genuinely new input:
  `claims_at` takes a `ClaimFilter`, applied to **step 1 only** (the persona
  table), never to the registered claims or the overrides. Access-token callers
  pass `ClaimFilter::Unfiltered` and are byte-identical to Phase 3.
  **North star 3** — criterion 23 is what fails if an ID token's claims get
  assembled anywhere else.
- **The ID token is a second call to `issue()`, made after the access token**,
  because `at_hash` is a hash *of* the access token and `c_hash` is a hash of the
  code. Two calls, one function, one persona→claims table.
- **`/_/pick` never mints anything.** It records the selection and stores a code
  bound to an owned `Persona` — cloned from the list, or built from the "mint one
  now" form. The exchange is the only place `issue()` runs, so a one-off identity
  and a loaded persona travel the identical path (spec, open question 10).
- **The loopback check uses the `url` crate.** It is already in the tree via
  `reqwest`, so it is free, and its IDNA normalization means a Cyrillic
  `lоcalhost` punycodes to `xn--…` and fails the literal comparison. This is the
  project's **single security boundary** (north star 1); hand-rolling a URL
  parser for it would be the one place a parser bug costs something.

`GET /_/` is the honest destination the banner's new `UI →` line points at, which
is why Phase 1 deferred that line.

## Files

- `Cargo.toml` — `url = "2"` as a direct dependency (already in the lock via `reqwest`)
- `src/oidc/redirect_uri.rs` — **new.** The one rejection: parse, then a literal
  host check against `localhost` / `*.localhost` / `127.0.0.0/8` / `[::1]`. No DNS
- `src/oidc/scope.rs` — **new.** `Scopes::parse`, `ClaimFilter::{Unfiltered, ByScope}`
- `src/store.rs` — **new.** `PendingRequests` (5 min), `Codes` (60 s, single-use),
  `Sessions` (per-`client_id` selections + `always_ask`), each sweeping expired
  entries on insert
- `src/session.rs` — **new.** Read the `lanyard_session` cookie from a `Cookie`
  header, emit the `Set-Cookie` with **no `Secure` and no `Max-Age`**
- `src/oidc/authorize.rs` — **new.** `GET`/`POST /oidc/authorize`: the rendered
  `400`, the redirected errors, the pending record, the remembered-selection skip
- `src/oidc/code.rs` — **new.** The code record, PKCE `S256`/`plain` verification,
  and the five distinct `invalid_grant` descriptions
- `src/oidc/userinfo.rs` — **new.** `GET`/`POST`, bearer-authenticated, scope-filtered
- `src/oidc/jws.rs` — `verify(key, token)` alongside `sign`, for `/userinfo`
- `src/oidc/issue.rs` — `claims_at` takes a `ClaimFilter`; `persona_claims()`
  extracted so `/userinfo` reads the same table
- `src/oidc/token.rs` — `authorization_code` as the **second arm** of the existing
  `grant_type` match, exactly as its module comment promised
- `src/oidc/routes.rs` — mount `authorize` and `userinfo`; discovery grows five
  keys and `response_types_supported` is corrected to `["code"]`
- `src/ui/mod.rs` — **new.** `GET /_/`, `POST /_/pick`, `POST /_/session`
- `src/ui/html.rs` — **new.** The escaping helper. `src/ui/lanyard.css` via `include_str!`
- `src/ui/form_post.rs` — **new.** The auto-submitting `form_post` page
- `src/app.rs` — `AppState` gains the three stores; router gains nothing new at
  the top level (both nests already exist)
- `src/banner.rs` — the `UI →` line
- `src/lib.rs` — declare `session`, `store`, `ui`
- `tests/http.rs` — the discovery test **changes**: `authorization_endpoint` and
  `userinfo_endpoint` move from the must-be-absent list to the must-be-present one
- `tests/authorize.rs`, `tests/browser_flow.rs` — **new.** The end-to-end code
  flow, PKCE, code hygiene, sessions and the cookie attributes
- `spikes/dotnet-web/` — repointed at lanyard on `:5000`, `client_id` `billing-web`
- `spikes/php-web/index.php` — repointed at lanyard on `:5001`
- `spikes/node-spa/` — **new.** Static page, vendored `oidc-client-ts`, no CDN
- `docs/decisions/evidence/` — the phase's screenshots
- `README.md` — the Scope paragraph, `/authorize`, `/userinfo`, the `/_/` UI, the
  loopback rule, the scope→claims table, and the two .NET gotchas

## Risks & unknowns

- **`nest("/_")` may not answer `GET /_/` the way criterion 25 reads it.** axum
  nests are historically fussy about the trailing slash on the nest root. The
  banner prints that exact URL, so verify with `curl -i` before checking the box;
  if `/_/` and `/_` disagree, route both rather than moving the URL.
- **`state` byte-for-byte is about the *decoded* value.** `serde_urlencoded`
  encodes a space as `+`, not `%20`; both decode to a space and every RP
  form-decodes. Assert criterion 9 on the decoded value, not on the raw query
  string, or the criterion fails for a reason that is not a bug.
- **`prompt=none` on the .NET/SPA path is not this phase's evidence.** Spec, out
  of scope: `SameSite=Lax` is not sent on a third-party iframe navigation, so a
  real silent renew gets `login_required`. Test `prompt=none` with `curl -b`
  (criterion 19) and do not chase an iframe result — that is Phase 9's.
- **`response_mode=form_post` may never arrive from .NET.** Spec, open question 7:
  Phase 0 saw .NET name it while defaulting to the implicit flow. With
  `ResponseType = "code"` it may send `query`. Implement it anyway (20 lines), but
  if the acceptance run shows `query`, **record the observation** rather than
  quietly claiming `form_post` was exercised by a real client.
- **The ID token will carry `nbf` and `jti`** because `claims_at` adds them to
  everything. Both are legal registered claims and stripping them means a second
  claim path. Decided, not accidental — say so in a comment where a reviewer will
  look.
- **Holding a `std::sync::Mutex` across an `.await` deadlocks under the
  multi-thread runtime.** Every store method must take the lock, do its work, and
  drop it before the handler awaits anything. Nothing in these handlers needs to
  await while holding one; keep it that way.
- **The `nobody` persona must survive scope filtering.** It has no `email` and no
  `name`, so `openid email profile` yields `sub` and the registered claims and
  nothing else. Criterion 6 says the *login* still succeeds — a filter that
  panics or 500s on an absent claim breaks the persona that exists to break
  applications.
- **Phase 3 must not regress.** `flaw` is out of scope on `/authorize` and on the
  ID token (spec, out of scope), and `failure-tokens.sh` still has to print seven
  lines and exit `0` (criterion 24). Run it before the docs box, not after.
- **`spikes/dotnet-web` currently points at `localhost:9400`** (Phase 0's
  `oidc-provider-mock`) on port `5112` with `client_id` `spike`. The spec's
  harness table says lanyard, `:5000`, `billing-web`. Repointing loses the `DROP=`
  subtraction harness's original target — keep the harness, change the values.
- **`node-spa` must not fetch `oidc-client-ts` from a CDN** (criterion 29's "no
  request to any CDN in the page source" is about lanyard's page, but criterion 2
  should not be the one place a network fetch sneaks in). Vendor the ESM bundle
  into the spike and serve it from the same static server.

## Steps

Each box ≈ one small commit moving an observable behavior. Check only when the
outcome is real, not when code is written.

- [x] **The one rejection, alone** — new `src/oidc/redirect_uri.rs`:
      `check(&str) -> Result<Url, String>` accepting `http`/`https` with a host of
      `localhost`, any `*.localhost`, anything in `127.0.0.0/8`, or `[::1]`; any
      port, path and query. Unit tests: `http://localhost:5000/cb` and
      `http://127.0.0.9:1/x?y=z` and `http://[::1]:5173/` pass;
      `https://evil.example.com/cb`, `http://web.localtest.me:5000/cb`,
      `com.example.app:/cb`, `urn:ietf:wg:oauth:2.0:oob` and a punycoded
      homograph `localhost` are rejected with a message naming the value.
      **No DNS lookup anywhere in the file.** Nothing calls it yet
- [x] **The three in-memory stores** — new `src/store.rs`: `PendingRequests`
      (unguessable id → the authorization parameters + issue time, 5 minutes),
      `Codes` (code → chosen `Persona`, `auth_time`, the request it came from;
      60 seconds; **`take` removes on read** so a second exchange finds nothing),
      `Sessions` (opaque id → per-`client_id` `{persona_id, auth_time}` plus one
      `always_ask` flag). Ids are `uuid::Uuid::new_v4`, already a dependency.
      Each insert sweeps expired entries, so nothing grows without bound and no
      background task exists. Unit tests: expiry, single-use, and that two
      `client_id`s in one session do not see each other. Hung on `AppState`
      behind `Mutex`; no route reads them yet
- [x] **Scope filtering inside the one function** — new `src/oidc/scope.rs`
      (`Scopes::parse`, `ClaimFilter::{Unfiltered, ByScope}`) and
      `issue::claims_at` grows the parameter, applied to **step 1 only**:
      `email`/`email_verified` behind `email`, `name`/`preferred_username` behind
      `profile`, `sub`/`roles`/`attributes` always. `persona_claims(persona,
      &filter)` is extracted so `/userinfo` can read the same table without
      minting. Every existing caller passes `Unfiltered`; `cargo test` is green
      with no assertion changed. **North star 3** — the filter is a parameter of
      the one function, not a second claim builder
- [x] **`/authorize` renders its rejection** — `GET`/`POST /oidc/authorize`
      mounted, parsing both query and form. A missing `client_id`, a missing
      `redirect_uri`, or one that fails the loopback check → `400`,
      `Content-Type: text/html`, **no `Location` header**, naming the offending
      value and stating the rule in one sentence. RFC 6749 §4.1.2.1: an error sent
      to an address we just refused to trust makes the boundary decorative.
      Criterion 5, both URIs, observed with `curl -i`
- [x] **`/authorize`'s redirected errors** — everything else redirects to the
      (validated) `redirect_uri` with `error`, `error_description` and the `state`
      that was sent: `unsupported_response_type`, `unsupported_response_mode`,
      `invalid_request` for a bad or half-supplied `code_challenge`,
      `request_not_supported` / `request_uri_not_supported`. The
      `unsupported_response_type` description names
      `options.ResponseType = "code"` **only when the requested type contains
      `id_token`** — Phase 0's obligation #4, and the one case where it is the
      actual diagnosis. Criterion 13, plus the no-`state` half of criterion 9
- [x] **`/authorize` stores a pending request and redirects to the picker** — the
      accepted parameters go into `PendingRequests` and the response is a `302`
      to `/_/?req=<id>`. Criterion 8's first hop, observed with `curl -i`.
      Keeping `/oidc` protocol-only is what buys the extra hop (CONCEPT §3)
- [x] **The picker renders** — new `src/ui/`: an escaping helper, `lanyard.css`
      pulled in with `include_str!`, and `GET /_/`. With `?req=` it lists every
      loaded persona as **one `<form method="post">` per person** (display name,
      id, email, roles) posting to `/_/pick`; with no `req` it lists the same
      people and says no login is in progress. `prefers-color-scheme` for dark
      mode. Criteria 25, 27 (a persona named `<script>alert(1)</script>` renders
      as `&lt;script&gt;` in `curl` output) and 29 (no `<script src>`, no CDN,
      no `node` on the build path)
- [x] **`POST /_/pick` completes a login** — records the selection in the session,
      sets the cookie, stores a code bound to the chosen persona and the pending
      request, consumes the pending record, and `302`s to the RP with `code` and
      `state`. The `Set-Cookie` is
      `lanyard_session=…; Path=/; HttpOnly; SameSite=Lax` with **no `Secure` and
      no `Max-Age`** — Phase 0's obligation #1, whose failure mode is "it asks me
      to pick a persona every single time". Criteria 17, and criterion 9's
      round-trip of `a b/c&d=e%2F` asserted on the **decoded** value
- [x] **`response_mode=form_post`** — a minimal page whose single form
      auto-submits `code` and `state` to the `redirect_uri` by `POST`, with a
      visible submit button inside `<noscript>`. The only JavaScript in the login
      path, and it degrades to one click
- [x] **`authorization_code` as the second arm** — `token.rs`'s `grant_type` match
      grows the arm its module comment promised. Access token minted through
      `issue()` with `Unfiltered`, `expires_in: 60` unchanged. `invalid_grant`
      with **five distinct descriptions**: unknown code, expired code, code
      presented twice, mismatched `redirect_uri`, missing `code_verifier`. PKCE
      `S256` and `plain` (and the omitted method, which RFC 7636 says is `plain`)
      genuinely verified, with a sixth description for a verifier that does not
      match. Criteria 11, 12, 14 (`scope` without `openid` → a `200` with **no**
      `id_token`). **This is not a contradiction of north star 1** — accept-
      everything is about registration, never about skipping cryptography
- [x] **The ID token** — a second call to `issue()` after the access token, with
      `ttl = 300`, `ClaimFilter::ByScope`, and the registered set passed as
      overrides: `aud` = the `client_id`, `auth_time`, `nonce` when one was sent,
      `at_hash` and `c_hash` as base64url of the leftmost 128 bits of `SHA-256` of
      the access token and of the code. Emitted **only when the authorization
      request's scope contained `openid`**. Criteria 10 and 15. The 300 vs 60
      asymmetry is the only lifetime in this project that is not 60 seconds, and
      it is deliberate: an ID token is a receipt, not a credential
- [x] **`/oidc/userinfo`** — `GET` or `POST`, `Authorization: Bearer`. `jws::verify`
      is added next to `sign` and checks signature, `iss` and `exp` and nothing
      else — there is no client to check. `200` with the same scope-filtered
      persona claims the ID token carried, `sub` always present, no `iss`/`aud`/
      `exp`/`nonce`/hashes. Failure is `401` with
      `WWW-Authenticate: Bearer error="invalid_token"`. Criterion 16, including
      `lanyard token --as ada --expired` → `401`. **The second place lanyard says
      no**, for PKCE's reason: a mock that answers an expired token hides a bug
- [x] **Sessions skip the picker, per `client_id`** — a second `/authorize` from a
      `client_id` with a remembered selection returns a code with no `/_/` in
      between; the same jar with a *different* `client_id` gets the picker.
      Criterion 18, and criterion 22 (restart `lanyard serve`, picker returns —
      sessions are in memory and that is a property worth having). **This is the
      first observable form of "three services, one instance, zero setup"**
- [x] **`prompt`, `max_age`, and "always ask"** — `prompt=login` and
      `select_account` force the picker; `prompt=none` **renders nothing ever** —
      a code if there is a remembered selection, `error=login_required` if not;
      `max_age` is compared against the remembered `auth_time`. The picker's
      checkbox sets `always_ask`, which forces the picker for every `client_id`,
      and the no-`req` page carries a standalone toggle (`POST /_/session`) so it
      can be turned off without starting a login. Criteria 19, 20, 21
- [x] **"Mint one now"** — a second form (`sub`, `name`, `email`, `roles`,
      and a textarea of extra claims as JSON) builds an owned `Persona` that is in
      no file, stores it against the code, and is **not remembered**: the next
      `/authorize` from that client shows the picker again. Criterion 28.
      Remembering a one-off would mean the session record growing from an id into
      a claim blob
- [x] **Discovery grows, and is corrected** — `authorization_endpoint`,
      `userinfo_endpoint`, `code_challenge_methods_supported: ["S256","plain"]`,
      `scopes_supported`, `response_modes_supported: ["query","form_post"]`,
      `authorization_code` in `grant_types_supported`, and
      **`response_types_supported` becomes exactly `["code"]`**. Phase 1's test
      moves both new endpoints out of its must-be-absent list. The banner gains
      its `UI →` line, deferred by Phase 1 on the grounds that it must not print a
      URL that 404s. Criterion 7, and the `UI →` half of criterion 25.
      Advertising `id_token` would tell a .NET app its default is supported and
      then reject it at request time — the exact fail-later failure Phase 1's rule
      exists to prevent
- [x] **Still one function, still unregressed** — criterion 23: the access token
      from a browser login as Ada with `audience=billing-api` and the token from
      `lanyard token --as ada --aud billing-api`, both through
      `jq -S 'del(.iat,.nbf,.exp,.jti,.client_id,.scope)'`, `diff` to empty. Then
      criterion 24: `spikes/dotnet-api/failure-tokens.sh` still prints seven lines
      and exits `0`, all six flaws behave as Phase 3 left them, and
      `client_credentials` still returns `expires_in: 60` and no `id_token`
- [x] **The complete flow by hand** — criterion 8 end to end with `curl -c/-b` and
      no browser: `/authorize` → `302 /_/?req=…` → `POST /_/pick` → `302` with
      `code` and `state` → `/oidc/token` → `200` with `access_token`, `id_token`,
      `token_type: "Bearer"`, `expires_in: 60`. Then criterion 10's `jose`
      verification of the `id_token` against the live JWKS, with `at_hash` and
      `c_hash` recomputed in one shell line and compared
- [x] **.NET logs in** — `spikes/dotnet-web` repointed at lanyard on `:5000` with
      `client_id` `billing-web` and `ResponseType = "code"`, keeping its `DROP=`
      subtraction harness. Criterion 1 (Ada → `email: ada@example.test`),
      criterion 6 (`nobody` → `email: (no email claim)`, and the **login itself
      succeeds**), criterion 26 (the whole of criterion 1 with JavaScript
      disabled). Screenshots into `docs/decisions/evidence/`. Record what
      `response_mode` actually arrived — the spec's open question 7 is owed an
      answer
- [x] **PHP logs in and reads `/userinfo`** — `spikes/php-web` repointed at
      lanyard on `:5001` with `client_id` `spike-php`. Criterion 3:
      `authenticate()` completes and `requestUserInfo('email')` returns
      `ada@example.test` — `/oidc/userinfo` answering a real client rather than a
      curl
- [x] **A public client with PKCE** — new `spikes/node-spa/`: a static page using
      `oidc-client-ts` (vendored, not from a CDN) on `:5173` with `client_id`
      `node-spa` and **no `client_secret` anywhere in its config**. Criterion 2:
      `signinRedirect()` with `code_challenge_method=S256` completes and
      `user.profile.email` renders. Note in the spike's README that the only
      lanyard-side setup was starting it
- [x] **The multi-project property, observed** — all three running: log in to
      `dotnet-web` as **Ada**, then `php-web` as **Mira**, same browser profile,
      that order. PHP shows Mira, .NET still shows Ada; revisit both protected
      pages and both complete **without the picker appearing**, each with its own
      persona. Criterion 4, with screenshots of both results. This is CONCEPT §3's
      central claim becoming visible for the first time
- [x] **The minimum CORS, one phase early** — new `src/oidc/cors.rs`, layered on
      the `/oidc` nest: echo the request `Origin`, allow `authorization` and
      `content-type`, answer preflight with `204`, and **never**
      `Allow-Credentials`. `/_/` and the seam answer no other origin. Added
      because acceptance criterion 2 and the spec's out-of-scope line contradict
      each other — see Progress notes
- [x] **Docs** — `README.md` gains: the login flow up top; `/oidc/authorize` and
      `/oidc/userinfo` parameter tables; the `/_/` picker and its "mint one now"
      panel; **the loopback rule stated in one line**, with `web.localtest.me`
      named as rejected-despite-resolving; the scope→claims table and why the
      access token is not filtered; the session cookie's attributes and the two
      resets (close the browser, restart lanyard); the 300 s ID token against the
      60 s access token; and a **troubleshooting section** carrying Phase 0's two
      .NET gotchas verbatim — `Correlation failed` with its real cause (the app is
      served from a non-localhost plain-HTTP origin) and .NET's `id_token` default
      for `ResponseType`. The Scope paragraph stops saying there is no browser flow

## Acceptance

Mirrors the spec's acceptance criteria. `/implement` isn't done until every box
here passes by driving the named client. Every run sets `LANYARD_DATA_DIR` and a
throwaway `XDG_CONFIG_HOME` and uses the real port 9500 so the issuer matches
what the browser is talking to. Browser criteria leave a screenshot in
`docs/decisions/evidence/`.

**Three real clients, in a real browser**

- [x] 1. `spikes/dotnet-web` completes a full login: `/secure` → lanyard → picker
      listing Ada, Mira and nobody → **Ada Bell** → back at `/secure` rendering
      `email: ada@example.test`. Screenshots of the picker and the result
- [x] 2. `spikes/node-spa` (`oidc-client-ts`, public client, no `client_secret`)
      completes `signinRedirect()` with `code_challenge_method=S256` and renders
      `user.profile.email`. Nothing was registered with lanyard first
- [x] 3. `spikes/php-web` (`jumbojett`) completes `authenticate()` and
      `requestUserInfo('email')` returns `ada@example.test`
- [x] 4. All three running: `dotnet-web` as Ada, then `php-web` as Mira, same
      browser profile. PHP shows Mira, .NET still shows Ada; revisiting each
      protected page completes **without the picker**, each with its own persona.
      Screenshots of both
- [x] 5. `redirect_uri=https://evil.example.com/cb` → `400`, `text/html`, **no
      `Location`**, browser still on `127.0.0.1:9500`, page names the URI and
      states the loopback rule. Same for `http://web.localtest.me:5000/cb`
- [x] 6. Logging in as `nobody` renders `email: (no email claim)` — and the login
      itself succeeds

**The protocol, by hand**

- [x] 7. Discovery has `authorization_endpoint` and `userinfo_endpoint` that both
      fetch, `authorization_code` in `grant_types_supported`, `S256` in
      `code_challenge_methods_supported`, and `response_types_supported`
      **exactly** `["code"]`
- [x] 8. A complete `curl -c/-b` flow: `/authorize` → `302 /_/?req=…`;
      `POST /_/pick` → `302 …/signin-oidc?code=…&state=…`; that code at
      `/oidc/token` → `200` with `access_token`, `id_token`, `token_type:
      "Bearer"`, `expires_in: 60`
- [x] 9. `state` echoed byte-for-byte including `a b/c&d=e%2F`; an `/authorize`
      with no `state` produces a redirect with no `state` parameter at all
- [x] 10. The `id_token` verifies with `jose` against the live JWKS with
      `audience` = the `client_id` and `issuer` = discovery's `issuer`; `nonce`
      equals what was sent; `at_hash` and `c_hash` equal the recomputed hashes
- [x] 11. PKCE: correct `S256` verifier → `200`; one character different → `400
      invalid_grant` naming the code verifier. Repeated with `plain` and with the
      method omitted
- [x] 12. Three `400 invalid_grant`s with three different descriptions: code
      reused, code exchanged after 70 seconds, code exchanged with a mismatched
      `redirect_uri`
- [x] 13. `response_type=id_token` → `302` with `error=unsupported_response_type`
      and a description containing `options.ResponseType = "code"`;
      `response_mode=fragment` → `302` with `unsupported_response_mode`
- [x] 14. `scope=profile email` with no `openid` completes with **no** `id_token`,
      and the access token is still valid at a resource server
- [x] 15. Three logins as Ada: `openid` → `sub`, no `email`, no `name`;
      `openid email` → `email` and `email_verified`, still no `name`;
      `openid email profile` → `name` and `preferred_username` too. `roles` in
      all three
- [x] 16. `/userinfo` with the criterion-8 access token → `200` equal to the ID
      token's scope-filtered persona claims; no `Authorization` → `401` with
      `WWW-Authenticate: Bearer`; `lanyard token --as ada --expired` → `401`

**Sessions and the cookie**

- [x] 17. `curl -i` shows `HttpOnly`, `SameSite=Lax`, `Path=/`, **no `Secure`**,
      **no `Max-Age`/`Expires`**
- [x] 18. Same jar, same `client_id` → `302` straight to the redirect URI, no
      `/_/`. Different `client_id`, same jar → `302` to `/_/?req=…`
- [x] 19. `prompt=login` with a selection → the picker; `prompt=none` with one →
      a code, no HTML; `prompt=none` with an empty jar → `302
      error=login_required` and a body containing no picker markup
- [x] 20. `max_age=1` against an older selection → the picker; `max_age=3600` →
      straight through
- [x] 21. "Always ask" enabled → criterion 18's second `/authorize` shows the
      picker for both client ids; unchecked → the skip returns
- [x] 22. Restarting `lanyard serve` and repeating criterion 18 shows the picker

**Still one function**

- [x] 23. Browser-login access token (`audience=billing-api`) and
      `lanyard token --as ada --aud billing-api`, both through
      `jq -S 'del(.iat,.nbf,.exp,.jti,.client_id,.scope)'`, `diff` to empty
- [x] 24. Unregressed: `failure-tokens.sh` prints seven lines and exits `0`;
      `lanyard token`/`env` and all six flaws unchanged; `client_credentials`
      still returns `expires_in: 60` and no `id_token`

**The UI**

- [x] 25. `GET /_/` with no `req` → `200 text/html` naming all three personas and
      saying no login is in progress. The banner's `UI →` line prints exactly that
      URL
- [x] 26. The whole of criterion 1 in a browser with **JavaScript disabled**
- [x] 27. A persona named `<script>alert(1)</script>` renders as `&lt;script&gt;`
      in `curl` output and as literal text in the browser, with no dialog
- [x] 28. "Mint one now" with `sub=zed`, `email=zed@example.test` and
      `{"department":"ops"}` lands `spikes/dotnet-web` authenticated with those
      claims, and the *next* `/authorize` from that client shows the picker
- [x] 29. With no `node` and no `npm` on `PATH`, `cargo build --release` succeeds
      and the binary serves a styled picker — CSS included, no CDN request in the
      page source

## Progress notes

Where reality diverged from the plan as written, and why.

- **`nest("/_")` really does not answer `GET /_/`.** The plan flagged this as a
  risk; it happened. axum 0.8 routes `/_` and `/_/pick` from the nest and 404s on
  `/_/`. Both spellings are now routed, `/_/` at the top level, per the plan's
  own instruction to route both rather than move the URL. `tests/browser_flow.rs`
  has the test that fails if the second route is ever tidied away.
- **The store gained a fourth lookup answer, `Spent`.** Criterion 12 asks for a
  code that was *reused* to be distinguishable from one that never existed, and a
  `take` that simply removes cannot tell them apart. Taking now leaves a
  tombstone that is swept after twice the lifetime.
- **`Expiring` is generic and the three stores are its instances.** The plan
  named `PendingRequests`, `Codes` and `Sessions`; the first two are the same
  data structure with different lifetimes, so they are one type used twice.
  `Sessions` is its own, because it never expires.
- **`claims_at` carries an `#[allow(clippy::too_many_arguments)]`** with the
  reasoning in a doc comment: bundling its eight arguments into a struct with a
  `Default` would let a caller skip a question, and the question it would skip is
  the filter.
- **`src/ui/html.rs` arrived at the `/authorize` rejection box, not the picker
  box.** The rendered `400` needed the escaping helper and the page shell first.
- **`max_age` is compared RFC-literally**: the picker appears when the elapsed
  time is *greater than* `max_age`, so `max_age=0` against a selection made in
  the same second still skips. Whole-second granularity, and the alternative
  (treating `0` as "always") would be lanyard inventing a rule.
- **A `redirect_uri` absent from the token request is accepted**; only a *wrong*
  one is refused. RFC 6749 §4.1.3 says to require it, but requiring a parameter
  an SDK chose not to resend is a registration-shaped "no" on an endpoint that
  has none, and it catches no bug.
- **`/userinfo` reads `persona_claims` for a loaded persona and falls back to
  filtering the token's own claims for one that is not.** The plan assumed the
  first path only; an identity from "mint one now" is in no file, and its
  `/userinfo` response still has to be the person the token says it is. Both
  paths go through the same `ClaimFilter`.
- **`spikes/dotnet-web`, `spikes/php-web` and `spikes/node-spa` are no longer
  gitignored.** They were Phase 0 throwaways; they are now Phase 4's three named
  acceptance clients, and a criterion that names an app it does not ship is a
  criterion nobody else can re-run. Build output and `composer install` output
  stay ignored; `node-spa/vendor/` is committed, because it is vendored rather
  than installed.
- **The spec's open question 7 has an answer: .NET sends
  `response_mode=form_post` even with `ResponseType = "code"`.** Observed on the
  `/secure` challenge from `spikes/dotnet-web`. The 20 lines were not
  speculative, and criterion 26 turns out to depend on them — with JavaScript
  disabled the `<noscript>` button is the whole last hop.
- **CORS was pulled in from Phase 5, in the minimum shape criterion 2 needs.**
  This is the one real contradiction in the spec: criterion 2 asks a browser
  public client on `:5173` to complete `signinRedirect()` against lanyard on
  `:9500`, and the Out of scope section says CORS is Phase 5 and "`/oidc/token`
  still answers a same-origin request only". Both cannot hold — a browser will
  not let a SPA read discovery or POST to the token endpoint cross-origin without
  the headers. The criterion won, on the grounds that it is what "done" means for
  this phase and a `node-spa` spike that cannot log in is a broken example.
  What came in is `src/oidc/cors.rs` and nothing else: no `refresh_token`, no
  `/end_session`, no `/introspect`, no `/revoke`, and **no
  `Allow-Credentials`**. **Phase 5's spec should be told it has one less thing
  to do, and this phase's spec should have its out-of-scope line corrected.**
- **The browser criteria were driven with Playwright against Chromium** rather
  than by hand, because the interactive browser tooling was not connected in
  this session. Same engine, real cookies, real `form_post`, scripting genuinely
  disabled for criterion 26 — the control run in the log shows the `<noscript>`
  button is only reached when scripting is off. Screenshots are in
  `docs/decisions/evidence/phase04-*.png`.
- **The web spikes gained landing pages after the phase was signed off.** Both
  used to start the flow on page load — `dotnet-web` by linking straight at
  `[Authorize]`d `/secure`, `php-web` by calling `authenticate()` at the top of
  `index.php` — so the front door was an instant redirect and the protocol was
  something that happened to you rather than something you could read. Both now
  render *Not signed in* plus a button, in a shared-looking page shell, and the
  redirects begin when you press it. Criteria 1 and 3 were re-driven from the new
  front doors and still pass; `docs/decisions/evidence/phase04-16..19` are the
  screenshots. Unifying the *claims* views across stacks is roadmap Phase 11.
