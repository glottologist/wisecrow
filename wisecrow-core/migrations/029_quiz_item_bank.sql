-- The quiz item bank.
--
-- Items are generated per grammar point and persist, so that generation is an
-- accumulating investment rather than a cost paid on every request. Content is
-- immutable once written: an attempt uploaded from a device that has been
-- offline for a fortnight must be gradeable against the item as the learner
-- actually saw it, and an edit would silently change the answer underneath it.
-- Editing therefore means inserting a new revision and retiring the old row;
-- nothing is ever hard-deleted.

CREATE TABLE IF NOT EXISTS quiz_items (
    id                 SERIAL PRIMARY KEY,
    rule_id            INTEGER NOT NULL REFERENCES grammar_rules(id) ON DELETE CASCADE,
    revision           INTEGER NOT NULL DEFAULT 1,
    kind               VARCHAR(16) NOT NULL CHECK (kind IN ('cloze', 'multiple_choice')),
    prompt             TEXT NOT NULL,
    answer             TEXT,
    accepted           JSONB NOT NULL DEFAULT '[]'::jsonb,
    options            JSONB,
    correct_option     VARCHAR(32),
    hint               TEXT,
    status             VARCHAR(16) NOT NULL DEFAULT 'candidate'
                       CHECK (status IN ('candidate', 'active', 'rejected', 'retired')),
    gate_reason        TEXT,
    -- Advisory only: the count of words outside the target level where a
    -- CEFRLex resource covers the language. It informs a reviewer; it never
    -- rejects an item on its own authority.
    out_of_level_words INTEGER,
    content_sha256     CHAR(64) NOT NULL,
    source             VARCHAR(32) NOT NULL DEFAULT 'llm',
    created_at         TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    promoted_at        TIMESTAMP WITH TIME ZONE,
    CONSTRAINT quiz_items_revision_positive CHECK (revision >= 1),
    CONSTRAINT quiz_items_content_hash_valid CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT quiz_items_cloze_has_answer
        CHECK (kind <> 'cloze' OR (answer IS NOT NULL AND btrim(answer) <> '')),
    CONSTRAINT quiz_items_choice_has_options
        CHECK (kind <> 'multiple_choice' OR (options IS NOT NULL AND correct_option IS NOT NULL)),
    UNIQUE (rule_id, content_sha256)
);

CREATE INDEX IF NOT EXISTS idx_quiz_items_rule_status ON quiz_items (rule_id, status);

-- Which items a learner has already been shown, so that selection can spread
-- exposure across the bank rather than drilling one sentence.
CREATE TABLE IF NOT EXISTS quiz_item_exposures (
    user_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    item_id   INTEGER NOT NULL REFERENCES quiz_items(id) ON DELETE CASCADE,
    served_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, item_id)
);

CREATE INDEX IF NOT EXISTS idx_quiz_item_exposures_recency
    ON quiz_item_exposures (user_id, served_at);

CREATE OR REPLACE FUNCTION wisecrow_quiz_items_immutable()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.prompt IS DISTINCT FROM OLD.prompt
       OR NEW.answer IS DISTINCT FROM OLD.answer
       OR NEW.accepted IS DISTINCT FROM OLD.accepted
       OR NEW.options IS DISTINCT FROM OLD.options
       OR NEW.correct_option IS DISTINCT FROM OLD.correct_option
       OR NEW.kind IS DISTINCT FROM OLD.kind THEN
        RAISE EXCEPTION
            'quiz item content is immutable; insert a new revision and retire this row';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS wisecrow_quiz_items_immutable ON quiz_items;
CREATE TRIGGER wisecrow_quiz_items_immutable
BEFORE UPDATE ON quiz_items
FOR EACH ROW EXECUTE FUNCTION wisecrow_quiz_items_immutable();
