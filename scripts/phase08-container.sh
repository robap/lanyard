#!/usr/bin/env bash
# Phase 8, the container path — criteria 3, 4, 7, 8, 15, 16 and 17, in one run.
#
# The alternative is a paragraph of shell in a review that nobody re-runs. This
# builds the image, stands it up on a user-defined network, probes it from a
# second container, runs `doctor` from inside and outside, and drives the
# `-p 9500:8080` mismatch — asserting each result rather than printing it.
#
#   scripts/phase08-container.sh
#
# **It never calls sudo.** Criterion 18 needs `127.0.0.1 lanyard` in
# /etc/hosts, and a machine-wide name resolution file is the operator's to
# edit. When the name does not already resolve to loopback this prints the line
# and stops.
#
# **`--format docker` is not optional.** `podman build` defaults to the OCI
# image format, which has no HEALTHCHECK: the instruction is dropped *silently*
# and `podman inspect .State.Health` is then absent, which reads as a broken
# health check rather than as a missing one.
set -euo pipefail

IMAGE=localhost/lanyard:dev
NET=lanyard-net
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK=$(mktemp -d)

# Everything this script creates, removed whether it passes or fails — a script
# that leaves a container called `lanyard` behind makes its own second run lie.
cleanup() {
  podman rm -f lanyard lan-8080 >/dev/null 2>&1 || true
  podman network rm -f "$NET" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

pass() { printf '  ok   %s\n' "$1"; }
fail() { printf '  FAIL %s\n' "$1" >&2; exit 1; }
step() { printf '\n%s\n' "$1"; }

# `curl` is not in the image and does not have to be on the host either; the
# host side of these assertions uses it, so say so once rather than failing
# obscurely on the first probe.
command -v curl >/dev/null || fail "curl is needed for the host-side probes"
command -v podman >/dev/null || fail "podman is needed"

# --------------------------------------------------------- criterion 15 --
step "Building $IMAGE from a clean context"
podman build --format docker -t "$IMAGE" "$REPO" >"$WORK/build.log" 2>&1 \
  || { cat "$WORK/build.log"; fail "the image did not build"; }
SIZE=$(podman image inspect "$IMAGE" --format '{{.Size}}')
printf '  size %s MB\n' "$((SIZE / 1000000))"
pass "the image builds"

# No shell, which is what makes the health probe have to be lanyard itself.
if podman run --rm --entrypoint /bin/sh "$IMAGE" -c true >/dev/null 2>&1; then
  fail "the image has a shell; it is not distroless"
fi
pass "there is no shell in the image"

# ---------------------------------------------------------- criterion 16 --
step "The same kid twice, with no volume either time"
kid_of() { curl -sf "$1/oidc/jwks" | sed 's/.*"kid":"\([^"]*\)".*/\1/'; }

podman run -d --rm --name lanyard -p 9500:9500 "$IMAGE" >/dev/null
for _ in $(seq 30); do curl -sf http://127.0.0.1:9500/_/health >/dev/null 2>&1 && break; sleep 1; done
FIRST=$(kid_of http://127.0.0.1:9500)
podman rm -f lanyard >/dev/null
sleep 1

podman run -d --rm --name lanyard -p 9500:9500 "$IMAGE" >/dev/null
for _ in $(seq 30); do curl -sf http://127.0.0.1:9500/_/health >/dev/null 2>&1 && break; sleep 1; done
SECOND=$(kid_of http://127.0.0.1:9500)
[ -n "$FIRST" ] && [ "$FIRST" = "$SECOND" ] || fail "kid changed between runs: $FIRST vs $SECOND"
pass "same kid on two fresh containers: $FIRST"

# The native binary, if it has been built, serves the same key — which is the
# whole of "a fresh install produces the same kid every time".
if [ -x "$REPO/target/release/lanyard" ]; then
  podman rm -f lanyard >/dev/null
  NATIVE_DIR="$WORK/native" ; mkdir -p "$NATIVE_DIR"
  LANYARD_DATA_DIR="$NATIVE_DIR" LANYARD_PORT=9501 "$REPO/target/release/lanyard" serve >/dev/null 2>&1 &
  NATIVE_PID=$!
  for _ in $(seq 20); do curl -sf http://127.0.0.1:9501/_/health >/dev/null 2>&1 && break; sleep 1; done
  NATIVE=$(kid_of http://127.0.0.1:9501)
  kill "$NATIVE_PID" 2>/dev/null || true
  [ "$NATIVE" = "$FIRST" ] || fail "the native binary serves $NATIVE, the image $FIRST"
  pass "the native binary serves the same kid"
  podman run -d --rm --name lanyard -p 9500:9500 "$IMAGE" >/dev/null
  for _ in $(seq 30); do curl -sf http://127.0.0.1:9500/_/health >/dev/null 2>&1 && break; sleep 1; done
fi

# ----------------------------------------------------------- criterion 4 --
step "The HEALTHCHECK, which is lanyard itself"
HEALTH=
for _ in $(seq 30); do
  HEALTH=$(podman inspect --format '{{.State.Health.Status}}' lanyard 2>/dev/null || true)
  [ "$HEALTH" = "healthy" ] && break
  sleep 1
done
[ "$HEALTH" = "healthy" ] || fail "never became healthy (status: ${HEALTH:-absent}) — was it built with --format docker?"
pass "healthy, with no shell and no curl in the image"

# ---------------------------------------------------------- criterion 17 --
step "A bind-mounted data directory"
# **Without `:U` this must refuse, and say why.** Rootless podman maps the
# container's root to the invoking user and every other uid into a subuid
# range, so a directory you own appears inside as root's.
mkdir -p "$WORK/no-map"
if OUT=$(podman run --rm -v "$WORK/no-map:/data" "$IMAGE" 2>&1); then
  fail "an unmapped bind mount should have refused"
fi
case "$OUT" in
  *"is not writable"*) : ;;
  *) fail "the refusal did not name the ownership problem: $OUT" ;;
esac
case "$OUT" in
  *"os error 13"*) fail "a bare errno reached the developer: $OUT" ;;
esac
grep -q -- "-v ./lanyard-data:/data:U" <<<"$OUT" || fail "no podman remedy in: $OUT"
grep -q -- '--user \$(id -u):\$(id -g)' <<<"$OUT" || fail "no docker remedy in: $OUT"
pass "an unmapped mount refuses and names both runtimes' fix"

# **`:U` is what makes it writable**; reading the key back *as you* additionally
# needs the invoking user's own uid inside the container, because `:U` chowns
# the directory to the mapped subuid rather than to you.
mkdir -p "$WORK/mapped"
podman run --rm --userns=keep-id --user "$(id -u):$(id -g)" \
  -v "$WORK/mapped:/data" -e LANYARD_PORT=9599 "$IMAGE" serve >/dev/null 2>&1 &
MOUNT_PID=$!
for _ in $(seq 20); do [ -f "$WORK/mapped/signing-key.pem" ] && break; sleep 1; done
kill "$MOUNT_PID" 2>/dev/null || true
[ -f "$WORK/mapped/signing-key.pem" ] || fail "no signing-key.pem was written"
MODE=$(stat -c '%a' "$WORK/mapped/signing-key.pem")
[ "$MODE" = "600" ] || fail "signing-key.pem is $MODE, not 600"
head -c 1 "$WORK/mapped/signing-key.pem" >/dev/null || fail "the key is not readable by the invoking user"
[ -f "$WORK/mapped/.gitignore" ] || fail "the data dir's .gitignore was not written"
pass "signing-key.pem at 0600, readable by $(id -un), beside its .gitignore"

# ----------------------------------------------------------- criterion 3 --
step "A second container on the same network"
podman network exists "$NET" || podman network create "$NET" >/dev/null
podman rm -f lanyard >/dev/null 2>&1 || true
podman run -d --rm --name lanyard --network "$NET" -p 9500:9500 \
  -e LANYARD_ISSUER=http://lanyard:9500/oidc "$IMAGE" >/dev/null
for _ in $(seq 30); do curl -sf http://127.0.0.1:9500/_/health >/dev/null 2>&1 && break; sleep 1; done

# **`--no-hosts` is load-bearing once the operator has added the /etc/hosts
# line criterion 18 needs.** Podman copies the host's /etc/hosts into every
# container, so `127.0.0.1 lanyard` lands inside this one too and shadows
# aardvark-dns — resolving `lanyard` to the container itself, where nothing is
# listening. The flag gives the container only its network's own resolution.
BODY=$(podman run --rm --no-hosts --network "$NET" docker.io/curlimages/curl \
  -sf http://lanyard:9500/_/health) || fail "a container on $NET could not reach lanyard by name"
grep -q '"status":"ok"' <<<"$BODY" || fail "unexpected health body: $BODY"
pass "curl in a second container reaches http://lanyard:9500/_/health"

# ----------------------------------------------------------- criterion 7 --
step "doctor inside the container"
# **Dialled by the issuer's own name**, which resolves inside the container via
# aardvark-dns on the user-defined network. `LANYARD_URL` is an address, not the
# issuer, and this is the run where the two are meant to be the same string.
podman exec -e LANYARD_URL=http://lanyard:9500 lanyard /lanyard doctor \
  >"$WORK/inside.txt" 2>&1 || fail "doctor inside exited non-zero:
$(cat "$WORK/inside.txt")"
cat "$WORK/inside.txt" | sed 's/^/  | /'
grep -q "JWKS .*http://lanyard:9500/oidc/jwks" "$WORK/inside.txt" \
  || fail "JWKS was not fetched at the advertised address"
grep -q "FAIL" "$WORK/inside.txt" && fail "a check failed inside the container"
grep -q "WARN" "$WORK/inside.txt" && fail "criterion 7 asks for every check OK, not merely no FAIL"
pass "every check OK inside, JWKS at the advertised address"

# ----------------------------------------------------------- criterion 8 --
step "The published port that is not the issuer's port"
podman run -d --rm --name lan-8080 -p 9501:8080 -e LANYARD_PORT=8080 "$IMAGE" >/dev/null
for _ in $(seq 30); do curl -sf http://127.0.0.1:9501/_/health >/dev/null 2>&1 && break; sleep 1; done

LANYARD_URL=http://localhost:9501 "$REPO/target/release/lanyard" doctor >"$WORK/mismatch.txt" 2>&1 \
  || fail "plain doctor must exit 0 on a WARN:
$(cat "$WORK/mismatch.txt")"
sed 's/^/  | /' "$WORK/mismatch.txt"
grep -q "Issuer .*WARN" "$WORK/mismatch.txt" || fail "the Issuer check did not warn"
grep -q "8080" "$WORK/mismatch.txt" || fail "the issuer's port is not named"
grep -q "9501" "$WORK/mismatch.txt" || fail "the published port is not named"
grep -q "iss=http://127.0.0.1:8080/oidc" "$WORK/mismatch.txt" \
  || fail "the consequence does not carry the iss that will be minted"
pass "WARN naming both ports, exit 0"

if LANYARD_URL=http://localhost:9501 "$REPO/target/release/lanyard" doctor --strict >/dev/null 2>&1; then
  fail "--strict must exit non-zero on a WARN"
fi
pass "--strict exits non-zero"

step "All of criteria 3, 4, 7, 8, 15, 16 and 17 passed."
printf '\nCriterion 18 is the one-name setup, and it needs a line only you can add:\n'
printf "    echo '127.0.0.1 lanyard' | sudo tee -a /etc/hosts\n"
if getent hosts lanyard >/dev/null 2>&1; then
  printf '  (it already resolves here)\n'
fi
