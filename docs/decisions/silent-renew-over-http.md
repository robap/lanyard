# Silent renew over HTTP: **HTTPS stays post-v1.**

Phase 9 ran the hidden-iframe renew for real, in two browser families, against
both of the loopback names lanyard answers to. It works over plain HTTP, in
Chrome and in Firefox, with no TLS and no `SameSite=None` — **when the app and
the issuer are dialled by the same host name.** When they are not, it fails
cleanly and lanyard says why. Neither outcome needs a CA, so HTTPS and
`lanyard trust` do not move ahead of Phases 10–11.

The answer to "does silent renew work over plain HTTP" is therefore neither yes
nor no. It is **"it depends which name you dialled"** — one rule, one README
line, one `doctor` note.

## The measurement

Full trace, verbatim: [`evidence/phase09-1-matrix.txt`](evidence/phase09-1-matrix.txt).

`spikes/node-spa` at `http://localhost:5173/?renew=iframe`, `oidc-client-ts`
3.4.0 with `automaticSilentRenew` and **no** `offline_access` — so no refresh
token, and `signinSilent()` takes the iframe branch. Google Chrome for Testing
149.0.7827.55 and Mozilla Firefox 152.0.4. The renew was never provoked from a
console: each cell logs in and waits for a 60-second access token to come within
`accessTokenExpiringNotificationTimeInSeconds` of expiry.

**Only two things change between the rows**: `LANYARD_ISSUER`, and the matching
`authority` in `index.html`.

| App origin | issuer origin | `Sec-Fetch-Site` | `Cookie` on the iframe's `/authorize` | outcome |
|---|---|---|---|---|
| `http://localhost:5173` | `http://localhost:9500` | `same-site` | `lanyard_session=…` **sent** | `302` with `code=`, `POST /oidc/token` `200`, `[node-spa] access token renewed` |
| `http://localhost:5173` | `http://127.0.0.1:9500` | `cross-site` | **no `Cookie` header at all** | `302` with `error=login_required`, no token request |

Identical in both browser families, in both directions. The spec's prediction
was that `SameSite` compares scheme and host and **ignores the port**, so
`localhost:5173` → `localhost:9500` is one site and `localhost:5173` →
`127.0.0.1:9500` is two. The browsers say so themselves, in a header:
`Sec-Fetch-Site: same-site` on one row and `cross-site` on the other.

The same-site row, verbatim:

```
GET http://localhost:9500/oidc/authorize?client_id=node-spa&…&prompt=none
Sec-Fetch-Dest: iframe
Sec-Fetch-Site: same-site
Cookie: lanyard_session=6eb3c7a5241d4089a6f3cdec29821001

<- HTTP 302
Location: http://localhost:5173/silent-callback.html?code=3392070449db411b9bb015e8cf1510b9&state=…
```

and the cross-site one, changing nothing but the name:

```
GET http://127.0.0.1:9500/oidc/authorize?client_id=node-spa&…&prompt=none
Sec-Fetch-Dest: iframe
Sec-Fetch-Site: cross-site
Cookie: (no Cookie header on the request)

<- HTTP 302
Location: http://localhost:5173/silent-callback.html?error=login_required&error_description=…
```

The address bar never moved in any of the four cells.

## Why this does not move HTTPS

The chain `docs/decisions/https-priority.md` named — *third-party iframe → needs
`SameSite=None` → needs `Secure` → needs HTTPS* — **is real, and this measurement
is where it would have bitten.** It does not, for one reason: on the row a
developer is actually on, the iframe is **not** third-party. There is nothing for
`SameSite=None` to fix, so there is nothing for `Secure` to require.

And on the row where the iframe *is* cross-site, `SameSite=None; Secure` would
not have needed a CA either — which was worth measuring rather than reasoning
about, so it was. On a throwaway patch to `src/session.rs`, reverted
immediately, the cross-site row **renews**, in both browser families, over plain
HTTP:

```
set_cookie: lanyard_session=<id>; Path=/; HttpOnly; SameSite=None; Secure
issuer:     http://127.0.0.1:9500/oidc     (the cross-site row)

chrome   Sec-Fetch-Site: cross-site   Cookie: lanyard_session=5d77779d…   302 …?code=…
firefox  Sec-Fetch-Site: cross-site   Cookie: lanyard_session=85a3cb96…   302 …?code=…
```

Both browsers store and send a `Secure` cookie set over `http://` on loopback,
because loopback is a secure context. **So flipping the cookie would "work",
today, with no TLS at all** — which is precisely why the reason for not doing it
has to be something other than "it wouldn't help".

It is refused on north star 4. A cookie that claims `Secure` while lanyard
serves `http://` is one that silently stops being sent the day anything about
the deployment changes — a host name that is not loopback, a reverse proxy, a
container reached by service name — and the symptom is "it asks me to pick a
person every single time", with nothing anywhere saying why. That is Phase 0's
obligation #1, and `src/session.rs`'s
`the_cookie_is_never_secure_and_never_expires` is a test whose stated purpose is
to fail the day somebody hardens it. It stays, and it still passes.

So the argument for TLS that CONCEPT §9 called the strongest one — silent renew
— turns out not to need it. **HTTPS stays post-v1.**

## What it costs to leave the default issuer alone

`src/config.rs` defaults the issuer to `http://127.0.0.1:9500/oidc`, and every
web spike is reached at `http://localhost:<port>`. **Out of the box, today, the
failing row is the shipped default.** A developer whose SPA renews silently has
to have set `LANYARD_ISSUER` — or dialled lanyard at `127.0.0.1` from an app also
served at `127.0.0.1`, which is the other same-site pairing.

This phase deliberately does not change that default (spec, open question 1).
Changing it is one line and a long tail — the banner, `doctor`, Phase 8's
container criteria, three spikes, every doc that quotes a URL — and it has a
real hazard of its own, measured here:

> `getent hosts localhost` on the machine that ran this matrix returns `::1`
> **only**, while `DEFAULT_BIND` is `127.0.0.1`. The predicted
> connection-refused did **not** happen: both browsers and `curl` fall back to
> the A record. It survived by fallback, not by design, and a future
> default-issuer phase has to make `DEFAULT_BIND` cover `::1` before it makes
> `localhost` the default name.

What ships instead is the diagnosis, at the two moments it is needed:

- **`lanyard doctor`** notes it when the resolved issuer's host is `127.0.0.1`,
  because `doctor` knows the issuer and cannot know the app's origin — so it is
  a note about a consequence, not a check with a verdict.
- **`/authorize` itself** says it, at the moment of failure, in the
  `error_description` of the `login_required` the iframe gets:

  > prompt=none was sent and no lanyard_session cookie arrived with the request,
  > so lanyard cannot tell who this browser is. The usual reason is a hidden
  > iframe: lanyard's session cookie is SameSite=Lax, and a browser does not send
  > it on a navigation from a site that is not lanyard's own. SameSite compares
  > the host and ignores the port, so an app on http://localhost is same-site to
  > an issuer on http://localhost and cross-site to one on http://127.0.0.1. The
  > other two reasons a cookie goes missing are that this browser logged out of
  > lanyard, and that lanyard restarted — sessions live in memory

  That sentence reaches the RP's own error handler through the iframe's query
  string **and** `lanyard logs --json`, which is where a developer will actually
  read it — an iframe's query string is not somewhere anybody looks.
  ([`evidence/phase09-2-six-causes.txt`](evidence/phase09-2-six-causes.txt).)

## What was checked and is not a problem

- **lanyard is frameable, and now asserted to be.** No `X-Frame-Options` and no
  `Content-Security-Policy` on the `prompt=none` success `302`, the
  `login_required` `302`, or the rendered `400`. It worked by omission; a test
  exists to fail the day a security-hygiene commit deletes the omission.
- **A renew is not a re-authentication.** Same `sub`, **same `auth_time`**,
  fresh `iat`, later `exp`, measured on the same-site row in both browsers — and
  the renewed access token verified against the live JWKS by `jose`, not by
  lanyard agreeing with itself.
- **A logout is not silently undone by a renew.** After `signoutRedirect()` the
  same iframe navigation gets `login_required` and the SPA shows its signed-out
  page.
- **Phase 5's refresh-token path is untouched.** Without `?renew=iframe` the same
  page still renews with one `POST /oidc/token` carrying
  `grant_type=refresh_token`, zero `/authorize` requests and zero iframes.
  ([`evidence/phase09-3-tokens-and-regressions.txt`](evidence/phase09-3-tokens-and-regressions.txt).)
- **`POST /oidc/token` cross-origin still works**, which is Phase 4's CORS doing
  its job: the code is exchanged from `localhost:5173` against `localhost:9500`.
- **One case stays broken and is documented instead.** A `prompt=none` whose
  `redirect_uri` fails the loopback check renders a `400`, which inside a hidden
  iframe is invisible: no `postMessage`, no network error, just
  `silentRequestTimeoutInSeconds` elapsing. Nothing can fix that without
  breaking the one rejection, so the README says so and points at `lanyard logs`.

## Revisits this closes and opens

- **Closes** `https-priority.md`'s *Revisit when*. That was the one named trigger
  and it has now been run.
- **Opens** nothing new for TLS. If a future phase moves the default issuer to
  `localhost`, `DEFAULT_BIND` has to cover `::1` first — see above.
