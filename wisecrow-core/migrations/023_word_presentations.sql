ALTER TABLE word_glosses
    ADD COLUMN IF NOT EXISTS display_form TEXT,
    ADD COLUMN IF NOT EXISTS teachable BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN IF NOT EXISTS image_query TEXT,
    ADD COLUMN IF NOT EXISTS presentation_version INTEGER NOT NULL DEFAULT 0;

UPDATE word_glosses
SET display_form = word
WHERE display_form IS NULL;

ALTER TABLE word_glosses
    ALTER COLUMN display_form SET NOT NULL,
    ADD CONSTRAINT word_glosses_display_form_nonempty
        CHECK (btrim(display_form) <> '' AND char_length(display_form) <= 200),
    ADD CONSTRAINT word_glosses_image_query_valid
        CHECK (image_query IS NULL OR (btrim(image_query) <> '' AND char_length(image_query) <= 200)),
    ADD CONSTRAINT word_glosses_presentation_version_nonnegative
        CHECK (presentation_version >= 0);

ALTER TABLE media_cache
    ADD COLUMN IF NOT EXISTS source_fingerprint VARCHAR(64),
    ADD CONSTRAINT media_cache_source_fingerprint_valid
        CHECK (source_fingerprint IS NULL OR source_fingerprint ~ '^[0-9a-f]{64}$');

CREATE VIEW translation_presentations AS
SELECT t.id AS translation_id,
       CASE
           WHEN linked.is_phrase THEN linked.from_phrase
           ELSE COALESCE(g.translation, t.from_phrase)
       END AS from_phrase,
       CASE
           WHEN linked.is_phrase THEN linked.to_phrase
           ELSE COALESCE(g.display_form, t.to_phrase)
       END AS to_phrase,
       fl.code AS native_lang,
       tl.code AS foreign_lang,
       CASE
           WHEN linked.is_phrase THEN TRUE
           ELSE COALESCE(g.teachable, TRUE)
       END AS teachable,
       CASE WHEN linked.is_phrase THEN NULL ELSE g.image_query END AS image_query,
       COALESCE(linked.is_phrase, FALSE) AS is_phrase,
       CASE
           WHEN linked.is_phrase THEN 0
           ELSE COALESCE(g.presentation_version, 0)
       END AS presentation_version
FROM translations t
JOIN languages fl ON fl.id = t.from_language_id
JOIN languages tl ON tl.id = t.to_language_id
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
  ON NOT COALESCE(linked.is_phrase, FALSE)
 AND g.native_lang = fl.code
 AND g.lang_code = tl.code
 AND g.word = lower(btrim(t.to_phrase, '.,!?;:"''¡¿'));
