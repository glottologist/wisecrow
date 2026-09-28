-- An example sentence edited, added or removed changes what a device should
-- show for every active item of its point, so it is reported through the
-- item feed as an upsert of each such item. Statement-level triggers with
-- transition tables record one event per item rather than one per example:
-- a rule write replaces every example row of the point in two statements.
CREATE OR REPLACE FUNCTION wisecrow_record_rule_example_insert()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO quiz_item_changes (item_id, language_code, operation)
    SELECT DISTINCT qi.id, l.code, 'U'
    FROM (SELECT DISTINCT rule_id FROM new_rows) changed
    JOIN quiz_items qi ON qi.rule_id = changed.rule_id AND qi.status = 'active'
    JOIN grammar_rules gr ON gr.id = qi.rule_id
    JOIN languages l ON l.id = gr.language_id;
    RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION wisecrow_record_rule_example_delete()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO quiz_item_changes (item_id, language_code, operation)
    SELECT DISTINCT qi.id, l.code, 'U'
    FROM (SELECT DISTINCT rule_id FROM old_rows) changed
    JOIN quiz_items qi ON qi.rule_id = changed.rule_id AND qi.status = 'active'
    JOIN grammar_rules gr ON gr.id = qi.rule_id
    JOIN languages l ON l.id = gr.language_id;
    RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION wisecrow_record_rule_example_update()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO quiz_item_changes (item_id, language_code, operation)
    SELECT DISTINCT qi.id, l.code, 'U'
    FROM (SELECT rule_id FROM new_rows UNION SELECT rule_id FROM old_rows) changed
    JOIN quiz_items qi ON qi.rule_id = changed.rule_id AND qi.status = 'active'
    JOIN grammar_rules gr ON gr.id = qi.rule_id
    JOIN languages l ON l.id = gr.language_id;
    RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS wisecrow_rule_example_insert ON rule_examples;
CREATE TRIGGER wisecrow_rule_example_insert
AFTER INSERT ON rule_examples
REFERENCING NEW TABLE AS new_rows
FOR EACH STATEMENT EXECUTE FUNCTION wisecrow_record_rule_example_insert();

DROP TRIGGER IF EXISTS wisecrow_rule_example_delete ON rule_examples;
CREATE TRIGGER wisecrow_rule_example_delete
AFTER DELETE ON rule_examples
REFERENCING OLD TABLE AS old_rows
FOR EACH STATEMENT EXECUTE FUNCTION wisecrow_record_rule_example_delete();

DROP TRIGGER IF EXISTS wisecrow_rule_example_update ON rule_examples;
CREATE TRIGGER wisecrow_rule_example_update
AFTER UPDATE ON rule_examples
REFERENCING OLD TABLE AS old_rows NEW TABLE AS new_rows
FOR EACH STATEMENT EXECUTE FUNCTION wisecrow_record_rule_example_update();
