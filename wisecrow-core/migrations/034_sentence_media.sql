-- Speech for sentences that belong to no translation: a grammar point's
-- example sentences. The key is the audio fingerprint of what is spoken
-- (sentence, language, voice profile), not the example row, because every
-- rule write replaces a point's example rows and a clip keyed on them would
-- be orphaned by the next re-seed or re-import. Two examples quoting one
-- sentence share one row and one file.
CREATE TABLE IF NOT EXISTS sentence_media (
    fingerprint   VARCHAR(64) PRIMARY KEY
                  CHECK (fingerprint ~ '^[0-9a-f]{64}$'),
    media_type    VARCHAR(16) NOT NULL,
    language_code VARCHAR(16) NOT NULL,
    sentence      TEXT NOT NULL,
    file_path     TEXT NOT NULL,
    created_at    TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP
);
