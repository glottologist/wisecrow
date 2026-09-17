-- Prebuild CONCURRENTLY before deploying these queries on an existing corpus.
-- Widens the covering index's predicate to admit single-character forms so
-- that Chinese and Japanese particles reach candidate discovery; learning
-- admission of such forms is gated by a current teachable presentation in the
-- ranking SQL, not by this index.
CREATE INDEX IF NOT EXISTS idx_translations_ranked_to_word_v2
    ON translations (
        from_language_id,
        to_language_id,
        lower(btrim(to_phrase, '.,!?;:"''¡¿')),
        corpus_frequency DESC,
        id
    )
    INCLUDE (to_phrase, from_phrase)
    WHERE corpus_frequency > 1
      AND LENGTH(from_phrase) BETWEEN 1 AND 200
      AND LENGTH(to_phrase) BETWEEN 1 AND 200;
