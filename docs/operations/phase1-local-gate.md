# Bitcoin Phase 1 local gate

Run:

```bash
just infra-up
just verify-phase1
just infra-down
```

The command validates the RSI Tech project manifest, Rust formatting, focused
Bitcoin and API linting, Bitcoin unit tests, the recorded three-observer
mempool fixture, the real alert-preview HTTP route, and the durable alert
definition/state/evaluation/outbox transaction against PostgreSQL. Evidence is
written to:

```text
artifacts/certification/<build-sha>/phase1-local/
```

To use an existing PostgreSQL instance instead of the Compose service:

```bash
MULTICHAIN_TEST_DATABASE_URL=postgres://user@127.0.0.1:5432/database \
  just verify-phase1
```

The target database must be disposable test infrastructure for which the
caller is authorized to create the Task 15 tables and rows.

A successful run means `local_gate=passed`. It deliberately emits
`promotion_verdict=hold` until separate evidence proves:

- three independent Bitcoin mainnet observers;
- production high availability and security controls;
- declared load and soak targets;
- a disaster-recovery drill; and
- reorg/correction propagation through the web workspace.

The alert transaction proof does not run or claim the complete Phase 0
durable-infrastructure acceptance path; that remains:

```bash
just infra-up
just verify-phase0
just infra-down
```

It also does not replace the repository-wide `just check` gate.
