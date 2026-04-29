# Your first ingest

This walkthrough takes you from an empty database to a learning session in
under ten minutes. We will use English (`en`) as the native language and
Spanish (`es`) as the foreign language; substitute any pair from
`wisecrow list-languages`.

## 1. Verify the database is reachable

```sh
psql "$WISECROW__DB_URL" -c '\dt'
```

You should connect successfully. The table list may be empty — that is fine,
migrations run on first ingest.

## 2. Pull a small corpus

OpenSubtitles is the smallest of the three corpora and the fastest to test.
The `ingest` command downloads, parses, and persists in one pass.

```sh
wisecrow ingest -n en -f es --corpus open_subtitles
```

What happens, in order:

1. The CLI validates language codes and constructs OPUS URLs.
2. A Tokio task is spawned per file (TMX + XML alignment).
3. Each task downloads (with retry/backoff), decompresses gzip, and streams
   the file through `quick-xml`.
4. A bounded mpsc channel (capacity 1000) feeds parsed pairs to a writer
   task, which batches them in groups of 1000 and writes them transactionally.
5. SIGINT and SIGTERM trigger a graceful shutdown; in-flight batches finish
   before exit.

Progress bars appear during download. When a file completes you will see a
log line like `Ingested 12345 items from es_OpenSubtitles.tmx`.

## 3. Inspect the data

```sh
psql "$WISECROW__DB_URL" <<'SQL'
SELECT count(*) FROM translations;
SELECT count(*) FROM languages;
SELECT from_phrase, to_phrase, frequency
  FROM translations
  ORDER BY frequency DESC
  LIMIT 10;
SQL
```

The most frequent rows tend to be short stop-word phrases: this is normal and
exactly the signal SRS card selection relies on.

## 4. Start a learning session

```sh
wisecrow learn -n en -f es --deck-size 20 --speed-ms 2000
```

The TUI opens. Use the keyboard to rate cards (`Again`, `Hard`, `Good`, `Easy`).
Pressing `q` pauses and saves the session — `wisecrow learn` with the same
language pair next time will resume it.

## What just happened

| Step | Code path |
|------|-----------|
| URL construction | `wisecrow_core::files::LanguageFiles::new` |
| Download + decompress | `wisecrow_core::downloader::Downloader::download_to` |
| Parse | `wisecrow_core::ingesting::parsing::CorpusParser` |
| Persist | `wisecrow_core::ingesting::persisting::DatabasePersister::consume` |
| Session creation | `wisecrow_core::srs::session::SessionManager::create` |
| Card scheduling | `wisecrow_core::srs::scheduler::CardManager::review` |

## Where to next

- Drill more cards or practise differently — see the [Learn TUI guide](../guides/learn-tui.md)
  and the [Dual n-back guide](../guides/dual-n-back.md).
- Add structured grammar rules to your deck — see [Grammar workflows](../guides/grammar-workflows.md).
- Move data between machines — see [Sync workflow](../guides/sync-workflow.md).
