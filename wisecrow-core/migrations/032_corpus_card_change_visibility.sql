-- The corpus and card feeds gain the commit-order guard the grammar feeds got.
--
-- Migration 031 explains the hazard at length and fixed it for the two feeds it
-- introduced, leaving these two — older by nine migrations — carrying it still.
-- The shape of the loss bears repeating: `BIGSERIAL` allocates its number when
-- a row is inserted rather than when the transaction commits, so a writer that
-- took a lower number can commit after one that took a higher number. A reader
-- that has already advanced its cursor past the higher number never asks for
-- the lower one again, and that change is gone from its world for good.
--
-- Stamping each row with its writing transaction lets a reader withhold a row
-- until every transaction beneath it has finished. The cost is latency and
-- never correctness: a change waits for its neighbours rather than arriving
-- early and alone.
--
-- Existing rows are stamped with this migration's own transaction, which is
-- the only honest answer available — the transactions that wrote them are long
-- gone and unrecorded. Once this migration commits, that identifier sits below
-- every snapshot taken afterwards, so those rows become visible immediately and
-- stay so. Both tables are small, so the rewrite the volatile default forces is
-- not worth avoiding.

ALTER TABLE corpus_changes
    ADD COLUMN IF NOT EXISTS xact_id BIGINT NOT NULL
    DEFAULT pg_current_xact_id()::TEXT::BIGINT;

ALTER TABLE card_changes
    ADD COLUMN IF NOT EXISTS xact_id BIGINT NOT NULL
    DEFAULT pg_current_xact_id()::TEXT::BIGINT;

CREATE INDEX IF NOT EXISTS idx_corpus_changes_visibility
    ON corpus_changes (from_language_code, to_language_code, xact_id, sequence);

CREATE INDEX IF NOT EXISTS idx_card_changes_visibility
    ON card_changes (user_id, xact_id, sequence);

-- Superseded by the indexes above rather than merely duplicated by them: every
-- read of these feeds now filters on `xact_id`, so an index that stops at
-- `sequence` can no longer serve one.
DROP INDEX IF EXISTS idx_corpus_changes_pair_sequence;
DROP INDEX IF EXISTS idx_card_changes_user_sequence;
