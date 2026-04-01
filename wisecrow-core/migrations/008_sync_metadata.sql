CREATE TABLE IF NOT EXISTS sync_metadata (
    id              SERIAL PRIMARY KEY,
    remote_url      TEXT NOT NULL,
    table_name      VARCHAR(64) NOT NULL,
    last_synced_at  TIMESTAMP WITH TIME ZONE,
    last_remote_id  INTEGER NOT NULL DEFAULT 0,
    UNIQUE (remote_url, table_name)
);
