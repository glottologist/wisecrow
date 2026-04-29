# Installation

## Prerequisites

| Requirement | Version | Why |
|-------------|---------|-----|
| Rust toolchain | stable (≥ 1.75) | Workspace targets `edition = "2021"` and uses recent `clap`/`tokio`/`sqlx` features. |
| PostgreSQL | 15+ | Wisecrow uses `unnest` array bulk-inserts and `ON CONFLICT DO UPDATE`. |
| `pkg-config` and OpenSSL headers | system-provided | Required by transitive dependencies on Linux. |
| Optional: ALSA dev headers | `libasound2-dev` (Debian) | Only when building with the `audio` feature. |

> **Note:** The repository ships a `flake.nix` and `devbox.json`. If you use
> Nix or [Devbox](https://www.jetify.com/devbox), `nix develop` or
> `devbox shell` gives you a ready toolchain.

## Clone the repository

```sh
git clone https://github.com/glottologist/wisecrow
cd wisecrow
```

## Build the CLI

The default build produces only the `wisecrow` binary; the workspace also
contains the experimental web and mobile front-ends.

```sh
cargo build --release -p wisecrow-core
```

The release binary ends up at `target/release/wisecrow`. Add it to your
`$PATH` or invoke via `./target/release/wisecrow`.

## Optional features

`wisecrow-core` exposes two cargo features:

| Feature | Adds | Cost |
|---------|------|------|
| `audio`  | Microsoft Edge TTS streaming via `msedge-tts`, playback via `rodio`. | Pulls ALSA on Linux. |
| `images` | Unsplash image fetch and inline rendering via `ratatui-image`. | Pulls `image` decoders. |

Enable them at build time:

```sh
cargo build --release -p wisecrow-core --features "audio images"
```

## Build the web UI (experimental)

The web crate uses Dioxus fullstack and requires the `dioxus-cli`:

```sh
cargo install dioxus-cli
cd wisecrow-web
dx serve --features server
```

The `server` feature pulls in `wisecrow-core`, so you need a configured
PostgreSQL connection (see [Configuration](./configuration.md)).

## Verify the install

```sh
wisecrow --help
wisecrow list-languages | head
```

You should see the full subcommand list and the first lines of the
102-language table.

## Next step

Continue with [Configuration](./configuration.md) to wire Wisecrow up to
PostgreSQL.
