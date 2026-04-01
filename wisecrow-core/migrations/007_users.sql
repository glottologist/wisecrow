CREATE TABLE IF NOT EXISTS users (
    id          SERIAL PRIMARY KEY,
    display_name VARCHAR(255) NOT NULL,
    created_at  TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO users (id, display_name) VALUES (1, 'Default User')
ON CONFLICT (id) DO NOTHING;

ALTER TABLE sessions ADD COLUMN IF NOT EXISTS user_id INTEGER
    REFERENCES users(id) ON DELETE CASCADE;
UPDATE sessions SET user_id = 1 WHERE user_id IS NULL;
ALTER TABLE sessions ALTER COLUMN user_id SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions (user_id);
