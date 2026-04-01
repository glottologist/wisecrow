CREATE TABLE IF NOT EXISTS dnb_sessions (
    id                SERIAL PRIMARY KEY,
    user_id           INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    native_lang       VARCHAR(16) NOT NULL,
    foreign_lang      VARCHAR(16) NOT NULL,
    mode              VARCHAR(32) NOT NULL,
    n_level_start     SMALLINT NOT NULL DEFAULT 2,
    n_level_peak      SMALLINT NOT NULL DEFAULT 2,
    n_level_end       SMALLINT NOT NULL DEFAULT 2,
    trials_completed  INTEGER NOT NULL DEFAULT 0,
    accuracy_audio    REAL,
    accuracy_visual   REAL,
    interval_ms_start INTEGER NOT NULL,
    interval_ms_end   INTEGER NOT NULL,
    started_at        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at      TIMESTAMP WITH TIME ZONE
);

CREATE TABLE IF NOT EXISTS dnb_trials (
    id                     SERIAL PRIMARY KEY,
    session_id             INTEGER NOT NULL REFERENCES dnb_sessions(id) ON DELETE CASCADE,
    trial_number           INTEGER NOT NULL,
    n_level                SMALLINT NOT NULL,
    audio_translation_id   INTEGER NOT NULL REFERENCES translations(id) ON DELETE CASCADE,
    visual_translation_id  INTEGER NOT NULL REFERENCES translations(id) ON DELETE CASCADE,
    audio_match            BOOLEAN NOT NULL,
    visual_match           BOOLEAN NOT NULL,
    audio_response         BOOLEAN,
    visual_response        BOOLEAN,
    response_time_ms       INTEGER,
    interval_ms            INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_dnb_sessions_user ON dnb_sessions (user_id);
CREATE INDEX IF NOT EXISTS idx_dnb_trials_session ON dnb_trials (session_id);
CREATE INDEX IF NOT EXISTS idx_dnb_trials_audio_translation ON dnb_trials (audio_translation_id);
CREATE INDEX IF NOT EXISTS idx_dnb_trials_visual_translation ON dnb_trials (visual_translation_id);
