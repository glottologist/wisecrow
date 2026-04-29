# Ingesting corpora

`ingest` is Wisecrow's main data-acquisition command. This guide covers the
trade-offs and patterns you will hit once you move past the introductory
walkthrough.

## Pick the right corpus

| Corpus | Source | Strengths | Weaknesses |
|--------|--------|-----------|------------|
| `open_subtitles` | OPUS-OpenSubtitles v2018 | Conversational; high colloquial coverage. | Profanity, OCR noise, alignment errors. |
| `cc_matrix` | OPUS-CCMatrix v1 | Vast volume, formal register. | Longer sentences; slower to ingest. |
| `nllb` | OPUS-NLLB v1 | Better for low-resource languages. | Heavier files; alignments are heuristic. |

Start with OpenSubtitles for big, common languages (English, Spanish,
French). Add CCMatrix once you have surface coverage and want to extend
intermediate vocabulary. NLLB is most useful when one of your languages is
not well represented in the other two.

## Filter at the CLI

The `--corpus` flag accepts a single space-delimited argument:

```sh
wisecrow ingest -n en -f ja --corpus "open_subtitles cc_matrix"
```

Internally the value is split on spaces (`clap` `value_delimiter = ' '`) and
each token is converted via `Corpus::try_from`. Unknown values produce a
clear error, not a silent skip.

## Cap file size

`--max-file-size-mb` (default `102_400`) caps the response body so a
runaway download does not fill your disk. The check happens in two places:

- The `Content-Length` header is checked before any bytes are written.
- The streaming write path also enforces the cap, so a server that
  withholds `Content-Length` cannot overflow it.

Decompression has its own ceiling: 1 GiB for gzip, plus path-traversal
defences for ZIP.

## Run multiple pairs in parallel

`ingest` already parallelises files **within** one language pair. To
parallelise across pairs, run the command in a shell loop:

```sh
for f in es fr de ja ko; do
  wisecrow ingest -n en -f "$f" --corpus open_subtitles &
done
wait
```

The shared `WISECROW__DB_*` configuration is fine — each invocation gets its
own pool with `MAX_DB_CONNECTIONS = 5`. Watch your disk and bandwidth budget.

## Re-ingest is idempotent

Re-running `ingest` over the same pair is safe. The persister upserts:

```sql
INSERT INTO translations (...)
SELECT ...
ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase)
DO UPDATE SET frequency = translations.frequency + 1
```

So existing pairs gain frequency rather than getting re-inserted. You can
schedule a periodic ingest as a quick frequency refresh.

## Keep downloads but skip persistence

If you want to mirror corpus files (for backups, offline labs, or
reproducibility) without writing them into the database, use:

```sh
wisecrow download -n en -f es --corpus open_subtitles --no-unpack
```

`--no-unpack=false` keeps the gzipped/zipped archives. The
`download-all` command takes an `--output-dir` and walks all foreign
languages — useful for fixture preparation:

```sh
wisecrow download-all -n en -o ./fixtures --corpus "open_subtitles"
```

## Layer external frequency lists

If you find OPUS frequencies too noisy, layer a Hermit Dave list on top.
This is not a CLI command yet, but the function is exposed in the library:

```rust,ignore
use wisecrow::frequency::FrequencyUpdater;
FrequencyUpdater::update_from_hermit_dave(&pool, "es").await?;
```

It rewrites `translations.frequency` in batches of 1000 with values from
the public 50k-word frequency lists. Source: `wisecrow-core/src/frequency.rs`.

## Troubleshooting

- **Stuck at "Downloading…"** — check egress to `object.pouta.csc.fi`. The
  default connect timeout is 30 s; the per-attempt read timeout is 5 min.
- **`HTTP 404` for an obscure pair** — not every pair exists in every corpus.
  Try with `--corpus open_subtitles` only or pick a different corpus.
- **`File too large` error** — bump `--max-file-size-mb` or split your
  ingest by corpus.
- **Slow inserts** — the bottleneck is usually disk-flush in the
  PostgreSQL WAL. Check `SELECT * FROM pg_stat_activity` while a run is
  in flight; use a tuned `postgresql.conf` for bulk loads if needed.
