ALTER TABLE translations ADD COLUMN IF NOT EXISTS frequency INTEGER NOT NULL DEFAULT 1;

CREATE TABLE IF NOT EXISTS cards (
    id              SERIAL PRIMARY KEY,
    translation_id  INTEGER NOT NULL REFERENCES translations(id) ON DELETE CASCADE,
    stability       REAL NOT NULL DEFAULT 0.0,
    difficulty      REAL NOT NULL DEFAULT 0.0,
    elapsed_days    INTEGER NOT NULL DEFAULT 0,
    scheduled_days  INTEGER NOT NULL DEFAULT 0,
    reps            INTEGER NOT NULL DEFAULT 0,
    lapses          INTEGER NOT NULL DEFAULT 0,
    state           SMALLINT NOT NULL DEFAULT 0,
    last_review     TIMESTAMP WITH TIME ZONE,
    due             TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (translation_id)
);

CREATE TABLE IF NOT EXISTS sessions (
    id              SERIAL PRIMARY KEY,
    native_lang     VARCHAR(16) NOT NULL,
    foreign_lang    VARCHAR(16) NOT NULL,
    deck_size       INTEGER NOT NULL,
    speed_ms        INTEGER NOT NULL DEFAULT 3000,
    started_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    paused_at       TIMESTAMP WITH TIME ZONE,
    completed_at    TIMESTAMP WITH TIME ZONE
);

CREATE TABLE IF NOT EXISTS session_cards (
    session_id      INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    card_id         INTEGER NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
    position        INTEGER NOT NULL,
    answered        BOOLEAN NOT NULL DEFAULT FALSE,
    rating          SMALLINT,
    answered_at     TIMESTAMP WITH TIME ZONE,
    PRIMARY KEY (session_id, card_id)
);

CREATE TABLE IF NOT EXISTS media_cache (
    id              SERIAL PRIMARY KEY,
    translation_id  INTEGER NOT NULL REFERENCES translations(id) ON DELETE CASCADE,
    media_type      VARCHAR(16) NOT NULL,
    file_path       TEXT NOT NULL,
    source_url      TEXT,
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (translation_id, media_type)
);

CREATE INDEX IF NOT EXISTS idx_cards_due ON cards (due);
CREATE INDEX IF NOT EXISTS idx_cards_state ON cards (state);
CREATE INDEX IF NOT EXISTS idx_translations_frequency ON translations (from_language_id, to_language_id, frequency DESC);
CREATE INDEX IF NOT EXISTS idx_session_cards_position ON session_cards (session_id, position);
