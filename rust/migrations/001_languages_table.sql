-- 1) Create the `languages` table
CREATE TABLE IF NOT EXISTS languages (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    code VARCHAR(16) NOT NULL UNIQUE  -- e.g. "en", "fr", "es"
);
