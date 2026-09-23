-- Stable identity for grammar points.
--
-- Mastery is keyed on a grammar point, so the point must survive re-seeding
-- with different model wording. The slug is assigned once and never rewritten;
-- every writer matches on it thereafter. Rule identifiers are preserved so
-- that nothing already referencing a rule is disturbed.

ALTER TABLE grammar_rules
    ADD COLUMN IF NOT EXISTS slug VARCHAR(96);

UPDATE grammar_rules
SET slug = regexp_replace(
        regexp_replace(lower(title), '[^a-z0-9]+', '-', 'g'),
        '(^-+|-+$)', '', 'g')
WHERE slug IS NULL;

-- A title without alphanumerics slugifies to the empty string; fall back to
-- the rule identifier so that the NOT NULL and non-empty checks below hold.
UPDATE grammar_rules
SET slug = 'rule-' || id
WHERE btrim(slug) = '';

-- Titles are unique per (language, level) but not per language, so two levels
-- may slugify alike. Disambiguate the later row by its identifier.
UPDATE grammar_rules AS outer_rule
SET slug = outer_rule.slug || '-' || outer_rule.id
WHERE EXISTS (
    SELECT 1
    FROM grammar_rules AS inner_rule
    WHERE inner_rule.language_id = outer_rule.language_id
      AND inner_rule.slug = outer_rule.slug
      AND inner_rule.id < outer_rule.id
);

ALTER TABLE grammar_rules
    ALTER COLUMN slug SET NOT NULL;

ALTER TABLE grammar_rules
    DROP CONSTRAINT IF EXISTS grammar_rules_slug_nonempty;
ALTER TABLE grammar_rules
    ADD CONSTRAINT grammar_rules_slug_nonempty CHECK (btrim(slug) <> '');

ALTER TABLE grammar_rules
    DROP CONSTRAINT IF EXISTS grammar_rules_language_slug_unique;
ALTER TABLE grammar_rules
    ADD CONSTRAINT grammar_rules_language_slug_unique UNIQUE (language_id, slug);

-- The old identity: titles are model output and change whenever the prompt or
-- the model does, which is precisely why they cannot key a learner's history.
ALTER TABLE grammar_rules
    DROP CONSTRAINT IF EXISTS grammar_rules_language_id_cefr_level_id_title_key;

-- Lets a curated reference inventory adopt an existing machine-generated
-- point under its own naming rather than duplicating it.
CREATE TABLE IF NOT EXISTS grammar_rule_aliases (
    language_id  INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    alias_slug   VARCHAR(96) NOT NULL,
    rule_id      INTEGER NOT NULL REFERENCES grammar_rules(id) ON DELETE CASCADE,
    created_at   TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (language_id, alias_slug)
);

CREATE INDEX IF NOT EXISTS idx_grammar_rule_aliases_rule
    ON grammar_rule_aliases (rule_id);

CREATE INDEX IF NOT EXISTS idx_grammar_rules_language_order
    ON grammar_rules (language_id, cefr_level_id, slug);
