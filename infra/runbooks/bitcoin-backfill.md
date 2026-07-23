# Bitcoin Historical Backfill

This runbook covers bounded, resumable replay from an unpruned Bitcoin Core
observer. Backfill is a recovery path and never bypasses the raw-observation
contract.

## Preconditions

- Select one mainnet or regtest observer with the complete requested block
  range. Do not use a pruned node for heights below its retained window.
- Keep wallet RPC disabled and use the node's cookie-authenticated private RPC
  endpoint.
- Confirm WAL, broker, raw archive, materializer, PostgreSQL, and ClickHouse
  health before starting.
- Create a request with a non-empty `source_id`, an inclusive ordered range, and
  `max_in_flight` between 1 and 256. Start conservatively; RPC reads may compete
  with the live observer.

## Required Ordering

For every height, the coordinator performs:

```text
getblockhash(height)
getblock(hash, 0)
validate native block hash and merkle commitment
archive exact recovered RPC observation
persist archive checkpoint
materialize facts keyed by block hash
persist materialization checkpoint
```

Fetches may be concurrent, but archive and materialization commits stay in
ascending height order. The checkpoint advances only after its complete next
value is durable. A retry may repeat an archive or materialization call, so both
sinks must deduplicate by deterministic logical identity.

## Resume and Reorg Handling

- Resume only when the stored request hash exactly matches source, range, and
  concurrency parameters.
- Resume from the height after `last_materialized_height`.
- If archive coverage is ahead of materialization coverage, reuse that raw
  coverage and retry materialization.
- Store facts by block hash plus revision, never by height alone.
- Compare `canonical_tip_hash_at_start` with the ending tip. A changed tip is
  evidence requiring range reconciliation; it is not silently rewritten or
  reported as a gap-free canonical snapshot.

## Verification

Run deterministic repository proof:

```bash
cargo test -p bitcoin-canonicality --test backfill
cargo test -p bitcoin-core-connector
```

When a local regtest node and cookie are explicitly provisioned, run the
integration profile for the selected range and verify:

```text
all requested heights have an archived exact payload
all materialized rows reference the fetched block hash
archive and materialization checkpoints end at the requested height
logical duplicate count is zero
tip-change evidence is recorded
```

If no node is provisioned, report the live range gate as `blocked:external`;
deterministic fixture proof remains valid but is not described as live-node
validation.

## Abort and Recovery

- Stop scheduling new fetches if WAL, archive, broker, PostgreSQL, or
  ClickHouse becomes unavailable.
- Do not delete WAL or raw archive objects.
- Preserve the latest durable checkpoint and exact error stage.
- Restart with the identical request. If parameters must change, create a new
  request identity instead of mutating the old job.
