-- Sentence-derived word candidates and their promotion into learning rows.
--
-- A candidate is a normalised word seen in enough corpus sentences of a pair,
-- with at most three source examples kept as evidence for the model. A
-- promotion links a candidate to the translation row that teaches it: either
-- an existing corpus row (owns_translation = false) or a generated row that
-- the promotion created (owns_translation = true). Generated rows are learning
-- material, not corpus evidence, so every source reader selects from the
-- corpus_evidence_translations view, which also excludes phrase-linked rows.

CREATE TABLE word_candidates (
    id                   BIGSERIAL PRIMARY KEY,
    native_language_id   INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    foreign_language_id  INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    word                 TEXT NOT NULL CHECK (char_length(word) BETWEEN 1 AND 64),
    surface              TEXT NOT NULL CHECK (char_length(surface) BETWEEN 1 AND 200),
    occurrence_count     BIGINT NOT NULL CHECK (occurrence_count >= 2),
    source_upper_id      INTEGER NOT NULL,
    revision             BIGINT NOT NULL DEFAULT 1 CHECK (revision >= 1),
    enriched_revision    BIGINT,
    presentation_version INTEGER NOT NULL DEFAULT 0,
    status               TEXT NOT NULL DEFAULT 'pending'
                         CHECK (status IN ('pending', 'accepted', 'rejected', 'failed')),
    CHECK (native_language_id <> foreign_language_id),
    UNIQUE (native_language_id, foreign_language_id, word)
);

CREATE TABLE word_candidate_examples (
    candidate_id          BIGINT NOT NULL REFERENCES word_candidates(id) ON DELETE CASCADE,
    ordinal               SMALLINT NOT NULL CHECK (ordinal BETWEEN 1 AND 3),
    source_translation_id INTEGER REFERENCES translations(id) ON DELETE SET NULL,
    native_sentence       TEXT NOT NULL CHECK (char_length(native_sentence) BETWEEN 1 AND 200),
    foreign_sentence      TEXT NOT NULL CHECK (char_length(foreign_sentence) BETWEEN 1 AND 200),
    PRIMARY KEY (candidate_id, ordinal)
);

CREATE TABLE word_promotions (
    candidate_id     BIGINT PRIMARY KEY REFERENCES word_candidates(id) ON DELETE CASCADE,
    translation_id   INTEGER UNIQUE REFERENCES translations(id) ON DELETE SET NULL,
    owns_translation BOOLEAN NOT NULL
);

CREATE INDEX idx_word_candidates_pending
    ON word_candidates (native_language_id, foreign_language_id, status,
                        occurrence_count DESC, word);

CREATE VIEW corpus_evidence_translations AS
    SELECT t.*
    FROM translations t
    WHERE NOT EXISTS (
            SELECT 1 FROM word_promotions p
            WHERE p.translation_id = t.id AND p.owns_translation
          )
      AND NOT EXISTS (
            SELECT 1 FROM phrase_translations p
            WHERE p.translation_id = t.id
          );

-- An owned generated row must stay excluded from evidence for as long as it
-- exists, so its link cannot be removed or disowned while the row and both
-- language parents are present. Once the translation (or a parent, through
-- its cascade) is gone, the FK's ON DELETE SET NULL and ordinary cleanup
-- proceed.
CREATE FUNCTION protect_owned_word_link() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.owns_translation AND OLD.translation_id IS NOT NULL
       AND EXISTS (
           SELECT 1 FROM translations t
           JOIN languages n ON n.id = t.from_language_id
           JOIN languages f ON f.id = t.to_language_id
           WHERE t.id = OLD.translation_id
       ) THEN
        IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION 'cannot unlink owned learning translation while it exists';
        END IF;
        IF NOT NEW.owns_translation OR NEW.translation_id IS DISTINCT FROM OLD.translation_id THEN
            RAISE EXCEPTION 'cannot unlink owned learning translation while it exists';
        END IF;
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER protect_owned_word_promotion
    BEFORE DELETE OR UPDATE ON word_promotions
    FOR EACH ROW EXECUTE FUNCTION protect_owned_word_link();
