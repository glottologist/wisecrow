-- Reader shelf. Statements match wisecrow-core/src/reader/import.rs as Task 10 writes it.

CREATE TABLE books (
    id                  SERIAL PRIMARY KEY,
    import_key          BYTEA NOT NULL UNIQUE CHECK (octet_length(import_key) = 32),
    file                TEXT NOT NULL,
    file_sha256         BYTEA NOT NULL CHECK (octet_length(file_sha256) = 32),
    series              TEXT NOT NULL,
    title               TEXT NOT NULL,
    cefr_level_id       INTEGER NOT NULL REFERENCES cefr_levels(id),
    source_language_id  INTEGER NOT NULL REFERENCES languages(id),
    first_page          INTEGER,
    last_page           INTEGER,
    segmenter_version   SMALLINT NOT NULL,
    status              TEXT NOT NULL CHECK (status IN ('ready', 'needs_pages', 'thin'))
);

CREATE TABLE book_sentences (
    id          SERIAL PRIMARY KEY,
    book_id     INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    chapter     INTEGER NOT NULL,
    paragraph   INTEGER NOT NULL,
    sentence    INTEGER NOT NULL,
    text        TEXT NOT NULL,
    UNIQUE (book_id, position)
);

CREATE TABLE book_translations (
    id                  SERIAL PRIMARY KEY,
    sentence_id         INTEGER NOT NULL REFERENCES book_sentences(id) ON DELETE CASCADE,
    language_id         INTEGER NOT NULL REFERENCES languages(id),
    text                TEXT NOT NULL,
    audio_fingerprint   VARCHAR(64) CHECK (
        audio_fingerprint IS NULL OR audio_fingerprint ~ '^[0-9a-f]{64}$'
    ),
    UNIQUE (sentence_id, language_id)
);

CREATE TABLE book_tokens (
    translation_id  INTEGER NOT NULL REFERENCES book_translations(id) ON DELETE CASCADE,
    ordinal         INTEGER NOT NULL,
    start_byte      INTEGER NOT NULL CHECK (start_byte >= 0),
    end_byte        INTEGER NOT NULL CHECK (end_byte >= start_byte),
    normalised      TEXT NOT NULL CHECK (char_length(normalised) BETWEEN 1 AND 64),
    PRIMARY KEY (translation_id, ordinal)
);

CREATE TABLE book_glosses (
    language_id         INTEGER NOT NULL REFERENCES languages(id),
    native_language_id  INTEGER NOT NULL REFERENCES languages(id),
    normalised          TEXT NOT NULL CHECK (char_length(normalised) BETWEEN 1 AND 64),
    meaning             TEXT,
    lemma               TEXT,
    class               TEXT,
    note                TEXT,
    status              TEXT NOT NULL CHECK (status IN ('ok', 'failed')),
    audio_fingerprint   VARCHAR(64) CHECK (
        audio_fingerprint IS NULL OR audio_fingerprint ~ '^[0-9a-f]{64}$'
    ),
    PRIMARY KEY (language_id, native_language_id, normalised),
    CHECK (
        status = 'failed'
        OR (meaning IS NOT NULL AND lemma IS NOT NULL AND class IS NOT NULL AND note IS NOT NULL)
    )
);

CREATE TABLE book_import_progress (
    book_id      INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    language_id  INTEGER NOT NULL REFERENCES languages(id),
    stage        TEXT NOT NULL DEFAULT 'translate' CHECK (
        stage IN ('translate', 'gloss', 'voice_sentences', 'voice_words', 'complete')
    ),
    cursor       INTEGER NOT NULL DEFAULT 0,
    failed_runs  INTEGER[] NOT NULL DEFAULT '{}',
    completed_at TIMESTAMPTZ,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (book_id, language_id)
);

-- The learner's place follows the ledger file, not a book revision.
CREATE TABLE reading_positions (
    user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    file         TEXT NOT NULL,
    language_id  INTEGER NOT NULL REFERENCES languages(id),
    chapter      INTEGER NOT NULL,
    paragraph    INTEGER NOT NULL,
    sentence     INTEGER NOT NULL,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, file, language_id)
);

CREATE TABLE decks (
    id           SERIAL PRIMARY KEY,
    user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name         TEXT NOT NULL CHECK (btrim(name) <> ''),
    native_lang  VARCHAR(16) NOT NULL,
    foreign_lang VARCHAR(16) NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (user_id, native_lang, foreign_lang, name)
);

CREATE TABLE deck_cards (
    deck_id         INTEGER NOT NULL REFERENCES decks(id) ON DELETE CASCADE,
    translation_id  INTEGER NOT NULL REFERENCES translations(id) ON DELETE CASCADE,
    added_at        TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (deck_id, translation_id)
);

CREATE TABLE reader_translations (
    language_id         INTEGER NOT NULL REFERENCES languages(id),
    native_language_id  INTEGER NOT NULL REFERENCES languages(id),
    normalised          TEXT NOT NULL CHECK (char_length(normalised) BETWEEN 1 AND 64),
    translation_id      INTEGER NOT NULL REFERENCES translations(id) ON DELETE CASCADE,
    owns_translation    BOOLEAN NOT NULL,
    PRIMARY KEY (language_id, native_language_id, normalised)
);

ALTER TABLE sessions
    ADD COLUMN deck_id INTEGER REFERENCES decks(id) ON DELETE SET NULL;

ALTER TABLE sessions
    ADD CONSTRAINT sessions_deck_or_area CHECK (deck_id IS NULL OR area IS NULL);

-- 042's evidence view, plus reader rows that this job created.
CREATE OR REPLACE VIEW corpus_evidence_translations AS
    SELECT t.*
    FROM translations t
    WHERE NOT EXISTS (
            SELECT 1 FROM word_promotions p
            WHERE p.translation_id = t.id AND p.owns_translation
          )
      AND NOT EXISTS (
            SELECT 1 FROM phrase_translations p
            WHERE p.translation_id = t.id AND p.owns_translation
          )
      AND NOT EXISTS (
            SELECT 1 FROM verb_form_translations v
            WHERE v.translation_id = t.id AND v.owns_translation
          )
      AND NOT EXISTS (
            SELECT 1 FROM reader_translations r
            WHERE r.translation_id = t.id AND r.owns_translation
          );
