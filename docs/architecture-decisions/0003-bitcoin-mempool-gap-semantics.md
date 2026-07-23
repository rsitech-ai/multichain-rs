# ADR 0003: Bitcoin mempool epochs and gap semantics

- Status: Accepted
- Date: 2026-07-23
- Owners: Bitcoin ingestion and reliability

## Context

Bitcoin has no authoritative global mempool. Each Bitcoin Core observer has
local policy, peer connectivity, timing, and restart history. ZMQ delivery can
lose messages. RPC reconciliation can prove the node's current state, but it
usually cannot reconstruct every missing addition, removal, conflict, or
replacement transition.

A transaction can leave and later re-enter the same observer's mempool. Those
membership epochs must remain distinct while later corrections to a single
epoch remain revisions of the same logical fact.

## Decision

Mempool state is always scoped by `source_id`. An observed acceptance starts a
membership epoch whose `membership_epoch_id` is the accepting observation ID.
When a snapshot supplies the first evidence that a transaction is present, the
epoch ID is:

```text
BLAKE3(
  "mempool-epoch/v1" ||
  source_id ||
  txid ||
  snapshot_observation_id
)
```

The fact key for membership contains the network, source, transaction ID, and
membership epoch ID, but never the fact revision. Removal reason and confidence
are revisable attributes. Confidence is one of `observed`, `reconciled`,
`inferred`, or `unknown`.

Every detected coverage problem opens a durable `CoverageInterval`. Its state
is:

- `complete`: range checks prove expected coverage;
- `known_incomplete`: some transitions are not known and have not been
  reconstructed;
- `state_reconciled`: a snapshot has restored the current projection, while
  missing transition history remains explicitly unknown.

Repair evidence is linked by observation ID. An interval is closed as
`complete` only after the relevant sequence/range invariants prove historical
coverage. Snapshot convergence by itself may close operational divergence but
must not relabel missing history as complete.

Union, intersection, quorum, and source-specific views are computed from
observer-local states. They do not erase source attribution.

## Rejected alternatives

- One global mempool table: it invents authority that Bitcoin does not provide.
- Using only `(source_id, txid)` as membership identity: it conflates re-entry
  epochs.
- Treating snapshot convergence as recovered transition history: the resulting
  timeline would claim events that were never observed.
- Inferring exact removal reasons whenever a transaction disappears: eviction,
  conflict, block inclusion, expiry, and restart can be ambiguous.
- Placing `revision` inside the membership fact key: it prevents stable epoch
  identity and correction conflict checks.

## Consequences

- All mempool analytics retain observer and evidence provenance.
- Some intervals remain permanently `known_incomplete`; this is correct,
  queryable uncertainty rather than an ingestion success.
- First-seen comparisons use source timestamps and clock-quality metadata and
  never imply global first propagation.
- Reconciliation must emit both state corrections and coverage-state changes.
- Alerting and APIs must surface incomplete coverage instead of silently
  treating missing observations as absence.

## Review trigger

Review this decision before adding direct P2P sensors, changing Bitcoin Core
sequence recovery, aggregating observers into a new product-level authority, or
claiming that a repair mechanism reconstructs transition history.
