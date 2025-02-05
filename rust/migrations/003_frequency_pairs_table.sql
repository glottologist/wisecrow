-- 3) Create the `frequency_tables` table
CREATE TABLE IF NOT EXISTS frequency_tables (
    id SERIAL PRIMARY KEY,
    language_id  INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    phrase       TEXT NOT NULL,
    frequency    INTEGER NOT NULL,

    -- For quick searching by language and phrase
    UNIQUE (language_id, phrase)
);

