# lanyard

A local OIDC provider with nothing to configure.

`lanyard serve` starts an OpenID Connect provider on `127.0.0.1:9500` with a
working discovery document, a stable signing key and three ready-made users. No
realm to create, no client to register, no admin console to visit.
`lanyard token --as ada --aud billing-api` prints a bearer token you can paste
into a `curl`, and pointing a web application at the issuer gets you a login
screen that is a list of people with no password field.

## Logging in

Point any OIDC client at `http://127.0.0.1:9500/oidc`, invent a `client_id`, and
sign in. There is nothing to register.

1. Your app redirects to `/oidc/authorize`.
2. lanyard redirects to its persona picker at `http://127.0.0.1:9500/_/`.
3. You click a person. No password, no consent screen.
4. lanyard redirects back to your app with a `code`, and your app exchanges it
   at `/oidc/token` for an access token and an ID token.

The next login from the same application in the same browser skips step 3 —
lanyard remembers who you picked, **per `client_id`**, so three apps on three
ports can be signed in as three different people at once. Closing the browser
resets it, and so does restarting lanyard.

**Want to see it working before wiring up your own app?**
[`spikes/`](spikes/) holds four small applications — .NET, PHP, a
no-build-step SPA, and a resource server — with a five-minute walkthrough that
gets all of them logged in as different people against one running lanyard.

A minimal .NET client, which is the whole configuration:

```csharp
builder.Services.AddAuthentication(/* … */)
    .AddCookie()
    .AddOpenIdConnect(options =>
    {
        options.Authority = "http://127.0.0.1:9500/oidc";
        options.RequireHttpsMetadata = false;   // lanyard serves HTTP
        options.ResponseType = "code";          // .NET defaults to id_token
        options.ClientId = "billing-web";       // invented here, unknown to lanyard
        options.Scope.Add("email");
        options.Scope.Add("profile");
    });
```

## Build and run

```
cargo build --release
./target/release/lanyard serve
```

```
lanyard 0.1.0
  Issuer    → http://127.0.0.1:9500/oidc
  UI        → http://127.0.0.1:9500/_/
  Listening → 127.0.0.1:9500
  Data dir  → /home/you/.local/share/lanyard
  Signing   → kid TXntCt2biz2Bj578hZocZOb2A2nQV9JfrBvFN55QWpU
  Personas  → built-in defaults (ada, mira, nobody)
```

Point your application's OIDC configuration at the issuer and it will find the
rest on its own:

```
curl -s http://127.0.0.1:9500/oidc/.well-known/openid-configuration | jq .
curl -s http://127.0.0.1:9500/oidc/jwks | jq .
```

## Read this before you use it

**The default signing key is public and forgeable.** It is committed to this
repository on purpose, so that `kid` and `n` above are the same on every machine
and a wiped data directory does not invalidate a cached JWKS. Anyone with a
checkout can mint a token your application will accept. Never expose lanyard to
an untrusted network, and never point a staging or production service at it.

`POST /_/api/token` and `POST /oidc/token` compound this: between them they mint
any claims for anyone who can reach the port, with no authentication at all. That
is the entire point — a test seam and a provider with no client registry — and it
is why the default bind is loopback. `LANYARD_BIND=0.0.0.0` turns that off, so do
it only on a network you control.

**The issuer is `http://127.0.0.1:9500/oidc`**, path segment included, and it
does not follow the `Host` header. Reaching lanyard at `http://localhost:9500`
still yields a document that says `127.0.0.1`, and a relying party that compares
the `iss` claim against the URL it dialled will reject the token. That mismatch —
not a bug, a deliberate refusal to let a request header decide who the issuer is
— is what `LANYARD_ISSUER` exists for. Set it to whatever name your application
will actually use:

```
LANYARD_ISSUER=http://lanyard:9500/oidc lanyard serve
```

**The discovery document lists only the endpoints that exist.** Today that is
`jwks_uri`, `token_endpoint`, `authorization_endpoint` and `userinfo_endpoint`;
there is no `end_session_endpoint` yet, because advertising an endpoint that
returns 404 sends a client down a path that cannot work. The document grows each
phase, and it is corrected when it turns out to have promised something:
`response_types_supported` is now exactly `["code"]`, because advertising
`id_token` would tell a .NET app that its *default* setting is supported and
then refuse it at request time.

**Any `aud` mints.** `--aud not-a-real-api` is a token, not an error. Audiences
follow the same no-registration rule as everything else, and an audience your API
does not expect is a test case worth having. Rejection happens at the API, not
here.

**A 60-second token stays valid for about six minutes against a stock .NET API.**
`TokenValidationParameters.ClockSkew` defaults to **five minutes**, so a
default-configured `AddJwtBearer` keeps accepting a lanyard token long after it
expired. If you cannot make short-TTL rejection happen locally you will conclude
lanyard's lifetimes are fake; they are not, your resource server is being
generous. One line fixes it:

```csharp
options.TokenValidationParameters.ClockSkew = TimeSpan.Zero;
```

Measured both ways in [`docs/decisions/dotnet-jwt-bearer-settings.md`](docs/decisions/dotnet-jwt-bearer-settings.md).

**Scope.** This is Phase 4. There is a discovery document, a JWKS, both halves
of `/oidc/token`, `/oidc/authorize`, `/oidc/userinfo`, the persona picker at
`/_/`, browser sessions, the `token` and `env` CLI commands, the six deliberate
failure flags, and the test seam. There is no `refresh_token` (even when
`offline_access` is requested), no `/end_session`, no `/introspect`, no
`/revoke`, no request log, and no consent screen — a consent screen is client
registration in a different costume.

## Configuration

Every setting is an environment variable read once at startup. All are optional.

| Variable | Default | Meaning |
|---|---|---|
| `LANYARD_ISSUER` | `http://127.0.0.1:{port}/oidc` | The `iss` claim and the discovery document's `issuer` |
| `LANYARD_BIND` | `127.0.0.1` | Listen address |
| `LANYARD_PORT` | `9500` | Listen port |
| `LANYARD_DATA_DIR` | `$XDG_DATA_HOME/lanyard` | Signing key lives here |
| `LANYARD_PERSONAS` | `$XDG_CONFIG_HOME/lanyard/users.yaml` | Persona file; when set, it must exist |
| `LANYARD_URL` | `http://127.0.0.1:{port}` | Where `lanyard token` and `lanyard env` reach the server |

`LANYARD_URL` is an address; `LANYARD_ISSUER` is a string that goes in a token.
They are deliberately separate knobs. Set the issuer to `http://lanyard:9500/oidc`
for a container network and that name resolves nowhere useful from your shell — a
CLI that derived its target from the issuer would be unreachable in exactly the
setup the issuer setting exists to support.

On first run the data directory is created containing `signing-key.pem` (mode
`0600`, seeded from the built-in default key) and a `.gitignore`. Replace
`signing-key.pem` with your own 2048-bit RSA key to get a signing key that is not
public. A `signing-key.pem` that cannot be parsed is fatal — lanyard will not
silently fall back to the default, because a token signed by a key you did not
expect is the most confusing failure this tool can produce.

## Personas

With no persona file, three users ship:

| id | name | email | roles |
|---|---|---|---|
| `ada` | Ada Bell | `ada@example.test` | `admin`, `user` |
| `mira` | Mira Okonkwo | `mira@example.test` | `user` |
| `nobody` | — | — | — |

`nobody` is the useful one. A token for `nobody` carries `sub` and the registered
claims and nothing else — no `email`, no `name`, no `roles`, no
`preferred_username`. It is the user who breaks applications and the one nobody
remembers to create.

Point `LANYARD_PERSONAS` at a YAML file to replace all three:

```yaml
client: billing-web          # optional, file-level. Reserved for a later phase.
personas:
  - id: ada
    name: Ada Bell
    email: ada@example.test
    roles: [admin, user]
    attributes:              # anything here becomes a top-level claim
      department: platform
  - id: ops
    name: Ops Bot
    client: ops-console      # optional, per-persona
```

The file replaces the built-ins entirely rather than merging with them. A
malformed file — an unknown key, a duplicate `id`, a missing `id` — makes `serve`
exit non-zero naming the offending key and the file path. There is no fallback to
the defaults: a persona that silently failed to load shows up three redirects
later as a name missing from the picker.

`client:` is accepted at both levels, echoed back by `/_/api/personas`, and read
by nothing yet.

## Minting a token from the command line

Probably the larger half of daily use, and the shorter path to being useful:
there is no redirect dance to get right.

```
$ lanyard token --as ada --aud billing-api
eyJhbGciOiJSUzI1NiIsImtpZCI6...

$ curl -H "Authorization: Bearer $(lanyard token --as ada --aud billing-api)" \
       localhost:8080/orders
```

`lanyard token` prints the token and a newline on stdout and nothing else — no
banner, no timing, no "minted for Ada" — so `$(...)` drops it straight into a
header. On failure **stdout stays empty**, the message goes to stderr and the
exit code is non-zero, because an API receiving the words "connection refused" as
a credential is worse than an API receiving nothing.

```
$ lanyard token --as ada
lanyard: nothing listening at http://127.0.0.1:9500 — is `lanyard serve` running?
```

`lanyard env` prints the same token shaped for `eval`:

```
$ eval "$(lanyard env --as ada --aud billing-api)"
$ echo $BEARER_TOKEN
eyJhbGciOiJSUzI1NiIsImtpZCI6...
```

The variable name is fixed at `BEARER_TOKEN`. When minting fails, `lanyard env`
prints nothing at all on stdout, so `eval "$(...)"` is a no-op that leaves your
shell exactly as it was.

| Flag | Default | Meaning |
|---|---|---|
| `--as` | *required* | Persona to mint for |
| `--aud` | none | The `aud` claim. Any value mints |
| `--scope` | none | Space-delimited, becomes the `scope` claim |
| `--url` | `$LANYARD_URL`, else `http://127.0.0.1:9500` | Where lanyard is listening |
| `--expired` | off | Mint a token that expired an hour ago |
| `--wrong-aud` | off | `aud` becomes `wrong-<requested>` |
| `--wrong-iss` | off | `iss` becomes `https://wrong-issuer.example.test` |
| `--bad-signature` | off | One bit of the signature flipped |
| `--alg-none` | off | An unsecured JWT: `alg: none`, no signature |
| `--unknown-kid` | off | A real signature under a `kid` in no JWKS |

The six failure flags are mutually exclusive with each other and work on `env`
too. See [Deliberately wrong tokens](#deliberately-wrong-tokens).

The CLI does not sign locally. It performs a real `client_credentials` grant
against `/oidc/token`, so the token arrives the way a production token arrives —
through the endpoint an SDK would use. Tokens are 60 seconds and the TTL is not
configurable.

## Deliberately wrong tokens

Testing that your API *accepts* a good token is the easy half. Six flags produce
the tokens that test the other half — each one structurally valid and
semantically wrong in exactly one way:

```sh
API=http://127.0.0.1:5080/orders
code() { curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $1" $API; }

code "$(lanyard token --as ada --aud billing-api)"                  # 200
code "$(lanyard token --as ada --aud billing-api --expired)"        # 401
code "$(lanyard token --as ada --aud billing-api --wrong-aud)"      # 401
code "$(lanyard token --as ada --aud billing-api --wrong-iss)"      # 401
code "$(lanyard token --as ada --aud billing-api --bad-signature)"  # 401
code "$(lanyard token --as ada --aud billing-api --alg-none)"       # 401
code "$(lanyard token --as ada --aud billing-api --unknown-kid)"    # 401 — or 200; see below
```

| Flag | What is wrong | What is still right |
|---|---|---|
| `--expired` | `iat` = `nbf` = an hour ago, `exp` 60s after that | signature, `kid`, `iss`, `aud`, `sub` |
| `--wrong-aud` | `aud` becomes `wrong-<what you asked for>` | signature, `kid`, `iss`, lifetime |
| `--wrong-iss` | `iss` becomes `https://wrong-issuer.example.test` | signature, `kid`, `aud`, lifetime |
| `--bad-signature` | one bit of the signature is flipped | header byte-for-byte, all claims |
| `--alg-none` | header is `{"alg":"none","typ":"JWT"}`, no signature at all | all claims |
| `--unknown-kid` | header `kid` is `lanyard-unknown-kid`, which is in no JWKS | RS256, a real signature, all claims |

**Each flaw breaks exactly one thing**, and that is the whole design. A resource
server stops at the first check it fails, so a token that is both expired and
wrongly signed tells you nothing about which check ran. The flags are mutually
exclusive for the same reason: `--expired --wrong-aud` is a usage error.

**`--expired` is expired by an hour, not by a second**, so it is refused even by
a resource server with a generous clock tolerance — including a stock .NET API,
whose default is five minutes.

**If `--unknown-kid` prints `200`, that is a finding about your stack, not a
bug.** The signature on that token is real; only the `kid` names a key that does
not exist. A library that honours `kid` refuses it — `jose` does, with
`JWKSNoMatchingKey`. A library that falls back to trying every key in the JWKS
accepts it, and **stock .NET `AddJwtBearer` does exactly that** (observed;
`docs/decisions/dotnet-jwt-bearer-settings.md`). Which half of that you are on
is worth knowing, and this flag is how you find out.

Nothing about minting a flawed token changes server state: `/oidc/jwks` still
carries the one real key afterwards. `spikes/dotnet-api/failure-tokens.sh` is
this block with a `PASS`/`FAIL` line per case.

## The authorization endpoint

`GET` or `POST /oidc/authorize`. This is where every other provider keeps a
client registry, a redirect allowlist and a consent screen. lanyard has none of
the three.

| Parameter | Handling |
|---|---|
| `client_id` | **Required.** Any value. It keys the browser session and becomes the ID token's `aud` |
| `redirect_uri` | **Required.** Any value whose host is loopback — [the one rejection](#the-one-rejection) |
| `response_type` | `code` only. Anything else is `unsupported_response_type` |
| `response_mode` | `query` (default) or `form_post`. `fragment` is `unsupported_response_mode` |
| `scope` | Any. `openid` decides whether an ID token is issued; `email` and `profile` gate claims |
| `state` | Echoed back byte-for-byte when sent, absent when not. Never invented |
| `nonce` | Carried into the ID token when sent |
| `code_challenge` / `code_challenge_method` | `S256` or `plain`. An omitted method is `plain` (RFC 7636). Absent means no PKCE |
| `prompt` | `login`, `select_account`, `none`, or absent. Anything else is ignored |
| `max_age` | Compared against the remembered selection's `auth_time` |
| `audience` / `resource` | Becomes the **access token's** `aud`, as on the `client_credentials` grant |
| `client_secret` | Accepted anywhere it is offered and checked nowhere |
| anything else | Ignored |

`client_id` is required not because lanyard knows it but because a session has to
be keyed on something, and an app that omits it would share a session with every
other app.

Then `grant_type=authorization_code` at [the token endpoint](#the-token-endpoint):

| Parameter | Required | Meaning |
|---|---|---|
| `code` | yes | Single-use, and it lives **60 seconds** |
| `code_verifier` | when a challenge was sent | Genuinely verified |
| `redirect_uri` | no | When sent, it must match the one `/authorize` received |
| `client_id` / `client_secret` | no | Never validated |

```json
{ "access_token": "eyJ…", "token_type": "Bearer", "expires_in": 60,
  "scope": "openid email profile", "id_token": "eyJ…" }
```

`id_token` is present **only when the authorization request's scope contained
`openid`**. Without it this is a plain OAuth 2.0 code flow and gets a plain
OAuth 2.0 response.

`invalid_grant` is a `400`, and its `error_description` says which of the six
things went wrong: the code is unknown, expired, or already exchanged; the
`redirect_uri` does not match; the `code_verifier` is missing; the
`code_verifier` does not match.

**PKCE really is verified, and single-use really is enforced.**
Accept-everything is about *registration* — who you say you are, where you say
you want to come back to. It was never about skipping the cryptography. A local
provider that rubber-stamps a wrong `code_verifier` lets a broken PKCE
implementation ship, and production is a bad place to find that out.

### The one rejection

A `redirect_uri` is accepted when its scheme is `http` or `https` **and its host
is `localhost`, any `*.localhost` name, any `127.0.0.0/8` address, or `[::1]`** —
any port, any path, any query. Everything else is refused.

The host is compared **literally, with no DNS lookup**, so
`http://web.localtest.me:5000/cb` is rejected even though it resolves to
127.0.0.1. A rule enforced by resolution depends on the network, is cacheable, is
different inside a container than outside one, and stops being one line to
explain. (`web.localtest.me` is also where a .NET app's own login breaks anyway,
for an unrelated reason — see [Troubleshooting](#troubleshooting).) Custom
schemes for native apps (`com.example.app:/cb`) and `urn:ietf:wg:oauth:2.0:oob`
are rejected too; when device and native flows come up they bring their own
phase.

A rejected `redirect_uri`, or a missing `client_id`, renders a **`400` at
lanyard** — `text/html`, no `Location` header, the browser stays where it is, and
the page names the value and states the rule. RFC 6749 §4.1.2.1 is explicit that
a server must not redirect an error to an address it has just decided not to
trust; doing so would make the one rejection decorative. Every *other* error
redirects, carrying `error`, `error_description` and the `state` that was sent,
so your app's own error handling runs.

## The persona picker

`http://127.0.0.1:9500/_/` — a list of people, no password field.

- One form per person, so the flow works with **JavaScript disabled**. The only
  script lanyard ships in the login path is the auto-submit on a
  `response_mode=form_post` response, and that page carries a visible button
  inside `<noscript>`.
- **Mint one now** — a panel taking `sub`, `name`, `email`, comma-separated
  `roles`, and a textarea of extra claims as JSON. Submitting it signs you in as
  an identity that is in no file. A one-off is deliberately **not remembered**:
  the next login from that application shows the picker again.
- **Always ask** — a checkbox that forces the picker for every `client_id` in
  this browser. It is on the picker and on the no-login version of the page, so
  it can be turned off without starting a login.
- Visiting `/_/` with no login in progress lists the same people and says so.
  That is the URL the banner prints.

Persona display strings are HTML-escaped. A persona file is your own, but its
`attributes` can come out of a fixture generator, and a picker that executes its
own persona list is a bad look for a tool whose pitch is "it catches your bugs".

The page is server-rendered HTML with one stylesheet inlined by `include_str!`.
No template engine, no bundler, no `npm`, nothing generated at build time:
`cargo build --release` on a machine with no `node` on `PATH` produces a binary
that serves a styled picker, and the page requests nothing from anywhere.
`prefers-color-scheme` handles dark mode.

## Sessions

One cookie:

```
lanyard_session=<opaque>; Path=/; HttpOnly; SameSite=Lax
```

**No `Secure`**, ever, while lanyard serves HTTP. A browser silently drops a
`Secure` cookie sent over `http://`, and the symptom is "it asks me to pick a
persona every single time" with nothing anywhere saying why. **No `Max-Age` or
`Expires`** either: it is a browser-session cookie, so closing the browser is a
working reset.

Server-side the cookie maps to a record holding, **per `client_id`**, the persona
chosen and the moment it was chosen. That per-`client_id` scoping is the whole
multi-project property: log in to one app as Ada and another as Mira in the same
browser, and neither disturbs the other.

Two resets, both worth knowing:

- **Close the browser.** The cookie has no expiry, so it goes.
- **Restart `lanyard serve`.** Sessions, pending authorization requests and
  authorization codes all live in memory and all die with the process.

The picker appears anyway when "always ask" is set, when `prompt=login` or
`prompt=select_account` is sent, or when `max_age` is sent and the remembered
selection is older than it. `prompt=none` **renders nothing, ever** — it returns
a code if there is a usable remembered selection and redirects with
`error=login_required` if there is not. That is not a claim that silent renew
works: lanyard's `Lax` cookie is not sent on a third-party iframe navigation, so
a real SPA renew will get `login_required`. It is a promise that lanyard never
puts a login screen somewhere nobody can click it.

## Scope, and what it filters

| Scope requested | Claims added |
|---|---|
| always | `sub` |
| `email` | `email`, `email_verified` |
| `profile` | `name`, `preferred_username` |
| always | `roles`, and the persona's arbitrary `attributes` |

`roles` and `attributes` are unscoped because OIDC defines no scope for them, and
hiding your own custom claims behind a standard scope would be lanyard inventing
a rule.

**This filter applies to the ID token and to `/userinfo`. It does not apply to
the access token, and that asymmetry is deliberate.** OIDC Core §5.4 specifies
claims-per-scope for exactly those two. Nothing specifies it for an access token
— an access token is not even required to be a JWT, and its `scope` is an
authorization grant for the resource server to read, not a claims filter. So:
filtered where it is specified, unchanged where it is not.

## The ID token

Built by the same function, from the same persona→claims table, as every other
token lanyard mints.

| Claim | Value |
|---|---|
| `iss` | the configured issuer |
| `sub` | the persona id |
| `aud` | **the `client_id`** — not the API audience |
| `iat`, `exp` | now, and now + **300** |
| `auth_time` | when the persona was picked, which may predate `iat` on a remembered session |
| `nonce` | echoed when the request sent one |
| `at_hash`, `c_hash` | base64url of the leftmost 128 bits of `SHA-256` of the access token and of the code |
| persona claims | filtered by scope, per the table above |

**The ID token lives 300 seconds while the access token still lives 60.** They
are not the same kind of thing. The access token is a credential and its short
life is the point — a 60-second access token that gets rejected is lanyard
working. The ID token is an authentication receipt, consumed once at login and
then exchanged for your app's own cookie; a 60-second one makes `oidc-client-ts`
consider the user expired seconds after signing in. It also carries `nbf` and
`jti`, because every lanyard token does; both are legal registered claims and
stripping them would mean a second claim path.

## `/oidc/userinfo`

`GET` or `POST`, with `Authorization: Bearer <access_token>`. Returns the same
scope-filtered persona claims the ID token carried, `sub` always present, and
none of `iss` / `aud` / `exp` / `nonce` / the hashes.

```
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:9500/oidc/userinfo | jq .
```

**This is the second place lanyard says no.** Access tokens are verified as
lanyard's own — signature, `iss` and `exp`, and nothing else, because there is no
client to check. A missing, malformed, unsigned, foreign or expired token gets
`401` with `WWW-Authenticate: Bearer error="invalid_token"`. The reason is PKCE's
reason: an app that reads `/userinfo` with an expired token has a bug, and a mock
that answers anyway hides it.

## Cross-origin requests

The `/oidc` endpoints answer a cross-origin `fetch`: the response echoes the
request's `Origin`, allows `authorization` and `content-type`, and answers
preflight. A single-page app on `http://localhost:5173` can therefore read
discovery and exchange its code without a proxy.

**Credentials are never allowed cross-origin.** A public client authenticates
with a bearer token, not with lanyard's cookie, and allowing credentialed
cross-origin requests would let any page you happen to visit drive your session.
`/_/` and the test seam answer no other origin at all.

## The token endpoint

`POST /oidc/token`, `application/x-www-form-urlencoded`. This is what the CLI
calls, and what an SDK doing client credentials will find in the discovery
document.

```
curl -X POST http://127.0.0.1:9500/oidc/token \
     -d grant_type=client_credentials -d persona=ada -d audience=billing-api
```

```json
{ "access_token": "eyJhbGciOiJSUzI1NiIs…", "token_type": "Bearer", "expires_in": 60 }
```

| Parameter | Required | Meaning |
|---|---|---|
| `grant_type` | yes | `client_credentials` or `authorization_code`. Anything else is `400 unsupported_grant_type` |
| `persona` | no | A loaded persona id. Unknown ids are a `400` naming the id |
| `audience` | no | The `aud` claim. `resource` (RFC 8707) is a synonym; `audience` wins |
| `scope` | no | Space-delimited. Becomes the `scope` claim and is echoed in the response |
| `client_id` / `client_secret` | no | Never validated. `client_id` becomes a claim when sent |
| `flaw` | no | One of `expired`, `wrong-aud`, `wrong-iss`, `bad-signature`, `alg-none`, `unknown-kid`. An unrecognized value is a `400`, not an ignored parameter |

**Client authentication is accepted in any form and checked in none.** HTTP
Basic, form parameters, or nothing at all — all four combinations mint. This is
the endpoint where every other provider would put a client registry, and it is
what lets three services on three ports share one running instance with no setup
between them.

The parameters above are the `client_credentials` grant's; `authorization_code`
takes [its own set](#the-authorization-endpoint). A `client_credentials` response
carries no `id_token` — client credentials has no user authentication event to
attest to — and neither grant issues a `refresh_token`. Unknown parameters are
ignored. Errors are `400` with `{"error", "error_description"}`.

## The test seam

`POST /_/api/token` mints a token directly, with no browser and no redirects.
**The body is the claims. The query string is how they are minted.**

```
curl -sX POST http://127.0.0.1:9500/_/api/token \
     -H 'content-type: application/json' \
     -d '{"sub":"ada","aud":"billing-api"}'
```

```json
{ "token": "eyJhbGciOiJSUzI1NiIs…", "claims": { "sub": "ada", "…": "…" } }
```

| Query parameter | Default | Meaning |
|---|---|---|
| `persona` | none | Mint a persona's claims: `?persona=ada` |
| `ttl` | `60` | Lifetime in seconds; `exp - iat` |
| `flaw` | none | One of the six. Echoed back as `"flaw"`, and an unrecognized value is a `400`, not an ignored parameter |

Body claims override persona claims, and override the registered claims too —
`iss` and `exp` included — so you can mint a deliberately wrong token:

```
curl -sX POST 'http://127.0.0.1:9500/_/api/token?persona=ada' \
     -H 'content-type: application/json' -d '{"iss":"http://evil.test"}'
```

`GET /_/api/personas` lists what is loaded. An unknown persona, a body that is
not a JSON object, or a non-numeric `ttl` returns `400` with
`{"error", "error_description"}`.

## Troubleshooting

**.NET: `Exception: Correlation failed.`** — the app is being served from an
origin that is not a secure context, so the browser dropped ASP.NET Core's
correlation cookie. .NET sets that cookie `SameSite=None`, and `SameSite=None`
*requires* `Secure`, and a `Secure` cookie is silently discarded over plain HTTP
on a host that is not `localhost`. It is not lanyard rejecting anything: the
authorization response came back fine and .NET could not find its own cookie to
match it against.

Serve your app from `http://localhost:<port>` or `http://127.0.0.1:<port>`, both
of which browsers treat as secure contexts. `http://web.localtest.me:5000` looks
equivalent and is not — which is the other reason lanyard rejects that host.

**.NET: the picker never appears, and you get
`unsupported_response_type` instead.** `AddOpenIdConnect`'s default
`ResponseType` is `id_token`, which is the implicit flow. lanyard implements the
authorization code flow only, and says so in the error:

```csharp
options.ResponseType = "code";
```

lanyard's `error_description` names that setting, but only when the requested
`response_type` actually contained `id_token` — which is the one case where it is
the diagnosis rather than a guess about which stack is calling.

**.NET: a 60-second token is still accepted six minutes later.** See
[`ClockSkew`](#read-this-before-you-use-it) above.

**"It asks me to pick a persona every single time."** Either "always ask" is on
in the picker, or your app is sending `prompt=login`, or the cookie is not coming
back. lanyard's cookie is deliberately never `Secure`; if you have put a proxy in
front of lanyard that rewrites cookies, that is where to look.

**A relying party rejects the token with an issuer mismatch.** Set
`LANYARD_ISSUER` to the name your application actually dials. See
[Read this before you use it](#read-this-before-you-use-it).

## Verifying a token

Tokens are checked by a real JWT library rather than by lanyard agreeing with
itself. `scripts/` holds a small [`jose`](https://github.com/panva/jose) harness
that resolves the JWKS over HTTP exactly as a relying party would:

```
cd scripts
npm install
node jose-verify.mjs "$TOKEN" http://127.0.0.1:9500/oidc billing-api
```

Node is a verification dependency only. lanyard itself is a single static binary
with no runtime to install.

## License

MIT
