-- 2) Create the `language_pairs` table to store which language translations are available
CREATE TABLE IF NOT EXISTS language_pairs (
    id SERIAL PRIMARY KEY,
    from_language_id INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    to_language_id   INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    
    -- We might want to ensure that we don't have duplicate pairs
    UNIQUE (from_language_id, to_language_id)
);

