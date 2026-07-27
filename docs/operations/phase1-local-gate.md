# Bitcoin Phase 1 local gate

Run:

```bash
just verify-phase1
```

The command validates the RSI Tech project manifest, Rust formatting, focused
Bitcoin and API linting, Bitcoin unit tests, the recorded three-observer
mempool fixture, and the real alert-preview HTTP route. Evidence is written to:

```text
artifacts/certification/<build-sha>/phase1-local/
```

A successful run means `local_gate=passed`. It deliberately emits
`promotion_verdict=hold` until separate evidence proves:

- three independent Bitcoin mainnet observers;
- production high availability and security controls;
- declared load and soak targets;
- a disaster-recovery drill; and
- reorg/correction propagation through the web workspace.

The local gate does not run or claim Phase 0 durable-infrastructure acceptance;
that remains:

```bash
just infra-up
just verify-phase0
just infra-down
```

It also does not replace the repository-wide `just check` gate.
