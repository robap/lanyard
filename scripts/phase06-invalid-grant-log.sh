#!/usr/bin/env bash
# Phase 6, criterion 11 — the ten sentences, read back out of the log.
#
# Phases 4 and 5 wrote ten distinct `invalid_grant` descriptions, each naming
# exactly one thing that went wrong, and every one of them was written to a
# `400` on a back-channel call the developer never sees. This script produces
# each failure with `curl`, reads the event stream back with `jq`, and asserts
# the sentence is there **verbatim**.
#
# Two of the ten need a clock nobody has in a shell: an authorization code lives
# 60 seconds (this script waits) and a refresh token lives eight hours (this
# script cannot). The refresh-expiry sentence is covered by
# `tests/live_log.rs::every_invalid_grant_description_reaches_the_log_verbatim`,
# which builds a store whose refresh tokens die on issue.
#
#   lanyard serve &
#   scripts/phase06-invalid-grant-log.sh
set -euo pipefail

BASE="${LANYARD_URL:-http://127.0.0.1:9500}"
CB="http://localhost:5000/signin-oidc"
JAR=$(mktemp)
NDJSON=$(mktemp)
trap 'rm -f "$JAR" "$NDJSON"' EXIT

curl -sf -X POST "$BASE/_/api/events/clear" -o /dev/null

# One login, driven the way a human drives it: authorize, then pick.
login() {  # login <extra-authorize-query>
  local extra="${1:-}"
  local req code
  req=$(curl -s -c "$JAR" -b "$JAR" -o /dev/null -D - \
      "$BASE/oidc/authorize?client_id=billing-web&redirect_uri=http%3A%2F%2Flocalhost%3A5000%2Fsignin-oidc&response_type=code&prompt=login${extra}" \
    | grep -i '^location:' | sed 's/.*req=//' | tr -d '\r\n')
  code=$(curl -s -c "$JAR" -b "$JAR" -o /dev/null -D - -X POST "$BASE/_/pick" \
      --data-urlencode "req=$req" --data-urlencode "persona=ada" \
    | grep -i '^location:' | sed 's/.*[?&]code=//;s/&.*//' | tr -d '\r\n')
  printf '%s' "$code"
}

exchange() { curl -s -X POST "$BASE/oidc/token" -d grant_type=authorization_code "$@"; }
renew()    { curl -s -X POST "$BASE/oidc/token" -d grant_type=refresh_token --data-urlencode "refresh_token=$1"; }

CHALLENGE="K2-ltc83acc4h0c9w6ESC_rEMTJ3F50BXVuGJSstw-cM"

# 1. never issued
exchange -d code=never-issued > /dev/null
# 2. exchanged twice
CODE=$(login "&scope=openid%20offline_access")
TOKENS=$(exchange -d "code=$CODE")
exchange -d "code=$CODE" > /dev/null
# 3. the wrong redirect_uri
CODE=$(login); exchange -d "code=$CODE" -d "redirect_uri=http://localhost:9999/cb" > /dev/null
# 4. a challenge, and no verifier
CODE=$(login "&code_challenge=$CHALLENGE&code_challenge_method=S256"); exchange -d "code=$CODE" > /dev/null
# 5. a challenge, and the wrong verifier
CODE=$(login "&code_challenge=$CHALLENGE&code_challenge_method=S256")
exchange -d "code=$CODE" -d code_verifier=not-the-verifier > /dev/null
# 6. a refresh token nobody issued
renew never-issued > /dev/null
# 7. a refresh token used twice — lanyard rotates
REFRESH=$(printf '%s' "$TOKENS" | jq -r .refresh_token)
ROTATED=$(renew "$REFRESH" | jq -r .refresh_token)
renew "$REFRESH" > /dev/null
# 8. a refresh token revoked at /oidc/revoke
curl -s -X POST "$BASE/oidc/revoke" --data-urlencode "token=$ROTATED" > /dev/null
renew "$ROTATED" > /dev/null
# 9. a code that ran out of time. 60 seconds is the redirect and the exchange
#    and nothing else, so this is the only way to see the sentence.
CODE=$(login)
echo "waiting 61s for the authorization code to expire…" >&2
sleep 61
exchange -d "code=$CODE" > /dev/null

# Read the whole ring back off the ndjson surface.
curl -sN --max-time 3 "$BASE/_/api/events?format=ndjson" > "$NDJSON" || true

FAILED=0
expect() {
  if jq -e --arg want "$1" 'select(.error_description == $want)' "$NDJSON" > /dev/null; then
    printf '  ok  %s\n' "$1"
  else
    printf 'MISS  %s\n' "$1"; FAILED=1
  fi
}

expect "no such authorization code; it was never issued, or lanyard was restarted since it was"
expect "that authorization code has already been exchanged; codes are single-use"
expect "that authorization code has expired; codes live 60 seconds, which is the redirect and the exchange and nothing else"
expect "redirect_uri \"http://localhost:9999/cb\" does not match the one this code was issued against, \"$CB\""
expect "this code was issued against a S256 code_challenge, so a code_verifier is required"
expect "the code_verifier does not match the S256 code_challenge this code was issued against"
expect "no such refresh token; it was never issued, or lanyard was restarted since it was"
expect "that refresh token has already been exchanged; lanyard rotates refresh tokens, so each one works once and the response carries its replacement"
expect "that refresh token has been revoked, either at /oidc/revoke or by logging out of lanyard"

exit "$FAILED"
