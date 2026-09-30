CREATE TABLE grammar_rule_comparisons (
    scope_sha256 BYTEA NOT NULL CHECK (octet_length(scope_sha256) = 32),
    candidate_sha256 BYTEA NOT NULL CHECK (octet_length(candidate_sha256) = 32),
    compared_sha256 BYTEA NOT NULL CHECK (octet_length(compared_sha256) = 32),
    is_duplicate BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (scope_sha256, candidate_sha256, compared_sha256)
);
