# Wisecrow Usage Guide

Wisecrow generates flashcard datasets from multilingual subtitle and translation corpora. It downloads data from OPUS (OpenSubtitles, CCMatrix, NLLB), parses TMX/XML alignment files, and stores translations in PostgreSQL.

## Prerequisites

- Rust toolchain (stable)
- PostgreSQL 15+
- A running PostgreSQL database

## Installation

```sh
cargo build --release
```

The binary is at `./target/release/wisecrow`.

## Configuration

Wisecrow reads database configuration from environment variables prefixed with `WISECROW__`.

### Option A: Direct URL

```sh
export WISECROW__DB_URL=postgres://user:password@localhost/wisecrow
```

### Option B: Component Variables

```sh
export WISECROW__DB_ADDRESS=localhost:5432
export WISECROW__DB_NAME=wisecrow
export WISECROW__DB_USER=wisecrow
export WISECROW__DB_PASSWORD=secret
```

You can also place these in a `.env` file in the project root.

### Logging

Wisecrow uses `tracing` with `RUST_LOG` for log level control:

```sh
export RUST_LOG=wisecrow=debug,info
export RUST_BACKTRACE=1
```

## Database Setup

Create the database, then let Wisecrow apply migrations automatically on first `ingest` run:

```sh
createdb wisecrow
```

Migrations create the following tables:

| Table | Purpose |
|-------|---------|
| `languages` | Language codes and names |
| `translations` | Source-target phrase pairs |

## Commands

### List supported languages

```sh
wisecrow list-languages
# alias:
wisecrow l
```

Prints all 102 supported ISO 639 language codes with their names.

### Download corpus files

Downloads translation data without ingesting into the database.

```sh
wisecrow download -n <native_lang> -f <foreign_lang> [OPTIONS]
# alias:
wisecrow d -n en -f es
```

### Ingest corpus files

Downloads corpus data, parses it, and persists translations into PostgreSQL. Requires a configured database connection.

```sh
wisecrow ingest -n <native_lang> -f <foreign_lang> [OPTIONS]
# alias:
wisecrow i -n en -f ja
```

## Options

These options apply to both `download` and `ingest`:

| Flag | Description | Default |
|------|-------------|---------|
| `-n`, `--native-lang` | Your native language code (required) | — |
| `-f`, `--foreign-lang` | Target language code (required) | — |
| `--corpus` | Filter corpora (space-delimited) | all |
| `--max-file-size-mb` | Maximum file size in MB | `102400` |
| `--unpack` | Decompress downloaded archives | `true` |

### Corpus filter values

| Value | Source |
|-------|--------|
| `open_subtitles` | OpenSubtitles v2018 |
| `cc_matrix` | CCMatrix v1 |
| `nllb` | NLLB v1 |

## Examples

Download only OpenSubtitles data for English-Spanish:

```sh
wisecrow download -n en -f es --corpus open_subtitles
```

Ingest all corpora for English-Japanese:

```sh
wisecrow ingest -n en -f ja
```

Ingest only CCMatrix and NLLB for English-German:

```sh
wisecrow ingest -n en -f de --corpus "cc_matrix nllb"
```

## Architecture

The ingestion pipeline uses a producer-consumer pattern over async channels:

1. **Download** — Fetches files with retry/backoff, decompresses gz/zip archives
2. **Parse** — Streams TMX and XML alignment files via `quick-xml`
3. **Persist** — Batches parsed translations (1000 per batch) and inserts into PostgreSQL within transactions

Each file is processed in its own Tokio task. The process handles SIGTERM and SIGINT for graceful shutdown.

## Supported Languages

102 languages are supported, including: Afrikaans, Amharic, Arabic, Bengali, Chinese, Czech, Danish, Dutch, English, Finnish, French, German, Greek, Hebrew, Hindi, Hungarian, Indonesian, Italian, Japanese, Korean, Malay, Norwegian, Persian, Polish, Portuguese, Romanian, Russian, Spanish, Swedish, Thai, Turkish, Ukrainian, Urdu, Vietnamese, and many more.

Run `wisecrow list-languages` for the full list.
