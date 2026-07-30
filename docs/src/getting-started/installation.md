# Installation

## Prerequisites

| Requirement | Version | Why |
|-------------|---------|-----|
| Rust toolchain | stable (≥ 1.75) | Workspace targets `edition = "2021"` and uses recent `clap`/`tokio`/`sqlx` features. |
| PostgreSQL | 15+ | Wisecrow uses `unnest` array bulk-inserts and `ON CONFLICT DO UPDATE`. |
| `pkg-config` and OpenSSL headers | system-provided | Required by transitive TTS deps (`msedge-tts` → `curl` / `native-tls`) on Linux. |
| Optional: ALSA dev headers | `libasound2-dev` (Debian) | Only when building with the `audio` feature. |

> **Note:** The repository ships a `flake.nix` and `devbox.json`. If you use
> Nix or [Devbox](https://www.jetify.com/devbox), `nix develop` or
> `devbox shell` gives you a ready toolchain (including OpenSSL `out`/`dev`
> outputs and `PKG_CONFIG_PATH` for `openssl-sys`).

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
| `tts` (default) | Microsoft Edge TTS generation via `msedge-tts`. | Needs outbound network at runtime. |
| `images` (default) | Unsplash image fetch and TUI rendering via `ratatui-image`. | Pulls `image` decoders; needs Unsplash API key for fetch. |
| `audio` | Adds local playback via `rodio` (implies `tts`). | Pulls ALSA on Linux. |

Defaults cover generation + images. Opt into local speaker playback:

```sh
cargo build --release -p wisecrow-core --features audio
```

## Build the web UI (experimental)

The web crate uses Dioxus fullstack and requires the `dioxus-cli`:

```sh
cargo install dioxus-cli
cd wisecrow-web
dx serve
```

Default features enable TTS audio and Unsplash images on learn cards.
`dx` enables the `server` / `web` halves itself. You need a configured
PostgreSQL connection (see [Configuration](./configuration.md)). For
images, set `WISECROW__UNSPLASH_API_KEY`.

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
