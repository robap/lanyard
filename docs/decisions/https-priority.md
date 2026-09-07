# HTTPS priority: **post-v1**

HTTPS and `lanyard trust` stay out of the v1 list. The roadmap is unchanged.

## The observation this rests on

A .NET 10.0.110 web app using `AddOpenIdConnect` completed a full interactive
browser login against a plain-HTTP issuer (`http://localhost:9400`) and rendered
the chosen persona's `email` claim, in **Google Chrome for Testing 149.0.7827.55**
and **Firefox 152.0.4**, with exactly one HTTP-related setting:
`options.RequireHttpsMetadata = false`.

Nothing had to be said about `CookieSecurePolicy`, `MinimumSameSitePolicy`, or
the correlation and nonce cookies. Full detail and the subtraction evidence:
[dotnet-http-settings.md](dotnet-http-settings.md).

PHP 8.3.6 with `jumbojett/openid-connect-php` v1.0.2 completed the same login
with **no** non-default settings at all — `setHttpUpgradeInsecureRequests(false)`
was tested and turned out not to be required
([php-spike.md](php-spike.md)).

## Why this is not simply "it worked"

CONCEPT §10 predicted .NET would set its correlation/nonce cookies `SameSite=None`,
which requires `Secure`, which requires HTTPS. **That prediction is confirmed
verbatim** — the app emitted, unprompted:

```
.AspNetCore.OpenIdConnect.Nonce.<id>=N; path=/signin-oidc; secure; samesite=none; httponly
.AspNetCore.Correlation.<id>=N;        path=/signin-oidc; secure; samesite=none; httponly
```

The login succeeded anyway because **`http://localhost` is a secure context**, so
browsers accept `Secure` cookies there. The chain CONCEPT §9 worried about is
real and fully present; it is defused by the origin, not by .NET being relaxed.

That distinction was worth measuring rather than assuming. Holding everything
else constant and moving only the app's own origin from `http://localhost:5000`
to `http://web.localtest.me:5000` reproduces the predicted failure on demand:

```
Microsoft.AspNetCore.Authentication.AuthenticationFailureException: Correlation failed.
```

So the decision is not "the `Secure` problem didn't happen." It is: **the
`Secure` problem happens exactly when the consuming app leaves localhost, and a
developer running `dotnet run` does not.**

## Why post-v1 is the right call

- **The default path is unaffected.** An app on `http://localhost:<port>` against
  lanyard on `http://localhost:9500` works today with one setting, in both browser
  families, with no divergence between them. That is lanyard's overwhelmingly
  common case and the one zero-setup startup exists to serve.
- **The failure is in the app's origin, not lanyard's address.** Nothing lanyard
  ships or configures changes it, and `lanyard trust` would not fix it — the app
  would need its *own* HTTPS. Shipping a CA in v1 would not buy the fix.
- **PHP is entirely clear**, so the constraint does not generalise across our
  stack. One runtime, one narrow condition.
- **The remaining argument for TLS is silent renew**, which CONCEPT §9 already
  names as the strongest one. That is Phase 9, and this spike deliberately did
  not test `prompt=none`. It is the right place for the question to be reopened.
  **It was, and it did not hold** — see *Revisited* below and
  [silent-renew-over-http.md](silent-renew-over-http.md).

## What this obliges v1 to do instead

Cheap, and they come out of the same evidence:

1. **Never set `Secure` on lanyard's own cookies in HTTP mode**, and keep them
   `SameSite=Lax`. CONCEPT §8 already says this; the spike is the evidence.
   Getting this wrong turns `http://lanyard:9500` into "it asks me to pick a
   persona every single time."
2. **Put "Correlation failed" in the README's troubleshooting section**, with the
   verbatim error and the actual cause — the app is being served from a
   non-localhost plain-HTTP origin. It is otherwise a genuinely opaque message,
   and CONCEPT §8 says these belong written down rather than papered over.
3. **`lanyard doctor` should notice** when the resolved issuer is a plain-HTTP
   non-localhost origin and say what that costs, rather than waiting for a
   45-minute debugging session.
4. **`examples/dotnet-web/` must set `options.ResponseType = "code"` explicitly.**
   Unrelated to HTTPS but discovered in the same pass: .NET's default is
   `id_token` — the implicit flow, which CONCEPT §4 puts out of lanyard's scope.
   Without it, the example fails against lanyard for a reason that has nothing to
   do with anything above.

## Revisited — Phase 9 ran it, and the answer is still post-v1

This section used to name Phase 9 (silent renew, `prompt=none`) as the one test
that could move HTTPS forward, on this chain: a hidden iframe makes lanyard's
session cookie third-party, which needs `SameSite=None`, which needs `Secure`.
**Phase 9 ran it in Chrome and Firefox and the chain does not close**, for a
reason nothing here predicted: on the configuration a developer is actually on,
the iframe is not third-party at all. `SameSite` ignores the port, so an app at
`http://localhost:5173` and an issuer at `http://localhost:9500` are one site,
the `Lax` cookie rides the iframe navigation, and the renew completes over plain
HTTP with no setting anywhere.

When they *are* two sites — the shipped default, whose issuer is
`http://127.0.0.1:9500` — the renew fails cleanly and `/authorize` says which of
six things went wrong. That is a naming problem with a one-line rule, not a
transport problem, and a CA would not fix it.

**The question is closed, not open: [silent-renew-over-http.md](silent-renew-over-http.md).**
It carries the traces, the counterfactual measurement of what
`SameSite=None; Secure` would have done, and the verdict in its first line.
