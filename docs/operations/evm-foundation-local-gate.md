# Ethereum/BSC raw-first foundation gate

Run:

```bash
just verify-evm-foundation
```

The command checks Rust formatting and focused Clippy policy, the shared
source-capture crate, existing recorded Ethereum and BSC canonicality paths,
and the cross-chain raw-first integration. Evidence is written to:

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
  and
- ambiguous WAL failures or contradictory commit ranges stop further capture
  until explicit recovery.

The gate deliberately emits `promotion_verdict=hold` until separate evidence
proves:

- an owned Reth execution plus consensus-client mainnet pair;
- an official BSC mainnet node;
- independently operated secondary reconciliation sources;
- live reorg, finality, reconnect, and gap-repair fault exercises; and
- declared load, soak, and disaster-recovery targets.

The current slice is a durable capture boundary and recorded-source proof. It
does not yet provide long-running Reth, beacon, or BSC network loops,
connector health telemetry, or production node configuration. WAL reopen
does expose the proven next collector sequence, and the capture session has a
tested resume path that prevents sequence reuse.
