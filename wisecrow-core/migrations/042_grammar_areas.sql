-- Grammar areas: a second axis of the syllabus, orthogonal to CEFR.
--
-- A rule carries the area it teaches. A top-ranked word carries a word class
-- and a lemma, recorded per language because a word's part of speech does not
-- depend on the language it is learned from. A verb form is stored once per
-- language, lemma and slot; its face, the native rendering a card shows, is
-- stored once per native language and linked to the translations row a card
-- is made from. Generated rows are learning material, not corpus evidence.

ALTER TABLE grammar_rules
    ADD COLUMN area VARCHAR(16)
        CHECK (area IN ('verb', 'preposition', 'pronoun', 'adjective', 'adverb', 'other'));

CREATE INDEX idx_grammar_rules_language_area ON grammar_rules (language_id, area);

-- `word` is normalised as migration 016 normalises a phrase: lower-cased, with
-- the edge punctuation of `MATCH_TRIM_CHARS` trimmed.
CREATE TABLE word_classes (
    language_id        INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    word               TEXT NOT NULL CHECK (char_length(word) BETWEEN 1 AND 64),
    class              VARCHAR(16) NOT NULL
                       CHECK (class IN ('verb', 'noun', 'adjective', 'adverb', 'pronoun',
                                        'preposition', 'conjunction', 'determiner',
                                        'numeral', 'interjection', 'other')),
    lemma              TEXT NOT NULL CHECK (char_length(lemma) BETWEEN 1 AND 64),
    classifier_version INTEGER NOT NULL CHECK (classifier_version > 0),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (language_id, word)
);

CREATE INDEX idx_word_classes_class ON word_classes (language_id, class);
CREATE INDEX idx_word_classes_lemma ON word_classes (language_id, lemma);

CREATE TABLE verb_forms (
    id               SERIAL PRIMARY KEY,
    language_id      INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    lemma            TEXT NOT NULL CHECK (char_length(lemma) BETWEEN 1 AND 64),
    slot             VARCHAR(32) NOT NULL CHECK (btrim(slot) <> ''),
    form             TEXT NOT NULL CHECK (char_length(form) BETWEEN 1 AND 64),
    attested         BOOLEAN NOT NULL DEFAULT FALSE,
    paradigm_version INTEGER NOT NULL CHECK (paradigm_version > 0),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (language_id, lemma, slot)
);

-- One face per native language. A face either owns the generated row it
-- created or reuses a row that already had exactly this key, which then keeps
-- its ordinary deck and evidence behaviour.
CREATE TABLE verb_form_translations (
    verb_form_id       INTEGER NOT NULL REFERENCES verb_forms(id) ON DELETE CASCADE,
    native_language_id INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    native_face        TEXT NOT NULL CHECK (char_length(native_face) BETWEEN 1 AND 200),
    translation_id     INTEGER NOT NULL REFERENCES translations(id) ON DELETE CASCADE,
    owns_translation   BOOLEAN NOT NULL,
    PRIMARY KEY (verb_form_id, native_language_id)
);

CREATE INDEX idx_verb_form_translations_translation
    ON verb_form_translations (translation_id);

-- The paradigm work ledger. A lemma rejected three times at a version stops
-- asking for work at that version and is listed by `export-grammar
-- --verb-forms`; a version bump starts every lemma afresh.
CREATE TABLE verb_paradigm_runs (
    language_id      INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    lemma            TEXT NOT NULL,
    paradigm_version INTEGER NOT NULL CHECK (paradigm_version > 0),
    status           VARCHAR(16) NOT NULL CHECK (status IN ('ok', 'rejected')),
    attempts         INTEGER NOT NULL DEFAULT 1 CHECK (attempts >= 1),
    last_error       TEXT,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (language_id, lemma, paradigm_version)
);

-- The presentation of a generated row that no word gloss may override. The
-- story reader uses the same table with owner 'reader'.
CREATE TABLE owned_presentations (
    translation_id INTEGER PRIMARY KEY REFERENCES translations(id) ON DELETE CASCADE,
    owner          VARCHAR(16) NOT NULL CHECK (owner IN ('verb_form', 'reader')),
    native_face    TEXT NOT NULL
                   CHECK (btrim(native_face) <> '' AND char_length(native_face) <= 200),
    teachable      BOOLEAN NOT NULL DEFAULT TRUE
);

-- Same columns, in the same order, as migration 023, plus a trailing `owner`.
-- Two slots that share a foreign spelling with a glossed word would otherwise
-- both show that word's gloss.
CREATE OR REPLACE VIEW translation_presentations AS
SELECT t.id AS translation_id,
       CASE
           WHEN owned.translation_id IS NOT NULL THEN owned.native_face
           WHEN linked.is_phrase THEN linked.from_phrase
           ELSE COALESCE(g.translation, t.from_phrase)
       END AS from_phrase,
       CASE
           WHEN owned.translation_id IS NOT NULL THEN t.to_phrase
           WHEN linked.is_phrase THEN linked.to_phrase
           ELSE COALESCE(g.display_form, t.to_phrase)
       END AS to_phrase,
       fl.code AS native_lang,
       tl.code AS foreign_lang,
       CASE
           WHEN owned.translation_id IS NOT NULL THEN owned.teachable
           WHEN linked.is_phrase THEN TRUE
           ELSE COALESCE(g.teachable, TRUE)
       END AS teachable,
       CASE
           WHEN owned.translation_id IS NOT NULL OR linked.is_phrase THEN NULL
           ELSE g.image_query
       END AS image_query,
       COALESCE(linked.is_phrase, FALSE) AS is_phrase,
       CASE
           WHEN owned.translation_id IS NOT NULL OR linked.is_phrase THEN 0
           ELSE COALESCE(g.presentation_version, 0)
       END AS presentation_version,
       owned.owner AS owner
FROM translations t
JOIN languages fl ON fl.id = t.from_language_id
JOIN languages tl ON tl.id = t.to_language_id
LEFT JOIN owned_presentations owned ON owned.translation_id = t.id
LEFT JOIN LATERAL (
    SELECT TRUE AS is_phrase,
           pt.translation AS from_phrase,
           p.phrase AS to_phrase
    FROM phrase_translations pt
    JOIN phrases p ON p.id = pt.phrase_id
    WHERE pt.translation_id = t.id
    LIMIT 1
) linked ON TRUE
LEFT JOIN word_glosses g
  ON owned.translation_id IS NULL
 AND NOT COALESCE(linked.is_phrase, FALSE)
 AND g.native_lang = fl.code
 AND g.lang_code = tl.code
 AND g.word = lower(btrim(t.to_phrase, '.,!?;:"''¡¿'));

-- A generated verb form is not corpus text, so neither ranking nor
-- attestation may count it. A reused row is corpus text and stays.
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
          );

-- Kept so that resuming a paused session restores the area it was drawn from.
ALTER TABLE sessions
    ADD COLUMN area VARCHAR(16)
        CHECK (area IN ('verb', 'preposition', 'pronoun', 'adjective', 'adverb'));
