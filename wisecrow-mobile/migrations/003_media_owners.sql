-- Media belongs to an owner, not only to a translation: a grammar example
-- sentence has a clip and no translation row. The key keeps the profile and
-- user, because ids and fingerprints are only meaningful within the server
-- that issued them, and records each clip's fingerprint so a stale clip can
-- be told from the one a later bank page names. SQLite cannot alter a
-- primary key, so the table is rebuilt and its rows carried over.
CREATE TABLE media_cache_v2 (
    profile_id       TEXT NOT NULL,
    user_id          INTEGER NOT NULL,
    owner_kind       TEXT NOT NULL CHECK (owner_kind IN ('translation', 'sentence')),
    owner_key        TEXT NOT NULL,
    media_type       TEXT NOT NULL CHECK (media_type IN ('audio', 'image')),
    file_name        TEXT NOT NULL,
    byte_length      INTEGER NOT NULL,
    attribution      TEXT,
    fingerprint      TEXT,
    last_accessed_at TEXT NOT NULL,
    PRIMARY KEY (profile_id, user_id, owner_kind, owner_key, media_type),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);

INSERT INTO media_cache_v2
    (profile_id, user_id, owner_kind, owner_key, media_type, file_name,
     byte_length, attribution, fingerprint, last_accessed_at)
SELECT profile_id, user_id, 'translation', CAST(translation_id AS TEXT), media_type,
       file_name, byte_length, attribution, NULL, last_accessed_at
FROM media_cache;

DROP TABLE media_cache;
ALTER TABLE media_cache_v2 RENAME TO media_cache;

-- A bank item carries its point's correct examples, as JSON, the way it
-- carries its options.
ALTER TABLE grammar_items ADD COLUMN examples_json TEXT NOT NULL DEFAULT '[]';

-- Set once a device has re-pulled a language's bank to pick up examples it
-- synced before they existed, so the re-pull happens exactly once.
ALTER TABLE grammar_sync_state ADD COLUMN examples_backfilled INTEGER NOT NULL DEFAULT 0;
