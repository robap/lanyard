# Browser flow and the persona picker — spec

**Status:** done · **Roadmap:** Phase 4 · **Slug:** 04-browser-flow-and-persona-picker

## Why

Everything shipped so far mints a token for a shell. Nothing has yet logged a
human into a web application, which is the half of lanyard people will screenshot
and the half the README leads with. This phase is "redirect in, click a person,
redirect out" (CONCEPT §5) and it is the largest phase in the roadmap.

- **North star 1 — accept everything.** `/authorize` is the endpoint where every
  other IdP puts a client registry, a redirect allowlist, and a consent screen.
  lanyard has none of the three. Any `client_id`, any `client_secret`, any
  `redirect_uri` whose host is loopback. That single rejection is the entire
  security boundary, and it is one line to explain.
- **North star 3 — issuance is one function, and this is the caller it was built
  for.** Phase 1's module comment says the browser flow "arrives as a *second*
  caller rather than as the original home of the claim logic". Phase 2 left
  `grant_type` dispatch with one arm so this phase adds an arm. If an ID token's
  claims get assembled anywhere but `issue()`, the structure was wrong and this
  is the phase that finds out.
- **Phase 0 measured this flow already, against a different provider.** A .NET
  app completes an interactive login over plain HTTP with exactly
  `RequireHttpsMetadata = false`, in Chrome and Firefox, and PHP needs nothing at
  all ([https-priority.md](../decisions/https-priority.md)). That spike left
  three standing obligations that come due here and nowhere else: never set
  `Secure` on lanyard's own cookie, name `options.ResponseType = "code"` in the
  `unsupported_response_type` error, and put "Correlation failed" in the README
  with its real cause. All three are in scope below.
- **The multi-project property becomes visible for the first time.** Three
  services on three ports against one running instance with zero setup between
  them is the claim in CONCEPT §3. Until now it has been unobservable, because a
  CLI token has no session to leak between projects. Sessions keyed per
  `client_id` is what keeps it true, and acceptance criterion 4 is what proves
  it.

## In scope

- **`GET`/`POST /oidc/authorize`** — authorization code, with PKCE (`S256` and
  `plain`), with a client secret, or with neither. `response_mode=query` and
  `response_mode=form_post`.
- **The one rejection: a `redirect_uri` whose host is not loopback**, decided by
  a literal host check with no DNS.
- **The persona picker at `/_/`** — a list of people, no password field, plus a
  "mint one now" panel for arbitrary claims. Server-rendered HTML, embedded CSS,
  and the core flow works with JavaScript disabled.
- **`authorization_code` as a second arm** on `POST /oidc/token`, returning an
  `id_token` alongside the access token when `openid` was requested.
- **`GET`/`POST /oidc/userinfo`** — bearer-authenticated, scope-filtered.
- **ID token claims**: `iss`, `sub`, `aud` (the `client_id`), `exp`, `iat`,
  `auth_time`, `nonce`, `at_hash`, `c_hash`, plus the persona's claims filtered
  by the granted scope.
- **Scope-filtered claims in the ID token and at `/userinfo`** — `email` scope
  gates `email`/`email_verified`, `profile` scope gates `name`/
  `preferred_username`. The access token is unchanged from Phase 2.
- **Sessions**: one `HttpOnly; SameSite=Lax; Path=/` cookie, never `Secure` in
  HTTP mode, holding an opaque id into in-memory per-`client_id` selections. Plus
  an "always ask" toggle in the picker, and `prompt=login|select_account|none`
  and `max_age` honoured against it.
- **Discovery grows** `authorization_endpoint`, `userinfo_endpoint`,
  `code_challenge_methods_supported`, `scopes_supported`, `response_modes_supported`;
  `grant_types_supported` gains `authorization_code`; and
  **`response_types_supported` is corrected to exactly `["code"]`**.
- **The banner grows a `UI →` line**, which Phase 1 deferred on the grounds that
  it must not print a URL that 404s.
- **`spikes/dotnet-web/` and `spikes/php-web/` are repointed at lanyard**, and
  **`spikes/node-spa/`** is added — a static page using `oidc-client-ts` as a
  public client. These are the three named acceptance clients.
- **README**: the Scope paragraph, the new endpoints, the `/_/` UI, and a
  troubleshooting section carrying the two .NET gotchas Phase 0 wrote down —
  "Correlation failed" and the `ResponseType` default.

## Out of scope

- **`refresh_token`, `/end_session`, `/introspect` and `/revoke`** — Phase 5. No
  `refresh_token` is issued here even when `offline_access` is requested.

  **CORS was in this list and came out during implementation.** It had to:
  criterion 2 asks `oidc-client-ts` to complete `signinRedirect()` from a page on
  `http://localhost:5173`, and a browser will not let a SPA read the discovery
  document or POST to `/oidc/token` on another origin without the headers. The
  two could not both be true, and the acceptance criterion is the stronger
  commitment. What shipped is the minimum: the `/oidc` endpoints echo the
  request's `Origin`, allow `authorization` and `content-type`, answer preflight,
  and **never** send `Access-Control-Allow-Credentials` — a public client
  authenticates with a bearer token, not with lanyard's cookie. `/_/` and the
  test seam answer no other origin at all. Phase 5 has one less thing to do.
- **The request log** (Phase 6). A failed login still has to be diagnosable, but
  from the error the RP receives and the page lanyard renders — not from a
  stream lanyard does not have yet. Phase 6's "which parameter mismatched" is
  what the PKCE and code failures below are written to make possible.
- **Filtering personas by `client_id`** (Phase 7). The picker shows every loaded
  persona to every client. `client:` is still parsed and read by nothing.
- **Silent renew's iframe mechanics** (Phase 9). `prompt=none` is implemented
  here because rendering a picker inside a hidden iframe is the wrong behavior on
  day one, not because this phase claims silent renew works: lanyard's `Lax`
  cookie is not sent on a third-party iframe navigation, so an actual SPA renew
  will get `login_required`. That is Phase 9's problem and its evidence.
- **A consent screen.** No scope approval, no "this app wants to…". Consent is
  the other thing `/authorize` normally does and it is registration in a
  different costume.
- **`flaw=` on `/authorize` or on the ID token.** Phase 3 deferred the seventh
  flaw (`at_hash` wrong) explicitly on the grounds that no ID token existed. One
  exists now; adding it is still a later phase's call, not a free rider on this
  one.
- **Non-loopback redirect URIs of any kind**, including custom schemes
  (`com.example.app:/cb`) for native apps and `urn:ietf:wg:oauth:2.0:oob`. When
  device or native flows come up they bring their own phase.
- **DNS resolution of the redirect host.** `http://web.localtest.me:5000/cb`
  resolves to 127.0.0.1 and is still rejected. See Behavior.
- **Implicit, hybrid, and ROPC.** `response_type` is `code` or it is an error.
- **Persisting sessions or authorization codes across a restart.** Both live in
  memory. Restarting lanyard logs everybody out, which is a property worth
  having.
- **Passwords, MFA, `acr`/`amr`, an account-chooser beyond the persona list.**

## Behavior

### `/authorize` accepts everything except one thing

| Parameter | Handling |
|---|---|
| `client_id` | **Required.** Any value. It is the session key and the ID token's `aud`. |
| `redirect_uri` | **Required.** Any value whose host is loopback — the one rejection. |
| `response_type` | `code` only. Anything else is `unsupported_response_type`. |
| `response_mode` | `query` (default) or `form_post`. `fragment` is `unsupported_response_mode`. |
| `scope` | Any. `openid` decides whether an ID token is issued; `email` and `profile` gate claims. |
| `state` | Echoed back byte-for-byte when sent; absent when not. Never invented. |
| `nonce` | Carried into the ID token when sent; absent when not. |
| `code_challenge` / `code_challenge_method` | `S256` or `plain` (RFC 7636's default when the method is omitted). Absent means no PKCE. |
| `prompt` | `login`, `select_account`, `none`, or absent. Others are ignored. |
| `max_age` | Honoured against the remembered selection's `auth_time`. |
| `audience` / `resource` | Optional, and becomes the **access token's** `aud`, exactly as on Phase 2's grant. |
| `client_secret` | Accepted anywhere it is offered and checked nowhere. |
| anything else | Ignored, per Phase 2's rule for unrecognized parameters. |

`GET` and `POST` both work — OIDC Core allows both and some SDKs use the second.

There is no client registration step anywhere in that table, and that is the
point. The `client_id` is required not because lanyard knows it but because
lanyard has to key a session on something, and an app that omits it is an app
whose sessions would collide with every other app's.

### Loopback is a literal check, not a resolution

The `redirect_uri` is parsed. Its scheme must be `http` or `https`, and its host
must be one of:

- `localhost`, or any name ending in `.localhost` (RFC 6761 reserves it)
- any address in `127.0.0.0/8`
- `::1`, written `[::1]`

Any port. Any path. Any query. Everything else is rejected, **including hostnames
that genuinely resolve to loopback** — `web.localtest.me`, `lvh.me`,
`app.local.dev`. That is deliberate:

- **No DNS lookup happens.** A rule enforced by resolution is a rule whose answer
  depends on the network, is slow, is cacheable, and is different inside a
  container than outside it. A literal check is the same everywhere and can be
  read out loud.
- **The developer's own hosts file becomes an attack surface** otherwise, and
  more importantly the boundary stops being explainable in one line, which is the
  property CONCEPT §3 is actually protecting.

Phase 0 already measured that `web.localtest.me` is where a .NET app's own login
breaks anyway, for an unrelated reason (`Correlation failed`, because it is not a
secure context). Rejecting it here costs a developer nothing they had.

### The redirect_uri rejection is rendered, not redirected

RFC 6749 §4.1.2.1 is explicit: when `redirect_uri` is missing or invalid, or
`client_id` is missing, the server **must not** redirect. Sending an `error=` to
an address you have just decided you do not trust is the one thing that would
make the boundary decorative. So those two cases render a `400 text/html` page at
lanyard, naming the offending value and stating the rule in one sentence.

**This corrects the roadmap's wording of its own criterion.** "Rejected with an
error the RP can read" is not achievable for this class of error — the RP is
never contacted. The observable is: the browser stays on lanyard, the status is
`400`, there is no `Location` header, and the page says which URI was rejected
and why. The audience for that message is the developer reading it, which is who
needed it.

Every other error does redirect, carrying `error`, `error_description`, and the
`state` that was sent, so the RP's own error handling runs:

| Condition | Redirected error |
|---|---|
| `response_type` other than `code` | `unsupported_response_type` |
| `response_mode=fragment` | `unsupported_response_mode` |
| `code_challenge_method` other than `S256`/`plain` | `invalid_request` |
| `code_challenge` present but empty, or method without challenge | `invalid_request` |
| `request` / `request_uri` present | `request_not_supported` / `request_uri_not_supported` |
| `prompt=none` with nothing to log in as | `login_required` |

**`unsupported_response_type` names the .NET setting.** Phase 0's obligation #4:
.NET's default `ResponseType` is `id_token`, so a `dotnet new` app with otherwise
correct settings sends the implicit flow and gets a bare protocol noun back. The
description reads:

```
response_type "id_token" is not supported; lanyard implements the authorization
code flow only. In .NET set options.ResponseType = "code".
```

The .NET sentence is emitted only when the requested `response_type` contains
`id_token`, because that is the one case where it is the actual diagnosis.

### The picker is a page, and the authorization request is a record

`/authorize` validates, stores a **pending request** — the parameters plus a
timestamp, under an unguessable id, in memory, for five minutes — and `302`s to
`/_/?req=<id>`. The picker renders from that record.

Protocol endpoints live under `/oidc`, the UI lives under `/_/`, and the redirect
is what keeps that line intact (CONCEPT §3). It also means the picker can be
reloaded, bookmarked mid-flow, or opened in another tab without re-validating an
authorization request, and it means the URL a developer is looking at while
picking a person is the URL they would visit to look at the persona list.

The page contains:

- **One `<form method="post">` per persona**, submitting to
  `/_/pick` with the request id and the persona id. A button per person, showing
  display name, id, email, and roles. No JavaScript.
- **A "mint one now" panel** — a second form with `sub`, `name`, `email`, `roles`
  (comma-separated), and a textarea of extra claims as JSON. Submitting it logs
  in as an identity that does not exist in any file.
- **An "always ask" checkbox**, which sets the flag described below.
- With **no `req` parameter**, the same page renders the persona list with a line
  saying no login is in progress. The banner's `UI →` URL therefore lands
  somewhere honest.

On submit, lanyard records the selection in the session, mints an authorization
code bound to the pending request and the chosen claims, and redirects to the
RP's `redirect_uri` with `code` and `state`. The pending record is consumed.

**Cross-site posts to `/_/pick` are not a concern worth a token.** The request id
is unguessable and single-use, and `SameSite=Lax` means a cross-site POST arrives
without the session cookie in the first place. Stated so the absence is a
decision rather than an oversight.

**Persona display strings are HTML-escaped.** A persona file is the developer's
own, but its `attributes` can come from a fixture generator, and a picker that
executes its own persona list is a bad look for a tool whose pitch is "it catches
your bugs".

### `form_post` exists because .NET was watched sending it

Phase 0's log recorded .NET setting its correlation and nonce cookies
`SameSite=None` "because it uses `response_mode=form_post`". Rather than assume
that only applied to the implicit response type it was defaulting to, lanyard
implements `form_post`: a minimal HTML page whose single form auto-submits the
`code` and `state` to the `redirect_uri` by POST, with a visible submit button
inside `<noscript>`. That auto-submit is the only JavaScript lanyard ships in the
login path, and it degrades to one click.

### Sessions are per `client_id`, and that is the multi-project property

One cookie:

```
lanyard_session=<opaque>; Path=/; HttpOnly; SameSite=Lax
```

No `Secure`, ever, while lanyard serves HTTP — Phase 0's obligation #1, whose
failure mode is "it asks me to pick a persona every single time". No `Max-Age`
either: it is a browser-session cookie, so closing the browser is a working reset
and no persona selection outlives the day by weeks. Server-side it maps to a
record holding, **per `client_id`**, the persona chosen and the `auth_time` it
was chosen at, plus one global `always_ask` flag.

A second `/authorize` from a `client_id` with a remembered selection skips the
picker entirely and returns a code. A first `/authorize` from a *different*
`client_id` in the same browser shows the picker, and choosing differently does
not disturb the first. That is criterion 4, and it is the first observable form
of "three services on three ports, one instance, zero setup between them".

The picker is shown anyway when: `always_ask` is set, `prompt=login` or
`prompt=select_account` is sent, or `max_age` is sent and the remembered
`auth_time` is older than it. `prompt=none` never renders anything — it returns a
code if there is a remembered selection for that `client_id`, and redirects with
`error=login_required` if there is not.

**An identity from the "mint one now" panel is not remembered.** The next
`/authorize` from that client shows the picker again. Remembering a one-off is
surprising, and the session record would have to grow from an id into a claim
blob to hold it.

Sessions, pending requests, and codes are all in memory and all die with the
process.

### The code exchange is a second `grant_type` arm

`POST /oidc/token` with `grant_type=authorization_code`, `code`,
`code_verifier` when a challenge was sent, and `redirect_uri` when one was sent
to `/authorize`. Client authentication is accepted in any form and checked in
none, exactly as on `client_credentials`.

Codes are **single-use** and live **60 seconds**. A code is issued after the
human has already clicked, so the only thing that fits inside 60 seconds is the
redirect and the exchange, which take milliseconds.

`invalid_grant` — a `400` in the OAuth shape — covers: an unknown code, an
expired code, a code presented twice, a `redirect_uri` that does not match the
one the code was issued against, a missing `code_verifier` when a challenge was
recorded, and a verifier that does not match the challenge. Each carries a
different `error_description` naming what failed, because Phase 6's log has to
be able to say "which parameter mismatched" and it can only report what this
phase distinguishes.

**PKCE is genuinely verified, and this does not contradict north star 1.**
Accept-everything is about *registration* — who you say you are, where you say
you want to come back to, what audience you name. It was never about skipping the
cryptography. A local IdP that rubber-stamps a wrong `code_verifier` lets a
broken PKCE implementation ship, and production is a bad place to find out. Same
reasoning for single-use codes and the `redirect_uri` match: they cost nothing
and they are the checks whose absence makes a mock diverge from the real thing.

Success is Phase 2's response shape plus one field:

```json
{ "access_token": "eyJ…", "token_type": "Bearer", "expires_in": 60,
  "scope": "openid email profile", "id_token": "eyJ…" }
```

`id_token` is present **only when the authorization request's scope contained
`openid`**. A request without it is a plain OAuth 2.0 code flow and gets a plain
OAuth 2.0 response — which is honest, and is a thing worth being able to test.

### The ID token, and why its lifetime is different

Built by `issue()`, from the same persona→claims table as everything else, with a
scope filter and an extra registered-claim set:

| Claim | Value |
|---|---|
| `iss` | the configured issuer, as always |
| `sub` | the persona id |
| `aud` | **the `client_id`** — not the API audience |
| `iat`, `exp` | now, and now + 300 |
| `auth_time` | when the persona was picked, which may predate `iat` on a remembered session |
| `nonce` | echoed when the request sent one; absent otherwise |
| `at_hash` | base64url of the leftmost 128 bits of `SHA-256(access_token)` |
| `c_hash` | base64url of the leftmost 128 bits of `SHA-256(code)` |
| persona claims | filtered by scope, per the table below |

**The ID token lives 300 seconds; the access token still lives 60.** They are not
the same kind of thing. The access token is a credential and its short life is
the entire point of CONCEPT §6 — a 60-second access token that gets rejected is
lanyard working. The ID token is an authentication receipt, consumed once at
login and then exchanged for the RP's own cookie; a 60-second one makes
`oidc-client-ts` consider the user expired seconds after signing in, which tests
Phase 9's renew path rather than this phase's login path. Five minutes is still
aggressive by any production standard.

**`c_hash` is emitted although the code flow does not require it.** OIDC Core
requires it only for `response_type=code id_token` and calls it optional
otherwise. Emitting a correct one costs a hash, and an RP that validates it
should find it valid — which is a better test of that RP than never exercising
its code path at all. `at_hash` is the same argument and is checked by more
libraries.

### Scope decides claims — in the ID token and at `/userinfo` only

| Scope requested | Claims added |
|---|---|
| always | `sub` |
| `email` | `email`, `email_verified` |
| `profile` | `name`, `preferred_username` |
| always | `roles` and the persona's arbitrary `attributes` |

`roles` and `attributes` are unscoped because OIDC does not define a scope for
them and hiding a developer's own custom claims behind a standard scope would be
inventing a rule.

**The access token is not filtered, and that asymmetry is deliberate.** OIDC Core
§5.4 specifies claims-per-scope for the ID token and the UserInfo response.
Nothing specifies it for an access token — an access token is not even required
to be a JWT, and its `scope` is an authorization grant for the resource server to
read, not a claims filter. So: filtered where it is specified, unchanged where it
is not. Phase 2's contract, its README table, and Phase 3's criterion 14 all stay
true.

Phase 0 is the evidence that filtering is right here rather than merely legal:
dropping `options.Scope.Add("email")` against `oidc-provider-mock` produced
`email: (no email claim)` in the .NET app. That is the behavior .NET developers
are calibrated to, and a lanyard that hands over `email` unasked would be the
mock-diverges-from-production bug that this whole project exists to not be.

### `/userinfo`

`GET` or `POST`, `Authorization: Bearer <access_token>`. `200
application/json` with the same scope-filtered claim set the ID token carried,
`sub` always present. Access tokens are verified as lanyard's own — signature,
`iss`, and `exp` — and nothing else; there is no client to check.

Failure is `401` with `WWW-Authenticate: Bearer error="invalid_token",
error_description="…"`, for a missing, malformed, unsigned, foreign, or expired
token. This is the second place lanyard says no, and the reason is the same as
PKCE's: an app that reads `/userinfo` with an expired token has a bug, and a
mock that answers anyway hides it. `jumbojett`'s `requestUserInfo()` is the
client that proves it works.

### Discovery, corrected

`authorization_endpoint`, `userinfo_endpoint`,
`code_challenge_methods_supported: ["S256", "plain"]`,
`scopes_supported: ["openid", "email", "profile"]`,
`response_modes_supported: ["query", "form_post"]`, and `authorization_code`
added to `grant_types_supported`.

And one correction: **`response_types_supported` becomes exactly `["code"]`**.
Phase 1 wrote `["id_token", "code"]` when nothing implemented either. Now that
`/authorize` exists, advertising `id_token` tells a .NET app that its default
setting is supported and then rejects it at request time — the exact
fail-later-and-further-away failure Phase 1's "discovery advertises only what
exists" rule was written to prevent.

The document becomes conforming with this phase, which was Phase 1's stated
expectation.

### No Node on the build path

The picker is server-rendered HTML built in Rust with an escaping helper, and one
CSS file pulled in with `include_str!`. No template engine, no bundler, no
`npm`, nothing generated at build time. `prefers-color-scheme` handles dark mode
so the README's light-mode screenshot (Phase 11) is what a default-light machine
sees.

## Acceptance criteria

Every run sets `LANYARD_DATA_DIR` and a throwaway `XDG_CONFIG_HOME`, as in Phases
2 and 3, and uses the real port 9500 so the issuer matches what the browser is
talking to. Browser criteria are driven in a real browser and leave a screenshot
in `docs/decisions/evidence/`, matching Phase 0's precedent. `jose` verification
means `scripts/jose-verify.mjs` against the live JWKS URL.

Harnesses: `spikes/dotnet-web/` on `http://localhost:5000` with `client_id`
`billing-web`, `spikes/php-web/` on `http://localhost:5001` with `client_id`
`spike-php`, `spikes/node-spa/` on `http://localhost:5173` with `client_id`
`node-spa`.

**Three real clients, in a real browser**

- [x] 1. `spikes/dotnet-web` (`AddOpenIdConnect`, `Authority` = lanyard,
      `RequireHttpsMetadata = false`, `ResponseType = "code"`) completes a full
      login: `/secure` redirects to lanyard, the picker lists Ada Bell, Mira
      Okonkwo and nobody, clicking **Ada Bell** lands back at `/secure`
      rendering `email: ada@example.test`. Screenshots of the picker and the
      result.
- [x] 2. `spikes/node-spa` using `oidc-client-ts` as a **public client** — no
      `client_secret` anywhere in its config — completes `signinRedirect()` with
      `code_challenge_method=S256` and renders `user.profile.email`. Nothing was
      registered with lanyard first; the only lanyard-side setup was starting it.
- [x] 3. `spikes/php-web` with `jumbojett/openid-connect-php` completes
      `authenticate()` and `requestUserInfo('email')` returns
      `ada@example.test` — which is `/oidc/userinfo` answering a real client, not
      a curl.
- [x] 4. With all three running: log in to `dotnet-web` as **Ada** and to
      `php-web` as **Mira**, in the same browser profile, in that order. The PHP
      app shows Mira and the .NET app still shows Ada. Then revisit each app's
      protected page: both complete **without the picker appearing**, each with
      its own persona. Screenshots of both results.
- [x] 5. `http://127.0.0.1:9500/oidc/authorize?client_id=x&response_type=code&redirect_uri=https%3A%2F%2Fevil.example.com%2Fcb`
      → `400`, `Content-Type: text/html`, **no `Location` header**, the browser
      still on `127.0.0.1:9500`, and the page names
      `https://evil.example.com/cb` and states the loopback rule. Same for
      `http://web.localtest.me:5000/cb`, which resolves to loopback and is
      rejected anyway.
- [x] 6. The `nobody` persona: log in to `spikes/dotnet-web` as nobody and the
      page renders `email: (no email claim)`. The login itself succeeds — the
      persona that breaks applications is a persona you can log in as.

**The protocol, by hand**

- [x] 7. `curl -s .../oidc/.well-known/openid-configuration | jq` has
      `authorization_endpoint` and `userinfo_endpoint` that both fetch,
      `grant_types_supported` containing `authorization_code`,
      `code_challenge_methods_supported` containing `S256`, and
      `response_types_supported` **exactly** `["code"]`.
- [x] 8. A complete flow with `curl -c/-b` and no browser: `GET /authorize` →
      `302` to `/_/?req=…`; `POST /_/pick` with the req id and `persona=ada` →
      `302` to `http://localhost:5000/signin-oidc?code=…&state=…`; that code
      exchanged at `/oidc/token` → `200` with `access_token`, `id_token`,
      `token_type: "Bearer"`, `expires_in: 60`.
- [x] 9. `state` is echoed byte-for-byte, including
      `a b/c&d=e%2F` sent url-encoded; and an `/authorize` with no `state`
      produces a redirect with no `state` parameter at all.
- [x] 10. The `id_token` from criterion 8 verifies with `jose` against the live
      JWKS with `audience` = the `client_id` and `issuer` = the discovery
      document's `issuer`. Its `nonce` equals what was sent. Its `at_hash`
      equals `base64url(SHA-256(access_token)[0..16])` and its `c_hash` equals
      the same over the code — both recomputed in one shell line and compared.
- [x] 11. PKCE: with `code_challenge_method=S256`, exchanging with the correct
      verifier → `200`; with a verifier one character different → `400`
      `invalid_grant` whose description names the code verifier. Repeat with
      `plain`, and with the method omitted (which is `plain`).
- [x] 12. Code hygiene, three `400 invalid_grant`s with three different
      descriptions: the same code exchanged twice, a code exchanged 70 seconds
      after issue, and a code exchanged with a `redirect_uri` that differs from
      the one `/authorize` received.
- [x] 13. `response_type=id_token` → `302` back to the redirect URI with
      `error=unsupported_response_type` and an `error_description` containing
      `options.ResponseType = "code"`. `response_mode=fragment` → `302` with
      `unsupported_response_mode`.
- [x] 14. `scope=profile email` with **no `openid`** completes and the token
      response has **no** `id_token` — and the access token is still valid at a
      resource server.
- [x] 15. Scope filtering, three logins as Ada, reading the decoded `id_token`:
      `openid` alone → `sub` present, no `email`, no `name`;
      `openid email` → `email` and `email_verified`, still no `name`;
      `openid email profile` → `name` and `preferred_username` too. `roles` is
      present in all three.
- [x] 16. `/userinfo` with the access token from criterion 8 → `200` whose body
      equals the ID token's scope-filtered persona claims (`sub` included, `iss`
      / `aud` / `exp` / `nonce` / hashes excluded). With no `Authorization`
      header → `401` carrying `WWW-Authenticate: Bearer`. With a token whose
      `exp` has passed (`lanyard token --as ada --expired`) → `401`.

**Sessions and the cookie**

- [x] 17. The `Set-Cookie` on lanyard's own response has `HttpOnly`,
      `SameSite=Lax`, `Path=/`, **no `Secure`**, and **no `Max-Age`/`Expires`**.
      Read from `curl -i`, not from source.
- [x] 18. Second `/authorize` with the same cookie jar and the same `client_id`
      → `302` straight to the redirect URI with a code, no `/_/` in between.
      With a different `client_id` and the same jar → `302` to `/_/?req=…`.
- [x] 19. `prompt=login` with a remembered selection → the picker. `prompt=none`
      with a remembered selection → a code, no HTML. `prompt=none` with an empty
      cookie jar → `302` with `error=login_required`, and the response body
      contains no picker markup.
- [x] 20. `max_age=1` with a selection made more than a second ago → the picker.
      `max_age=3600` → straight through.
- [x] 21. The "always ask" checkbox: after enabling it, criterion 18's second
      `/authorize` shows the picker instead of returning a code, for both
      client ids. Unchecking it restores the skip.
- [x] 22. Restarting `lanyard serve` and repeating criterion 18 shows the
      picker — sessions are in memory and a restart logs everybody out.

**Still one function**

- [x] 23. The access token from a browser login as Ada with
      `audience=billing-api`, and the token from
      `lanyard token --as ada --aud billing-api`, both decoded and passed
      through `jq -S 'del(.iat,.nbf,.exp,.jti,.client_id,.scope)'`, `diff` to
      empty. Two callers of `issue()`, one claim set — Phase 3's criterion 14,
      extended to the caller it was written for.
- [x] 24. Unregressed: `spikes/dotnet-api/failure-tokens.sh` still prints its
      seven lines and exits `0`; `lanyard token`, `lanyard env` and all six
      flaws behave as Phase 3 left them; `POST /oidc/token` with
      `grant_type=client_credentials` still returns `expires_in: 60` and no
      `id_token`.

**The UI**

- [x] 25. `GET /_/` with no `req` → `200 text/html` naming all three built-in
      personas and saying no login is in progress. The banner's `UI →` line
      prints exactly that URL.
- [x] 26. The whole of criterion 1 repeated in a browser with **JavaScript
      disabled**, completing to the same rendered email claim.
- [x] 27. A personas file whose `name` is `<script>alert(1)</script>` renders as
      `&lt;script&gt;` in `curl` output, and the browser shows the literal text
      with no dialog.
- [x] 28. "Mint one now": submitting `sub=zed`, `email=zed@example.test` and
      extra claims `{"department":"ops"}` lands `spikes/dotnet-web`
      authenticated with those claims in its claim list, and the *next*
      `/authorize` from that client shows the picker rather than remembering
      Zed.
- [x] 29. In an environment with no `node` and no `npm` on `PATH`,
      `cargo build --release` succeeds and the resulting binary serves a styled
      picker — CSS included, no request to any CDN in the page source.

## Open questions

None blocking. Ten decisions in **Behavior** go beyond what the roadmap states,
listed here because they are the ones worth disagreeing with before a plan
exists:

1. **The `redirect_uri` rejection renders at lanyard instead of reaching the RP**,
   which contradicts the literal wording of the roadmap's fifth criterion.
   Redirecting an error to an address you have just refused to trust makes the
   one rejection decorative; RFC 6749 §4.1.2.1 says so too. Criterion 5 is
   written to the corrected behavior.
2. **Loopback is a literal host check with no DNS**, so `web.localtest.me` is
   rejected even though it resolves to 127.0.0.1. A rule enforced by resolution
   depends on the network and stops being one line to explain.
3. **PKCE, code single-use, code expiry, and the `redirect_uri` match are
   genuinely enforced.** Accept-everything is about registration, not about
   skipping cryptography — but this is the second and third place lanyard says
   no, and it deserves to be argued with rather than assumed.
4. **The ID token lives 300 seconds while the access token stays at 60.** The
   only place a lifetime in this project is not 60 seconds.
5. **Scope filters the ID token and `/userinfo`, but not the access token.**
   Specified where OIDC specifies it, unchanged where it does not — and Phase 0
   observed `oidc-provider-mock` filtering, which is what .NET developers expect.
6. **`prompt=none` is implemented in this phase**, overlapping what Phase 9
   owns, because the alternative is rendering a picker inside a hidden iframe.
   This phase does not claim silent renew works; it claims the picker never
   appears where it cannot be clicked.
7. **`response_mode=form_post` is implemented now**, on the strength of Phase 0's
   observation that .NET sends it. If the .NET acceptance run shows `query` is
   what actually arrives, this is 20 lines that could have waited — and it is
   still the cheapest insurance against the phase's primary client.

   **Answered: .NET sends `form_post`, even with `ResponseType = "code"`.**
   Observed on the `/secure` challenge from `spikes/dotnet-web`, whose
   authorization request carries `response_mode=form_post&code_challenge_method=S256`.
   Not 20 lines that could have waited: criterion 26 depends on them, because
   with JavaScript disabled the `<noscript>` button on that page is the whole
   last hop of the login.
8. **The picker is reached by a redirect to `/_/?req=…`** rather than rendered at
   `/authorize`, at the cost of one extra hop, to keep `/oidc` protocol-only.
9. **The session cookie has no expiry**, so closing the browser is a reset, and
   server-side session state is in memory, so restarting lanyard is a reset.
10. **An identity minted from the "mint one now" panel is not remembered.**

Two things are noted rather than decided: whether `/authorize` should eventually
accept `flaw=` (Phase 3 deferred the seventh flaw for want of an ID token, and
one now exists), and whether the picker should expose the persona list of a
specific `client_id` (Phase 7's question, whose `client:` field is still parsed
and read by nothing).
