# .NET over plain HTTP — the minimal settings

**Phase 0 validation spike.** Everything below was observed by completing a real
interactive login in a real browser, not reasoned about.

## What was run

| | |
|---|---|
| .NET SDK | 10.0.110 (`ubuntu.24.04-x64`, Pop!_OS 24.04) |
| Package | `Microsoft.AspNetCore.Authentication.OpenIdConnect` 10.0.11 |
| Provider | `oidc-provider-mock` 0.4.6 via `uvx`, issuer `http://localhost:9400` |
| App | `dotnet new web`, `AddCookie()` + `AddOpenIdConnect()`, served at `http://localhost:5000` |
| Persona | `--user-claims '{"sub":"ada","email":"ada@example.test","name":"Ada Bell"}'` |

Driven with Selenium against the two locally installed browsers (the Chrome MCP
was unavailable this session; see the plan's Progress notes).

| Browser | Version | Result |
|---|---|---|
| Google Chrome for Testing | 149.0.7827.55 | Login completed, page rendered `email: ada@example.test` |
| Firefox | 152.0.4 | Login completed, page rendered `email: ada@example.test` |

Screenshots in [`evidence/`](evidence/): `*-1-authorize.png` is the provider's
identity picker with Ada on it, `*-2-result.png` is the app rendering the claim
after the redirect back.

**No divergence between the two.** The `SameSite` disagreement the spec expected
to matter did not arise, because .NET does not leave the cookies unmarked — it
marks them explicitly (see below), and both browsers honour that mark the same way.

## The minimal set

Four settings. Each has an observed failure when removed; nothing else survived
subtraction.

| Setting | Failure when removed |
|---|---|
| `options.Authority = "http://localhost:9400"` | `System.InvalidOperationException: Provide Authority, MetadataAddress, Configuration, or ConfigurationManager to OpenIdConnectOptions` |
| `options.RequireHttpsMetadata = false` | `System.InvalidOperationException: The MetadataAddress or Authority must use HTTPS unless disabled for development by setting RequireHttpsMetadata=false.` |
| `options.ClientId = "spike"` | `System.ArgumentNullException: Value cannot be null. (Parameter 'ClientId')` |
| `options.ResponseType = "code"` | Provider rejects the request: `Error: unsupported_response_type — The response type 'id_token' is not supported by the server.` |

### Removed — no failure, so not minimal

| Setting | What happened without it |
|---|---|
| `options.ClientSecret` | Login completed unchanged. The handler used PKCE (`code_challenge_method=S256`) and the provider accepted the token request with no client authentication. |
| `options.SaveTokens = true` | Login completed unchanged; only affects whether tokens are stashed in the auth properties. |
| `options.Scope.Add("profile")` | Login completed and `name = Ada Bell` still arrived. |
| `options.Scope.Add("email")` | Login still completed, but the page rendered `email: (no email claim)`. Kept for the acceptance criterion, not for the protocol. |

### `ResponseType` is the surprising one

**.NET's default `ResponseType` is `id_token`, not `code`** — the implicit flow.
A .NET app pointed at a provider with otherwise-default settings will send
`response_type=id_token` and be rejected by any provider that does not implement
implicit.

CONCEPT §4 puts implicit explicitly out of scope for lanyard. So this is a
standing bit of lanyard's own compatibility surface, not a quirk of the mock:

- The `examples/dotnet-web/` example (CONCEPT §11) **must** set
  `options.ResponseType = "code"` explicitly, and say why in a comment.
- lanyard's error for an unsupported `response_type` should name the setting —
  something closer to "response_type=id_token is not supported; set
  `options.ResponseType = \"code\"`" than a bare `unsupported_response_type`.
  This is a cheap, high-value diagnostic for the stack we care most about.

## What the cookies actually looked like

The app logged every `Set-Cookie` it emitted. Values elided:

```
SET-COOKIE /secure :: .AspNetCore.OpenIdConnect.Nonce.<id>=N; expires=…; path=/signin-oidc; secure; samesite=none; httponly
SET-COOKIE /secure :: .AspNetCore.Correlation.<id>=N;        expires=…; path=/signin-oidc; secure; samesite=none; httponly
SET-COOKIE /signin-oidc :: .AspNetCore.Cookies=<value>;      path=/; samesite=lax; httponly
```

**CONCEPT §10's prediction is confirmed verbatim.** .NET does set its correlation
and nonce cookies `SameSite=None`, and therefore also `Secure`, with no
configuration asking it to. It does this because it uses
`response_mode=form_post`, which makes the callback a cross-site POST.

The login nevertheless succeeded over plain HTTP, and the reason matters:
**`http://localhost` is a secure context**, so browsers accept a `Secure` cookie
there. This is exactly the distinction CONCEPT §8 draws. Nothing had to be said
about `CookieSecurePolicy` or `MinimumSameSitePolicy` — the defaults were fine
because of the origin, not because .NET was relaxed.

## The boundary, measured

Because the whole login rides on a `Secure` cookie that only works by the
localhost exception, the obvious question is what happens one step off localhost.

Same app, same settings, same provider still at `http://localhost:9400` — only
the **app's own origin** moved from `http://localhost:5000` to
`http://web.localtest.me:5000` (a plain hostname that resolves to loopback, so
it is not a secure context):

```
Microsoft.AspNetCore.Authentication.AuthenticationFailureException:
  An error was encountered while handling the remote login.
 ---> Microsoft.AspNetCore.Authentication.AuthenticationFailureException: Correlation failed.
```

The provider was happy — the browser arrived at `/signin-oidc?code=…&state=…`.
The browser had simply refused to store the `Secure` correlation cookie on a
non-secure origin, so there was nothing to correlate against. The browser's
cookie jar for that origin was empty.

**"Correlation failed" is reproducible on demand, and the variable is the
consuming app's origin — not lanyard's address and not HTTP-vs-HTTPS as such.**

Two consequences for lanyard:

- **Never set `Secure` on lanyard's own cookies in HTTP mode.** CONCEPT §8
  already says this; this is the evidence for it. lanyard's persona session
  cookie must be `SameSite=Lax` and unmarked, or a developer who follows §8's
  advice to use `http://lanyard:9500` will be asked to pick a persona forever.
- **A .NET app served from a non-localhost HTTP origin cannot complete an OIDC
  login at all**, against lanyard or anything else. That is a fact about the app,
  not about lanyard, and it belongs in the README's Docker-gotchas section next
  to §8 — with this exact error text, since "Correlation failed" is otherwise
  deeply unhelpful.

Not tested: lanyard itself on a non-localhost HTTP origin while the app stays on
localhost. Lower risk, because lanyard controls its own cookie flags and the
bullet above pins them; worth a check when Phase 9 (silent renew) lands, since an
iframe re-introduces the third-party cookie problem this spike sidestepped.
