# syntax=docker/dockerfile:1.7
#
# Multistage build for the Wisecrow stack.
#   * stage 1 (builder)  — compile the `wisecrow` CLI binary and bundle the
#                          Dioxus fullstack web app (server + WASM client).
#   * stage 2 (runtime)  — slim Debian image carrying the CLI, the bundled
#                          server, and the SQL migrations.

ARG RUST_VERSION=1.88
ARG DEBIAN_RELEASE=bookworm

FROM rust:${RUST_VERSION}-${DEBIAN_RELEASE} AS builder
ENV CARGO_TERM_COLOR=always

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Dioxus CLI bundles the fullstack server + WASM client. Pinned to a 0.7
# minor so dist layout stays predictable; bump together with the workspace
# `dioxus = "0.7"` dependency.
RUN cargo install --locked dioxus-cli@^0.7 \
 && rustup target add wasm32-unknown-unknown

WORKDIR /build
COPY . .

# CLI binary — default features only (no audio/images) so the runtime image
# does not need alsa/system multimedia libs. Override --features at build
# time via `--build-arg` if you need them on the server.
RUN cargo build --release --bin wisecrow

# Fullstack web bundle. `Dioxus.toml` configures `out_dir = "dist"` inside
# the wisecrow-web crate, so artifacts land in /build/wisecrow-web/dist.
RUN cd wisecrow-web \
 && dx bundle --release --platform web

FROM debian:${DEBIAN_RELEASE}-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates libssl3 tini wget \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/wisecrow              /usr/local/bin/wisecrow
COPY --from=builder /build/wisecrow-core/migrations             /app/migrations
COPY --from=builder /build/wisecrow-web/dist                    /app/web

ENV IP=0.0.0.0 \
    PORT=8080 \
    RUST_LOG=info \
    RUST_BACKTRACE=1

EXPOSE 8080
WORKDIR /app/web

# dx 0.7 names the fullstack server after the crate. The wildcard guards
# against minor renames between dx releases (e.g. server vs <crate>).
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["sh", "-c", "exec $(find /app/web -maxdepth 1 -type f -executable | head -n 1)"]
