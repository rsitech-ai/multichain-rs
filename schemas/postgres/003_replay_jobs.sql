CREATE TABLE IF NOT EXISTS replay_jobs (
    request_hash BYTEA PRIMARY KEY
        CHECK (octet_length(request_hash) = 32),
    replay_job_id BYTEA NOT NULL UNIQUE
        CHECK (octet_length(replay_job_id) = 16),
    chain TEXT NOT NULL
        CHECK (chain = 'bitcoin'),
    network TEXT NOT NULL
        CHECK (network IN ('mainnet', 'regtest')),
    source_id TEXT NOT NULL
        CHECK (length(trim(source_id)) > 0),
    start_height BIGINT NOT NULL
        CHECK (start_height BETWEEN 0 AND 4294967295),
    end_height_inclusive BIGINT NOT NULL
        CHECK (
            end_height_inclusive BETWEEN start_height AND 4294967295
        ),
    max_in_flight INTEGER NOT NULL
        CHECK (max_in_flight BETWEEN 1 AND 256),
    state TEXT NOT NULL
        CHECK (
            state IN (
                'pending',
                'running',
                'failed',
                'completed',
                'cancelled'
            )
        ),
    last_archived_height BIGINT
        CHECK (
            last_archived_height IS NULL
            OR last_archived_height BETWEEN start_height AND end_height_inclusive
        ),
    last_materialized_height BIGINT
        CHECK (
            last_materialized_height IS NULL
            OR last_materialized_height BETWEEN start_height AND end_height_inclusive
        ),
    canonical_tip_hash_at_start BYTEA NOT NULL
        CHECK (octet_length(canonical_tip_hash_at_start) = 32),
    canonical_tip_hash_at_end BYTEA
        CHECK (
            canonical_tip_hash_at_end IS NULL
            OR octet_length(canonical_tip_hash_at_end) = 32
        ),
    error_code TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CHECK (
        last_materialized_height IS NULL
        OR (
            last_archived_height IS NOT NULL
            AND last_materialized_height <= last_archived_height
        )
    ),
    CHECK (
        state <> 'completed'
        OR last_materialized_height = end_height_inclusive
    )
);

CREATE INDEX IF NOT EXISTS replay_jobs_source_state_idx
    ON replay_jobs (source_id, state, updated_at DESC);
