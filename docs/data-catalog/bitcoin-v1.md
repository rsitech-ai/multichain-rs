# Bitcoin Dataset v1

Bitcoin facts are append-only, source-lineaged, and revision-aware. Current
truth is selected explicitly by revision; background MergeTree merges are never
part of query correctness.

## Identity and Units

- Blocks use the 32-byte header hash encoded as 64 lowercase hexadecimal
  characters. Height is a branch position, not a unique block identity.
- Transactions use both `txid` and `wtxid`. Inputs identify exact outpoints.
- Output ownership is not inferred. `script_pubkey_id` is SHA-256 over exact
  script bytes; a presentation address is a derived optional encoding.
- Amounts are integer satoshis. API responses encode them as decimal strings;
  ClickHouse stores bounded values as `UInt64`.
- Block time is header seconds. Observation, source, and record times state
  their units explicitly and are not interchangeable.

## Tables

| Table | Logical key | Revision/current rule |
|---|---|---|
| `bitcoin_blocks` | network + block hash | highest revision; inspect `canonicality` |
| `bitcoin_transactions` | network + txid + inclusion block | highest inclusion revision |
| `bitcoin_inputs` | network + txid + input index + inclusion block | highest inclusion revision |
| `bitcoin_outputs` | network + txid + output index + inclusion block | highest inclusion revision |
| `bitcoin_utxo_state_events` | network + outpoint + revision | fold events in revision order |
| `bitcoin_mempool_membership_revisions` | network + source + txid + epoch | highest epoch revision |
| `bitcoin_conflict_edges` | network + source + outpoint + transaction pair | immutable evidence revisions |
| `bitcoin_cluster_snapshots` | network + source + cluster + snapshot sequence | latest explicit sequence |
| `canonicality_revisions` | chain + network + subject | highest revision |
| `source_health_intervals` | chain + network + source + interval start | preserve history |
| `coverage_intervals` | chain + network + dataset + source + range start | highest evidence revision |

All history tables use ordinary `MergeTree`. The
`bitcoin_blocks_current` view uses `argMax(..., revision)` and `max(revision)`;
equivalent current queries for other facts must do the same or use a controlled
projection built from that rule.

## Canonicality and Reorgs

`candidate`, `canonical`, and `non_canonical` are revision values, not mutable
row flags. A reorg appends disconnect revisions before replacement-branch
connect revisions. Both inclusions remain queryable. UTXO reversals and mempool
reacceptance are explicit events.

## Source Coverage and Gaps

Every block batch records `source_id`, `source_session_id`, `observation_id`,
parser version, and record time. Mempool membership is always source-specific.
Union, intersection, and quorum are derived views over eligible observer
health.

Current-state reconciliation does not erase a historical gap.
`source_health_intervals` and `coverage_intervals` retain incomplete ranges and
their evidence even after the current source becomes healthy.

## Retention and Replay

Exact raw observations and manifests in object storage are permanent replay
truth. ClickHouse fact histories may be rebuilt from them. A connector WAL
segment is reclaimable only after broker and archive coverage both include its
complete committed range.

## Sample Current Query

```sql
SELECT
    network,
    block_hash,
    argMax(height, revision) AS height,
    argMax(canonicality, revision) AS canonicality,
    max(revision) AS as_of_revision
FROM multichain.bitcoin_blocks
WHERE network = 'mainnet'
GROUP BY network, block_hash
HAVING canonicality = 'canonical'
ORDER BY height DESC
LIMIT 100;
```
