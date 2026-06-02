-- Persist the live dual-n-back adaptation counter so the web server can compute
-- adaptation and termination statelessly across submit_nback_trial calls. The
-- CLI keeps this in process memory; the web tier reloads it each request.

ALTER TABLE dnb_sessions
    ADD COLUMN IF NOT EXISTS consecutive_below_start SMALLINT NOT NULL DEFAULT 0;
