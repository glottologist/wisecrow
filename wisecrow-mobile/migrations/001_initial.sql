PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;

CREATE TABLE profiles (
    id TEXT PRIMARY KEY,
    origin TEXT NOT NULL UNIQUE,
    imported_ca_fingerprint TEXT,
    active INTEGER NOT NULL DEFAULT 0 CHECK (active IN (0, 1)),
    active_user_id INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX one_active_profile ON profiles(active) WHERE active = 1;

CREATE TABLE profile_users (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL,
    display_name TEXT NOT NULL,
    device_id TEXT NOT NULL,
    PRIMARY KEY (profile_id, user_id)
);

CREATE TABLE language_pairs (
    profile_id TEXT NOT NULL,
    user_id INTEGER NOT NULL,
    native TEXT NOT NULL,
    "foreign" TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('absent', 'downloading', 'ready', 'failed')),
    snapshot_watermark INTEGER,
    snapshot_after_id INTEGER NOT NULL DEFAULT 0,
    change_cursor INTEGER NOT NULL DEFAULT 0,
    estimated_bytes INTEGER,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (profile_id, user_id, native, "foreign"),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);

CREATE TABLE languages (
    profile_id TEXT NOT NULL,
    user_id INTEGER NOT NULL,
    code TEXT NOT NULL,
    name TEXT NOT NULL,
    script_direction TEXT NOT NULL CHECK (script_direction IN ('ltr', 'rtl')),
    PRIMARY KEY (profile_id, user_id, code),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);

CREATE TABLE translations (
    profile_id TEXT NOT NULL,
    user_id INTEGER NOT NULL,
    native TEXT NOT NULL,
    "foreign" TEXT NOT NULL,
    translation_id INTEGER NOT NULL,
    from_phrase TEXT NOT NULL,
    to_phrase TEXT NOT NULL,
    frequency INTEGER NOT NULL,
    is_phrase INTEGER NOT NULL CHECK (is_phrase IN (0, 1)),
    PRIMARY KEY (profile_id, user_id, native, "foreign", translation_id),
    FOREIGN KEY (profile_id, user_id, native, "foreign")
        REFERENCES language_pairs(profile_id, user_id, native, "foreign") ON DELETE CASCADE
);
CREATE INDEX translations_rank
    ON translations(profile_id, user_id, native, "foreign", frequency DESC, translation_id);

CREATE TABLE cards (
    profile_id TEXT NOT NULL,
    user_id INTEGER NOT NULL,
    translation_id INTEGER NOT NULL,
    stability REAL NOT NULL,
    difficulty REAL NOT NULL,
    elapsed_days INTEGER NOT NULL,
    scheduled_days INTEGER NOT NULL,
    reps INTEGER NOT NULL,
    lapses INTEGER NOT NULL,
    state INTEGER NOT NULL,
    last_review TEXT,
    due TEXT NOT NULL,
    server_cursor INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (profile_id, user_id, translation_id),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);

CREATE TABLE learn_sessions (
    profile_id TEXT NOT NULL,
    user_id INTEGER NOT NULL,
    id TEXT NOT NULL,
    native TEXT NOT NULL,
    "foreign" TEXT NOT NULL,
    deck_size INTEGER NOT NULL,
    speed_ms INTEGER NOT NULL,
    current_index INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL CHECK (status IN ('active', 'paused', 'complete')),
    started_at TEXT NOT NULL,
    PRIMARY KEY (profile_id, user_id, id),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);
CREATE TABLE learn_session_cards (
    profile_id TEXT NOT NULL,
    user_id INTEGER NOT NULL,
    session_id TEXT NOT NULL,
    translation_id INTEGER NOT NULL,
    position INTEGER NOT NULL,
    answered INTEGER NOT NULL DEFAULT 0 CHECK (answered IN (0, 1)),
    rating INTEGER,
    answered_at TEXT,
    PRIMARY KEY (profile_id, user_id, session_id, translation_id),
    FOREIGN KEY (profile_id, user_id, session_id)
        REFERENCES learn_sessions(profile_id, user_id, id) ON DELETE CASCADE
);

CREATE TABLE review_outbox (
    profile_id TEXT NOT NULL,
    user_id INTEGER NOT NULL,
    event_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    translation_id INTEGER NOT NULL,
    rating INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 4),
    occurred_at TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'rejected', 'applied')),
    rejection_reason TEXT,
    PRIMARY KEY (profile_id, user_id, event_id),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);

CREATE TABLE nback_outbox (
    profile_id TEXT NOT NULL,
    user_id INTEGER NOT NULL,
    client_session_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'rejected', 'applied')),
    rejection_reason TEXT,
    PRIMARY KEY (profile_id, user_id, client_session_id),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);

CREATE TABLE cached_quizzes (
    profile_id TEXT NOT NULL,
    user_id INTEGER NOT NULL,
    cache_key TEXT NOT NULL,
    label TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (profile_id, user_id, cache_key),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);

CREATE TABLE media_cache (
    profile_id TEXT NOT NULL,
    user_id INTEGER NOT NULL,
    translation_id INTEGER NOT NULL,
    media_type TEXT NOT NULL CHECK (media_type IN ('audio', 'image')),
    file_name TEXT NOT NULL,
    byte_length INTEGER NOT NULL,
    attribution TEXT,
    last_accessed_at TEXT NOT NULL,
    PRIMARY KEY (profile_id, user_id, translation_id, media_type),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);

CREATE TABLE sync_state (
    profile_id TEXT NOT NULL,
    user_id INTEGER NOT NULL,
    card_cursor INTEGER NOT NULL DEFAULT 0,
    phase TEXT NOT NULL DEFAULT 'idle'
        CHECK (phase IN ('idle', 'reviews', 'nback', 'cards', 'snapshots', 'deltas', 'finishing')),
    last_success_at TEXT,
    last_error_kind TEXT,
    last_error_at TEXT,
    PRIMARY KEY (profile_id, user_id),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);
