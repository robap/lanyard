# .NET as a resource server — the minimal settings, and the clock skew

**Phase 2 acceptance harness.** Everything below was observed by running a real
.NET minimal API against a real `lanyard serve` and watching real HTTP status
codes, not reasoned about. The resource-server counterpart to Phase 0's
[dotnet-http-settings.md](dotnet-http-settings.md).

## What was run

| | |
|---|---|
| .NET SDK | 10.0.110 (`x86_64`, Pop!_OS 24.04) |
| Package | `Microsoft.AspNetCore.Authentication.JwtBearer` 10.0.11 |
| Provider | `lanyard serve` at `http://127.0.0.1:9500`, issuer `http://127.0.0.1:9500/oidc` |
| App | `AddJwtBearer`, one `/orders` endpoint with `RequireAuthorization()`, at `http://127.0.0.1:5080` |
| Token | `lanyard token --as ada --aud billing-api` — a real `client_credentials` grant, 60-second TTL |

The harness is [`spikes/dotnet-api/`](../../spikes/dotnet-api/). It keeps two
runtime knobs so both halves of every claim below come from one build:
`DROP=Name,Name` omits a setting, `CLOCK_SKEW=zero|default` picks the tolerance.

## The minimal set

Three settings. Each has an observed failure when removed; nothing else was
needed at all.

```csharp
builder.Services
    .AddAuthentication(JwtBearerDefaults.AuthenticationScheme)
    .AddJwtBearer(options =>
    {
        options.Authority = "http://127.0.0.1:9500/oidc";
        options.RequireHttpsMetadata = false;
        options.Audience = "billing-api";
    });
```

| Setting | Failure when removed |
|---|---|
| `options.Authority` | App starts, `/orders` → `401`. `SecurityTokenInvalidIssuerException: IDX10204: Unable to validate issuer. validationParameters.ValidIssuer is null or whitespace AND validationParameters.ValidIssuers is null or empty.` The handler has no metadata at all — no issuer, no keys. |
| `options.RequireHttpsMetadata = false` | App does not start. `System.InvalidOperationException: The MetadataAddress or Authority must use HTTPS unless disabled for development by setting RequireHttpsMetadata=false.` |
| `options.Audience` | App starts, `/orders` → `401`. `SecurityTokenInvalidAudienceException: IDX10208: Unable to validate audience. validationParameters.ValidAudience is null or whitespace and validationParameters.ValidAudiences is null.` |

**`Audience` is not optional the way it looks.** `ValidateAudience` defaults to
`true`, so leaving `Audience` unset does not mean "accept any audience" — it
means "reject everything". A developer who omits it because their tokens have no
`aud` gets a 401 with a message about `validationParameters`, which reads like a
bug in the provider.

Nothing had to be said about `TokenValidationParameters` beyond `ClockSkew`
below, about `MetadataAddress`, or about signing keys: the JWKS is resolved from
the discovery document.

### `AddJwtBearer` accepts a discovery document with no `authorization_endpoint`

**This answers the open question Phase 1 left.** lanyard's document advertises
only what exists, and at Phase 2 that is `jwks_uri` and `token_endpoint`. The
handler's own `ConfigurationManager` consumed it without complaint:

```
DISCOVERY OK: issuer=http://127.0.0.1:9500/oidc jwks_uri=http://127.0.0.1:9500/oidc/jwks
              signing_keys=1 authorization_endpoint=(absent)
              token_endpoint=http://127.0.0.1:9500/oidc/token
```

So `authorization_endpoint` stays out of the document until Phase 4 implements
it. No field was added defensively.

## ClockSkew — the finding this phase existed to catch

**`TokenValidationParameters.ClockSkew` defaults to five minutes.** A
60-second lanyard token is therefore accepted by a default-configured .NET API
for about six, and the roadmap's "the same call 90 seconds later returns 401"
would quietly fail — or worse, quietly pass for the wrong reason once somebody
waited long enough.

Same token, same API, same lanyard. Only `ClockSkew` moved:

| `ClockSkew` | `/orders` at t+0 | at t+92s |
|---|---|---|
| `TimeSpan.Zero` | `200` | **`401`** — `WWW-Authenticate: Bearer error="invalid_token", error_description="The token expired at '09/05/2026 02:32:19'"` |
| .NET's default (5 min) | `200` | **`200`** |

The clearest demonstration is one token across a restart: a token that had just
been refused `401` by the zero-skew build was accepted `200` by the default-skew
build seconds later, with nothing else changed.

**The measured boundary is 363 seconds** — polled every 5s until refusal, which
is `exp` (mint + 60s) plus the 300-second skew. Not derived from the constant;
watched.

So any .NET API used to check that lanyard's short lifetimes are real must set:

```csharp
options.TokenValidationParameters.ClockSkew = TimeSpan.Zero;
```

This is README material rather than a footnote. A developer who cannot make
short-TTL rejection happen locally will conclude lanyard's TTLs are fake, and
will be wrong — their resource server is being generous, and it is being generous
by five minutes in a tool whose whole point is 60-second tokens.

It is also the resource-server half of CONCEPT §8's clock-drift discussion: the
same tolerance that makes a container with a drifting clock work is what makes
expiry unobservable.

## Two smaller observations

- **Audience mismatch is a clean 401.** `lanyard token --as ada --aud not-a-real-api`
  mints successfully — any `aud` mints, that is north star 1 — and `/orders`
  answers `401` with `IDX10214: Audience validation failed`. Rejection happens at
  the API, which is exactly where it should.
- **.NET remaps `sub`.** With `MapInboundClaims` at its default, the `sub` claim
  arrives as `ClaimTypes.NameIdentifier`
  (`http://schemas.xmlsoap.org/ws/2005/05/identity/claims/nameidentifier`), not as
  `sub`. Not a lanyard concern, but it is the first thing that looks broken when
  someone reads `User.FindFirst("sub")` and gets null.
