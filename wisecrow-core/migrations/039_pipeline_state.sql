-- Progress for the corpus and grammar services. A row here is written only
-- after the corresponding work finished. Existing translation rows are not
-- evidence that an archive was ingested.
CREATE TABLE pipeline_archives (
    native_code VARCHAR(16) NOT NULL,
    foreign_code VARCHAR(16) NOT NULL,
    corpus VARCHAR(32) NOT NULL,
    archive_url TEXT NOT NULL,
    status VARCHAR(16) NOT NULL CHECK (status IN ('ingested', 'unavailable')),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (native_code, foreign_code, archive_url)
);

CREATE TABLE pipeline_rank_stamps (
    foreign_code VARCHAR(16) PRIMARY KEY,
    evidence_checksum BYTEA NOT NULL CHECK (octet_length(evidence_checksum) = 32),
    ranked_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE pipeline_picture_misses (
    fingerprint VARCHAR(64) PRIMARY KEY
        CHECK (fingerprint ~ '^[0-9a-f]{64}$'),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
