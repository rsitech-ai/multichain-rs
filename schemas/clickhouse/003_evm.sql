CREATE TABLE IF NOT EXISTS multichain.evm_blocks
(
    chain_id UInt64,
    block_hash FixedString(66),
    parent_block_hash FixedString(66),
    block_number UInt64,
    canonicality LowCardinality(String),
    finality LowCardinality(String),
    revision UInt64,
    source_id String,
    source_session_id FixedString(32),
    observation_id FixedString(64),
    parser_version LowCardinality(String),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (chain_id, block_hash, revision);

CREATE TABLE IF NOT EXISTS multichain.evm_transactions
(
    chain_id UInt64,
    block_hash FixedString(66),
    block_number UInt64,
    transaction_index UInt32,
    transaction_hash FixedString(66),
    sender FixedString(42),
    recipient Nullable(FixedString(42)),
    value UInt256,
    nonce UInt64,
    gas_limit UInt64,
    max_fee_per_gas Nullable(UInt256),
    blob_versioned_hashes Array(FixedString(66)),
    canonicality LowCardinality(String),
    finality LowCardinality(String),
    revision UInt64,
    source_id String,
    source_session_id FixedString(32),
    observation_id FixedString(64),
    parser_version LowCardinality(String),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (chain_id, transaction_hash, block_hash, revision);

CREATE TABLE IF NOT EXISTS multichain.evm_receipts
(
    chain_id UInt64,
    block_hash FixedString(66),
    transaction_hash FixedString(66),
    success Bool,
    cumulative_gas_used UInt64,
    canonicality LowCardinality(String),
    finality LowCardinality(String),
    revision UInt64,
    source_id String,
    source_session_id FixedString(32),
    observation_id FixedString(64),
    parser_version LowCardinality(String),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (chain_id, transaction_hash, block_hash, revision);

CREATE TABLE IF NOT EXISTS multichain.evm_logs
(
    chain_id UInt64,
    block_hash FixedString(66),
    transaction_hash FixedString(66),
    log_index UInt64,
    address FixedString(42),
    topics Array(FixedString(66)),
    raw_data_hex String,
    canonicality LowCardinality(String),
    finality LowCardinality(String),
    revision UInt64,
    source_id String,
    source_session_id FixedString(32),
    observation_id FixedString(64),
    parser_version LowCardinality(String),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (chain_id, transaction_hash, log_index, block_hash, revision);

CREATE TABLE IF NOT EXISTS multichain.evm_decoder_revisions
(
    chain_id UInt64,
    contract_address FixedString(42),
    valid_from_block UInt64,
    valid_to_block Nullable(UInt64),
    decoder_version String,
    selector_or_topic FixedString(66),
    decode_status LowCardinality(String),
    decoded_json String,
    raw_data_hex String,
    native_fact_id FixedString(64),
    revision UInt64,
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (chain_id, contract_address, selector_or_topic, valid_from_block, revision);

CREATE VIEW IF NOT EXISTS multichain.evm_blocks_current AS
SELECT
    chain_id,
    block_hash,
    argMax(parent_block_hash, revision) AS parent_block_hash,
    argMax(block_number, revision) AS block_number,
    argMax(canonicality, revision) AS canonicality,
    argMax(finality, revision) AS finality,
    max(revision) AS revision,
    argMax(source_id, revision) AS source_id,
    argMax(observation_id, revision) AS observation_id
FROM multichain.evm_blocks
GROUP BY chain_id, block_hash;
