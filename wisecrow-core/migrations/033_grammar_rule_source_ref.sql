-- Where a grammar point was read from.
--
-- A point synthesised from a document cites the document and the page the
-- model took it from, e.g. `yo-puedo-1-2021.pdf p.148`, so that its prose can
-- be checked against the source it claims. Seeded and hand-written points have
-- no such place, which is why the column is nullable rather than defaulted.

ALTER TABLE grammar_rules
    ADD COLUMN IF NOT EXISTS source_ref TEXT;
