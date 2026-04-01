CREATE TABLE IF NOT EXISTS cefr_levels (
    id          SERIAL PRIMARY KEY,
    code        VARCHAR(4) NOT NULL UNIQUE,
    name        VARCHAR(64) NOT NULL,
    sort_order  SMALLINT NOT NULL
);

INSERT INTO cefr_levels (code, name, sort_order) VALUES
    ('A1', 'Beginner', 1),
    ('A2', 'Elementary', 2),
    ('B1', 'Intermediate', 3),
    ('B2', 'Upper Intermediate', 4),
    ('C1', 'Advanced', 5),
    ('C2', 'Proficiency', 6)
ON CONFLICT (code) DO NOTHING;

CREATE TABLE IF NOT EXISTS grammar_rules (
    id              SERIAL PRIMARY KEY,
    language_id     INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    cefr_level_id   INTEGER NOT NULL REFERENCES cefr_levels(id),
    title           TEXT NOT NULL,
    explanation     TEXT NOT NULL,
    source          VARCHAR(32) NOT NULL DEFAULT 'manual',
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (language_id, cefr_level_id, title)
);

CREATE TABLE IF NOT EXISTS rule_examples (
    id              SERIAL PRIMARY KEY,
    rule_id         INTEGER NOT NULL REFERENCES grammar_rules(id) ON DELETE CASCADE,
    sentence        TEXT NOT NULL,
    translation     TEXT,
    is_correct      BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE INDEX IF NOT EXISTS idx_grammar_rules_lang_level
    ON grammar_rules (language_id, cefr_level_id);
CREATE INDEX IF NOT EXISTS idx_rule_examples_rule
    ON rule_examples (rule_id);
