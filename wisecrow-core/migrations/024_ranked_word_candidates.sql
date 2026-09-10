CREATE INDEX IF NOT EXISTS idx_translations_ranked_to_word
    ON translations (
        from_language_id,
        to_language_id,
        lower(btrim(to_phrase, '.,!?;:"''¡¿')),
        corpus_frequency DESC,
        id
    )
    WHERE corpus_frequency > 1
      AND LENGTH(from_phrase) BETWEEN 2 AND 200
      AND LENGTH(to_phrase) BETWEEN 2 AND 200;
