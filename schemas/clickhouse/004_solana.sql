CREATE TABLE IF NOT EXISTS multichain.solana_transactions
(
    signature String,
    slot UInt64,
    blockhash String,
    message_version LowCardinality(String),
    static_account_keys Array(String),
    address_table_lookup_accounts Array(String),
    raw_transaction_hex String,
    fee UInt64,
    compute_units_consumed Nullable(UInt64),
    execution_status LowCardinality(String),
    execution_error String,
    canonicality LowCardinality(String),
    commitment LowCardinality(String),
    coverage_tier LowCardinality(String),
    revision UInt64,
    source_id String,
    source_session_id FixedString(32),
    observation_id FixedString(64),
    parser_version LowCardinality(String),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (signature, slot, blockhash, revision);

CREATE TABLE IF NOT EXISTS multichain.solana_instructions
(
    signature String,
    slot UInt64,
    blockhash String,
    outer_index UInt16,
    inner_index Nullable(UInt16),
    program_id_index UInt8,
    account_indexes Array(UInt8),
    raw_data_hex String,
    canonicality LowCardinality(String),
    commitment LowCardinality(String),
    revision UInt64,
    source_id String,
    source_session_id FixedString(32),
    observation_id FixedString(64),
    parser_version LowCardinality(String),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (signature, slot, blockhash, outer_index, inner_index, revision);

CREATE TABLE IF NOT EXISTS multichain.solana_logs
(
    signature String,
    slot UInt64,
    blockhash String,
    log_index UInt32,
    message String,
    revision UInt64,
    source_id String,
    source_session_id FixedString(32),
    observation_id FixedString(64),
    parser_version LowCardinality(String),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (signature, slot, blockhash, log_index, revision);

CREATE TABLE IF NOT EXISTS multichain.solana_balance_changes
(
    signature String,
    slot UInt64,
    blockhash String,
    account_index UInt16,
    pre_lamports UInt64,
    post_lamports UInt64,
    revision UInt64,
    source_id String,
    observation_id FixedString(64),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (signature, slot, blockhash, account_index, revision);

CREATE TABLE IF NOT EXISTS multichain.solana_token_balance_changes
(
    signature String,
    slot UInt64,
    blockhash String,
    account_index UInt16,
    mint String,
    pre_amount UInt256,
    post_amount UInt256,
    decimals UInt8,
    revision UInt64,
    source_id String,
    observation_id FixedString(64),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (signature, slot, blockhash, account_index, mint, revision);

CREATE TABLE IF NOT EXISTS multichain.solana_account_writes
(
    pubkey String,
    slot UInt64,
    blockhash String,
    owner String,
    lamports UInt64,
    raw_data_hex String,
    executable Bool,
    rent_epoch UInt64,
    write_version UInt64,
    canonicality LowCardinality(String),
    commitment LowCardinality(String),
    coverage_tier LowCardinality(String),
    revision UInt64,
    source_id String,
    source_session_id FixedString(32),
    observation_id FixedString(64),
    parser_version LowCardinality(String),
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (pubkey, slot, blockhash, write_version, revision);

CREATE TABLE IF NOT EXISTS multichain.solana_decoder_revisions
(
    program_id String,
    slot UInt64,
    instruction_identity String,
    decoder_version Nullable(String),
    decode_status LowCardinality(String),
    decoded_json Nullable(String),
    error Nullable(String),
    raw_data_hex String,
    native_fact_id FixedString(64),
    revision UInt64,
    recorded_at_unix_ns Int64
)
ENGINE = MergeTree
ORDER BY (program_id, slot, instruction_identity, revision);

CREATE VIEW IF NOT EXISTS multichain.solana_transactions_current AS
SELECT
    signature,
    slot,
    blockhash,
    argMax(canonicality, revision) AS canonicality,
    argMax(commitment, revision) AS commitment,
    max(revision) AS revision,
    argMax(source_id, revision) AS source_id,
    argMax(observation_id, revision) AS observation_id
FROM multichain.solana_transactions
GROUP BY signature, slot, blockhash;

CREATE VIEW IF NOT EXISTS multichain.solana_account_writes_current AS
SELECT
    pubkey,
    argMax(slot, revision) AS slot,
    argMax(blockhash, revision) AS blockhash,
    argMax(owner, revision) AS owner,
    argMax(lamports, revision) AS lamports,
    argMax(raw_data_hex, revision) AS raw_data_hex,
    argMax(commitment, revision) AS commitment,
    max(revision) AS revision,
    argMax(source_id, revision) AS source_id,
    argMax(observation_id, revision) AS observation_id
FROM multichain.solana_account_writes
GROUP BY pubkey
HAVING commitment != 'dead';
