CREATE TABLE IF NOT EXISTS mobile_devices (
    user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    id            UUID NOT NULL,
    display_name  VARCHAR(128) NOT NULL,
    created_at    TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_seen_at  TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    revoked_at    TIMESTAMP WITH TIME ZONE,
    PRIMARY KEY (user_id, id)
);

CREATE TABLE IF NOT EXISTS corpus_changes (
    sequence              BIGSERIAL PRIMARY KEY,
    translation_id        INTEGER NOT NULL,
    from_language_code     VARCHAR(16) NOT NULL,
    to_language_code       VARCHAR(16) NOT NULL,
    from_phrase            TEXT,
    to_phrase              TEXT,
    frequency              INTEGER,
    is_phrase              BOOLEAN,
    operation              CHAR(1) NOT NULL CHECK (operation IN ('U', 'D')),
    changed_at             TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_corpus_changes_pair_sequence
    ON corpus_changes (from_language_code, to_language_code, sequence);

CREATE TABLE IF NOT EXISTS card_review_baselines (
    user_id          INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    translation_id   INTEGER NOT NULL REFERENCES translations(id) ON DELETE CASCADE,
    stability        DOUBLE PRECISION NOT NULL,
    difficulty       DOUBLE PRECISION NOT NULL,
    elapsed_days     INTEGER NOT NULL,
    scheduled_days   INTEGER NOT NULL,
    reps             INTEGER NOT NULL,
    lapses           INTEGER NOT NULL,
    state            SMALLINT NOT NULL,
    last_review      TIMESTAMP WITH TIME ZONE,
    due              TIMESTAMP WITH TIME ZONE NOT NULL,
    captured_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, translation_id)
);

INSERT INTO card_review_baselines (
    user_id, translation_id, stability, difficulty, elapsed_days,
    scheduled_days, reps, lapses, state, last_review, due
)
SELECT user_id, translation_id, stability, difficulty, elapsed_days,
       scheduled_days, reps, lapses, state, last_review, due
FROM cards
ON CONFLICT (user_id, translation_id) DO NOTHING;

CREATE TABLE IF NOT EXISTS review_events (
    user_id          INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    event_id         UUID NOT NULL,
    device_id        UUID,
    translation_id   INTEGER NOT NULL REFERENCES translations(id) ON DELETE CASCADE,
    rating           SMALLINT NOT NULL CHECK (rating BETWEEN 1 AND 4),
    occurred_at      TIMESTAMP WITH TIME ZONE NOT NULL,
    received_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    source           VARCHAR(16) NOT NULL CHECK (source IN ('web', 'mobile', 'nback')),
    PRIMARY KEY (user_id, event_id),
    FOREIGN KEY (user_id, device_id)
        REFERENCES mobile_devices(user_id, id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_review_events_replay
    ON review_events (user_id, translation_id, occurred_at, event_id);

CREATE TABLE IF NOT EXISTS card_changes (
    sequence          BIGSERIAL PRIMARY KEY,
    user_id           INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    translation_id    INTEGER NOT NULL,
    operation         CHAR(1) NOT NULL CHECK (operation IN ('U', 'D')),
    changed_at        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_card_changes_user_sequence
    ON card_changes (user_id, sequence);

CREATE TABLE IF NOT EXISTS mobile_nback_uploads (
    user_id             INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    client_session_id   UUID NOT NULL,
    device_id           UUID NOT NULL,
    payload_sha256      BYTEA NOT NULL,
    server_session_id   INTEGER REFERENCES dnb_sessions(id) ON DELETE SET NULL,
    processed_at        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, client_session_id),
    FOREIGN KEY (user_id, device_id)
        REFERENCES mobile_devices(user_id, id) ON DELETE RESTRICT
);

CREATE OR REPLACE FUNCTION wisecrow_record_corpus_change()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    from_code VARCHAR(16);
    to_code VARCHAR(16);
BEGIN
    IF TG_OP = 'DELETE' THEN
        SELECT from_language.code, to_language.code
        INTO from_code, to_code
        FROM languages AS from_language, languages AS to_language
        WHERE from_language.id = OLD.from_language_id
          AND to_language.id = OLD.to_language_id;

        INSERT INTO corpus_changes (
            translation_id, from_language_code, to_language_code, operation
        )
        VALUES (OLD.id, from_code, to_code, 'D');
        RETURN OLD;
    END IF;

    SELECT from_language.code, to_language.code
    INTO from_code, to_code
    FROM languages AS from_language, languages AS to_language
    WHERE from_language.id = NEW.from_language_id
      AND to_language.id = NEW.to_language_id;

    INSERT INTO corpus_changes (
        translation_id, from_language_code, to_language_code, from_phrase,
        to_phrase, frequency, is_phrase, operation
    )
    VALUES (
        NEW.id, from_code, to_code, NEW.from_phrase, NEW.to_phrase,
        NEW.frequency,
        EXISTS (
            SELECT 1
            FROM phrase_translations
            WHERE translation_id = NEW.id
        ),
        'U'
    );
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION wisecrow_record_phrase_membership_change()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    old_translation_id INTEGER;
    new_translation_id INTEGER;
BEGIN
    IF TG_OP <> 'INSERT' THEN
        old_translation_id := OLD.translation_id;
    END IF;
    IF TG_OP <> 'DELETE' THEN
        new_translation_id := NEW.translation_id;
    END IF;

    INSERT INTO corpus_changes (
        translation_id, from_language_code, to_language_code, from_phrase,
        to_phrase, frequency, is_phrase, operation
    )
    SELECT translation.id, from_language.code, to_language.code,
           translation.from_phrase, translation.to_phrase,
           translation.frequency,
           EXISTS (
               SELECT 1
               FROM phrase_translations
               WHERE translation_id = translation.id
           ),
           'U'
    FROM (
        SELECT old_translation_id AS id
        WHERE old_translation_id IS NOT NULL
        UNION
        SELECT new_translation_id AS id
        WHERE new_translation_id IS NOT NULL
    ) AS affected
    JOIN translations AS translation ON translation.id = affected.id
    JOIN languages AS from_language
      ON from_language.id = translation.from_language_id
    JOIN languages AS to_language
      ON to_language.id = translation.to_language_id;

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION wisecrow_record_card_change()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        INSERT INTO card_changes (
            user_id, translation_id, operation
        )
        SELECT OLD.user_id, OLD.translation_id, 'D'
        WHERE EXISTS (
            SELECT 1 FROM users WHERE id = OLD.user_id
        );
        RETURN OLD;
    END IF;

    INSERT INTO card_changes (
        user_id, translation_id, operation
    )
    VALUES (NEW.user_id, NEW.translation_id, 'U');
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS wisecrow_corpus_change ON translations;
CREATE TRIGGER wisecrow_corpus_change
AFTER INSERT OR UPDATE OR DELETE ON translations
FOR EACH ROW EXECUTE FUNCTION wisecrow_record_corpus_change();

DROP TRIGGER IF EXISTS wisecrow_phrase_membership_change ON phrase_translations;
CREATE TRIGGER wisecrow_phrase_membership_change
AFTER INSERT OR UPDATE OF translation_id OR DELETE ON phrase_translations
FOR EACH ROW EXECUTE FUNCTION wisecrow_record_phrase_membership_change();

DROP TRIGGER IF EXISTS wisecrow_card_change ON cards;
CREATE TRIGGER wisecrow_card_change
AFTER INSERT OR UPDATE OR DELETE ON cards
FOR EACH ROW EXECUTE FUNCTION wisecrow_record_card_change();
