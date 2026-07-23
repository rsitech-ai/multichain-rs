# Local runtime validation reflection

## Task

- **ID / title:** Four-chain and durable-platform local runtime validation
- **Date:** 2026-07-23
- **Scope:** Bitcoin Core, Reth, official BSC, Agave, and the local Redpanda,
  MinIO, ClickHouse, and PostgreSQL pipeline
- **Authority boundary:** Local branch changes, official downloads, disposable
  processes and containers, tests, and exact task-scoped cleanup; no push,
  production credentials, paid endpoints, or external deployment

## Success and risk

- **Success criteria:** One command produces non-skipped runtime evidence for
  every selected scope, preserves exact versions and identities, and cleans up
  without modifying unrelated services.
- **Hypothesis 1:** Pinned native arm64 binaries can prove every chain execution
  boundary without mainnet synchronization.
- **Hypothesis 2:** Existing integration tests will pass unchanged against the
  pinned current ClickHouse image once the Compose stack is healthy.
- **Hypothesis 3:** Fail-closed shell behavior will retain the exact first error
  rather than reducing it to a later timeout.
- **Rollback path:** Stop only registered PIDs and the exact Compose project,
  retain evidence, leave the affected scope non-passing, and keep deterministic
  replay evidence as the safe baseline.

## Candidate directions

| Candidate | Expected benefit | Main risk | Evidence before choice | Decision |
| --- | --- | --- | --- | --- |
| Native disposable clients plus task-scoped Compose | Strong local execution evidence with bounded resources | Client CLI drift and real schema incompatibilities | Official arm64 assets existed; Docker was available; ports were free | Retained |
| Fixtures and mocks only | Fast and deterministic | Would not remove live runtime blockers or expose current client/storage behavior | Repository already had strong fixture coverage | Rejected as insufficient |
| Mainnet nodes and external providers | Highest production fidelity | Exceeded local authority, storage, credentials, time, and topology constraints | No production endpoints or credentials were provisioned | Deferred to production gates |

## Evidence

- **First meaningful failure signal:** Reth returned
  `-32602 invalid transaction request`; the initial script discarded the JSON-RPC
  error and later reported an empty transaction-hash timeout.
- **Commands or runtime checks:** Per-scope `just validate-local <scope>`,
  aggregate `just validate-local`, non-skipped infrastructure tests, and
  `just check`.
- **What the evidence ruled in or out:** Reth needed an explicit fee field;
  BSC 1.7.3 had removed `--dev`; Agave required a wider dynamic port range;
  ClickHouse 26.3 rejected aggregate-alias shadowing and nullable sorting keys;
  `curl --user` caused a gitleaks finding even with a known local credential.

## Decision

- **Root cause or remaining unknown:** The blockers were version-specific
  interface and query-contract drift, not missing platform architecture.
- **Retained fix / direction:** Preserve JSON-RPC error objects, query node fee
  data, generate a supported one-validator Parlia chain, satisfy Agave's port
  contract, qualify ClickHouse aggregate weights, use a non-null sorting-key
  expression, and keep validation credentials out of curl auth syntax.
- **Why alternatives were rejected:** Adding mocks would hide the failures;
  downgrading clients would validate stale behavior; globally enabling nullable
  ClickHouse keys would weaken schema discipline; provisioning production
  services was outside authority.
- **Residual risk:** Local chains do not prove independent regions, mainnet
  finality, source diversity, production HA, sustained throughput, or soak
  behavior.
- **Rollback trigger:** Any checksum mismatch, occupied port, insufficient disk,
  unavailable readiness dependency, failed cleanup, or contradiction between
  local and recorded mainnet semantics leaves the gate non-passing.

## Reusable lesson

- **Pattern to retain:** Treat client releases and storage engines as executable
  contracts. Preserve their first structured error, encode discovered
  constraints in a focused regression test, then rerun the real boundary.
- **Pattern to avoid:** Translating an upstream error into an empty value,
  assuming inherited Geth flags still exist in BSC, or treating schema text
  inspection as proof that current ClickHouse accepts it.
- **Where it applies next:** Production observer provisioning, version upgrades,
  replay migrations, and every acceptance gate that crosses an external client
  or storage boundary.
