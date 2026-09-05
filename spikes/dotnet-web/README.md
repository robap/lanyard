# dotnet-web — a browser login, end to end

An ASP.NET Core app with `AddOpenIdConnect` pointed at lanyard and nothing else
configured. It is Phase 4's primary acceptance client: criteria 1, 6, 26 and 28
are this app in a browser.

```
lanyard serve &          # 127.0.0.1:9500
cd spikes/dotnet-web
dotnet run               # http://localhost:5000
```

Then open <http://localhost:5000/> — **not** `/secure`. The front door is a page
with a button on it, so the redirect chain starts when you press something rather
than before you have read anything.

| Route | What it does |
|---|---|
| `/` | Landing page. *Not signed in* + a **Log in** button, or who you are + **View my claims** and **Log out** |
| `/secure` | `[Authorize]`. Renders `email:` and the full claim list. Hitting it directly still auto-challenges — that is real .NET behaviour and worth being able to see |
| `/logout` | Clears **this app's** cookie, not lanyard's. It is not a full log-out — see below |

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

## `/logout` does not log you out

It clears ASP.NET Core's own auth cookie, so the next `/secure` goes back to
lanyard — which still remembers you and signs you straight back in, with no
picker. There are two sessions and this ends one of them.

That is not a bug in this spike so much as a missing endpoint. The real thing is
one click that clears both: `SignOutAsync` over the cookie *and* OIDC schemes,
which makes `AddOpenIdConnect` redirect the browser to lanyard's
`end_session_endpoint`, which drops `lanyard_session` and sends you back here.
`/logout` becomes three lines the day lanyard advertises that endpoint —
[Phase 5](../../ROADMAP.md#phase-5--session-lifecycle-and-the-remaining-endpoints).

Until then, tick **Always ask** in the picker or restart `lanyard serve`.

## The subtraction harness

`DROP=Name,Name` omits settings so their absence is observable without a rebuild.
A setting nobody removed is not evidence of a minimum.

```
DROP=RequireHttpsMetadata dotnet run    # discovery over http:// is refused
DROP=ResponseType dotnet run            # .NET defaults to id_token; lanyard says so
DROP=ScopeEmail dotnet run              # /secure renders `email: (no email claim)`
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
