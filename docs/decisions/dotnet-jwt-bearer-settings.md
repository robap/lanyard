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

## What .NET says to each of the six deliberate failure tokens

**Phase 3.** Same harness, same build, `ClockSkew = Zero`, `Audience =
"billing-api"`. Every token below is `lanyard token --as ada --aud billing-api
--<flag>`; the lines are `spikes/dotnet-api/failure-tokens.sh`'s output and the
harness's own `OnTokenValidated`/`OnAuthenticationFailed` log, pasted verbatim.

```
PASS  (good token)     200
PASS  --expired        401
PASS  --wrong-aud      401
PASS  --wrong-iss      401
PASS  --bad-signature  401
PASS  --alg-none       401
PASS  --unknown-kid    200  ← accepted: this RP does not honour kid
```

```
TOKEN OK: sub=ada
TOKEN REFUSED: SecurityTokenExpiredException: IDX10223: Lifetime validation failed. The token is expired. ValidTo (UTC): '9/5/2026 2:16:30 AM', Current time (UTC): '9/5/2026 3:15:30 AM'.
TOKEN REFUSED: SecurityTokenInvalidAudienceException: IDX10214: Audience validation failed. See https://aka.ms/identitymodel/app-context-switches
TOKEN REFUSED: SecurityTokenInvalidIssuerException: IDX10205: Issuer validation failed. Issuer: 'https://wrong-issuer.example.test'. Did not match: validationParameters.ValidIssuer: 'null' or validationParameters.ValidIssuers: 'null' or validationParameters.ConfigurationManager.CurrentConfiguration.Issuer: 'http://127.0.0.1:9500/oidc'. For more details, see https://aka.ms/IdentityModel/issuer-validation. 
TOKEN REFUSED: SecurityTokenInvalidSignatureException: IDX10511: Signature validation failed. Keys tried: 'Microsoft.IdentityModel.Tokens.RsaSecurityKey, KeyId: 'TXntCt2biz2Bj578hZocZOb2A2nQV9JfrBvFN55QWpU', InternalId: 'TXntCt2biz2Bj578hZocZOb2A2nQV9JfrBvFN55QWpU'. , KeyId: TXntCt2biz2Bj578hZocZOb2A2nQV9JfrBvFN55QWpU\n'. 
TOKEN REFUSED: SecurityTokenInvalidSignatureException: IDX10504: Unable to validate signature, token does not have a signature: '[PII of type 'Microsoft.IdentityModel.Logging.SecurityArtifact' is hidden. For more details, see https://aka.ms/IdentityModel/PII.]'.
TOKEN OK: sub=ada
```

The seventh line is `--unknown-kid`, and it says `TOKEN OK`.

| flag | status | exception | the first check that failed |
|---|---|---|---|
| *(none)* | `200` | — | none; `TOKEN OK: sub=ada` |
| `--expired` | `401` | `SecurityTokenExpiredException` | `IDX10223` lifetime |
| `--wrong-aud` | `401` | `SecurityTokenInvalidAudienceException` | `IDX10214` audience |
| `--wrong-iss` | `401` | `SecurityTokenInvalidIssuerException` | `IDX10205` issuer |
| `--bad-signature` | `401` | `SecurityTokenInvalidSignatureException` | `IDX10511` signature |
| `--alg-none` | `401` | `SecurityTokenInvalidSignatureException` | `IDX10504` no signature at all |
| `--unknown-kid` | **`200`** | — | **none — accepted** |

**Two flaws share an exception type, and that is recorded rather than tidied
away.** `--bad-signature` and `--alg-none` are both
`SecurityTokenInvalidSignatureException`, told apart only by the `IDX` code:
`IDX10511` is "I tried the key and the bytes did not match", `IDX10504` is
"there are no bytes to try". `jose` distinguishes them by class
(`JWSSignatureVerificationFailed` vs `JOSENotSupported`); .NET does not.

### The finding: `AddJwtBearer` accepts a token whose `kid` is in no JWKS

`--unknown-kid` returned **`200`**, and the harness logged `TOKEN OK: sub=ada`.
`lanyard-unknown-kid` appears nowhere in `/oidc/jwks` — which carries exactly one
key, `TXntCt2biz2Bj578hZocZOb2A2nQV9JfrBvFN55QWpU` — and the request finished in
3.3 ms, so nothing stalled refetching metadata either.

This is not a lanyard bug and not a harness misconfiguration. The signature on
that token is **real**: it is made by the real key, and only the header's `kid`
names a key that does not exist. Microsoft.IdentityModel does not require the
`kid` to resolve — when it does not match, it falls back to trying the keys it
has, one of which verifies. `jose`, pointed at the same JWKS, refuses the same
token with `JWKSNoMatchingKey: no applicable key found in the JSON Web Key Set`.

**Two correct-looking libraries disagree about the same token, and that is the
entire reason the flag exists.** A stack that ignores `kid` cannot detect a
token signed by a retired or rotated key, because it will keep trying every key
it holds until one works. If that matters to you, you now know which half of
your stack you have; if it does not, you know that too. What you cannot do is
find this out from a provider that only mints good tokens.

Consequences for anyone reading this table:

- `failure-tokens.sh` expects `200` here, with the deviation printed on the line
  and `UNKNOWN_KID_STATUS=401` available for an RP that does honour `kid`.
- The spec's acceptance criterion 1 was written expecting six `401`s. It was
  corrected from this observation rather than the other way round; the token was
  **not** additionally corrupted to force a refusal, because a token that breaks
  two things at once tells you nothing about which check ran, and that property
  is the whole design.

### `--expired` survives .NET's real five minutes

The criterion the hour-long shift exists for. With `CLOCK_SKEW=default dotnet
run` — the 300 seconds measured above, not this harness's zero:

```
TOKEN REFUSED: SecurityTokenExpiredException: IDX10223: Lifetime validation failed. The token is expired. ValidTo (UTC): '9/5/2026 2:17:08 AM', Current time (UTC): '9/5/2026 3:16:08 AM'.
```

`401`, with a good token still `200` seconds later on the same build. A token
expired by 30 seconds returns `200` here; one expired by an hour does not, which
is why `--expired` is a fixed one-hour shift and not a tunable.

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
