# **The image ships one statically linked binary and nothing else** — no shell,
# no curl, no package manager (north star 5). That is a real constraint, and it
# is what forces the health probe below to be lanyard itself.
#
# **Where that binary comes from is the only thing that varies.** `BINARY_STAGE`
# picks between compiling it here — the default, so a checkout and
# `podman build .` is still the whole story — and copying one a release already
# built for this architecture. The final stage is the same either way, byte for
# byte, because it is the part Phase 8 proved and a second Dockerfile would
# drift from it.
ARG BINARY_STAGE=source

# ---------------------------------------------------------------- builder --
# Alpine's rust image targets musl natively, so a static binary needs no cross
# toolchain and none of it lands on the development machine. `--locked` because
# an image built from a resolved-anew lock file is not the thing the tests ran
# against.
FROM docker.io/library/rust:1-alpine AS binary-source

# `musl-dev` for the C runtime startup files the linker wants. Nothing else:
# `reqwest` is built with `default-features = false`, so there is no TLS stack
# and therefore no OpenSSL to find.
RUN apk add --no-cache musl-dev

WORKDIR /src
COPY . .
RUN cargo build --release --locked --bin lanyard
RUN install -D -m 0755 target/release/lanyard /out/lanyard

# Distroless has no shell, so `/data` cannot be created with `RUN mkdir` in the
# final stage. It is made here and copied in with its ownership.
RUN mkdir -p /empty-data

# --------------------------------------------------------------- prebuilt --
# The release has already produced a static musl binary for this architecture,
# on a runner of this architecture, and compiling it a second time would only
# be a chance for the two to differ. The GHCR workflow extracts one from the
# release archive and builds with `--build-arg BINARY_STAGE=prebuilt`.
#
# Alpine is here for the two `RUN`s and nothing from it reaches the final image.
FROM docker.io/library/alpine AS binary-prebuilt
ARG BINARY=lanyard
COPY ${BINARY} /out/lanyard
RUN chmod 0755 /out/lanyard && mkdir -p /empty-data

# The one line that makes the final stage indifferent to which of the two ran.
# Only the selected stage is built; the other's base image is never pulled.
FROM binary-${BINARY_STAGE} AS binary

# ------------------------------------------------------------------ final --
FROM gcr.io/distroless/static-debian12:nonroot

COPY --from=binary /out/lanyard /lanyard
# 65532 is distroless's `nonroot`. The directory arrives owned by it, so the
# ordinary no-volume run can write its key without any flag.
COPY --from=binary --chown=65532:65532 /empty-data /data

# **The bind address is a property of the deployment, and the image is the
# deployment** (CONCEPT §7). The native binary keeps its loopback default; here,
# `podman run -p 9500:9500` has to work with no flags.
ENV LANYARD_BIND=0.0.0.0
# Not `$XDG_DATA_HOME`, which in a distroless image resolves under a home
# directory nobody thinks about.
ENV LANYARD_DATA_DIR=/data

EXPOSE 9500

# **There is no `curl` and no `sh` to run one.** That is what makes `doctor`'s
# exit-code contract load-bearing rather than tidy: the probe must be `0` for a
# lanyard that is serving but oddly configured, or a container never reports
# healthy and everything waiting on it never starts.
#
# `podman build` defaults to the OCI format, which has no HEALTHCHECK and drops
# this instruction **silently**. Build with `--format docker`.
HEALTHCHECK --interval=10s --timeout=5s --start-period=5s --retries=3 \
    CMD ["/lanyard", "doctor", "--quiet"]

ENTRYPOINT ["/lanyard"]
CMD ["serve"]

# No VOLUME. The deterministic default key means a fresh container with no
# volume produces the same `kid` and the same public key every time — the
# property Phase 1 built, and the answer to CONCEPT §8's ephemeral-keys gotcha.
# A volume is for someone who has replaced the key with their own.
