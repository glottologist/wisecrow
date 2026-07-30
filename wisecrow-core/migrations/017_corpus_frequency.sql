-- `translations.frequency` means two unrelated things depending on how a row
-- got its value. From `frequency --from-corpus` it is a corpus count. Otherwise
-- it is a collision count, because the ingest upsert is
-- `ON CONFLICT ... DO UPDATE SET frequency = frequency + 1`, so a pair carried
-- by two corpora, or by one corpus ingested twice, is incremented regardless of
-- how common the word is.
--
-- Decks filtered on that column, so collision-inflated rows reached learners:
-- 3,246,887 Irish rows sat above 1, of which 1,689 had ever been ranked, and the
-- Irish deck served 3.2M sentences ordered by nothing. No arithmetic on
-- `frequency` can separate the two causes after the fact.
--
-- `corpus_frequency` is written only by ranking, so NULL means "never ranked" —
-- exactly what a deck filter needs. `frequency` keeps its one honest job of
-- counting ingest collisions.
--
-- Deliberately not backfilled. Ranking only ever matches single-word rows, so
-- there is no expression that recovers which existing counts came from it; the
-- languages already seeded are re-ranked instead, which is cheap because that is
-- all ranking touches (gd 21,522, cy 67,658, ga 1,689 entries).
ALTER TABLE translations ADD COLUMN IF NOT EXISTS corpus_frequency INTEGER;

-- Replaces the equivalent index on `frequency`, which decks no longer order by.
DROP INDEX IF EXISTS idx_translations_frequency;

CREATE INDEX IF NOT EXISTS idx_translations_corpus_frequency
    ON translations (from_language_id, to_language_id, corpus_frequency DESC);
