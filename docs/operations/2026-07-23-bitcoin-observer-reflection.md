# Bitcoin Observer Recovery Reflection

## Task

- **ID / title:** Task 7, Bitcoin Core observer connector
- **Date:** 2026-07-23
- **Scope:** Private ZMQ/RPC capture, durable ordering, gap recovery, and tests
- **Authority boundary:** Local implementation and validation only; no push and no live-node mutation

## Success and Risk

- **Success criteria:** Exact source bytes reach the WAL before interpretation; gaps remain explicit after state convergence.
- **Hypothesis 1:** One capture task can impose a deterministic order over three topic receivers.
- **Hypothesis 2:** An atomic RPC snapshot can repair current mempool state without inventing unseen transitions.
- **Hypothesis 3:** A bounded native Rust ZMQ client avoids a system `libzmq` dependency.
- **Rollback path:** Revert commit `9819c1d`; existing fixture ingestion and durable storage remain independent.

## Candidate Directions

| Candidate | Expected benefit | Main risk | Evidence before choice | Decision |
|---|---|---|---|---|
| Native Rust `zeromq` with bounded channel | No host C library and explicit backpressure | New dependency and reconnect behavior | Crate supports Tokio SUB sockets and multipart messages | Retained |
| Decode transactions/blocks in capture loop | Immediate semantic events | Latency, parser faults, and loss of raw-first boundary | Architecture requires exact observation first | Rejected |

## Evidence

- **First meaningful failure signal:** Strict dependency policy rejected required Bitcoin licenses in Task 6; Task 7 later hit `No space left on device` during a clean test build.
- **Commands or runtime checks:** `cargo test -p bitcoin-core-connector`, the optional regtest integration test, and `just check`.
- **What the evidence ruled in or out:** Deterministic ordering, RPC snapshot convergence, parent-walk recovery, and repository compatibility pass locally. A live Core reorg/transient drill remains unproven without `bitcoind` and a cookie.

## Decision

- **Root cause or remaining unknown:** ZMQ cannot recover a transaction that arrived and disappeared while disconnected.
- **Retained fix / direction:** Persist a recovered snapshot with `recovered_by_rpc`, realign on a later live sequence, and retain the closed interval as `known_incomplete`.
- **Why alternatives were rejected:** Fabricating add/remove events would falsely claim transition evidence.
- **Residual risk:** Live reconnect timing and Core-version behavior still require the regtest drill.
- **Rollback trigger:** Any live test that shows ordering drift, missed WAL durability, or a current projection divergent from Core.

## Reusable Lesson

- **Pattern to retain:** Repair present state independently from historical coverage truth.
- **Pattern to avoid:** Treating successful resynchronization as proof that no observations were missed.
- **Where it applies next:** Block canonicality, multi-observer mempools, Solana fork rollback, and replay health.
