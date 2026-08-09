-- Frequent multi-word units extracted from distinct foreign surfaces.
-- Translation rows are created per native language and linked back, so
-- provenance is a join and a re-gloss updates the linked row.
CREATE TABLE IF NOT EXISTS phrases (
    id             SERIAL PRIMARY KEY,
    language_id    INTEGER NOT NULL REFERENCES languages(id),
    phrase         TEXT NOT NULL,
    token_count    INTEGER NOT NULL CHECK (token_count BETWEEN 2 AND 5),
    sentence_count INTEGER NOT NULL,
    created_at     TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (language_id, phrase)
);

-- ON DELETE SET NULL: pruning deletes translations directly; the phrase
-- keeps its translation text and becomes eligible for re-promotion.
CREATE TABLE IF NOT EXISTS phrase_translations (
    id                 SERIAL PRIMARY KEY,
    phrase_id          INTEGER NOT NULL REFERENCES phrases(id) ON DELETE CASCADE,
    native_language_id INTEGER NOT NULL REFERENCES languages(id),
    translation        TEXT NOT NULL,
    translation_id     INTEGER REFERENCES translations(id) ON DELETE SET NULL,
    translated_at      TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (phrase_id, native_language_id)
);

CREATE INDEX IF NOT EXISTS idx_phrase_translations_translation
    ON phrase_translations (translation_id);
