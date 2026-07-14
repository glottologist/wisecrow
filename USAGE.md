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

### LLM Provider

The `gloss` and `graded-reader` commands — and `preview --gloss-unknowns` — call a large language model. Configure a provider before using them:

```sh
export WISECROW__LLM_PROVIDER=anthropic   # or: openai
export WISECROW__LLM_API_KEY=sk-...
```

## Database Setup

Create the database, then let Wisecrow apply migrations automatically on first `ingest` run:

```sh
createdb wisecrow
```

Migrations run automatically on the first database-backed command and create the
full schema, including:

| Table | Purpose |
|-------|---------|
| `languages` | Language codes and names |
| `translations` | Source-target phrase pairs, with frequency |
| `cards`, `sessions`, `session_cards` | Per-user SRS learning state |
| `users`, `auth_sessions` | Accounts and web login sessions |
| `cefr_levels`, `grammar_rules`, `rule_examples` | CEFR grammar knowledge base |
| `glosses` | Cached Leipzig glosses |
| `dnb_sessions`, `dnb_trials` | Dual n-back history |
| `media_cache` | Fetched audio/image metadata |
| `sync_clients` | Per-client corpus-sync API keys |

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

## Refresh word frequencies

Corpus ingestion counts how often each phrase appears, which is a rough proxy for how common a word is. The `frequency` command replaces those counts with authoritative figures from the [Hermit Dave FrequencyWords](https://github.com/hermitdave/FrequencyWords) lists, so that new cards surface in true frequency order. It updates the `frequency` column for translations whose source phrase matches a listed word.

```sh
# Download and apply the Hermit Dave list for a language:
wisecrow frequency --lang es
# alias:
wisecrow fr --lang es

# Or apply a local `word count` file (one entry per line):
wisecrow frequency --lang es --file es_50k.txt
```

| Flag | Description | Default |
|------|-------------|---------|
| `-l`, `--lang` | Language code whose frequencies to update (required) | — |
| `--file` | Apply a local frequency file instead of downloading | — |

## Acquisition Loop Commands

These commands turn the ingested corpus and your learning history into study material.

### Gloss a sentence

Produces a Leipzig interlinear gloss of a single sentence via the configured LLM. Results are cached in the database, so repeating a sentence returns instantly.

```sh
wisecrow gloss --sentence "Меня зовут Иван" --lang ru
# alias:
wisecrow gl -s casa -l es
```

| Flag | Description | Default |
|------|-------------|---------|
| `-s`, `--sentence` | Sentence to gloss (required) | — |
| `-l`, `--lang` | Language code of the sentence (required) | — |
| `--refresh` | Bypass the cache and re-prompt the LLM | `false` |

Requires a database connection and a configured LLM provider.

### Generate a graded reader

Generates a personalised reading passage at a target CEFR level, built from the vocabulary you have already learned. Prints the passage followed by a glossary.

```sh
wisecrow graded-reader -n en -f es --cefr B1
# alias:
wisecrow gr -n en -f es --cefr B1 --format html --output reader.html
```

| Flag | Description | Default |
|------|-------------|---------|
| `-n`, `--native-lang` | Your native language code (required) | — |
| `-f`, `--foreign-lang` | Target language code (required) | — |
| `--cefr` | Target CEFR level (`A1`–`C2`) (required) | — |
| `--seed-states` | SRS card states to draw learned words from (comma-separated) | `2` |
| `--seed-min-stability` | Minimum FSRS stability for seed words | — |
| `--seed-limit` | Maximum number of seed words | `30` |
| `--length-words` | Approximate passage length in words | `200` |
| `--format` | Output format: `md` or `html` | `md` |
| `--output` | Write to a file instead of stdout | stdout |
| `--user-id` | User whose learned vocabulary seeds the passage | `1` |

Requires a database connection and a configured LLM provider.

### Preview subtitles

Parses a subtitle file (`.srt`, `.vtt`, `.ass`/`.ssa`), tokenises the foreign-language text, and prints a frequency table annotated with your SRS knowledge status — a difficulty preview before you watch.

```sh
wisecrow preview --file episode.srt -n en -f es
# alias:
wisecrow pv --file episode.vtt -n en -f es --unknown-only
```

| Flag | Description | Default |
|------|-------------|---------|
| `--file` | Path to the subtitle file (required) | — |
| `-n`, `--native-lang` | Your native language code (required) | — |
| `-f`, `--foreign-lang` | Subtitle language code (required) | — |
| `--unknown-only` | Show only tokens you have not learned | `false` |
| `--no-srs` | Skip the SRS lookup; mark every token unknown | `false` |
| `--top-n` | Limit output to the N most frequent tokens | all |
| `--gloss-unknowns` | LLM-translate tokens not found in the corpus | `false` |
| `--user-id` | User whose SRS history is used | `1` |

Requires a database connection unless `--no-srs` is set. `--gloss-unknowns` additionally requires a configured LLM provider.

## Web App, Accounts & Deployment

Wisecrow ships a Dioxus fullstack web UI (`wisecrow-web`) that serves the SRS
learning loop, dual n-back, and quizzes over HTTPS. It is **multi-user and
requires login** — every request is authenticated and scoped to the signed-in
user. There is no public signup: an administrator provisions accounts with the
`wisecrow user` command.

Deploying the web app (TLS termination, secrets, first-admin bootstrap, sync
keys) is documented in [`DEPLOYMENT.md`](DEPLOYMENT.md).

### Manage accounts

```sh
wisecrow user add --email you@example.com --display-name "You" --admin
wisecrow user list
wisecrow user passwd --email user@example.com
wisecrow user disable --email user@example.com
```

`add`/`passwd` prompt for a password (or read `WISECROW__INIT_PASSWORD` for
non-interactive provisioning). `disable` clears the password and revokes the
user's sessions.

### Manage sync clients

Per-client, individually revocable keys for remote→local corpus sync:

```sh
wisecrow sync-client add --name laptop     # prints the key once
wisecrow sync-client list
wisecrow sync-client revoke --name laptop
```

The pulling instance sends the key as the `x-api-key` header.

## Additional CLI Commands

Beyond download/ingest and the acquisition loop, these commands are available.
Run `wisecrow <command> --help` for the full flag list.

| Command (alias) | Purpose |
|---|---|
| `learn` (`r`) | Interactive terminal SRS study session over an ingested deck. |
| `quiz` (`q`) | Generate a grammar quiz from a PDF and run it in the terminal. |
| `nback` (`nb`) | Interactive dual n-back training session. |
| `sync` (`s`) | Pull corpus/grammar data from a remote Wisecrow instance. |
| `download-all` (`da`) | Download every supported pair for a native language. |
| `prefetch-media` (`pm`) | Pre-fetch audio/images for a pair into the media cache. |
| `seed-grammar` (`sg`) | LLM-seed CEFR grammar rules for a language and levels. |
| `import-grammar` (`ig`) | Import grammar rules from a JSON file. |
| `import-pdf` (`ip`) | Extract and import grammar rules from a PDF. |
| `generate-exercises` (`ge`) | Generate LLM grammar exercises for a language/level. |
| `user` (`u`) | Manage web accounts (see above). |
| `sync-client` (`sc`) | Manage sync-client keys (see above). |

## Architecture

The ingestion pipeline uses a producer-consumer pattern over async channels:

1. **Download** — Fetches files with retry/backoff, decompresses gz/zip archives
2. **Parse** — Streams TMX and XML alignment files via `quick-xml`
3. **Persist** — Batches parsed translations (1000 per batch) and inserts into PostgreSQL within transactions

Each file is processed in its own Tokio task. The process handles SIGTERM and SIGINT for graceful shutdown.

## Supported Languages

102 languages are supported, including: Afrikaans, Amharic, Arabic, Bengali, Chinese, Czech, Danish, Dutch, English, Finnish, French, German, Greek, Hebrew, Hindi, Hungarian, Indonesian, Italian, Japanese, Korean, Malay, Norwegian, Persian, Polish, Portuguese, Romanian, Russian, Spanish, Swedish, Thai, Turkish, Ukrainian, Urdu, Vietnamese, and many more.

Run `wisecrow list-languages` for the full list.
