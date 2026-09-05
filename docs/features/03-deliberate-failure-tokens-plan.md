# Deliberate failure tokens — plan

**Status:** done · **Spec:** [03-deliberate-failure-tokens-spec.md](03-deliberate-failure-tokens-spec.md) · **Roadmap:** Phase 3

## Approach

One new type, `oidc::flaw::Flaw`, is the only place the six names exist. It
parses from the wire (`Flaw::parse`), renders the `400` description, and is
threaded to the two places damage can be done: `issue::claims_at` for the three
claim-level flaws and `jws::sign` for the three header/signature-level ones.
Every surface — `/oidc/token`, the seam, the CLI — is a caller that turns a
string into a `Flaw` and passes it along. **No surface builds a flawed token
itself**; if one does, north star 3 is lost in the phase that was supposed to
settle it, and step 11's equivalence test is what fails.

Four shapes are settled deliberately:

- **`jws::sign` takes `Option<Flaw>` rather than a private `Mode` enum of its
  own.** A second vocabulary means a mapping table between them, and a mapping
  table is the seam two names drift through. `jws` already documents that Phase 3
  is why it hand-rolls JWS; it is allowed to know the word.
- **The flaw is applied as a fourth step inside `claims_at`, after the
  overrides** — reversing Phase 1's "the body wins over everything" for this one
  input (spec, open question 4). `expired` therefore *overwrites* `iat`/`nbf`/
  `exp` rather than shifting the `now` fed into step 2, so a body that posted its
  own `exp` cannot resurrect the token. **North star 4** — a negative test that
  quietly passes is worse than no flag.
- **`issue()` returns the unix time it used.** `expires_in` becomes
  `exp − issued_at`, floored at zero, and reading the clock a second time in
  `token.rs` would make the ordinary case report `59` or `61` at random. The
  return type becomes a small `Issued { token, claims, issued_at }` struct rather
  than a third tuple element.
- **The CLI still does not sign.** The six flags collapse to one `Option<&str>`
  on `MintRequest` and become one more form field on the grant it already posts.
  A local string edit for `--bad-signature` would be the second signing path this
  whole phase exists to not have.

## Files

- `src/oidc/flaw.rs` — **new.** The `Flaw` enum, `parse`, the wire names, the
  `400` description, and the constants (`EXPIRED_SHIFT`, the wrong issuer, the
  unknown kid)
- `src/oidc/mod.rs` — declare `flaw`
- `src/oidc/issue.rs` — `issue`/`claims_at` take `Option<Flaw>`; step 4 applies
  it; `Issued` replaces the returned tuple
- `src/oidc/jws.rs` — `sign` takes `Option<Flaw>`: the `alg: none` header, the
  unknown `kid`, the flipped signature bit
- `src/oidc/token.rs` — `flaw` form parameter, its `400`, honest `expires_in`
- `src/seam.rs` — `flaw` query parameter, its `400`, `"flaw"` echoed in the
  response
- `src/client.rs` — `MintRequest.flaw`, sent as a form field
- `src/main.rs` — six mutually exclusive flags on `MintArgs`, on both `token`
  and `env`
- `tests/issue.rs` — flaw precedence over a posted `exp`; the shape of each
  claim set
- `tests/http.rs` — both surfaces, the `400`s, the echo, the equivalence diffs,
  JWKS unchanged after minting all six
- `tests/cli.rs` — the six flags, mutual exclusion, `--help`, `env`
- `spikes/dotnet-api/failure-tokens.sh` — **new.** The seven-case script
- `spikes/dotnet-api/README.md` — name the script
- `docs/decisions/dotnet-jwt-bearer-settings.md` — new section: what .NET says to
  each of the six, verbatim
- `README.md` — the six-line block, the flag table, both parameter tables, the
  corrected Scope paragraph

## Risks & unknowns

- **`--unknown-kid` may return `200` from .NET, not `401`.** This is the largest
  risk in the phase and it is structural, not a bug: Microsoft.IdentityModel has
  historically fallen back to trying every key in the JWKS when `kid` does not
  match (`TryAllIssuerSigningKeys`), and the signature on this token is *real*.
  The spec anticipates the behavior — "a library that ignores `kid` and tries
  every key will accept this token; finding that out about your stack is the
  point" — but acceptance criterion 1 demands six `401`s. **If it returns `200`:
  record the observation verbatim in the decision note, and take it back to the
  spec. Do not "fix" it by also corrupting the signature** — that makes the token
  break two things at once and destroys the one property the whole design rests
  on. It may also make .NET refetch the JWKS, which shows up as latency and extra
  log lines rather than a failure.
- **`--alg-none`'s header must decode to exactly `{"alg":"none","typ":"JWT"}`.**
  `serde_json` without `preserve_order` emits object keys sorted, and `alg` < `typ`
  sorts correctly by luck rather than by design. Assert the exact byte string in
  a unit test so a future `serde_json` feature flag cannot quietly break
  criterion 6.
- **`--bad-signature`'s header must be byte-identical to a good token's.** It is
  only byte-identical if the flaw is applied to the signature *after* signing,
  never by re-serializing anything. Flip the bit on the signature bytes before
  base64, not on the encoded string.
- **clap's mutual exclusion must name both flags on stderr** (criterion 18).
  `#[arg(long, group = "flaw")]` on all six should produce
  `the argument '--expired' cannot be used with '--wrong-aud'`; if the derive
  produces a group that permits multiples, declare the `ArgGroup` explicitly.
  Verify against real stderr, not against the docs.
- **Empty values must mean "absent" on both surfaces.** `Form::get` already
  treats `-d flaw=` as absent; the seam's `HashMap` does not, and would parse
  `""` as an unknown flaw and answer `400` where the grant answers `200`. Make
  the seam agree.
- **`expires_in` and the clock.** Reading `unix_now()` twice makes the
  unflawed case report 59 or 61 intermittently — a test that fails once a week.
  This is why `issue()` hands back its own `issued_at`.
- **The acceptance run needs three processes and a real wait.** `lanyard serve`,
  `dotnet run`, and a second `dotnet run` with `CLOCK_SKEW=default` for
  criterion 3. Criterion 3 is the one the hour-long shift exists for; do not
  skip it because criterion 1 already showed a `401`.
- **`jose`'s error names in the spec's table are predicted, not observed.** The
  spec says so: where an observed name differs, the table is wrong and gets
  corrected from the observation — in both the spec and this plan.

## Steps

Each box ≈ one small commit moving an observable behavior. Check only when the
outcome is real, not when code is written.

- [x] **`Flaw`, the one vocabulary** — new `src/oidc/flaw.rs`: the six-variant
      enum, `Flaw::parse(&str) -> Result<Flaw, String>` whose `Err` is the exact
      description from the spec (naming the bad value and listing all six),
      `as_str()` for the wire name, and the constants `EXPIRED_SHIFT = 3600`,
      `WRONG_ISSUER = "https://wrong-issuer.example.test"`,
      `UNKNOWN_KID = "lanyard-unknown-kid"`. Unit tests: every name round-trips,
      and the rejection message for `expried` contains `expried` and all six
      valid values. Nothing calls it yet
- [x] **The three claim-level flaws, applied last** — `issue::claims_at` grows an
      `Option<Flaw>` and a step 4 after the overrides: `expired` sets
      `iat = nbf = now − 3600` and `exp = iat + ttl`; `wrong-aud` replaces `aud`
      with `wrong-<requested>`, or `wrong-audience` when `aud` is absent or not a
      string; `wrong-iss` replaces `iss`. `issue()` returns
      `Issued { token, claims, issued_at }`. Unit tests in `tests/issue.rs` prove
      the flaw beats a body that posted its own `exp` and its own `aud`.
      **North star 4** — the flaw outranking the body is what keeps `flaw=expired`
      from ever returning a live token
- [x] **The seam takes `?flaw=`** — parsed, applied, and echoed as `"flaw"` in
      the `{"token","claims"}` response; an unrecognized value is a `400` in the
      OAuth shape; an empty `flaw=` reads as absent.
      `curl -sX POST '…/_/api/token?persona=ada&flaw=wrong-aud' -d '{"aud":"billing-api"}'`
      → `claims.aud` is `wrong-billing-api` and the decoded token payload equals
      `claims` exactly
- [x] **`jws::sign` grows the header/signature modes** — `sign` takes
      `Option<Flaw>`: `alg-none` emits the header `{"alg":"none","typ":"JWT"}`
      with an empty third segment and no signing at all; `unknown-kid` puts
      `lanyard-unknown-kid` in the header and signs with the real key;
      `bad-signature` signs normally and flips the low bit of the signature's
      last byte before encoding. Unit tests: the `alg-none` header's exact bytes,
      the empty third segment, and a `bad-signature` token whose header segment is
      byte-identical to a good one's. **This is the bill Phase 1 ran up** — the
      reason `jws` is hand-rolled instead of a JWT library
- [x] **`/oidc/token` takes `flaw=`, and tells the truth about `expires_in`** —
      the form parameter maps through `Flaw::parse` to a `400 invalid_request`
      on an unrecognized value, and `expires_in` becomes
      `exp − issued_at` floored at zero. `-d flaw=expired` → `200` with
      `expires_in: 0` and no `flaw` field in the body; no `flaw` at all →
      `expires_in: 60`, unchanged. Grant dispatch is untouched, as Phase 2's plan
      said it would be
- [x] **The six CLI flags** — `MintArgs` grows six mutually exclusive booleans,
      shared by `token` and `env`, collapsed to one `Option<&str>` on
      `MintRequest` and posted as one more form field.
      `lanyard token --as ada --aud billing-api --expired` prints a token whose
      `exp` is in the past; two flags at once exits non-zero with clap's usage
      error naming both, and stdout stays empty. **The CLI still does not sign**
- [x] **Six distinct `jose` errors, observed** — each flag through
      `scripts/jose-verify.mjs` against the live JWKS exits `1` with its own
      `ERROR <name>: <message>`, and an unflawed token still exits `0`. Paste the
      six observed lines into the spec's table; where an observation differs from
      the predicted name, **the table is corrected from the observation** and the
      difference noted in Progress notes
- [x] **`spikes/dotnet-api/failure-tokens.sh`** — the seven cases as one script:
      good → `200`, six flags → `401`, one `PASS`/`FAIL` line each, exit `0` only
      when all seven match. Same shape Phase 11 wants in every
      `examples/*/test.sh`, so it is worth getting right once. Named in the
      spike's README
- [x] **The script run green against the harness** — `lanyard serve` plus
      `dotnet run` (ClockSkew Zero): seven lines, exit `0`. Then
      `CLOCK_SKEW=default dotnet run` and `--expired` **still** `401` — the
      criterion the hour-long shift exists for, against the five minutes Phase 2
      measured
- [x] **The six refusals, written down** — a new section in
      `docs/decisions/dotnet-jwt-bearer-settings.md` carrying the harness's
      `TOKEN REFUSED: <ExceptionType>: <message>` line for each of the six,
      verbatim, plus the `TOKEN OK: sub=ada` for the good one. Where two flaws
      share an exception type, record that as observed rather than tidying it
      away. This is the troubleshooting table for anyone whose RP is refusing a
      token they believe is good
- [x] **Equivalence, flaws included** — `tests/http.rs`: claims from the grant
      and from the seam agree under `del(.iat,.nbf,.exp,.jti,.client_id)` for
      `wrong-iss` and for `wrong-aud`, Phase 2's no-flaw diff still empty, and
      `/oidc/jwks` still holds exactly one key with an unchanged `kid` after all
      six have been minted. **North star 3** — two callers, one claim set, flaws
      included; flaws mint, they do not mutate
- [x] **Docs** — `README.md` gains the six-line block from CONCEPT §6 (above
      where the login screenshot will go), the six flags in the CLI flag table,
      `flaw` in both the token-endpoint and seam parameter tables with a note that
      an unrecognized value is a `400` rather than an ignored parameter, and a
      sharp edge for whatever criterion 1 actually showed about `--unknown-kid`.
      The Scope paragraph stops saying "no deliberately-wrong tokens yet"

## Progress notes

- **The `--unknown-kid` risk landed.** Stock `AddJwtBearer` answered `200` and
  logged `TOKEN OK: sub=ada` for a token whose `kid` is in no JWKS — the plan's
  largest predicted risk, and structural rather than a bug: the signature is
  real, and Microsoft.IdentityModel does not require `kid` to resolve. Handled
  exactly as the plan directed — recorded verbatim in the decision note, taken
  back to the spec (acceptance criterion 1 corrected from the observation), and
  **not** "fixed" by also corrupting the signature. `failure-tokens.sh` expects
  `200` there, prints why on that line, and takes `UNKNOWN_KID_STATUS=401` for a
  `kid`-honouring RP. `jose` refuses the same token with `JWKSNoMatchingKey`,
  so the two libraries disagree — which is the finding.
- **Step 7, `jose`:** all six predicted error names were confirmed against
  `jose` 6.x — nothing in the spec's table needed correcting. The table now
  carries the six observed `ERROR <name>: <message>` lines verbatim rather than
  the two-column prediction.

## Acceptance

Mirrors the spec. Every run sets `LANYARD_DATA_DIR` and a throwaway
`XDG_CONFIG_HOME`; `jose` means `scripts/jose-verify.mjs` against the live JWKS
URL; the .NET harness is `spikes/dotnet-api/` on `http://127.0.0.1:5080` with
`ClockSkew = Zero` unless stated.

**Against a real resource server**

- [x] 1. `spikes/dotnet-api/failure-tokens.sh` prints `200` for the good token and
      `401` for each of the six, and exits `0` — **corrected from the
      observation**: `--unknown-kid` is `200` against stock .NET, and the script
      expects that with the deviation printed on the line. See Progress notes
- [x] 2. The harness log carries `TOKEN OK: sub=ada` for the good token and a
      `TOKEN REFUSED: <ExceptionType>: <message>` for each of the six, all seven
      pasted verbatim into `docs/decisions/dotnet-jwt-bearer-settings.md`
- [x] 3. With `CLOCK_SKEW=default dotnet run`, `--expired` still returns `401`
- [x] 4. The block exactly as it appears in `README.md`, copy-pasted into a shell
      unedited, produces `200` then six `401`s

**Against `jose`**

- [x] 5. Each flag exits `1` with its own error and a good token exits `0`
      printing its payload — `--expired` `JWTExpired`, `--wrong-aud` and
      `--wrong-iss` `JWTClaimValidationFailed` told apart by the claim named,
      `--bad-signature` `JWSSignatureVerificationFailed`, `--alg-none`
      `JOSENotSupported`, `--unknown-kid` `JWKSNoMatchingKey`. An observed name
      that differs corrects the table

**The shape of each flawed token**

- [x] 6. `--alg-none`: segment 1 decodes to exactly `{"alg":"none","typ":"JWT"}`,
      three parts on `.`, third is the empty string
- [x] 7. `--unknown-kid`: header `alg` is `RS256`, `kid` is `lanyard-unknown-kid`,
      that string is absent from `/oidc/jwks`, signature decodes to 256 bytes
- [x] 8. `--bad-signature`: header segment byte-identical to a good token's,
      payload decodes with `sub: "ada"`, signature decodes to 256 bytes
- [x] 9. `--expired`: `nbf == iat`, `exp − iat == 60`, `now − exp` between 3500
      and 3600
- [x] 10. `--wrong-aud` with `--aud billing-api` → `wrong-billing-api`; without
      `--aud` → `wrong-audience`; `iss` matches discovery in both
- [x] 11. `--wrong-iss`: `iss` is `https://wrong-issuer.example.test`, `aud` is
      still `billing-api`, header `kid` is the real one

**Both surfaces, still one function**

- [x] 12. `?persona=ada&flaw=wrong-aud` with `{"aud":"billing-api"}` →
      `claims.aud` is `wrong-billing-api`, `"flaw": "wrong-aud"` is echoed, and
      the token's decoded payload equals `claims` exactly
- [x] 13. `-d grant_type=client_credentials -d persona=ada -d audience=billing-api -d flaw=expired`
      → `200`, `expires_in: 0`, no `flaw` field, `exp` in the past
- [x] 14. CLI and seam claims for `--wrong-iss` diff to empty under
      `jq -S 'del(.iat,.nbf,.exp,.jti,.client_id)'`; same for `--wrong-aud`
- [x] 15. Phase 2's criterion 15 still passes: the same diff with no flaw is
      still empty
- [x] 16. `/oidc/jwks` has one key with an unchanged `kid` after minting all six

**Refusals and ergonomics**

- [x] 17. `-d flaw=expried` → `400 invalid_request` naming `expried` and all six,
      with no `access_token`; `?flaw=expried` on the seam → the same shape; the
      server still serves
- [x] 18. `lanyard token --as ada --expired --wrong-aud` exits non-zero, stdout is
      0 bytes, stderr names both flags
- [x] 19. `lanyard token --help` and `lanyard env --help` each list all six;
      `eval "$(lanyard env --as ada --aud billing-api --expired)"` sets
      `BEARER_TOKEN` to a token whose `exp` is in the past
- [x] 20. Unregressed: no flaw → `200` at `/orders`, and `POST /oidc/token` with
      no `flaw` returns `expires_in: 60`
