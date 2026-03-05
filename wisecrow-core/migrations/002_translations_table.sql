CREATE TABLE IF NOT EXISTS translations (
    id SERIAL PRIMARY KEY,
    from_language_id INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    from_phrase      TEXT NOT NULL,
    to_language_id   INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    to_phrase        TEXT NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (from_language_id, from_phrase, to_language_id)
);
