# Ethereum/BSC raw-first foundation gate

Run:

```bash
just verify-evm-foundation
```

The command checks Rust formatting and focused Clippy policy, the shared
source-capture and source-runtime crates, chain-specific HTTP request plans,
recorded Ethereum and BSC canonicality paths, and both recorded and loopback
raw-first integrations. Evidence is written to:

```text
artifacts/certification/<build-sha>/evm-foundation-local/
```

A successful run means `local_gate=passed`. It proves:

- Reth, Ethereum consensus, and BSC payload bytes become WAL-durable before
  interpretation;
- Ethereum execution and consensus observations route to one versioned
  Ethereum raw topic while BSC routes to a separate raw topic;
- a malformed payload remains exact replay truth after parser failure;
- broker redelivery is idempotent for the deterministic observation identity;
- each source connection has an independent source session and local order;
- Reth execution, Beacon consensus, and official BSC JSON-RPC/REST polling
  cycles retain exact response bodies with bounded reads;
- transient source failures open explicit incomplete intervals and close them
  only after a recovered observation;
- shutdown interrupts retry and polling waits without interrupting an accepted
  observation between WAL commit and broker acknowledgement; and
- ambiguous WAL failures or contradictory commit ranges stop further capture
  until explicit recovery.

The gate deliberately emits `promotion_verdict=hold` until separate evidence
proves:

- an owned Reth execution plus consensus-client mainnet pair;
- an official BSC mainnet node;
- independently operated secondary reconciliation sources;
- live reorg, finality, reconnect, and gap-repair fault exercises; and
- declared load, soak, and disaster-recovery targets.

The current slice includes a reusable long-running HTTP polling loop and
in-memory source-health state, but it is not yet a deployed connector process.
It does not provide production node manifests, persisted health/incident
telemetry, credentials/authentication adapters, independently located mainnet
sources, or mainnet reconciliation. WAL reopen exposes the proven next
collector sequence, and the capture session has a tested resume path that
prevents sequence reuse.
