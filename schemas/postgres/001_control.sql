CREATE TABLE IF NOT EXISTS source_checkpoints (
    checkpoint_kind TEXT NOT NULL
        CHECK (checkpoint_kind IN ('broker', 'archive')),
    source_id TEXT NOT NULL,
    source_session_id BYTEA NOT NULL
        CHECK (octet_length(source_session_id) = 16),
    last_collector_sequence BYTEA NOT NULL
        CHECK (octet_length(last_collector_sequence) = 8),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (checkpoint_kind, source_id, source_session_id)
);

CREATE TABLE IF NOT EXISTS archive_manifests (
    manifest_hash BYTEA PRIMARY KEY
        CHECK (octet_length(manifest_hash) = 32),
    source_session_id BYTEA NOT NULL
        CHECK (octet_length(source_session_id) = 16),
    first_collector_sequence BYTEA NOT NULL
        CHECK (octet_length(first_collector_sequence) = 8),
    last_collector_sequence BYTEA NOT NULL
        CHECK (octet_length(last_collector_sequence) = 8),
    object_key TEXT NOT NULL UNIQUE,
    object_sha256 BYTEA NOT NULL
        CHECK (octet_length(object_sha256) = 32),
    compressed_bytes BYTEA NOT NULL
        CHECK (octet_length(compressed_bytes) = 8),
    record_count BYTEA NOT NULL
        CHECK (octet_length(record_count) = 8),
    previous_manifest_hash BYTEA
        CHECK (
            previous_manifest_hash IS NULL
            OR octet_length(previous_manifest_hash) = 32
        ),
    committed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE INDEX IF NOT EXISTS archive_manifests_session_range_idx
    ON archive_manifests (source_session_id, first_collector_sequence);

CREATE TABLE IF NOT EXISTS archive_manifest_heads (
    source_session_id BYTEA PRIMARY KEY
        CHECK (octet_length(source_session_id) = 16),
    manifest_hash BYTEA NOT NULL UNIQUE
        REFERENCES archive_manifests (manifest_hash),
    last_collector_sequence BYTEA NOT NULL
        CHECK (octet_length(last_collector_sequence) = 8),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
