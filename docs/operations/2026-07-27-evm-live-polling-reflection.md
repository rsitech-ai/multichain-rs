# EVM live polling reflection

## Task

- **ID / title:** Ethereum and BSC raw-first live polling
- **Date:** 2026-07-27
- **Scope:** Shared HTTP source runtime, Reth/Beacon/BSC request plans, local proof
- **Authority boundary:** Local implementation, verification, PR, and merge; no production source provisioning

## Success and Risk

- **Success criteria:** Exact response bytes become WAL-durable before interpretation; retries, gaps, cancellation, and routing are explicit and tested.
- **Hypothesis 1:** A shared sequential HTTP runtime can preserve durability without merging chain semantics.
- **Hypothesis 2:** Chain-specific plans can share transport while retaining distinct finality and topic routing.
- **Hypothesis 3:** Loopback fault tests provide useful repository proof without implying mainnet readiness.
- **Rollback path:** Revert the runtime and plan commit; the existing recorded-source capture path remains independent.

## Candidate Directions

| Candidate | Expected benefit | Main risk | Evidence before choice | Decision |
| --- | --- | --- | --- | --- |
| Shared runtime plus chain-owned plans | One audited durability/retry boundary | Accidental semantic coupling | Existing connectors already separate domain and finality logic | Retained |
| Independent network loop per connector | Maximum local autonomy | Repeated WAL, retry, and cancellation bugs | Three adapters require the same ordering invariant | Rejected |

## Evidence

- **First meaningful failure signal:** TDD compile failure showed that no source runtime API existed.
- **Commands or runtime checks:** Focused RED/GREEN tests, loopback HTTP integration, `just verify-evm-foundation`, workspace Clippy/tests, `cargo deny`, `gitleaks`, Protobuf compatibility, Compose validation, and shell contracts.
- **What the evidence ruled in or out:** The local code path, bounded body handling, response retention, WAL/broker order, and topic routing are proven; mainnet independence and operational qualification are not.

## Decision

- **Root cause or remaining unknown:** The prior slice had durable recorded capture but no reusable long-running source transport.
- **Retained fix / direction:** One single-writer runtime persists accepted responses before applying their success/failure disposition; adapters own request and finality semantics.
- **Why alternatives were rejected:** Per-connector loops would duplicate the highest-risk ordering and cancellation logic.
- **Residual risk:** Authentication, persisted health incidents, semantic JSON-RPC error classification, mainnet reconciliation, load, and disaster recovery remain unproven.
- **Rollback trigger:** Any evidence of sequence reuse, silent gap closure, unbounded reads, wrong-chain routing, or publication before WAL commit.

## Reusable Lesson

- **Pattern to retain:** Treat retryable error bodies as replayable source observations and classify them only after durable capture.
- **Pattern to avoid:** Reporting cancellation through the same outcome as a completed polling cycle.
- **Where it applies next:** Solana Yellowstone reconnect/gap handling and production EVM connector services.
