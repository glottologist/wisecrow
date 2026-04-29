# Configuration reference

Wisecrow's configuration surface is small and entirely environment-driven.
This page is the authoritative listing — see
[Getting started › Configuration](../getting-started/configuration.md) for the
narrative version.

## Variable index

All variables are prefixed with `WISECROW__` (double underscore separator).

| Variable | Type | Required | Used by |
|----------|------|----------|---------|
| `DB_URL` | URL | one of A | All database commands |
| `DB_ADDRESS` | `host:port` | one of B | All database commands |
| `DB_NAME` | string | one of B | All database commands |
| `DB_USER` | string | one of B | All database commands |
| `DB_PASSWORD` | secret | one of B | All database commands |
| `LLM_PROVIDER` | `anthropic` \| `openai` | optional | `seed-grammar`, `generate-exercises` |
| `LLM_API_KEY` | secret | with `LLM_PROVIDER` | same |
| `UNSPLASH_API_KEY` | secret | optional | `learn`, `prefetch-media` (with `images`) |
| `REMOTE_URL` | URL | optional | reserved |
| `REMOTE_API_KEY` | secret | optional | reserved |
| `SYNC_API_KEY` | secret | optional | `sync` |

> **Group A**: `DB_URL` alone is sufficient and takes precedence.
>
> **Group B**: All four (`DB_ADDRESS`, `DB_NAME`, `DB_USER`, `DB_PASSWORD`)
> must be set together if `DB_URL` is omitted.

## Loading order

1. `dotenvy::dotenv()` loads `.env` from the current working directory.
2. `config-rs` reads `WISECROW__*` from the process environment, mapping
   `__` to a key separator.
3. The flat key-set is deserialised into `wisecrow_core::config::Config`.

If neither `db_url` nor a complete component set is present,
`Config::database_url()` returns `WisecrowError::ConfigurationError`.

## Secrets

Five fields are wrapped in `SecureString`:

```rust,ignore
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecureString(String);
```

The wrapper:

- zeroes its backing buffer on drop,
- exposes the raw value only through `expose(&self) -> &str`,
- omits its content from `Debug`.

It is **not** a defence against memory dumps or hostile attackers with
`/proc` access — it is a defence against the value leaking into log output,
panics, or `Debug`-derived structs.

## Logging

`tracing` is initialised via `tracing_subscriber::fmt::init()`. Verbosity is
controlled by the standard `RUST_LOG` envvar. Common settings:

| `RUST_LOG` value | What you get |
|------------------|--------------|
| `info` | Top-level lifecycle messages: connection, migrations, batch counts. |
| `wisecrow=debug,info` | All debug events from the crate, default level for everything else. |
| `wisecrow=trace` | Per-pair parse messages (extremely chatty). |

## Cargo features

`wisecrow-core` exposes:

| Feature | Default | Brings in |
|---------|---------|-----------|
| `audio` | off | `msedge-tts`, `rodio` for the audio cache and TUI playback. |
| `images` | off | `image`, `ratatui-image` for inline image rendering. |

`wisecrow-web` exposes:

| Feature | Default | Brings in |
|---------|---------|-----------|
| `server` | off | `wisecrow-core`, `tokio`, `sqlx`, `dotenvy`, `config`, `tempfile`. Server-side routes. |
| `web`    | off | `dioxus/web` (WASM target). |
| `audio`  | off | implies `server` and adds Edge TTS playback wiring. |
| `images` | off | implies `server` and adds Unsplash + image decode. |

## Operational defaults

Constants you might want to know about — they are not configurable at run
time, but they document the safe envelope:

| Constant | Value | Defined in |
|----------|------:|------------|
| Max DB connections | 5 | `bin/wisecrow.rs` |
| Download retries | 3 | `downloader::DownloadConfig::default` |
| Connect timeout | 30 s | `downloader::CONNECT_TIMEOUT_SECS` |
| Read timeout (per attempt) | 300 s | `downloader::DownloadConfig::default` |
| Max file size | 100 GiB (`102_400` MiB) | `downloader::DownloadConfig::default` |
| Max decompressed size | 1 GiB | `downloader::MAX_DECOMPRESSED_BYTES` |
| Mpsc channel bound | 1000 | `ingesting::CHANNEL_BOUND` |
| Translation batch size | 1000 | `ingesting::persisting::TRANSLATION_BATCH_SIZE` |
| Frequency batch size | 1000 | `frequency::BATCH_SIZE` |
| TUI tick rate | 100 ms | `tui::TICK_RATE_MS` |
| N-back match probability | 30 % | `dnb::MATCH_PROBABILITY` |
| Min vocab pool for n-back | 8 | `dnb::MIN_VOCAB_POOL_SIZE` |
| N-back interval bounds | 1500–5000 ms | `dnb::scoring::MIN_INTERVAL_MS`, `MAX_INTERVAL_MS` |
| N-back N-level bounds | 1–9 | `dnb::scoring::MIN_N_LEVEL`, `MAX_N_LEVEL` |
| LLM token budget | 4096 | `grammar::seeder::MAX_LLM_TOKENS` |
