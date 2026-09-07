# dotnet-web — a browser login, end to end

An ASP.NET Core app with `AddOpenIdConnect` pointed at lanyard and nothing else
configured. It is Phase 4's primary acceptance client and Phase 5's: criteria 1,
6, 26 and 28 of one and 1, 6 and 26 of the other are this app in a browser.

```
lanyard serve &          # 127.0.0.1:9500
cd spikes/dotnet-web
dotnet run               # http://localhost:5000

# Or point it at a lanyard reachable by another name — Phase 8's one-name setup:
LANYARD_AUTHORITY=http://lanyard:9500/oidc dotnet run
```

Then open <http://localhost:5000/> — **not** `/secure`. The front door is a page
with a button on it, so the redirect chain starts when you press something rather
than before you have read anything.

| Route | What it does |
|---|---|
| `/` | The whole page. *Not signed in* + a **Log in** button, or who you are, the claim table, and the two log-out buttons |
| `/secure` | `[Authorize]`, then a redirect back to `/`. It is the **login button's destination** — the 401 is what makes `AddOpenIdConnect` build the redirect to lanyard. Hitting it directly still auto-challenges, which is real .NET behaviour and worth being able to see |
| `/logout` | **The real log-out.** `SignOutAsync` over the cookie *and* OIDC schemes — see below |
| `/logout-local` | Clears **this app's** cookie only, and nothing of lanyard's. It is what makes the two-sessions problem visible |

The page itself is [`../shared/page.html`](../shared/page.html), read at runtime
and shared with `php-web` and `node-spa`. Nothing about it is .NET's.

## The whole configuration

```csharp
options.Authority = "http://127.0.0.1:9500/oidc";
options.RequireHttpsMetadata = false;   // lanyard serves HTTP
options.ClientId = "billing-web";       // invented here; lanyard has never heard of it
options.ClientSecret = "unchecked";     // accepted, and checked by nobody
options.ResponseType = "code";          // .NET's default is id_token — see below
options.SaveTokens = true;
options.Scope.Add("email");
options.Scope.Add("profile");
```

Two of those lines are the ones people lose an afternoon to:

- **`RequireHttpsMetadata = false`** — without it .NET refuses to fetch discovery
  over `http://`. This is the *only* setting a plain-HTTP provider needs, measured
  by subtraction in [`../../docs/decisions/https-priority.md`](../../docs/decisions/https-priority.md).
- **`ResponseType = "code"`** — `AddOpenIdConnect` defaults to `id_token`, the
  implicit flow, which lanyard does not implement. Omit this line and you get
  `unsupported_response_type` with an `error_description` that names the setting.

**This app really does send `response_mode=form_post`**, even with
`ResponseType = "code"`. Watch the `Location` header on `/secure`'s challenge. It
is why lanyard implements that mode, and why the login still completes with
JavaScript switched off — the auto-submitting page carries a `<noscript>` button.

## Two log-outs, and the difference is the point

```csharp
// /logout — both sessions
await ctx.SignOutAsync(CookieAuthenticationDefaults.AuthenticationScheme);
await ctx.SignOutAsync(OpenIdConnectDefaults.AuthenticationScheme,
    new AuthenticationProperties { RedirectUri = "/" });

// /logout-local — this app's cookie only
await ctx.SignOutAsync(CookieAuthenticationDefaults.AuthenticationScheme);
```

**There is no lanyard URL anywhere in this file.** The second `SignOutAsync`
makes `AddOpenIdConnect` read `end_session_endpoint` out of the discovery
document and build the redirect itself, adding `id_token_hint` because
`SaveTokens = true` kept one and using its own `SignedOutCallbackPath`
(`/signout-callback-oidc`) as the `post_logout_redirect_uri`. The chain you see
in the network tab is:

```
GET 302  localhost:5000/logout
GET 302  127.0.0.1:9500/oidc/end_session?post_logout_redirect_uri=…&id_token_hint=…&state=…
GET 302  localhost:5000/signout-callback-oidc?state=…
GET 200  localhost:5000/
```

**`/logout-local` ends one of the two sessions**, so the next login goes back to
lanyard — which still remembers you and signs you straight back in, with no
picker. That is not a bug; it is what an SSO session is, and it is the row of the
table you need in order to see the difference the real log-out makes.

## The subtraction harness

`DROP=Name,Name` omits settings so their absence is observable without a rebuild.
A setting nobody removed is not evidence of a minimum.

```
DROP=RequireHttpsMetadata dotnet run    # discovery over http:// is refused
DROP=ResponseType dotnet run            # .NET defaults to id_token; lanyard says so
DROP=ScopeEmail dotnet run              # the claim table has no email row at all
DROP=ClientSecret dotnet run            # completes anyway: lanyard checks no secret
```

`DROP=ScopeEmail` is the one worth running twice. lanyard filters the ID token by
granted scope exactly the way a production provider does, so dropping the scope
really does drop the claim — a mock that handed over `email` unasked would be the
mock-diverges-from-production bug this project exists not to be.

Every response's `Set-Cookie` headers are echoed to stdout. That is a diagnostic,
not an auth setting: it is how the `SameSite` and `Secure` attributes on .NET's
correlation and nonce cookies get observed rather than assumed.

## If it says `Correlation failed`

The app is being served from an origin that is not a secure context, so the
browser dropped .NET's correlation cookie. Serve it from `http://localhost:5000`
or `http://127.0.0.1:5000`. See the main [README's troubleshooting
section](../../README.md#troubleshooting).
