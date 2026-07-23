# ADR 0002: Stable fact identity and append-only revisions

- Status: Accepted
- Date: 2026-07-23
- Owners: Canonicality and materialization

## Context

Parsers, protocol decoders, labels, canonicality decisions, and finality
projections change over time. Reprocessing the same observations must correct
derived data without changing the logical identity of the subject being
described. At-least-once delivery also requires deterministic retry behavior.

## Decision

Every logical fact has a deterministic `fact_key` derived only from the
chain-native subject and fact type. The key does not contain a revision,
parser version, recorded timestamp, label version, or mutable canonicality
state.

Each change appends a new monotonically increasing revision. Its identifier is:

```text
BLAKE3(
  "fact/v1" ||
  fact_key ||
  revision_be_u64 ||
  payload_sha256
)
```

A writer may append revision `n + 1` only when revision `n` is the current
stored value. Retrying the same `fact_id` is idempotent. A different payload for
the same `(fact_key, revision)` is a hard conflict and must not be silently
overwritten. `supersedes_fact_id` links the new revision to the prior revision.

Current-state reads select the maximum valid revision explicitly, using
controlled projections or query-time `argMax`-style logic. Correctness does not
depend on eventual background deduplication.

## Rejected alternatives

- Placing `revision` inside the logical fact key: this makes every correction a
  different logical subject and prevents conflict detection.
- Updating facts in place: this destroys auditability and replay comparison.
- Deriving identity from database row IDs: it makes independently replayed
  results non-deterministic.
- Allowing last-writer-wins for the same revision: it hides divergent decoders
  and corrupts lineage.
- Depending on `ReplacingMergeTree` background merges for correctness: merges
  are eventual and are not a read consistency contract.

## Consequences

- Corrections, reorgs, and decoder upgrades preserve a complete revision trail.
- Writers need compare-and-append concurrency control per `fact_key`.
- Consumers must understand revision events and must not cache facts forever by
  `fact_id` alone.
- Parser and build versions remain provenance fields, not identity fields.
- Conflicting replays fail visibly and can be quarantined for investigation.

## Review trigger

Review this decision before introducing mutable fact storage, changing logical
keys, adding multi-writer revision allocation, or altering the definition of
payload equivalence.
