-- The dashboard's thirty-day review strip reads one learner's review log by
-- time. Migration 022 indexes the log for replay, which leads with the
-- translation, so a per-learner date range would otherwise scan every event
-- the learner has ever made.
CREATE INDEX IF NOT EXISTS idx_review_events_user_time
    ON review_events (user_id, occurred_at);
