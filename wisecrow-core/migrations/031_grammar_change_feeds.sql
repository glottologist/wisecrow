-- Change feeds a device can follow without skipping a row.
--
-- `BIGSERIAL` allocates its number when a row is inserted, not when the
-- transaction commits. Two writers therefore commit out of order routinely: a
-- transaction that took its number first can commit last, by which time a
-- client that advanced its cursor past the later number will never ask for the
-- earlier one again, and that change is lost to it for good.
--
-- Recording the writing transaction closes the gap. A reader serves only rows
-- whose transaction finished before every transaction still in flight, so a
-- sequence number is handed out only once nothing can still appear beneath it.
-- The cost is latency, never correctness: a change waits for its neighbours
-- rather than arriving early and alone.
--
-- `pg_current_xact_id()` is `xid8`, a 64-bit counter that does not wrap, so the
-- `BIGINT` it casts to orders correctly for the life of the database. It is
-- called only from these triggers, which run in a transaction that is already
-- writing; a read-only path must not borrow it, because calling it assigns a
-- transaction identifier that would otherwise never be needed.

CREATE TABLE IF NOT EXISTS grammar_changes (
    sequence   BIGSERIAL PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rule_id    INTEGER NOT NULL,
    operation  CHAR(1) NOT NULL CHECK (operation IN ('U', 'D')),
    xact_id    BIGINT NOT NULL DEFAULT pg_current_xact_id()::TEXT::BIGINT,
    changed_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_grammar_changes_visibility
    ON grammar_changes (user_id, xact_id, sequence);

CREATE TABLE IF NOT EXISTS quiz_item_changes (
    sequence      BIGSERIAL PRIMARY KEY,
    item_id       INTEGER NOT NULL,
    language_code VARCHAR(16) NOT NULL,
    operation     CHAR(1) NOT NULL CHECK (operation IN ('U', 'D')),
    xact_id       BIGINT NOT NULL DEFAULT pg_current_xact_id()::TEXT::BIGINT,
    changed_at    TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_quiz_item_changes_visibility
    ON quiz_item_changes (language_code, xact_id, sequence);

-- A deleted learner takes their mastery with them, and the cascade fires this
-- trigger while the user row is already gone. The delete branch therefore
-- records nothing unless the learner survives, exactly as the card feed does.
CREATE OR REPLACE FUNCTION wisecrow_record_grammar_change()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        INSERT INTO grammar_changes (user_id, rule_id, operation)
        SELECT OLD.user_id, OLD.rule_id, 'D'
        WHERE EXISTS (SELECT 1 FROM users WHERE id = OLD.user_id);
        RETURN OLD;
    END IF;

    INSERT INTO grammar_changes (user_id, rule_id, operation)
    VALUES (NEW.user_id, NEW.rule_id, 'U');
    RETURN NEW;
END;
$$;

-- The bank is keyed by language rather than by learner: a device syncs the
-- items for the languages it is studying, and the point an item belongs to is
-- what says which language that is.
CREATE OR REPLACE FUNCTION wisecrow_record_quiz_item_change()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    code VARCHAR(16);
BEGIN
    IF TG_OP = 'DELETE' THEN
        SELECT languages.code
        INTO code
        FROM grammar_rules
        JOIN languages ON languages.id = grammar_rules.language_id
        WHERE grammar_rules.id = OLD.rule_id;

        IF code IS NOT NULL THEN
            INSERT INTO quiz_item_changes (item_id, language_code, operation)
            VALUES (OLD.id, code, 'D');
        END IF;
        RETURN OLD;
    END IF;

    SELECT languages.code
    INTO code
    FROM grammar_rules
    JOIN languages ON languages.id = grammar_rules.language_id
    WHERE grammar_rules.id = NEW.rule_id;

    INSERT INTO quiz_item_changes (item_id, language_code, operation)
    VALUES (NEW.id, code, 'U');
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS wisecrow_grammar_change ON grammar_mastery;
CREATE TRIGGER wisecrow_grammar_change
AFTER INSERT OR UPDATE OR DELETE ON grammar_mastery
FOR EACH ROW EXECUTE FUNCTION wisecrow_record_grammar_change();

DROP TRIGGER IF EXISTS wisecrow_quiz_item_change ON quiz_items;
CREATE TRIGGER wisecrow_quiz_item_change
AFTER INSERT OR UPDATE OR DELETE ON quiz_items
FOR EACH ROW EXECUTE FUNCTION wisecrow_record_quiz_item_change();
