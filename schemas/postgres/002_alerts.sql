CREATE TABLE IF NOT EXISTS alert_definitions (
    alert_id TEXT NOT NULL
        CHECK (alert_id ~ '^[!-~]+$' AND length(alert_id) <= 128),
    definition_version BIGINT NOT NULL
        CHECK (definition_version > 0),
    kind TEXT NOT NULL
        CHECK (kind = 'bitcoin_mempool_quorum_vbytes_above'),
    definition JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'enabled'
        CHECK (status IN ('draft', 'enabled', 'paused', 'archived')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (alert_id, definition_version)
);

CREATE TABLE IF NOT EXISTS alert_evaluations (
    evaluation_id TEXT PRIMARY KEY
        CHECK (evaluation_id ~ '^[0-9a-f]{64}$'),
    alert_id TEXT NOT NULL,
    definition_version BIGINT NOT NULL,
    kind TEXT NOT NULL
        CHECK (kind = 'bitcoin_mempool_quorum_vbytes_above'),
    network TEXT NOT NULL
        CHECK (network ~ '^[!-~]+$' AND length(network) <= 64),
    input_revision BIGINT NOT NULL
        CHECK (input_revision > 0),
    input_fact_ids TEXT[] NOT NULL
        CHECK (cardinality(input_fact_ids) BETWEEN 1 AND 128),
    source_health JSONB NOT NULL,
    result JSONB NOT NULL,
    transition TEXT NOT NULL
        CHECK (
            transition IN (
                'pending',
                'triggered',
                'confirmed',
                'corrected',
                'retracted',
                'degraded_source',
                'below_threshold',
                'cooldown_suppressed',
                'duplicate_revision'
            )
        ),
    evaluated_at_unix_seconds BIGINT NOT NULL
        CHECK (evaluated_at_unix_seconds >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY (alert_id, definition_version)
        REFERENCES alert_definitions (alert_id, definition_version),
    UNIQUE (alert_id, definition_version, input_revision)
);

CREATE INDEX IF NOT EXISTS alert_evaluations_history_idx
    ON alert_evaluations (
        alert_id,
        definition_version,
        input_revision DESC
    );

CREATE TABLE IF NOT EXISTS alert_state (
    alert_id TEXT NOT NULL,
    definition_version BIGINT NOT NULL,
    last_evaluation_id TEXT NOT NULL
        REFERENCES alert_evaluations (evaluation_id),
    last_input_revision BIGINT NOT NULL
        CHECK (last_input_revision > 0),
    active BOOLEAN NOT NULL,
    consecutive_true_evaluations INTEGER NOT NULL
        CHECK (consecutive_true_evaluations BETWEEN 0 AND 65535),
    last_evaluated_at_unix_seconds BIGINT NOT NULL
        CHECK (last_evaluated_at_unix_seconds >= 0),
    last_triggered_at_unix_seconds BIGINT
        CHECK (
            last_triggered_at_unix_seconds IS NULL
            OR last_triggered_at_unix_seconds >= 0
        ),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (alert_id, definition_version),
    FOREIGN KEY (alert_id, definition_version)
        REFERENCES alert_definitions (alert_id, definition_version)
);

CREATE TABLE IF NOT EXISTS alert_outbox (
    idempotency_key TEXT PRIMARY KEY
        CHECK (idempotency_key ~ '^[0-9a-f]{64}$'),
    evaluation_id TEXT NOT NULL UNIQUE
        REFERENCES alert_evaluations (evaluation_id),
    alert_id TEXT NOT NULL,
    definition_version BIGINT NOT NULL,
    payload JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'delivered')),
    delivery_attempts INTEGER NOT NULL DEFAULT 0
        CHECK (delivery_attempts >= 0),
    last_error TEXT,
    available_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    delivered_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY (alert_id, definition_version)
        REFERENCES alert_definitions (alert_id, definition_version),
    CHECK (
        (status = 'pending' AND delivered_at IS NULL)
        OR (status = 'delivered' AND delivered_at IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS alert_outbox_pending_idx
    ON alert_outbox (available_at, created_at)
    WHERE status = 'pending';
