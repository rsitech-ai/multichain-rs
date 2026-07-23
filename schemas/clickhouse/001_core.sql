CREATE DATABASE IF NOT EXISTS multichain;

CREATE TABLE IF NOT EXISTS multichain.fixture_facts
(
    fact_id String,
    fact_key String,
    revision UInt64,
    fixture_id String,
    value String,
    source_sequence UInt64,
    chain LowCardinality(String),
    network LowCardinality(String),
    canonicality LowCardinality(String),
    parser_version String,
    source_id String,
    source_session_id String,
    observation_id String,
    valid_from_unix_ns Int64,
    recorded_at_unix_ns Int64
)
ENGINE = ReplacingMergeTree(revision)
ORDER BY (fixture_id, fact_id);
