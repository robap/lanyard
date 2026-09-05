#!/usr/bin/env bash
# The negative-path suite: one good token and the six deliberate failures,
# against the Phase 2 harness's `/orders`.
#
#   lanyard serve &                 # 127.0.0.1:9500
#   dotnet run                      # ClockSkew = Zero
#   ./failure-tokens.sh
#
# Seven lines of output, one per case, and exit 0 only when all seven match.
# This is the README's six-line block with a pass/fail line around it, and the
# shape Phase 11 wants in every `examples/*/test.sh` — so it is worth getting
# right once here.
#
# **Six of the seven expectations are what a correct resource server does. The
# seventh is what this one was observed to do.** `--unknown-kid` is accepted by
# stock `AddJwtBearer`, which does not require the header's `kid` to match a key
# in the JWKS and finds the real key anyway. That is not a lanyard bug and not a
# harness bug: it is the finding the flag exists to produce, recorded in
# `docs/decisions/dotnet-jwt-bearer-settings.md`. Against an RP that does honour
# `kid` — `jose` does — the expected status is 401, so set
# `UNKNOWN_KID_STATUS=401` when pointing this script at one.
#
# `--expired` is expected to fail against `CLOCK_SKEW=default` too: the shift is
# an hour, which clears .NET's five-minute default tolerance.
set -uo pipefail

API=${API:-http://127.0.0.1:5080/orders}
LANYARD=${LANYARD:-lanyard}
PERSONA=${PERSONA:-ada}
AUD=${AUD:-billing-api}
UNKNOWN_KID_STATUS=${UNKNOWN_KID_STATUS:-200}

failed=0

# <label> <expected status> <note> [flag]
check() {
  local label=$1 expected=$2 note=$3 token status
  shift 3

  if ! token=$("$LANYARD" token --as "$PERSONA" --aud "$AUD" "$@"); then
    printf 'FAIL  %-16s could not mint\n' "$label"
    failed=1
    return
  fi

  status=$(curl -s -o /dev/null -w '%{http_code}' \
    -H "Authorization: Bearer $token" "$API")

  if [ "$status" = "$expected" ]; then
    printf 'PASS  %-16s %s%s\n' "$label" "$status" "$note"
  else
    printf 'FAIL  %-16s %s (expected %s)%s\n' "$label" "$status" "$expected" "$note"
    failed=1
  fi
}

# A good token has to be accepted, or the six refusals below prove nothing: an
# API that is refusing everything would "pass" five of them.
check '(good token)' 200 ''
check '--expired' 401 '' --expired
check '--wrong-aud' 401 '' --wrong-aud
check '--wrong-iss' 401 '' --wrong-iss
check '--bad-signature' 401 '' --bad-signature
check '--alg-none' 401 '' --alg-none
check '--unknown-kid' "$UNKNOWN_KID_STATUS" \
  '  ← accepted: this RP does not honour kid (docs/decisions/dotnet-jwt-bearer-settings.md)' \
  --unknown-kid

exit $failed
