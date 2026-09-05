# Deliberate failure tokens — spec

**Status:** done · **Roadmap:** Phase 3 · **Slug:** 03-deliberate-failure-tokens

## Why

Testing that an API *accepts* a good token is the easy half and everybody does
it. Testing that it correctly *rejects* an expired one, a wrong-audience one, or
one signed by a key it has never seen is the half that never gets written —
because producing those tokens is annoying enough that people skip it. Six flags
turn that into six lines of shell.

- **North star 4 — real tokens, including deliberately wrong ones.** "Producing
  bad tokens is a first-class feature, not a config hack" is the roadmap's
  wording. Some competing tools can be coaxed into emitting a bad token by
  editing a realm; none advertise it as the point. This is the differentiator,
  and CONCEPT §6 puts the six-line block in the README above the login
  screenshot.
- **North star 3 — issuance is one function, and this phase is the bill.** Phase
  1 committed to hand-rolled JWS specifically so this phase could exist:
  `jws::sign` was written by hand because "every correct JWT library makes a
  genuine `alg: none` token and a deliberately broken signature impossible."
  Phase 2 fixed where the flaws attach — an extra form parameter on
  `/oidc/token`, `flaw=` on the seam's query string. Both bets get settled here.
  If a second signing path appears in this phase, both were wrong.
- **The CLI still does not sign.** `--bad-signature` is a one-line string edit if
  the CLI does it locally, and that one line is a second code path that will
  drift from the server's. The CLI *asks the server for a broken token*, over the
  same `client_credentials` grant it already uses. Everything below is server
  behavior; the flags are how you spell it.
- **Phase 2 found the number that makes this phase honest.** `AddJwtBearer`'s
  `ClockSkew` defaults to five minutes — measured boundary 363 seconds
  ([dotnet-jwt-bearer-settings.md](../decisions/dotnet-jwt-bearer-settings.md)).
  A token that expired thirty seconds ago is accepted by a stock .NET API. An
  `--expired` flag that produces one is a negative test that passes for the wrong
  reason, which is worse than no flag at all.

## In scope

- **Six flaws, one per named failure mode**: `expired`, `wrong-aud`,
  `wrong-iss`, `bad-signature`, `alg-none`, `unknown-kid`.
- **`POST /oidc/token`** grows one form parameter, `flaw`. The Phase 2 contract
  is otherwise untouched.
- **`POST /_/api/token`** grows one query parameter, `flaw`, with the same six
  values and the same meanings. The body still means claims.
- **`lanyard token` and `lanyard env`** grow six boolean flags — `--expired`,
  `--wrong-aud`, `--wrong-iss`, `--bad-signature`, `--alg-none`,
  `--unknown-kid` — mutually exclusive with each other.
- **`jws::sign` grows a header/signature mode**, because three of the six are not
  expressible as claims. This is the reason Phase 1 did not use a JWT library.
- **`spikes/dotnet-api/failure-tokens.sh`** — the roadmap's "six lines of shell,
  run as one script", pointed at the Phase 2 harness's `/orders`. Good → 200,
  six flags → 401, one line of output per case.
- **`docs/decisions/dotnet-jwt-bearer-settings.md` grows a section** recording
  what .NET says to each of the six — the harness already prints
  `TOKEN REFUSED: <ExceptionType>: <message>`, so the six reasons are one run
  away and they are the troubleshooting table for anyone whose RP is refusing a
  token they think is good.
- **README** gains the block, and its Scope paragraph stops saying "no
  deliberately-wrong tokens yet".

## Out of scope

- **A seventh flaw.** `nbf` in the future, `typ` wrong, a missing `sub`, `at_hash`
  wrong (CONCEPT §6 wants that one, but it needs Phase 4's ID token to exist),
  RS512-when-you-said-RS256, a symmetric HS256 confusion token. Six is the
  roadmap's list and the README's block. The seventh arrives when something
  observed asks for it.
- **Composing flaws.** One per token — see the first open question.
- **`none` in the discovery document.** `id_token_signing_alg_values_supported`
  stays `["RS256"]`. lanyard does not *support* unsigned tokens as a mode; it
  produces one when asked to produce a broken one. Advertising `none` invites an
  SDK to negotiate it, which is a different and much worse feature.
- **Rotating or adding a second signing key** so `unknown-kid` can name a real
  one. The unknown kid is a string that is in no JWKS anywhere; key rotation is
  not on the roadmap at all.
- **Logging the flaw.** Nothing prints a per-request line today, and the request
  log is Phase 6. The flag is explicit at the call site; the server stays quiet.
- **A flaw on the browser flow** (`/authorize`) or on ID tokens. Phase 4 has no
  ID token yet; when it does, the same `flaw` parameter is where it attaches.
- **`--ttl`**, still. `--expired` is the sanctioned way to get a token outside
  its lifetime, and it is a fixed shape rather than an arbitrary number.

## Behavior

### One parameter, three surfaces, six values

`flaw=<value>` on `POST /oidc/token` (form) and on `POST /_/api/token` (query).
The CLI flag is the wire value with dashes and a `--` in front, which makes the
table below the only thing anyone has to remember.

| Flag | `flaw=` | What is wrong | What is still right |
|---|---|---|---|
| `--expired` | `expired` | `iat` = `nbf` = now − 3600, `exp` = now − 3540 | signature, `kid`, `iss`, `aud`, `sub` |
| `--wrong-aud` | `wrong-aud` | `aud` becomes `wrong-<requested>` | signature, `kid`, `iss`, lifetime |
| `--wrong-iss` | `wrong-iss` | `iss` becomes `https://wrong-issuer.example.test` | signature, `kid`, `aud`, lifetime |
| `--bad-signature` | `bad-signature` | the low bit of the signature's last byte is flipped | header byte-for-byte, all claims |
| `--alg-none` | `alg-none` | header is `{"alg":"none","typ":"JWT"}`, signature segment empty | all claims |
| `--unknown-kid` | `unknown-kid` | header `kid` is `lanyard-unknown-kid` | RS256, real signature, all claims |

**Each flaw breaks exactly one thing.** That is the whole design constraint. A
token that is both expired and wrongly signed tells you nothing about which check
your resource server ran, because it stops at the first one. So `--expired` is
correctly signed with the real `kid`; `--unknown-kid` is signed by the real key
and carries perfect claims; `--bad-signature` keeps a byte-identical header so
key lookup succeeds and the failure lands on the signature.

**`--expired` is expired by an hour, not by a second.** Phase 2 measured a stock
`AddJwtBearer` accepting a lanyard token for 363 seconds — `exp` plus five
minutes of default `ClockSkew`. An hour clears that, clears every other default
tolerance we know of, and is still obviously "expired" rather than "ancient" when
a human reads the claims. `iat` and `nbf` move with `exp`, because a token issued
now that expired an hour ago is a shape no IdP produces and some libraries reject
for the wrong reason. The lifetime stays 60 seconds; the whole token is shifted
back.

**`wrong-<requested>` rather than a fixed sentinel.** `--aud billing-api
--wrong-aud` yields `wrong-billing-api`, which cannot collide with what was
asked for, needs no collision check, and reads correctly in the refusal:
`Audience validation failed. Audiences: 'wrong-billing-api'. Did not match:
validationParameters.ValidAudience: 'billing-api'`. With no audience requested it
is `wrong-audience`; if `aud` is present but not a string (the seam's body can
post an array), it is replaced wholesale with `wrong-audience`.

**`.test` for the wrong issuer.** A reserved TLD that is guaranteed not to
resolve, matching the personas' `ada@example.test`, so a client that tries to
fetch discovery from the token's `iss` fails immediately instead of reaching
something on the internet.

**`--unknown-kid` is a real signature the RP cannot find the key for.** That
makes it a genuine test of whether the relying party honours `kid` at all: a
library that ignores `kid` and tries every key in the JWKS will *accept* this
token. Finding that out about your stack is the point. `jose` does not, and
reports a key-lookup failure distinct from a signature failure.

**Observed:** stock .NET `AddJwtBearer` **accepts** it — `200`, `TOKEN OK:
sub=ada`. `jose`, against the same JWKS, refuses it with `JWKSNoMatchingKey`.
Two correct-looking libraries disagree about the same token, which is exactly
the disagreement this flag exists to surface.

**`--alg-none` is an unsecured JWT, not RS256 with a rewritten header** (the
roadmap says so explicitly). Three segments, the third empty:
`<header>.<payload>.` — RFC 7515 §A.5's compact serialization. There is no `kid`
in the header, because a token with no signature has no key.

### The flaw is applied last, and it wins

Order inside `issue()`: persona claims, registered claims, the caller's overrides
(the seam's body, the endpoint's `audience`/`scope`/`client_id`), then the flaw.
Phase 1 wrote "the body wins over everything, `iss` and `exp` included… that is
what keeps Phase 3's failure flags sugar over this function"; this phase makes
the flaw the one thing that outranks it. `flaw=expired` returning a live token
because the body happened to carry an `exp` is the failure mode this whole phase
exists to prevent — a negative test that quietly passes. Anyone who wants an
exact `exp` can still post one, without a flaw.

### An unrecognized `flaw` is a refusal, not an ignored parameter

Phase 2 established that unknown *parameters* on `/oidc/token` are ignored, per
OAuth's own rule. A known parameter with an unrecognized *value* is different:
`flaw=expried` silently minting a perfectly good token means a test suite that
reports six passes and proves nothing. Both surfaces answer `400` in the OAuth
shape, naming the value and listing the six:

```json
{ "error": "invalid_request",
  "error_description": "unknown flaw \"expried\"; expected one of expired, wrong-aud, wrong-iss, bad-signature, alg-none, unknown-kid" }
```

At the CLI, two flags at once is a usage error from `clap` — non-zero exit,
nothing on stdout, as with every other CLI failure.

### What each surface returns

`/oidc/token` keeps the Phase 2 OAuth response exactly, with one honesty fix:
**`expires_in` is the token's actual remaining life, floored at zero**, so
`flaw=expired` reports `0` rather than claiming 60 seconds it does not have. No
new field; an SDK reading this response should see nothing unusual, because the
whole point is that the token looks ordinary until it is validated.

The seam is a test fixture rather than an SDK surface, so it does echo what it
was asked for: the `{"token", "claims"}` response gains `"flaw": "<value>"` when
one was applied. For the three claim-level flaws, `claims` shows the damage —
the past `exp`, the `wrong-billing-api` — because the seam's contract is that
`claims` is what was signed. For the three header-level flaws `claims` is
untouched and correct, which is exactly true and is why the echoed `flaw` field
earns its place: it is the only way a fixture can assert it got the token it
asked for.

The JWKS does not change. Nothing about minting a flawed token mutates server
state, so `/oidc/jwks` still carries exactly one key, the real one, afterwards.

### The six lines

```sh
API=http://127.0.0.1:5080/orders
code() { curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $1" $API; }

code "$(lanyard token --as ada --aud billing-api)"                  # 200
code "$(lanyard token --as ada --aud billing-api --expired)"        # 401
code "$(lanyard token --as ada --aud billing-api --wrong-aud)"      # 401
code "$(lanyard token --as ada --aud billing-api --wrong-iss)"      # 401
code "$(lanyard token --as ada --aud billing-api --bad-signature)"  # 401
code "$(lanyard token --as ada --aud billing-api --alg-none)"       # 401
code "$(lanyard token --as ada --aud billing-api --unknown-kid)"    # 401
```

This is the README block and `spikes/dotnet-api/failure-tokens.sh` is the same
thing with a pass/fail line per case. It is also the shape Phase 11 wants in
every `examples/*/test.sh`, so getting it right here is getting it right eleven
times.

## Acceptance criteria

Every run sets `LANYARD_DATA_DIR` and a throwaway `XDG_CONFIG_HOME`, as in Phase
2. `jose` verification means `scripts/jose-verify.mjs` against the **live JWKS
URL**. The .NET harness is `spikes/dotnet-api/` on `http://127.0.0.1:5080`,
`ClockSkew = Zero` unless stated.

**Against a real resource server**

- [x] 1. `spikes/dotnet-api/failure-tokens.sh` against a running harness prints
      `200` for the good token and `401` for each of the six, and exits `0`. One
      script, seven lines of output.

      **Corrected from the observation.** Five of the six are `401`;
      `--unknown-kid` is `200`, because stock `AddJwtBearer` does not require the
      header's `kid` to resolve and finds the real key anyway. The script expects
      `200` there, prints the deviation on that line, and takes
      `UNKNOWN_KID_STATUS=401` for an RP that does honour `kid` — `jose` does.
      The token was **not** additionally corrupted to force a refusal: a token
      that breaks two things at once says nothing about which check ran, and that
      is the design constraint the whole phase rests on. Written up in
      [dotnet-jwt-bearer-settings.md](../decisions/dotnet-jwt-bearer-settings.md);
      it is the most interesting thing this phase found.
- [x] 2. The harness's own log carries a `TOKEN OK: sub=ada` for the good token
      and a `TOKEN REFUSED: <ExceptionType>: <message>` line for each of the six.
      Those six lines are pasted verbatim into
      `docs/decisions/dotnet-jwt-bearer-settings.md` under a new section. Where
      two flaws produce the same exception type, that is recorded as observed
      rather than hidden.
- [x] 3. With `CLOCK_SKEW=default dotnet run` — .NET's real five minutes —
      `--expired` **still** returns `401`. This is the criterion the hour-long
      shift exists for; a token expired by seconds returns `200` here, and Phase
      2 already measured why.
- [x] 4. The block exactly as it appears in `README.md`, copy-pasted into a shell
      unedited, produces `200` then six `401`s. **Corrected from the same
      observation as criterion 1:** against a stock .NET API it produces `200`,
      five `401`s, then `200` for `--unknown-kid`. The block's comments say so
      on that line, and the paragraph under it explains what a `200` there tells
      you about your stack.

**Against `jose`**

- [x] 5. Each flag through `scripts/jose-verify.mjs` exits `1` with its own
      error, and a good token still exits `0` printing its payload:

      | flag | observed `ERROR <name>: <message>` |
      |---|---|
      | `--expired` | `JWTExpired: "exp" claim timestamp check failed` |
      | `--wrong-aud` | `JWTClaimValidationFailed: unexpected "aud" claim value` |
      | `--wrong-iss` | `JWTClaimValidationFailed: unexpected "iss" claim value` |
      | `--bad-signature` | `JWSSignatureVerificationFailed: signature verification failed` |
      | `--alg-none` | `JOSENotSupported: Unsupported "alg" value for a JSON Web Key Set` |
      | `--unknown-kid` | `JWKSNoMatchingKey: no applicable key found in the JSON Web Key Set` |

      **Observed** against `jose` 6.x on 2026-09-04, not predicted: every name
      in the original table was confirmed, so nothing needed correcting.

      Six lines, six distinct outcomes — `wrong-aud` and `wrong-iss` share a
      class and are told apart by the claim named in the message, which is what
      the roadmap's "distinct, correct error for each" requires. If an observed
      name differs from this table, the table is wrong and gets corrected from
      the observation.

**The shape of each flawed token**

- [x] 6. `--alg-none`: segment 1 base64url-decodes to exactly
      `{"alg":"none","typ":"JWT"}` — no `kid`, no `RS256` anywhere — the token
      splits into three parts on `.`, and the third is the empty string. Not an
      RS256 token with a rewritten header: there are no signature bytes at all.
- [x] 7. `--unknown-kid`: the header's `alg` is `RS256`, its `kid` is
      `lanyard-unknown-kid`, and that string does not appear in
      `curl -s .../oidc/jwks`. The signature segment decodes to 256 bytes.
- [x] 8. `--bad-signature`: the header segment is **byte-identical** to that of a
      good token minted the same way, the payload decodes to JSON with
      `sub: "ada"`, and the signature decodes to 256 bytes — so `jose` reaches
      the signature check and fails there (criterion 5), rather than failing at
      key lookup or on a malformed token.
- [x] 9. `--expired`: the decoded payload has `nbf == iat`, `exp - iat == 60`,
      and `now - exp` between 3500 and 3600.
- [x] 10. `--wrong-aud` with `--aud billing-api` → payload `aud` is
      `wrong-billing-api`; without `--aud` → `wrong-audience`. `iss` matches the
      discovery document's `issuer` in both.
- [x] 11. `--wrong-iss`: payload `iss` is `https://wrong-issuer.example.test`,
      `aud` is still `billing-api`, and the header `kid` is the real one from
      `/oidc/jwks`.

**Both surfaces, still one function**

- [x] 12. `curl -sX POST '.../_/api/token?persona=ada&flaw=wrong-aud' -d '{"aud":"billing-api"}'`
      → `claims.aud` is `wrong-billing-api`, the response carries
      `"flaw": "wrong-aud"`, and the returned token's decoded payload equals the
      returned `claims` object exactly.
- [x] 13. `curl -s -X POST .../oidc/token -d grant_type=client_credentials -d persona=ada -d audience=billing-api -d flaw=expired`
      → `200` whose body has `expires_in: 0`, no `flaw` field, and an
      `access_token` whose `exp` is in the past.
- [x] 14. Claims from `lanyard token --as ada --aud billing-api --wrong-iss` and
      from the seam with `?persona=ada&flaw=wrong-iss` and body
      `{"aud":"billing-api"}`, both decoded and passed through
      `jq -S 'del(.iat,.nbf,.exp,.jti,.client_id)'`, `diff` to empty. Same for
      `--wrong-aud`. Two callers, one claim set, flaws included.
- [x] 15. Phase 2's criterion 15 still passes unchanged: the same diff with no
      flaw on either side is still empty.
- [x] 16. `curl -s .../oidc/jwks | jq '.keys | length'` is `1` and the `kid` is
      unchanged after minting all six. Flaws mint; they do not mutate.

**Refusals and ergonomics**

- [x] 17. `-d flaw=expried` on `/oidc/token` → `400`, `"error":"invalid_request"`,
      description containing `expried` and all six valid values, and **no**
      `access_token` in the body. `?flaw=expried` on the seam → the same `400`
      shape. The server is still serving afterwards.
- [x] 18. `lanyard token --as ada --expired --wrong-aud > /tmp/t 2>/tmp/e`
      exits non-zero, `/tmp/t` is 0 bytes, and `/tmp/e` names both flags.
- [x] 19. `lanyard token --help` lists all six flags; `lanyard env --help` lists
      the same six; `eval "$(lanyard env --as ada --aud billing-api --expired)"`
      sets `BEARER_TOKEN` to a token whose `exp` is in the past.
- [x] 20. Unregressed: `lanyard token --as ada --aud billing-api` with no flaw
      → `200` at `/orders`, and `POST /oidc/token` with no `flaw` parameter
      returns `expires_in: 60`.

## Open questions

None blocking. Six decisions in **Behavior** go beyond what the roadmap states —
listed here because they are the ones worth disagreeing with before a plan
exists:

1. **One flaw per token; the flags are mutually exclusive.** Each flag isolates
   one rejection reason, and a resource server reports only the first check it
   fails — so `--expired --wrong-aud` tests strictly less than either flag alone
   while costing conflict rules, a precedence order, and a wire encoding for
   sets. If it is ever wanted, `flaw` becomes space-delimited like `scope` and
   nothing else changes. Additive on purpose.
2. **`--expired` is a fixed one-hour shift, not a tunable.** The number comes
   from Phase 2's measured 363-second .NET boundary. A tunable here is `--ttl`
   with a different name, and `--ttl` is on the roadmap's *explicitly deferred,
   possibly forever* list.
3. **`aud` becomes `wrong-<requested>`** rather than a fixed sentinel like
   `urn:lanyard:wrong`. Collision-proof without a check, and self-explanatory in
   the refusal message.
4. **The flaw is applied after the caller's overrides and wins over them**,
   reversing Phase 1's "the body wins over everything" for this one input.
5. **The seam echoes `flaw`; `/oidc/token` does not.** The seam is a fixture and
   needs to confirm it got what it asked for — especially for the three
   header-level flaws, where the claims look perfect. The OAuth response stays
   the shape an SDK expects, with `expires_in: 0` as its only tell.
6. **An unknown `flaw` value is a `400`**, even though Phase 2 ignores unknown
   parameters. The alternative silently mints a good token for a typo'd negative
   test.
