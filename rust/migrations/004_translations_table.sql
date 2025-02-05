-- 4) Create the `translations` table
CREATE TABLE IF NOT EXISTS translations (
    id SERIAL PRIMARY KEY,

    from_language_id INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    from_phrase      TEXT NOT NULL,

    to_language_id   INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    to_phrase        TEXT NOT NULL,

    -- Optional: store audio data directly as bytes (you could also store a path or a URL)
    pronunciation_audio BYTEA,
    
    -- Optional: store image data; again, you could store a path/URL instead if preferred
    image_data BYTEA,

    -- For auditing or ordering by creation
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,

    -- We might want to ensure a single from/to pair is unique
    UNIQUE (from_language_id, from_phrase, to_language_id)
);


