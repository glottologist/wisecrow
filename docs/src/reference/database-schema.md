# Database schema

Wisecrow's migrations are embedded in `wisecrow-core` and applied via
`sqlx::migrate!("./migrations")` the first time a command opens a pool. The
files live at `wisecrow-core/migrations/` and run in numeric order.

| File | Purpose |
|------|---------|
| `001_languages_table.sql` | `languages` — registry of ISO 639 codes. |
| `002_translations_table.sql` | `translations` — source/target phrase pairs. |
| `003_performance_indexes.sql` | Indexes for translation lookups. |
| `004_frequency_and_srs.sql` | `frequency` column, `cards`, `sessions`, `session_cards`, `media_cache`. |
| `005_fix_translation_unique_constraint.sql` | Tightens the unique constraint to include `to_phrase`. |
| `006_cefr_grammar.sql` | `cefr_levels`, `grammar_rules`, `rule_examples`. |
| `007_users.sql` | `users` table and default user seed. |
| `008_sync_metadata.sql` | `sync_metadata` for sync cursor tracking. |
| `009_dnb.sql` | `dnb_sessions` and `dnb_trials`. |
| `010_glosses.sql` | `glosses` cache for Leipzig interlinear glosses. |
| `011_card_user_scoping.sql` | Adds `cards.user_id`, replaces unique key with `(translation_id, user_id)`. |

## Core tables

### `languages`

```sql
CREATE TABLE languages (
    id   SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    code VARCHAR(16)  NOT NULL UNIQUE
);
```

Seeded lazily by `DatabasePersister::ensure_language` whenever a new code is
encountered. The 102 languages recognised by the CLI are listed in
`SUPPORTED_LANGUAGE_INFO` (`wisecrow-core/src/cli.rs`).

### `translations`

```sql
CREATE TABLE translations (
    id               SERIAL PRIMARY KEY,
    from_language_id INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    from_phrase      TEXT NOT NULL,
    to_language_id   INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    to_phrase        TEXT NOT NULL,
    frequency        INTEGER NOT NULL DEFAULT 1,
    created_at       TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (from_language_id, from_phrase, to_language_id, to_phrase)
);
```

Repeated ingest is safe: the upsert increments `frequency` instead of erroring.

### `cards`

```sql
CREATE TABLE cards (
    id              SERIAL PRIMARY KEY,
    translation_id  INTEGER NOT NULL REFERENCES translations(id) ON DELETE CASCADE,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    stability       REAL NOT NULL DEFAULT 0.0,
    difficulty      REAL NOT NULL DEFAULT 0.0,
    elapsed_days    INTEGER NOT NULL DEFAULT 0,
    scheduled_days  INTEGER NOT NULL DEFAULT 0,
    reps            INTEGER NOT NULL DEFAULT 0,
    lapses          INTEGER NOT NULL DEFAULT 0,
    state           SMALLINT NOT NULL DEFAULT 0,
    last_review     TIMESTAMP WITH TIME ZONE,
    due             TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (translation_id, user_id)
);
```

The `user_id` column makes SRS state per-user: each user has their own card row
for a given translation. Migration 011 backfills `user_id = 1` (the default
user from migration 007) for any pre-existing rows.

`state` follows `CardStatus`:

| Value | Status |
|-------|--------|
| `0` | New |
| `1` | Learning |
| `2` | Review |
| `3` | Relearning |

> **Note:** FSRS uses `f64` internally; the `cards` table stores `REAL` (`f32`).
> The narrowing in `srs::scheduler::f64_to_f32_clamped` is intentional and
> guarantees no NaN/Infinity ever reaches the database.

### `sessions` and `session_cards`

```sql
CREATE TABLE sessions (
    id           SERIAL PRIMARY KEY,
    native_lang  VARCHAR(16) NOT NULL,
    foreign_lang VARCHAR(16) NOT NULL,
    deck_size    INTEGER NOT NULL,
    speed_ms     INTEGER NOT NULL DEFAULT 3000,
    started_at   TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    paused_at    TIMESTAMP WITH TIME ZONE,
    completed_at TIMESTAMP WITH TIME ZONE
);

CREATE TABLE session_cards (
    session_id   INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    card_id      INTEGER NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    answered     BOOLEAN NOT NULL DEFAULT FALSE,
    rating       SMALLINT,
    answered_at  TIMESTAMP WITH TIME ZONE,
    PRIMARY KEY (session_id, card_id)
);
```

Resumable sessions are those with `paused_at IS NOT NULL` and
`completed_at IS NULL`. `SessionManager::resume` returns the most recent one
per `(user_id, native_lang, foreign_lang)`.

## Grammar tables

### `cefr_levels`

Pre-seeded with all six CEFR codes (`A1`–`C2`).

### `grammar_rules`

```sql
CREATE TABLE grammar_rules (
    id            SERIAL PRIMARY KEY,
    language_id   INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    cefr_level_id INTEGER NOT NULL REFERENCES cefr_levels(id),
    title         TEXT NOT NULL,
    explanation   TEXT NOT NULL,
    source        VARCHAR(32) NOT NULL DEFAULT 'manual',
    created_at    TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at    TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (language_id, cefr_level_id, title)
);
```

`source` is a `RuleSource`: `manual`, `ai`, or `pdf`. The unique constraint
on `(language_id, cefr_level_id, title)` makes the import-from-AI flow
idempotent.

### `rule_examples`

```sql
CREATE TABLE rule_examples (
    id          SERIAL PRIMARY KEY,
    rule_id     INTEGER NOT NULL REFERENCES grammar_rules(id) ON DELETE CASCADE,
    sentence    TEXT NOT NULL,
    translation TEXT,
    is_correct  BOOLEAN NOT NULL DEFAULT TRUE
);
```

Examples can be incorrect on purpose — they power "spot the mistake"
multiple-choice quizzes.

## Dual n-back tables

### `dnb_sessions`

Stores the start, peak, and end `n_level` plus the rolling accuracy figures.
A session is open until `completed_at` is set by
`DnbSessionRepository::complete_session`.

### `dnb_trials`

One row per trial. Foreign keys link both stimuli back to the
`translations` table so cards can be reviewed by translation id when
`apply_srs_feedback` runs.

## Sync table

```sql
CREATE TABLE sync_metadata (
    remote_url     TEXT NOT NULL,
    table_name     TEXT NOT NULL,
    last_synced_at TIMESTAMP WITH TIME ZONE NOT NULL,
    PRIMARY KEY (remote_url, table_name)
);
```

Records the most recent successful sync per remote per table. Used as a
liveness marker, not as a cursor — the cursor is the remote primary key.

## Media cache

```sql
CREATE TABLE media_cache (
    id             SERIAL PRIMARY KEY,
    translation_id INTEGER NOT NULL REFERENCES translations(id) ON DELETE CASCADE,
    media_type     VARCHAR(16) NOT NULL,
    file_path      TEXT NOT NULL,
    source_url     TEXT,
    created_at     TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (translation_id, media_type)
);
```

`media_type` is `audio` or `image`. The on-disk files live under
`$XDG_DATA_HOME/wisecrow/cache/{audio,image}/<translation_id>.<ext>`.
`MediaCache` always validates the cached path is under the cache root before
serving it, so a poisoned database row cannot cause the TUI to read an
arbitrary file.

### `glosses`

```sql
CREATE TABLE glosses (
    id              SERIAL PRIMARY KEY,
    sentence_hash   CHAR(64) NOT NULL,
    lang_code       VARCHAR(16) NOT NULL,
    gloss_text      TEXT NOT NULL,
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (sentence_hash, lang_code)
);
CREATE INDEX idx_glosses_lookup ON glosses (sentence_hash, lang_code);
```

Cache for `wisecrow gloss` and the TUI's `g`-keypress overlay.
`sentence_hash` is the SHA-256 hex of the sentence; the `(hash, lang_code)`
pair is unique. This lets the same row serve both the freeform CLI path
(arbitrary input string) and the TUI path (a card's `to_phrase`) without an
FK to `translations`. `--refresh` forces an `INSERT ... ON CONFLICT DO
UPDATE` to replace the cached gloss in place.

## Indexes worth knowing

- `idx_cards_due` — used by `CardManager::due_cards`.
- `idx_translations_frequency (from_language_id, to_language_id, frequency DESC)`
  — used by `VocabularyQuery::unlearned` and `VocabularyQuery::learned`.
- `idx_session_cards_position` — used to load decks in display order.
- `idx_dnb_trials_session` and the audio/visual translation indexes — used by
  `apply_srs_feedback`.
- `idx_glosses_lookup (sentence_hash, lang_code)` — used by the gloss cache
  on every `wisecrow gloss` call and TUI `g`-overlay fetch.
