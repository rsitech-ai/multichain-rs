# EVM Dataset v1

Ethereum mainnet and BNB Smart Chain mainnet share native execution fact
shapes keyed by EIP-155 `chain_id`. They do not share a finality adapter.

## Identity and Units

- Every block, transaction, receipt, and log key begins with `chain_id`.
- Hashes and addresses use canonical `0x`-prefixed lowercase encodings.
- Transaction and fee values remain unsigned 256-bit integers in ClickHouse
  and decimal strings at API boundaries.
- Contract creation retains a null recipient.
- Log identity is chain ID plus transaction hash plus receipt-global log index.
- Exact raw source payloads remain in the observation archive; native fact rows
  identify source, session, observation, parser version, and record time.

## Native Tables

`evm_blocks`, `evm_transactions`, `evm_receipts`, and `evm_logs` use ordinary
`MergeTree` histories. Current queries explicitly select the highest revision.
`evm_blocks_current` demonstrates the required `argMax(..., revision)` rule.

Ethereum finality values are `pending`, `included`, `canonical_head`, `safe`,
`finalized`, and `reorged`. BSC uses `pending`, `included`,
`canonical_head`, `fast_finalized`, and `reorged`. The API rejects vocabulary
from the other chain.

## Decoder Isolation

`evm_decoder_revisions` is separate from native facts. ABI deployments are
chain-, address-, and block-range-qualified. Overlapping ranges fail closed.
Unknown selectors/topics and decoder failures retain native bytes. A decoder
replay appends a new decoder revision and never mutates or blocks native
ingestion.

Coverage flags for full native history, archive state, and traces are exposed
independently. A full execution node must not imply archive-state or trace
coverage.
