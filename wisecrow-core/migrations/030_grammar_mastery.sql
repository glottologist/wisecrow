-- Grammar sessions, attempts and the two mastery projections.
--
-- Nothing here decides anything. `grammar_attempts` is the record: every
-- submission a learner makes, immutable, keyed by a client-generated event
-- identifier so that a device uploading after a fortnight offline writes each
-- attempt exactly once. `grammar_mastery` and `grammar_review_baselines` are
-- projections of that stream, rebuilt by the pure code in `wisecrow-learning`
-- rather than accumulated by the database. Holding both scheduling and
-- accuracy would risk them drifting apart; deriving both from one stream makes
-- drift impossible.

CREATE TABLE IF NOT EXISTS grammar_sessions (
    id             UUID PRIMARY KEY,
    user_id        INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    language_id    INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    cefr_level_id  INTEGER REFERENCES cefr_levels(id),
    kind           VARCHAR(16) NOT NULL CHECK (kind IN ('practice', 'placement')),
    started_at     TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at   TIMESTAMP WITH TIME ZONE
);

CREATE INDEX IF NOT EXISTS idx_grammar_sessions_user
    ON grammar_sessions (user_id, started_at);

-- The immutable submission stream. `item_id` is restricted rather than
-- cascaded because an attempt must remain gradeable against the item revision
-- the learner actually saw; retirement is a status change, never a deletion.
--
-- The composite device foreign key matches simply, so a web attempt leaving
-- `device_id` null satisfies it while a mobile attempt must name a device that
-- belongs to the same user.
CREATE TABLE IF NOT EXISTS grammar_attempts (
    user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    event_id      UUID NOT NULL,
    session_id    UUID NOT NULL REFERENCES grammar_sessions(id) ON DELETE CASCADE,
    device_id     UUID,
    rule_id       INTEGER NOT NULL REFERENCES grammar_rules(id) ON DELETE CASCADE,
    item_id       INTEGER NOT NULL REFERENCES quiz_items(id) ON DELETE RESTRICT,
    item_revision INTEGER NOT NULL,
    ordinal       SMALLINT NOT NULL,
    correct       BOOLEAN NOT NULL,
    hint_shown    BOOLEAN NOT NULL DEFAULT FALSE,
    occurred_at   TIMESTAMP WITH TIME ZONE NOT NULL,
    received_at   TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    source        VARCHAR(16) NOT NULL CHECK (source IN ('web', 'mobile')),
    PRIMARY KEY (user_id, event_id),
    CONSTRAINT grammar_attempts_ordinal_positive CHECK (ordinal >= 1),
    FOREIGN KEY (user_id, device_id) REFERENCES mobile_devices(user_id, id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_grammar_attempts_replay
    ON grammar_attempts (user_id, rule_id, occurred_at, event_id);

CREATE INDEX IF NOT EXISTS idx_grammar_attempts_session
    ON grammar_attempts (session_id);

-- The projections. `accuracy` is null until the point has been attempted,
-- which is what keeps an unseen point grey rather than colouring it as failed.
CREATE TABLE IF NOT EXISTS grammar_mastery (
    user_id        INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rule_id        INTEGER NOT NULL REFERENCES grammar_rules(id) ON DELETE CASCADE,
    stability      DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    difficulty     DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    elapsed_days   INTEGER NOT NULL DEFAULT 0,
    scheduled_days INTEGER NOT NULL DEFAULT 0,
    reps           INTEGER NOT NULL DEFAULT 0,
    lapses         INTEGER NOT NULL DEFAULT 0,
    state          SMALLINT NOT NULL DEFAULT 0,
    accuracy       DOUBLE PRECISION,
    attempts       INTEGER NOT NULL DEFAULT 0,
    last_review    TIMESTAMP WITH TIME ZONE,
    due            TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, rule_id),
    CONSTRAINT grammar_mastery_state_known CHECK (state BETWEEN 0 AND 3),
    CONSTRAINT grammar_mastery_accuracy_bounded
        CHECK (accuracy IS NULL OR (accuracy >= 0.0 AND accuracy <= 1.0))
);

CREATE INDEX IF NOT EXISTS idx_grammar_mastery_due ON grammar_mastery (user_id, due);

-- Where replay starts. A baseline is captured once per point, before the first
-- attempt it covers, and never moves: replay from it must reproduce the
-- projection exactly, whatever order the attempts arrived in.
CREATE TABLE IF NOT EXISTS grammar_review_baselines (
    user_id        INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rule_id        INTEGER NOT NULL REFERENCES grammar_rules(id) ON DELETE CASCADE,
    stability      DOUBLE PRECISION NOT NULL,
    difficulty     DOUBLE PRECISION NOT NULL,
    elapsed_days   INTEGER NOT NULL,
    scheduled_days INTEGER NOT NULL,
    reps           INTEGER NOT NULL,
    lapses         INTEGER NOT NULL,
    state          SMALLINT NOT NULL,
    last_review    TIMESTAMP WITH TIME ZONE,
    due            TIMESTAMP WITH TIME ZONE NOT NULL,
    captured_at    TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, rule_id)
);

-- A placement run. It rides on one ordinary grammar session, named here
-- explicitly, so that its answers are graded and recorded exactly like any
-- other practice: the same items, the same grading path, the same attempt
-- rows. The session carries no level of its own, because a run walks several.
CREATE TABLE IF NOT EXISTS placement_attempts (
    id            UUID PRIMARY KEY,
    user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    language_id   INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    session_id    UUID NOT NULL REFERENCES grammar_sessions(id) ON DELETE CASCADE,
    level_reached VARCHAR(4),
    started_at    TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at  TIMESTAMP WITH TIME ZONE,
    UNIQUE (session_id)
);

CREATE INDEX IF NOT EXISTS idx_placement_attempts_user
    ON placement_attempts (user_id, started_at);
