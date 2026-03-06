ALTER TABLE translations
    DROP CONSTRAINT IF EXISTS translations_from_language_id_from_phrase_to_language_id_key;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'translations_unique_pair'
    ) THEN
        ALTER TABLE translations
            ADD CONSTRAINT translations_unique_pair
            UNIQUE (from_language_id, from_phrase, to_language_id, to_phrase);
    END IF;
END $$;
