# ADR 0001: Observation order and durability

- Status: Accepted
- Date: 2026-07-23
- Owners: Ingestion platform

## Context

The platform consumes independent channels from node-local blockchain
observers. Bitcoin Core ZMQ topics may be delayed, interleaved, duplicated, or
lost, and their source-specific sequence values do not provide one total order
across topics. Wall-clock timestamps are also affected by scheduler delay and
clock correction.

The source payload must be replayable exactly as observed. A durability
timestamp cannot be embedded by mutating an observation frame after that frame
has been flushed, because doing so would invalidate checksums and make crash
recovery ambiguous.

## Decision

Each connector owns a `source_session_id` and allocates one strictly increasing
`collector_sequence` across every channel in that session before appending an
observation. The replay-stable observation identifier is:

```text
BLAKE3(
  "observation/v1" ||
  source_id_utf8 ||
  source_session_id ||
  collector_sequence_be_u64 ||
  payload_sha256
)
```

`payload_sha256` is the SHA-256 digest of the exact source payload bytes. The
observation frame is immutable after append.

Durability is represented by a separate immutable `WalCommit`. A commit covers
an inclusive collector-sequence range and WAL byte-offset range. Its
`durable_at_unix_ns` is assigned only after the group commit succeeds.
`CommittedObservation` is a derived read model that joins an observation to the
covering commit record.

WAL recovery validates frame lengths, checksums, commit hashes, monotonic
collector sequences, and commit-range containment. It truncates only an invalid
or uncommitted tail. A sequence regression within a source session is a hard
error.

A WAL segment is reclaimable only when both of these durable checkpoints cover
the complete segment:

1. the broker has acknowledged every record using `acks=all`; and
2. a verified object-storage manifest covers the exact byte range and checksum.

Reclamation is recoverable and auditable. Broker acknowledgement alone is not
archive proof.

## Rejected alternatives

- Wall-clock time as the only cross-topic ordering key: clocks and scheduling do
  not establish a replay-stable total order.
- Per-topic counters as the platform order: they cannot order events across
  channels.
- Mutating WAL frames after `fsync`: this breaks immutability, checksums, and
  deterministic crash recovery.
- Treating transport idempotence as logical idempotence: producer retries do not
  protect downstream fact identity.
- Deleting WAL data after broker acknowledgement alone: a broker is not the
  permanent raw archive.

## Consequences

- Every observation is attributable to an exact source session and ordered
  deterministically within that session.
- Durability latency remains measurable without making observation identity
  circular.
- Connectors must serialize sequence allocation across their input channels.
- WAL group-commit records and dual broker/archive checkpoints become mandatory
  operational state.
- There is intentionally no claim of a total order between independent source
  sessions.

## Review trigger

Review this decision before changing observation identity, WAL framing,
group-commit semantics, archive format, or the durability/reclamation SLO. Also
review it if a source exposes a cryptographically verifiable global ordering
primitive that could replace source-session ordering.
