CREATE TABLE grammar_llm_cache (
    request_sha256 BYTEA PRIMARY KEY CHECK (octet_length(request_sha256) = 32),
    provider_identity TEXT NOT NULL,
    max_tokens BIGINT NOT NULL CHECK (max_tokens > 0),
    response TEXT NOT NULL CHECK (length(response) > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);
