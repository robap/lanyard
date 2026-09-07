# **The image ships one statically linked binary and nothing else** — no shell,
# no curl, no package manager (north star 5). That is a real constraint, and it
# is what forces the health probe below to be lanyard itself.

# ---------------------------------------------------------------- builder --
# Alpine's rust image targets musl natively, so a static binary needs no cross
# toolchain and none of it lands on the development machine. `--locked` because
# an image built from a resolved-anew lock file is not the thing the tests ran
# against.
FROM docker.io/library/rust:1-alpine AS builder

# `musl-dev` for the C runtime startup files the linker wants. Nothing else:
# `reqwest` is built with `default-features = false`, so there is no TLS stack
# and therefore no OpenSSL to find.
RUN apk add --no-cache musl-dev

WORKDIR /src
COPY . .
RUN cargo build --release --locked --bin lanyard

# Distroless has no shell, so `/data` cannot be created with `RUN mkdir` in the
# final stage. It is made here and copied in with its ownership.
RUN mkdir -p /empty-data

# ------------------------------------------------------------------ final --
FROM gcr.io/distroless/static-debian12:nonroot

COPY --from=builder /src/target/release/lanyard /lanyard
# 65532 is distroless's `nonroot`. The directory arrives owned by it, so the
# ordinary no-volume run can write its key without any flag.
COPY --from=builder --chown=65532:65532 /empty-data /data

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
