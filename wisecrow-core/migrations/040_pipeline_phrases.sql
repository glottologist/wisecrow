-- Phrase mining for the corpus unit. A gloss that reuses a corpus line must
-- not remove that line from the evidence view, or the rank checksum changes
-- and the mine starts again. Rows created for a phrase stay out of the view.
-- Links that already exist default to owned, so this migration does not put
-- them back into the corpus.
ALTER TABLE phrase_translations
    ADD COLUMN owns_translation BOOLEAN NOT NULL DEFAULT TRUE;

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
          );

CREATE TABLE pipeline_phrase_mines (
    foreign_code VARCHAR(16) PRIMARY KEY,
    evidence_checksum BYTEA NOT NULL CHECK (octet_length(evidence_checksum) = 32),
    surface_cursor TEXT NOT NULL DEFAULT '',
    generation BIGINT NOT NULL DEFAULT 1 CHECK (generation > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE pipeline_phrase_counts (
    foreign_code VARCHAR(16) NOT NULL,
    phrase TEXT NOT NULL,
    token_count INTEGER NOT NULL CHECK (token_count BETWEEN 2 AND 5),
    sentence_count INTEGER NOT NULL CHECK (sentence_count > 0),
    PRIMARY KEY (foreign_code, phrase)
);

CREATE TABLE pipeline_phrase_stamps (
    foreign_code VARCHAR(16) PRIMARY KEY,
    evidence_checksum BYTEA NOT NULL CHECK (octet_length(evidence_checksum) = 32),
    mined_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE pipeline_corpus_turn (
    id BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    kind VARCHAR(16) NOT NULL DEFAULT 'gloss'
        CHECK (kind IN ('gloss', 'phrase_audio', 'word_audio')),
    gloss_pair VARCHAR(16) NOT NULL DEFAULT '',
    phrase_audio_pair VARCHAR(16) NOT NULL DEFAULT ''
);

INSERT INTO pipeline_corpus_turn (id) VALUES (TRUE);
