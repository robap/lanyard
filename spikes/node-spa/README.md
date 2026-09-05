# node-spa — a public client, with PKCE, and no secret

A single static page using [`oidc-client-ts`](https://github.com/authts/oidc-client-ts)
as a **public client** against lanyard. It is the third of Phase 4's three
acceptance clients, and the one that exercises `code_challenge_method=S256`.

```
lanyard serve &                        # 127.0.0.1:9500
python3 -m http.server 5173            # from this directory
open http://localhost:5173/
```

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
- **`mgr.signoutRedirect()` ends both sessions.** It reads
  `end_session_endpoint` out of the discovery document; no lanyard URL appears
  in `index.html`. Afterwards the refresh token it was holding reads
  `{"active": false}` at `/oidc/introspect`.
