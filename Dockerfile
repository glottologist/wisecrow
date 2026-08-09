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

# Dioxus CLI bundles the fullstack server + WASM client. Keep its exact
# version aligned with the resolved `dioxus` crate and bump both together.
RUN cargo install --locked dioxus-cli@0.7.3 \
 && rustup target add wasm32-unknown-unknown

WORKDIR /build
COPY . .

# CLI binary — default features include TTS + images (no rodio/ALSA).
RUN cargo build --release --bin wisecrow

# Fullstack web bundle. `Dioxus.toml` configures `out_dir = "dist"` inside
# the wisecrow-web crate, so artifacts land in /build/wisecrow-web/dist.
# Default features already enable TTS audio + Unsplash images (no rodio).
RUN cd wisecrow-web \
 && dx bundle --release --platform web

FROM debian:${DEBIAN_RELEASE}-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates libssl3 tini wget \
 && rm -rf /var/lib/apt/lists/*

# Run as a non-root system user.
RUN useradd --system --uid 10001 --user-group --no-create-home wisecrow

WORKDIR /app

COPY --from=builder /build/target/release/wisecrow              /usr/local/bin/wisecrow
COPY --from=builder /build/wisecrow-core/migrations             /app/migrations
COPY --from=builder /build/wisecrow-web/dist                    /app/web

RUN chown -R wisecrow:wisecrow /app

# `--no-create-home` leaves $HOME at a path that does not exist, and /home is
# root-owned, so `dirs::data_local_dir()` resolved to a directory the app could
# not create: every media cache initialisation failed with permission denied
# before any provider was reached, and no card ever played audio or showed an
# image. XDG_DATA_HOME points the cache somewhere the app owns.
ENV XDG_DATA_HOME=/var/lib/wisecrow
RUN mkdir -p /var/lib/wisecrow && chown -R wisecrow:wisecrow /var/lib/wisecrow

# App-terminated TLS: the server serves HTTPS on 8443 when
# WISECROW__TLS_CERT_PATH / WISECROW__TLS_KEY_PATH are set (see
# docker-compose.deploy.yml); otherwise it serves plain HTTP on this port.
ENV IP=0.0.0.0 \
    PORT=8443 \
    RUST_LOG=info \
    RUST_BACKTRACE=0

EXPOSE 8443
USER wisecrow
WORKDIR /app/web

# dx 0.7 names the fullstack server after the crate. The wildcard guards
# against minor renames between dx releases (e.g. server vs <crate>).
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["sh", "-c", "exec $(find /app/web -maxdepth 1 -type f -executable | head -n 1)"]
