-- A deck teaches a word by showing the native phrase the corpus put beside it,
-- and for most words the corpus put exactly one there. Measured on Gaelic, 200
-- of the deck's 315 words have a single candidate partner, so there is nothing
-- to choose between and no ordering rule can help: whatever the corpus stored is
-- what the learner sees. That is why `Die! → An!` heads the Irish deck and
-- `and Room → dhomh.` sits in Gaelic's second tier.
--
-- The corpus is not short of evidence, only of *single-word* evidence. `dìreach`
-- is chosen from one row and appears in 4,007 of them; `agus` from one row and
-- appears in 3,486. This table holds a translation for the words whose corpus
-- pairing nothing corroborates, so the deck has something to fall back on.
--
-- Distinct from `glosses` (migration 010), which caches a Leipzig interlinear
-- gloss of a *sentence* as free text for reading support. This is a canonical
-- one-to-three-word translation of a single word, and the two are neither
-- interchangeable nor produced by the same prompt.
CREATE TABLE IF NOT EXISTS word_glosses (
    id              SERIAL PRIMARY KEY,
    -- The language being learned, and the word as `unlearned` normalises it:
    -- lower-cased and stripped of the edge punctuation in MATCH_TRIM_CHARS, so
    -- this joins the deck's `norm_to` directly and needs no second convention.
    lang_code       VARCHAR(16) NOT NULL,
    word            TEXT NOT NULL,
    -- The language the learner reads. `glosses` has no such column and so
    -- assumes one native language; a deck does not, and a gloss is only correct
    -- for the language it was asked in.
    native_lang     VARCHAR(16) NOT NULL,
    translation     TEXT NOT NULL,
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (lang_code, word, native_lang)
);

-- The deck joins on all three columns of the unique key, so the constraint's own
-- index serves the lookup and no second one is created.
