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
| `028_grammar_syllabus_identity.sql` | `grammar_rule_aliases`; gives `grammar_rules` a stable `slug` and provenance. |
| `029_quiz_item_bank.sql` | `quiz_items`, `quiz_item_exposures` — the persisted exercise bank. |
| `030_grammar_mastery.sql` | `grammar_sessions`, `grammar_attempts`, `grammar_mastery`, `grammar_review_baselines`, `placement_attempts`. |
| `031_grammar_change_feeds.sql` | `grammar_changes`, `quiz_item_changes` — commit-order-safe feeds for devices. |
| `032_corpus_card_change_visibility.sql` | Gives `corpus_changes` and `card_changes` the same `xact_id` guard. |

Migrations 012–027 are omitted here; they add the media, phrase, word-candidate
and mobile-sync tables described in their own sections.

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

### `grammar_rule_aliases`

```sql
CREATE TABLE grammar_rule_aliases (
    rule_id INTEGER NOT NULL REFERENCES grammar_rules(id) ON DELETE CASCADE,
    slug    VARCHAR(128) NOT NULL,
    PRIMARY KEY (rule_id, slug)
);
```

A grammar point is identified by its slug rather than by its title, because a
title is prose and prose gets rewritten. When
[`refresh-syllabus`](cli-reference.md#refresh-syllabus) improves the wording of
a machine-generated point, the old slug is kept here so that attempts recorded
against it still find their point.

### Provenance

`grammar_rules.source` records where a point came from, and a learner may
reasonably trust the tiers differently:

| Source | Meaning | Shown as |
|--------|---------|----------|
| `reference` | Imported from a curated inventory. | Curated |
| `manual` | Entered by hand. | Curated |
| `llm`, `ai`, `pdf` | Generated or extracted. | Generated |

[`refresh-syllabus`](cli-reference.md#refresh-syllabus) rewrites only the
generated tiers; curated wording is the reason it was curated.

## Quiz item bank

### `quiz_items`

```sql
CREATE TABLE quiz_items (
    id             SERIAL PRIMARY KEY,
    rule_id        INTEGER NOT NULL REFERENCES grammar_rules(id) ON DELETE CASCADE,
    revision       INTEGER NOT NULL DEFAULT 1,
    kind           VARCHAR(16) NOT NULL CHECK (kind IN ('cloze', 'multiple_choice')),
    prompt         TEXT NOT NULL,
    answer         TEXT,
    accepted       JSONB NOT NULL DEFAULT '[]'::jsonb,
    options        JSONB,
    correct_option VARCHAR(32),
    status         VARCHAR(16) NOT NULL DEFAULT 'candidate'
                   CHECK (status IN ('candidate', 'active', 'rejected', 'retired')),
    content_sha256 CHAR(64) NOT NULL,
    UNIQUE (rule_id, content_sha256)
);
```

Content is immutable once written, enforced by a trigger: an attempt uploaded
from a device that has been offline for a fortnight must be gradeable against
the item as the learner actually saw it, and an edit would silently change the
answer underneath it. Editing means inserting a new revision and retiring the
old row, so nothing is ever hard-deleted. `status` moves only through
[`promote-items`](cli-reference.md#promote-items).

Options carry stable identifiers (`o1`…`oN`) rather than positions, so a
shuffled presentation cannot change which answer was given.

### `quiz_item_exposures`

Records which items a learner has already been shown, so that selection spreads
exposure across the bank rather than drilling one sentence.

## Grammar mastery tables

### `grammar_sessions`

A sitting, `practice` or `placement`. It is a first-class row because the rating
rule depends on it: within one session the first item served for a point opens
an interaction, and without a session identifier "the first attempt" cannot be
told from a retry.

### `grammar_attempts`

```sql
CREATE TABLE grammar_attempts (
    user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    event_id      UUID NOT NULL,
    session_id    UUID NOT NULL REFERENCES grammar_sessions(id) ON DELETE CASCADE,
    device_id     UUID,
    rule_id       INTEGER NOT NULL REFERENCES grammar_rules(id) ON DELETE CASCADE,
    item_id       INTEGER NOT NULL REFERENCES quiz_items(id) ON DELETE RESTRICT,
    item_revision INTEGER NOT NULL,
    ordinal       SMALLINT NOT NULL,
    correct       BOOLEAN NOT NULL,
    hint_shown    BOOLEAN NOT NULL DEFAULT FALSE,
    occurred_at   TIMESTAMP WITH TIME ZONE NOT NULL,
    source        VARCHAR(16) NOT NULL CHECK (source IN ('web', 'mobile')),
    PRIMARY KEY (user_id, event_id)
);
```

This is the record; everything else about mastery is a projection of it. The
event identifier is generated by the client, so a device uploading after a
fortnight offline writes each attempt exactly once. `occurred_at` is when the
learner answered, which is not when the server heard about it.

### `grammar_mastery` and `grammar_review_baselines`

Two projections of that one stream. `grammar_mastery` holds the FSRS card state
and an exponentially weighted accuracy; `grammar_review_baselines` records where
replay starts, captured once per point before the first attempt it covers and
never moved. Replay from a baseline must reproduce the projection exactly,
whatever order the attempts arrived in.

Holding both scheduling and accuracy risks them drifting apart; deriving both
from one stream makes drift impossible.

The policy that colours the brainmap:

| Accuracy | Attempts ≥ 3 | Fewer than 3 |
|---------:|--------------|--------------|
| ≥ 0.85 | Green | Provisional green |
| ≥ 0.60 | Amber | Provisional amber |
| below | Red | Provisional red |
| _no attempts_ | Unseen | Unseen |

Accuracy is exponentially weighted with α = 0.3 — roughly a five-attempt memory
— and the series starts at the first outcome rather than at one half, which
would paint every freshly attempted point amber whatever the learner answered.
A provisional band renders outlined rather than filled, because one unlucky
answer should not present as settled knowledge of failure. A point with no
attempts is never coloured as known, or as failed.

### `placement_attempts`

One placement run, riding on an ordinary grammar session so that its answers are
graded and recorded exactly like any other practice. It carries no level of its
own, because a run walks several; `level_reached` is null for a learner who did
not pass A1.

## Grammar change feeds

### `grammar_changes` and `quiz_item_changes`

```sql
CREATE TABLE grammar_changes (
    sequence   BIGSERIAL PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rule_id    INTEGER NOT NULL,
    operation  CHAR(1) NOT NULL CHECK (operation IN ('U', 'D')),
    xact_id    BIGINT NOT NULL DEFAULT pg_current_xact_id()::TEXT::BIGINT,
    changed_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

`BIGSERIAL` allocates its number when the row is inserted, not when the
transaction commits, so two writers commit out of order routinely. A client that
advanced its cursor past a number committed early would never ask for a lower
number committed late, and that change would be lost to it for good.

Recording the writing transaction closes the gap. Readers filter with

```sql
WHERE xact_id < pg_snapshot_xmin(pg_current_snapshot())::TEXT::BIGINT
```

which serves only rows whose transaction finished before every transaction still
in flight. The cost is latency, never correctness: a change waits for its
neighbours rather than arriving early and alone. An empty page therefore leaves
the client's cursor exactly where it was.

Triggers on `grammar_mastery` and `quiz_items` populate the feeds.

The older `corpus_changes` and `card_changes` feeds from migration 022 were born
without this guard and carried the hazard until migration 032 gave them the same
`xact_id` column and the same predicate on every read, watermarks included. A
watermark that ran ahead of what the pages can serve would invite a client past
precisely the rows the column exists to protect, which is why it is not enough to
filter the pages alone.

One reader is deliberately left unfiltered. The per-card cursor carried by a
card *snapshot* is a version stamp that the change feed later compares for
equality, not a position a client advances from, and filtering it would fail a
request whose neighbour merely happened to be in flight.

## Mobile offline store

`wisecrow-mobile` keeps its own SQLite database, migrated from
`wisecrow-mobile/migrations/`. Every table is keyed by `(profile_id, user_id)`
and cascades from `profile_users`, so signing a profile out removes its copy of
the material with it. `002_grammar.sql` adds the grammar half.

| Table | Owner | Purpose |
|-------|-------|---------|
| `grammar_rules` | server | Mirror of the syllabus points the device has been served. |
| `grammar_items` | server | Mirror of the item bank, answers included. |
| `grammar_mastery` | server | Mirror of the mastery projection, overwritten by each page. |
| `grammar_attempt_outbox` | device | Answers waiting to be told to the server. |
| `grammar_sync_state` | device | How far each feed has been followed, per language. |

The mirrors and the outbox move in opposite directions. A mirror row is only
ever replaced by what the server sends, and its cursor only ever moves forward
over changes the server has declared visible. An outbox row is written by the
device and deleted only once the server has acknowledged the answer it holds.

### `grammar_items`

```sql
CREATE TABLE grammar_items (
    profile_id     TEXT NOT NULL,
    user_id        INTEGER NOT NULL,
    item_id        INTEGER NOT NULL,
    rule_id        INTEGER NOT NULL,
    revision       INTEGER NOT NULL,
    language       TEXT NOT NULL,
    prompt         TEXT NOT NULL,
    hint           TEXT,
    options        TEXT NOT NULL DEFAULT '[]',
    answer         TEXT,
    accepted       TEXT NOT NULL DEFAULT '[]',
    correct_option TEXT,
    PRIMARY KEY (profile_id, user_id, item_id),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);
```

This is the one place an answer travels to a device. The web presentation never
carries one, but a device out of contact still has to tell the learner whether
they were right. The verdict it shows is provisional: the server regrades every
answer when the outbox drains, and the mastery it returns is what counts.
`options` and `accepted` are JSON arrays; `correct_option` being present is what
makes an item multiple choice rather than a cloze.

### `grammar_attempt_outbox`

```sql
CREATE TABLE grammar_attempt_outbox (
    profile_id   TEXT NOT NULL,
    user_id      INTEGER NOT NULL,
    event_id     TEXT NOT NULL,
    session_id   TEXT NOT NULL,
    item_id      INTEGER NOT NULL,
    revision     INTEGER NOT NULL,
    answer       TEXT NOT NULL,
    chose_option INTEGER NOT NULL CHECK (chose_option IN (0, 1)),
    hint_shown   INTEGER NOT NULL CHECK (hint_shown IN (0, 1)),
    ordinal      INTEGER NOT NULL CHECK (ordinal >= 1),
    occurred_at  TEXT NOT NULL,
    language     TEXT NOT NULL,
    level        TEXT,
    queued_at    INTEGER NOT NULL,
    PRIMARY KEY (profile_id, user_id, event_id),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);
```

`event_id` is the idempotency key the server deduplicates on, so a batch replayed
after a lost response is accepted once and reported `Duplicate` thereafter.
`queued_at` is a monotonic counter within the device and orders the drain, which
`occurred_at` alone cannot: the server's rating rule turns on which answer came
first within a session, and two answers can share a timestamp.

### `grammar_sync_state`

One row per learner per language, holding `bank_cursor` and `mastery_cursor`.
A cursor advances only to the `next_cursor` the server returns, and an empty
page returns the cursor unchanged — see the change feeds above for why a client
that advanced past an empty page would silently lose work.

`002_grammar.sql` also rebuilds `sync_state` to widen its `phase` check with
`grammar_attempts`, `grammar_bank` and `grammar_mastery`. SQLite cannot alter a
constraint in place, so the table is recreated and its rows carried across.

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
