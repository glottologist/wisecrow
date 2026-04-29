# Troubleshooting

A non-exhaustive list of failure modes and their root causes. Issues are
grouped by where they tend to surface.

## Configuration

### `Either db_url or all of db_address, db_name, db_user, db_password must be set`

`Config::database_url` returns this when neither form A nor a *complete*
form B is present. Common slip-ups:

- Missing `WISECROW__DB_PASSWORD` — easy to forget if you committed a `.env`
  with the username but not the password.
- Wrong separator — must be `__` (double underscore).
- A typo in the prefix. The loader is strict about `WISECROW__`.

Check with:

```sh
env | grep WISECROW__
```

### `Configuration error: relative URL without a base`

`config.rs` builds the URL from the components via `url::Url::parse(...)`.
A bare host like `localhost` works; a host with a slash inside it (or a
trailing colon without a port) fails.

## Database

### `Persistence migration error`

The migrations are embedded into the binary. The error wraps a
`sqlx::migrate::MigrateError`; the most common causes are:

- The user lacks `CREATE TABLE` — Wisecrow needs DDL privileges.
- A previous custom migration left `_sqlx_migrations` in an inconsistent
  state. `SELECT * FROM _sqlx_migrations ORDER BY version` shows the log;
  truncating it is destructive — only do it on a database you can recreate.
- The PostgreSQL version is < 12. The bulk-insert path uses
  `unnest(..., ...)` which requires PG ≥ 12.

### `relation "sessions" does not exist`

The pool was opened against the wrong database. Sanity-check with:

```sh
psql "$WISECROW__DB_URL" -c "\\dt"
```

If the table list is empty, run any database-touching command (e.g.
`wisecrow ingest`) — migrations run on first connection.

## Ingest

### Stuck on a single file forever

The retry loop sleeps `2^attempt` seconds. With `max_retries = 3` the worst
case is 14 s of back-off plus three full reads. If you see no progress at
all, inspect with `wisecrow=trace` logging — you will see the chunk-loop
lines.

### `HTTP 404 Not Found`

Not every language pair exists in every corpus. Try:

```sh
wisecrow ingest -n en -f gd --corpus open_subtitles
```

If 404s keep coming, drop `--corpus` and let Wisecrow attempt the others.
The CLI continues on per-file errors.

### `File too large: ... bytes (max: ...)`

The `--max-file-size-mb` ceiling triggered. Either bump it explicitly or
split your ingest by corpus:

```sh
wisecrow ingest -n en -f de --corpus cc_matrix --max-file-size-mb 200000
```

The 1 GiB decompression cap is **not** configurable via flags — change
`MAX_DECOMPRESSED_BYTES` in `downloader.rs` if you understand the
implication.

### Inserts plateau under load

`DatabasePersister::consume` flushes in batches of 1000 inside a single
transaction. If your PG instance is small, a parallel ingest of multiple
language pairs can pile up. Solutions:

- Reduce concurrency by serialising the calls (one pair at a time).
- Bump `max_connections` in `postgresql.conf`.
- Pre-warm `random_page_cost`, `shared_buffers`, and disable `synchronous_commit`
  for the duration of the ingest.

## Learn TUI

### "No cards available. Ingest some data first with `wisecrow ingest`."

This is expected when the user has no cards and no unlearned vocabulary
matches the language pair. Make sure:

- Translations exist: `SELECT count(*) FROM translations;`
- The language codes match `languages.code` on both sides.
- The unlearned filter `LENGTH(phrase) BETWEEN 2 AND 200` did not eliminate
  the entire corpus (this happens with extremely noisy TMX files).

### Auto-advance feels too fast / too slow

Use `+`/`-` while the TUI is running, or pass `--speed-ms`. Anything outside
`[500, 10000]` is silently clamped by `SpeedController::new`.

### Audio plays the wrong language

The Edge TTS voice is selected per the foreign-language code. If the code is
not in the voice list, playback is suppressed and the TUI logs a warning.
Run with `wisecrow=debug` to see which voice was attempted.

## Dual n-back

### "Not enough vocabulary (X items, need 8+)"

`DnbEngine::new` requires `MIN_VOCAB_POOL_SIZE = 8`. The pool is fetched
via `DnbSessionRepository::load_vocab` with `LIMIT 100`. Ingest more, or
drop the limit (in source) if you have small corpora intentionally.

### Trials appear to terminate early

You hit one of the termination criteria:

- 50 trials elapsed, or
- both channels' rolling accuracy is below 40 %, or
- your n-level dropped three windows in a row below the start.

Check `dnb_sessions.n_level_peak` vs `n_level_start` after the fact.

## LLM

### `LLM error: Failed to parse LLM response as JSON`

The model returned text that wasn't valid JSON. The seeder strips fenced
code-blocks (` ``` ` and ` ```json `) before parsing, so the only common
failure is the model adding prose. If it persists:

- Lower `MAX_LLM_TOKENS` to discourage rambling.
- Inspect the raw response with `RUST_LOG=wisecrow::grammar::seeder=trace`.
- For OpenAI, switch to a model that respects JSON-mode and adapt
  `OpenAiProvider` accordingly.

### `Anthropic API error 429`

You are rate-limited. The crate does not retry LLM calls — the seeder fails
fast. Re-run with a longer interval between commands or bump your tier.

## Sync

### Sync hangs at "languages: ..."

Pagination is forwarded through `?after_id=N`. If the remote does not
respect the parameter, the local pulls the same page forever. Check with:

```sh
curl -s "$REMOTE/api/sync_languages?after_id=99999999" | jq length
```

It should return `[]` or a small array.

### `Sync error: URL join failed`

The base URL is parsed with `Url::parse`. URLs without a trailing slash join
oddly:

```text
https://example.com/foo  + /api/sync_languages → https://example.com/api/sync_languages   (drops /foo)
https://example.com/foo/ + /api/sync_languages → https://example.com/api/sync_languages   (also drops!)
```

Pass the bare host (`https://example.com`) and let Wisecrow add the path.

## Web UI

The web UI is experimental and is not covered here. If you hit issues,
build with `--features "server web"` and run `dx serve --features
"server web"` from `wisecrow-web/`. Check the server log for SQL errors —
they are the same as for the CLI.

## Logs

For every issue above, the first thing to do is enable verbose logging:

```sh
RUST_LOG=wisecrow=debug,info wisecrow ...
```

The crate uses the `tracing` crate end-to-end, so debug events include the
filename and line number when the subscriber is configured for them.
