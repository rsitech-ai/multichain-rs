# Phase 0 synthetic vertical evidence

## Verification scope

- Verified at: `2026-07-23T13:22:29Z`
- Verification base commit: `b50d94fec43e36ff6d1e542043fd496fa3f4cfb6`
- Source: deterministic fixture `phase0-001`
- Collector session: `50505050505050505050505050505050`
- Collector range: `0..=0`
- Source-native sequence: `42`

The verification worktree contained the Task 5 implementation that was
committed immediately after the gate. The base commit above is recorded because
a commit cannot contain its own resulting object ID.

## Durable-path evidence

`just verify-phase0` passed against the disposable Compose environment:

```text
phase0_synthetic: PASS
phase0_restart_replay: PASS
logical_fact_duplicates: 0
archive_manifest_gaps: 0 after the explicit repair fixture
```

The synthetic test proved one committed WAL observation, consumed the matching
logical observation from Redpanda, replayed the exact bytes through a committed
MinIO manifest, and materialized the deterministic fact in ClickHouse.

- Manifest SHA-256:
  `910a5b3c375900a81fa591f89b8268477c601caa189626174263da586de66718`
- Manifest collector range: `0..=0`
- Fact ID:
  `005046306e6d1a3ba16e299004a5c90c5d44544b2af3f76dc080284f27d9b83e`
- ClickHouse physical rows after merge: `1`
- ClickHouse distinct logical facts: `1`

The REST assertions proved fixture truth metadata and observation lineage. The
WebSocket assertion received a real upgrade and a snapshot containing the same
fact ID.

## Restart, replay, and gap evidence

The fault test stopped the normalizer after receiving the Redpanda record and
before its ClickHouse insert. It deliberately stored and committed no consumer
offset, created a second consumer with the same group ID, received the same
record again, and then committed the deterministic fact. Replaying the insert
did not increase the distinct logical fact count.

Collector coverage `[42, 44]` produced this explicit incident:

```text
status=known_incomplete
missing_first=43
missing_last=43
```

Adding the repair observation produced coverage `[42, 43, 44]` and zero
remaining incomplete intervals.

Full raw-archive replay also exposed and fixed an overlap bug: an exact
previously committed range is now resolved to its existing manifest before
manifest-chain append, so at-least-once replay is idempotent.

## Serving and readiness evidence

The gate launched the repository query API on an isolated verification port and
required its component identity to prevent another local service from
satisfying the probe:

```json
{"component":"query-api","ready":true,"broker":"ready","clickhouse":"ready","postgres":"ready","broker_checkpoint":"ready","archive_checkpoint":"ready"}
```

An independent assertion using a source session with no durable broker or
archive checkpoints returned `ready=false`. ClickHouse readiness uses an
authenticated SQL query rather than the unauthenticated `/ping` endpoint.

## Commands

```bash
just infra-up
just verify-phase0
cargo clippy --workspace --all-targets --all-features -- -D warnings
just check
just infra-down
```

The final `just check` and teardown are recorded in the task handoff after the
feature commit.
