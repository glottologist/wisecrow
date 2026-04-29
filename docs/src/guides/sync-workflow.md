# Sync workflow

The `sync` command pulls `languages`, `translations`, and `grammar_rules`
from a remote Wisecrow instance and merges them into your local database.
It is one-way (pull-only) and idempotent: running it twice in a row is a
no-op the second time.

## Concept

```text
┌──────────────────┐    /api/sync_languages?after_id=...    ┌──────────────────┐
│ local Wisecrow   │ ─────────────────────────────────────▶ │  remote Wisecrow │
│ PostgreSQL       │ ◀───── paginated DTOs (id-cursor) ──── │  HTTP server     │
└──────────────────┘                                        └──────────────────┘
```

The cursor is the remote primary key (`after_id`). Each page is upserted
into the local database in code-not-id space, so the local PKs can drift
freely from the remote.

## Run it

```sh
wisecrow sync --remote https://wisecrow.example.com --api-key "$WISECROW_REMOTE_KEY"
```

| Flag | Required | Purpose |
|------|:--------:|---------|
| `--remote` | yes | Base URL of the remote. The path components are joined via `Url::join`. |
| `--api-key` | no | Sent verbatim as the `x-api-key` header. |

You can also set `WISECROW__SYNC_API_KEY` and skip `--api-key`.

## What is synced

| Table | Endpoint | Conflict resolution |
|-------|----------|--------------------|
| `languages` | `/api/sync_languages` | Insert by code; updates do nothing meaningful (codes are unique). |
| `translations` | `/api/sync_translations` | Upsert with `frequency = GREATEST(local, remote)`. |
| `grammar_rules` (and examples) | `/api/sync_grammar_rules` | Upsert by `(language_id, cefr_level_id, title)`; examples are wiped and re-inserted to keep them consistent. |

`media_cache`, `cards`, `sessions`, `dnb_*`, and `users` are **not** synced
— they are per-instance state.

## Track what was synced

After a successful run the local `sync_metadata` table is updated:

```sql
SELECT remote_url, table_name, last_synced_at
FROM sync_metadata
ORDER BY last_synced_at DESC;
```

The row exists per `(remote_url, table_name)`, and `last_synced_at` is the
end-of-table timestamp, not a per-row marker. If a sync errors halfway
through, the row is not updated and the next run starts from the last
successful `after_id`.

## Standing up the remote

The sync client expects three GET endpoints:

| Endpoint | Query | Returns |
|----------|-------|---------|
| `GET /api/sync_languages?after_id=N` | `after_id` | `Vec<SyncLanguageDto>` ordered by id ascending |
| `GET /api/sync_translations?after_id=N` | `after_id` | `Vec<SyncTranslationDto>` ordered by id ascending |
| `GET /api/sync_grammar_rules?after_id=N` | `after_id` | `Vec<SyncGrammarRuleDto>` ordered by id ascending |

Implementations live in `wisecrow-web/src/server/sync.rs` (server feature
required). DTO definitions are in [`wisecrow-dto`](../api/wisecrow-dto.md#sync-dtos).

## When to use it

- **Bootstrapping a new replica.** Start with an empty database, run
  `wisecrow sync` once. You inherit all translations and grammar rules.
- **Adding language pairs from a colleague.** They ingest, you sync. No
  need to re-download the corpora.
- **Backups via replication.** Sync into a read-only replica and snapshot it.

## Limitations

- Sync is one-way. Pushing local data is not implemented.
- There is no conflict UI — the upsert rules above always win.
- Large syncs are linear: paginate-and-upsert is `O(n)` round-trips. A
  fresh sync of a heavy production database may take a few minutes.
- The HTTP client uses a shared `reqwest::Client` with rustls-tls; HTTP
  proxies must be configured via `HTTPS_PROXY`.
