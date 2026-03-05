CREATE INDEX IF NOT EXISTS idx_translations_from_phrase
    ON translations (from_language_id, from_phrase);

CREATE INDEX IF NOT EXISTS idx_translations_to_phrase
    ON translations (to_language_id, to_phrase);

ALTER TABLE translations
    ADD CONSTRAINT chk_from_phrase_length CHECK (char_length(from_phrase) <= 1000);

ALTER TABLE translations
    ADD CONSTRAINT chk_to_phrase_length CHECK (char_length(to_phrase) <= 1000);
