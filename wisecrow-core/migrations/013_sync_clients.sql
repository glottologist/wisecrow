-- Per-client sync credentials. Replaces the single shared WISECROW__SYNC_API_KEY
-- with named, individually revocable keys. Only the SHA-256 hash of each key is
-- stored; the raw key is shown once, at creation time.

CREATE TABLE IF NOT EXISTS sync_clients (
    id         SERIAL PRIMARY KEY,
    name       VARCHAR(255) NOT NULL UNIQUE,
    key_hash   BYTEA NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    revoked_at TIMESTAMP WITH TIME ZONE
);
