-- The grammar half of the offline store.
--
-- The bank and the mastery projection are mirrors: the server owns them, the
-- device holds a copy, and each is followed by a cursor that only ever moves
-- forward over changes the server has said are safe to serve. The outbox is
-- the opposite — the device owns it, and a row leaves only once the server has
-- acknowledged the answer it holds.
--
-- An item carries its answer here, which the served presentation never does. A
-- device out of contact still has to tell the learner whether they were right;
-- the verdict it shows is provisional, and the server regrades every answer
-- when the outbox drains.

CREATE TABLE grammar_rules (
    profile_id  TEXT NOT NULL,
    user_id     INTEGER NOT NULL,
    rule_id     INTEGER NOT NULL,
    language    TEXT NOT NULL,
    slug        TEXT NOT NULL,
    title       TEXT NOT NULL,
    explanation TEXT NOT NULL,
    level       TEXT NOT NULL,
    PRIMARY KEY (profile_id, user_id, rule_id),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);

CREATE INDEX idx_grammar_rules_language
    ON grammar_rules(profile_id, user_id, language, level);

CREATE TABLE grammar_items (
    profile_id     TEXT NOT NULL,
    user_id        INTEGER NOT NULL,
    item_id        INTEGER NOT NULL,
    rule_id        INTEGER NOT NULL,
    revision       INTEGER NOT NULL,
    language       TEXT NOT NULL,
    prompt         TEXT NOT NULL,
    hint           TEXT,
    options        TEXT NOT NULL DEFAULT '[]',
    answer         TEXT,
    accepted       TEXT NOT NULL DEFAULT '[]',
    correct_option TEXT,
    PRIMARY KEY (profile_id, user_id, item_id),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);

CREATE INDEX idx_grammar_items_rule ON grammar_items(profile_id, user_id, rule_id);

CREATE TABLE grammar_mastery (
    profile_id     TEXT NOT NULL,
    user_id        INTEGER NOT NULL,
    rule_id        INTEGER NOT NULL,
    stability      REAL NOT NULL DEFAULT 0.0,
    difficulty     REAL NOT NULL DEFAULT 0.0,
    elapsed_days   INTEGER NOT NULL DEFAULT 0,
    scheduled_days INTEGER NOT NULL DEFAULT 0,
    reps           INTEGER NOT NULL DEFAULT 0,
    lapses         INTEGER NOT NULL DEFAULT 0,
    state          INTEGER NOT NULL DEFAULT 0,
    accuracy       REAL,
    attempts       INTEGER NOT NULL DEFAULT 0,
    last_review    TEXT,
    due            TEXT NOT NULL,
    PRIMARY KEY (profile_id, user_id, rule_id),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);

-- Answers waiting to be told to the server.
--
-- `queued_at` is monotonic within a device and orders the drain, because the
-- server's rating rule depends on which answer came first within a session and
-- `occurred_at` alone can tie.
CREATE TABLE grammar_attempt_outbox (
    profile_id   TEXT NOT NULL,
    user_id      INTEGER NOT NULL,
    event_id     TEXT NOT NULL,
    session_id   TEXT NOT NULL,
    item_id      INTEGER NOT NULL,
    revision     INTEGER NOT NULL,
    answer       TEXT NOT NULL,
    chose_option INTEGER NOT NULL CHECK (chose_option IN (0, 1)),
    hint_shown   INTEGER NOT NULL CHECK (hint_shown IN (0, 1)),
    ordinal      INTEGER NOT NULL CHECK (ordinal >= 1),
    occurred_at  TEXT NOT NULL,
    language     TEXT NOT NULL,
    level        TEXT,
    queued_at    INTEGER NOT NULL,
    PRIMARY KEY (profile_id, user_id, event_id),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);

CREATE INDEX idx_grammar_outbox_drain
    ON grammar_attempt_outbox(profile_id, user_id, queued_at);

-- One row per learner: how far each grammar feed has been followed.
CREATE TABLE grammar_sync_state (
    profile_id     TEXT NOT NULL,
    user_id        INTEGER NOT NULL,
    language       TEXT NOT NULL,
    bank_cursor    INTEGER NOT NULL DEFAULT 0,
    mastery_cursor INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (profile_id, user_id, language),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);

-- The phase machine gains three grammar steps, so its check constraint has to
-- widen. SQLite cannot alter a constraint in place; the table is rebuilt with
-- its rows carried across, which is the documented way and is safe here
-- because a sync in flight holds a lock on the profile and cannot be running
-- while migrations apply.
CREATE TABLE sync_state_next (
    profile_id TEXT NOT NULL,
    user_id INTEGER NOT NULL,
    card_cursor INTEGER NOT NULL DEFAULT 0,
    phase TEXT NOT NULL DEFAULT 'idle'
        CHECK (phase IN ('idle', 'reviews', 'nback', 'grammar_attempts', 'cards',
                         'snapshots', 'deltas', 'grammar_bank', 'grammar_mastery',
                         'finishing')),
    last_success_at TEXT,
    last_error_kind TEXT,
    last_error_at TEXT,
    PRIMARY KEY (profile_id, user_id),
    FOREIGN KEY (profile_id, user_id)
        REFERENCES profile_users(profile_id, user_id) ON DELETE CASCADE
);

INSERT INTO sync_state_next (
    profile_id, user_id, card_cursor, phase, last_success_at, last_error_kind, last_error_at
)
SELECT profile_id, user_id, card_cursor, phase, last_success_at, last_error_kind, last_error_at
FROM sync_state;

DROP TABLE sync_state;
ALTER TABLE sync_state_next RENAME TO sync_state;
