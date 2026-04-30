-- Add per-user state to cards. Pre-existing rows are backfilled to user_id=1
-- (the default user seeded in 007_users.sql). The previous UNIQUE
-- (translation_id) constraint becomes (translation_id, user_id) so multiple
-- users can each have their own card for the same translation.

ALTER TABLE cards ADD COLUMN IF NOT EXISTS user_id INTEGER
    REFERENCES users(id) ON DELETE CASCADE;
UPDATE cards SET user_id = 1 WHERE user_id IS NULL;
ALTER TABLE cards ALTER COLUMN user_id SET NOT NULL;

ALTER TABLE cards DROP CONSTRAINT IF EXISTS cards_translation_id_key;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'cards_translation_user_unique'
    ) THEN
        ALTER TABLE cards ADD CONSTRAINT cards_translation_user_unique
            UNIQUE (translation_id, user_id);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_cards_user ON cards (user_id);
