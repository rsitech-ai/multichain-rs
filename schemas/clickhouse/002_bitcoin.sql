CREATE TABLE IF NOT EXISTS multichain.bitcoin_blocks
(
    network LowCardinality(String),
    block_hash FixedString(64),
    parent_block_hash FixedString(64),
    height UInt32,
    block_time UInt32,
    transaction_count UInt32,
    canonicality LowCardinality(String),
    revision UInt64,
    source_id String,
    source_session_id FixedString(32),
    observation_id FixedString(64),
    parser_version LowCardinality(String),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (network, block_hash, revision);

CREATE TABLE IF NOT EXISTS multichain.bitcoin_transactions
(
    network LowCardinality(String),
    block_hash FixedString(64),
    height UInt32,
    transaction_index UInt32,
    txid FixedString(64),
    wtxid FixedString(64),
    virtual_size UInt64,
    canonicality LowCardinality(String),
    revision UInt64,
    source_id String,
    source_session_id FixedString(32),
    observation_id FixedString(64),
    parser_version LowCardinality(String),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (network, txid, block_hash, revision);

CREATE TABLE IF NOT EXISTS multichain.bitcoin_inputs
(
    network LowCardinality(String),
    block_hash FixedString(64),
    txid FixedString(64),
    input_index UInt32,
    previous_txid FixedString(64),
    previous_vout UInt32,
    sequence UInt32,
    canonicality LowCardinality(String),
    revision UInt64,
    source_id String,
    source_session_id FixedString(32),
    observation_id FixedString(64),
    parser_version LowCardinality(String),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (network, txid, input_index, block_hash, revision);

CREATE TABLE IF NOT EXISTS multichain.bitcoin_outputs
(
    network LowCardinality(String),
    block_hash FixedString(64),
    txid FixedString(64),
    output_index UInt32,
    value_sats UInt64,
    script_pubkey_id FixedString(64),
    script_pubkey_hex String,
    canonicality LowCardinality(String),
    revision UInt64,
    source_id String,
    source_session_id FixedString(32),
    observation_id FixedString(64),
    parser_version LowCardinality(String),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (network, txid, output_index, block_hash, revision);

CREATE TABLE IF NOT EXISTS multichain.bitcoin_utxo_state_events
(
    network LowCardinality(String),
    outpoint_txid FixedString(64),
    outpoint_vout UInt32,
    event_type LowCardinality(String),
    spending_txid Nullable(FixedString(64)),
    value_sats Nullable(UInt64),
    script_pubkey_id Nullable(FixedString(64)),
    revision UInt64,
    observation_id FixedString(64),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (network, outpoint_txid, outpoint_vout, revision);

CREATE TABLE IF NOT EXISTS multichain.bitcoin_mempool_membership_revisions
(
    network LowCardinality(String),
    source_id String,
    txid FixedString(64),
    epoch_id FixedString(64),
    epoch_revision UInt32,
    membership LowCardinality(String),
    cause LowCardinality(String),
    source_observed_at_unix_ns Nullable(Int64),
    recorded_at_unix_ns Int64,
    observation_id FixedString(64)
)
ENGINE = MergeTree
ORDER BY (network, source_id, txid, epoch_id, epoch_revision);

CREATE TABLE IF NOT EXISTS multichain.bitcoin_conflict_edges
(
    network LowCardinality(String),
    source_id String,
    spent_txid FixedString(64),
    spent_vout UInt32,
    txid FixedString(64),
    conflicting_txid FixedString(64),
    confidence LowCardinality(String),
    revision UInt64,
    observation_id FixedString(64),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (network, source_id, spent_txid, spent_vout, txid, conflicting_txid, revision);

CREATE TABLE IF NOT EXISTS multichain.bitcoin_cluster_snapshots
(
    network LowCardinality(String),
    source_id String,
    snapshot_sequence UInt64,
    cluster_id FixedString(64),
    member_txids Array(FixedString(64)),
    total_fee_sats UInt64,
    total_vsize UInt64,
    quality_flags Array(String),
    observation_id FixedString(64),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (network, source_id, cluster_id, snapshot_sequence);

CREATE TABLE IF NOT EXISTS multichain.canonicality_revisions
(
    chain LowCardinality(String),
    network LowCardinality(String),
    subject_key String,
    canonicality LowCardinality(String),
    height_or_slot UInt64,
    revision UInt64,
    observation_id FixedString(64),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (chain, network, subject_key, revision);

CREATE TABLE IF NOT EXISTS multichain.source_health_intervals
(
    chain LowCardinality(String),
    network LowCardinality(String),
    source_id String,
    source_session_id FixedString(32),
    interval_start_unix_ns Int64,
    interval_end_unix_ns Nullable(Int64),
    state LowCardinality(String),
    cause LowCardinality(String),
    revision UInt64
)
ENGINE = MergeTree
ORDER BY (chain, network, source_id, interval_start_unix_ns, revision);

CREATE TABLE IF NOT EXISTS multichain.coverage_intervals
(
    chain LowCardinality(String),
    network LowCardinality(String),
    dataset LowCardinality(String),
    source_id String,
    range_start UInt64,
    range_end UInt64,
    completeness LowCardinality(String),
    revision UInt64,
    evidence_ids Array(String)
)
ENGINE = MergeTree
ORDER BY (chain, network, dataset, source_id, range_start, revision);

CREATE VIEW IF NOT EXISTS multichain.bitcoin_blocks_current AS
SELECT
    network,
    block_hash,
    argMax(parent_block_hash, source.revision) AS parent_block_hash,
    argMax(height, source.revision) AS height,
    argMax(block_time, source.revision) AS block_time,
    argMax(transaction_count, source.revision) AS transaction_count,
    argMax(canonicality, source.revision) AS canonicality,
    max(source.revision) AS revision,
    argMax(source_id, source.revision) AS source_id,
    argMax(source_session_id, source.revision) AS source_session_id,
    argMax(observation_id, source.revision) AS observation_id,
    argMax(parser_version, source.revision) AS parser_version,
    argMax(recorded_at_unix_ns, source.revision) AS recorded_at_unix_ns
FROM multichain.bitcoin_blocks AS source
GROUP BY network, block_hash;
