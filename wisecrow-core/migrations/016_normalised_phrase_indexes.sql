-- Frequency lists hold bare words ("beth"), while corpus phrases carry case and
-- edge punctuation ("Beth", "O."). Matching therefore compares a normalised
-- form of each phrase, and without these expression indexes every batch of the
-- `frequency` command would fall back to a sequential scan.
--
-- The btrim character set must stay in step with `MATCH_TRIM_CHARS` in
-- wisecrow-core/src/frequency.rs; the two are compared against each other.

CREATE INDEX IF NOT EXISTS idx_translations_from_phrase_normalised
    ON translations (from_language_id, lower(btrim(from_phrase, '.,!?;:"''¡¿')));

CREATE INDEX IF NOT EXISTS idx_translations_to_phrase_normalised
    ON translations (to_language_id, lower(btrim(to_phrase, '.,!?;:"''¡¿')));
