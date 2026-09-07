# node-spa — a public client, with PKCE, and no secret

A single static page using [`oidc-client-ts`](https://github.com/authts/oidc-client-ts)
as a **public client** against lanyard. It is the third of Phase 4's three
acceptance clients, and the one that exercises `code_challenge_method=S256`.

```
lanyard serve &                        # 127.0.0.1:9500
python3 -m http.server 5173            # from this directory
open http://localhost:5173/
```

## Two renews, one page

`automaticSilentRenew` has two mechanisms behind it and this spike runs both.
Which one it takes is decided by a single scope, so the toggle is a single query
parameter:

| URL | scope | how it renews | evidence for |
|---|---|---|---|
| `http://localhost:5173/` | `… offline_access` | `POST /oidc/token`, `grant_type=refresh_token` | Phase 5 |
| `http://localhost:5173/?renew=iframe` | no `offline_access` | hidden `<iframe>` at `/authorize?prompt=none` | Phase 9 |

**Without `offline_access` there is no refresh token**, and `oidc-client-ts`
falls back to the iframe. Everything else — the manager, the page, the picker,
the logout links — is the same code on both paths, which is the point: the
difference a developer actually has to reason about is one word in `scope`.

The toggle rides on `redirect_uri` and `post_logout_redirect_uri` too
(`http://localhost:5173/?renew=iframe`), so it survives the round trip through
lanyard. That works with no registration step for the same reason the port does:
lanyard's one rejection is about the redirect's **host**, and nothing else.

### The iframe path needs a callback page of its own

`silent_redirect_uri` **defaults to `redirect_uri`** — which here is `/`, whose
script completes a *redirect* login the moment it sees `code=`. Loaded inside
the renew iframe it would run `signinRedirectCallback()` against a state entry
written for the silent flow and fail. `silent-callback.html` exists for that,
calls `signinSilentCallback()` and nothing else, and renders nothing:

```
$ open 'http://localhost:5173/silent-callback.html?code=abc&state=x'   # blank, silent
$ open 'http://localhost:5173/?code=abc&state=x'                       # FAILED: No matching state found in storage
```

Every real SPA ships this file. The trap is that it *looks* optional.

### Running the iframe path against both of lanyard's names

`SameSite` compares scheme and host and **ignores the port**, so which of
lanyard's two loopback names the SPA dials decides whether the hidden iframe is
same-site or cross-site — and the shipped default is the cross-site one:

```
# same-site: app on localhost, lanyard's issuer on localhost
LANYARD_ISSUER=http://localhost:9500/oidc lanyard serve &
#   … and set `authority` in index.html to match.

# cross-site: the stock issuer, app still on localhost
lanyard serve &
```

What each of those does to the iframe's `Cookie` header is measured, not
assumed: [`docs/decisions/silent-renew-over-http.md`](../../docs/decisions/silent-renew-over-http.md).

**The only lanyard-side setup was starting it.** There is no client
registration step, no redirect allowlist to add `http://localhost:5173/` to,
and no secret to copy anywhere. `client_id: 'node-spa'` was invented in
`index.html` and lanyard has never heard of it.

`shared` is a **symlink to `../shared`**, so `python3 -m http.server` serves the
one `spikes/shared/page.html` that `dotnet-web` and `php-web` read on the server.
The SPA fetches it and fills it in the browser — same file, same markup, no copy
that can drift.

`oidc-client-ts` is **vendored** into `vendor/`, not fetched from a CDN
(v3.4.0, Apache-2.0, the published `dist/browser/oidc-client-ts.min.js`). The
spike therefore runs with no network and no `npm install`, and nothing in this
repository's login path reaches out to anybody. `python3 -m http.server` is a
static server, so `node` is not needed to *run* this either — which is the same
property criterion 29 asks of lanyard itself.

## What it is evidence of

- A public client with no `client_secret` completes the authorization code flow.
- PKCE is genuinely verified: `oidc-client-ts` sends `code_challenge_method=S256`
  and lanyard checks the verifier. Tampering with the verifier gets
  `400 invalid_grant`.
- `user.profile.email` renders, which means the ID token's claims survived the
  scope filter for `openid email profile`.
- **A renew with no redirect.** `offline_access` gets it a refresh token and
  `automaticSilentRenew` spends it: the network tab shows a `POST /oidc/token`
  with `grant_type=refresh_token`, there is no `/authorize`, no iframe, and the
  address bar never leaves `localhost:5173`. That is a different mechanism from
  `prompt=none` in a hidden iframe, which is
  [Phase 9](../../ROADMAP.md)'s subject and is untouched by this.
  `accessTokenExpiringNotificationTimeInSeconds` is lowered to 10 because the
  default is 60 and lanyard's access token *lives* 60 — at the default the
  renew would fire the instant the token arrived, or never.
- **A renew in a hidden iframe** (`?renew=iframe`). No refresh token, so
  `signinSilent()` navigates an invisible frame to `/authorize?prompt=none` and
  the whole question is whether the browser attaches `lanyard_session` to it.
  The address bar still never moves. This is the one login path that cannot be
  tested with `curl`, because only a browser decides what a third-party iframe
  navigation carries.
- **`mgr.signoutRedirect()` ends both sessions.** It reads
  `end_session_endpoint` out of the discovery document; no lanyard URL appears
  in `index.html`. Afterwards the refresh token it was holding reads
  `{"active": false}` at `/oidc/introspect`.
