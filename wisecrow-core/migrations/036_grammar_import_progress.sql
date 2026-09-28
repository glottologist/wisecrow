CREATE TABLE grammar_import_progress (
    language_id INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    document_sha256 BYTEA NOT NULL CHECK (octet_length(document_sha256) = 32),
    cefr_level_id INTEGER NOT NULL REFERENCES cefr_levels(id) ON DELETE CASCADE,
    importer_version SMALLINT NOT NULL,
    document_name TEXT NOT NULL,
    next_chunk INTEGER NOT NULL CHECK (next_chunk >= 0),
    completed BOOLEAN NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (language_id, document_sha256, cefr_level_id, importer_version)
);
