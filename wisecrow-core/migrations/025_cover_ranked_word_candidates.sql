-- Create the replacement CONCURRENTLY before migrating an existing production corpus.
-- Included text lets both ranking stages avoid fetching every candidate from the heap.
CREATE INDEX IF NOT EXISTS idx_translations_ranked_to_word_covering
    ON translations (
        from_language_id,
        to_language_id,
        lower(btrim(to_phrase, '.,!?;:"''¡¿')),
        corpus_frequency DESC,
        id
    )
    INCLUDE (to_phrase, from_phrase)
    WHERE corpus_frequency > 1
      AND LENGTH(from_phrase) BETWEEN 2 AND 200
      AND LENGTH(to_phrase) BETWEEN 2 AND 200;

DROP INDEX IF EXISTS idx_translations_ranked_to_word;
