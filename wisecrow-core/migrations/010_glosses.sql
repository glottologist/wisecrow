CREATE TABLE IF NOT EXISTS glosses (
    id              SERIAL PRIMARY KEY,
    sentence_hash   CHAR(64) NOT NULL,
    lang_code       VARCHAR(16) NOT NULL,
    gloss_text      TEXT NOT NULL,
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (sentence_hash, lang_code)
);

CREATE INDEX IF NOT EXISTS idx_glosses_lookup ON glosses (sentence_hash, lang_code);
