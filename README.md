# Multichain

[![CI](https://github.com/rsitech-ai/multichain-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/rsitech-ai/multichain-rs/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Multichain is a Rust foundation for source-qualified blockchain observation,
replay, canonicality, and serving across:

- Bitcoin mainnet
- Ethereum mainnet
- Solana mainnet-beta
- BNB Smart Chain mainnet

Its central rule is simple: **persist the exact source observation before
interpreting it**. A transaction, slot, block, or mempool record always keeps
the identity of the observer that saw it and the observation order needed to
replay later corrections.

> Status: v0.1.0 is a developer preview. The repository has complete local
> runtime and deterministic replay evidence for all four network models. It is
> not a hosted service, a wallet, trading software, or a production mainnet
> deployment.

## What is implemented

| Network | Repository scope |
| --- | --- |
| Bitcoin | Bitcoin Core ZMQ/RPC capture, crash-safe WAL, observer-local mempools, gap recovery, block DAG, UTXO apply/revert, RBF and CPFP relationships, source-qualified facts and APIs |
| Ethereum | Chain-ID-safe domain, WAL-first exact Reth/consensus source capture, execution-payload joins, reorg revisions, safe/finalized ancestry, decoder and fact isolation |
| Solana | Fork-qualified transaction identity, Yellowstone protobuf capture, dual-provider gap/divergence semantics, reversible selected-account projections, decoder revisions |
| BNB Smart Chain | Official-client semantics, independently routed WAL-first source capture, chain-ID isolation, head/finalized tracking, fast-finality stall and regression handling |
| Platform | Framed WAL and archive format, Redpanda/Kafka adapters, ClickHouse facts, PostgreSQL checkpoints, S3-compatible archive path, REST and WebSocket serving |

Local validation uses disposable, loopback-only runtimes. The passing gate
exercised three Bitcoin Core observers, Reth, the official BSC client, Agave,
Redpanda, ClickHouse, PostgreSQL, and MinIO. See
[local runtime validation](docs/operations/local-runtime-validation.md) for
the exact versions, checksums, evidence, and explicit non-claims.

## Architecture

```text
chain sources
    │
    ▼
Rust connectors ──> local durable WAL ──> Redpanda / Kafka
                                             │
                       ┌─────────────────────┴──────────────────┐
                       ▼                                        ▼
               raw object archive                       normalized facts
                                                        and revisions
                                                               │
                                      ┌────────────────────────┼─────────────┐
                                      ▼                        ▼             ▼
                                 ClickHouse               PostgreSQL     alerts
                                      └────────────────────────┬─────────────┘
                                                               ▼
                                                   REST / WebSocket / gRPC
```

The codebase is a modular monorepo. It keeps pure chain and state logic in
crates, source I/O in connectors, revisions in processors, and external
interfaces in services.

## Requirements

- Rust 1.97.1 (pinned by `rust-toolchain.toml`)
- A C/C++ toolchain and CMake; Linux builds also require libcurl development
  headers for the bundled librdkafka build
- Docker with Compose for the durable platform tests
- `just`, `buf`, `cargo-deny`, and `gitleaks` for the full repository gate
- At least 6 GiB free for each native local-runtime validation scope

The local runtime suite downloads checksum-pinned chain clients on demand. It
does not require production node credentials.

## Quick start

Run the repository-only verification:

```bash
just check
```

Run the locally owned Bitcoin Phase 1 checks and produce an explicit
pass-versus-production-HOLD record. The gate requires PostgreSQL; start the
local services first:

```bash
just infra-up
just verify-phase1
just infra-down
```

See the [Phase 1 local gate](docs/operations/phase1-local-gate.md) for the
evidence boundary and the `MULTICHAIN_TEST_DATABASE_URL` override for an
existing PostgreSQL instance.

Run the repository-owned Ethereum/BSC raw-first gate:

```bash
just verify-evm-foundation
```

This gate needs no mainnet credentials. It proves the shared durable capture
boundary and separate chain routing, then returns `promotion_verdict=hold`
until owned mainnet sources and operational fault evidence exist. See the
[EVM foundation local gate](docs/operations/evm-foundation-local-gate.md).

Start the local durable services and run the synthetic end-to-end path:

```bash
just infra-up
just verify-phase0
just infra-down
```

Validate all four native chain runtimes sequentially:

```bash
just validate-local
```

Run one bounded scope:

```bash
just validate-local bitcoin
just validate-local ethereum
just validate-local bsc
just validate-local solana
just validate-local platform
```

Validation artifacts are written under `artifacts/local-validation/` and are
ignored by Git. Every process and Compose resource is registered under a
task-specific identity before cleanup.

## Developer release

GitHub releases provide source archives and host-specific operator bundles.
The bundle contains the functional v0.1.0 executables:

- `bitcoin-core-connector`
- `native-normalizer`
- `query-api`
- `stream-gateway`
- `fixture-source`

It also contains configuration, schemas, licenses, a build manifest, and a
bounded smoke test. Other workspace binaries are scaffolding and are not
represented as operational release components.

To build and verify a bundle:

```bash
./scripts/release/build-release.sh v0.1.0
./scripts/release/smoke-release.sh \
  dist/multichain-rs-v0.1.0-$(rustc -Vv | awk '/^host:/ {print $2}').tar.gz \
  dist/multichain-rs-v0.1.0-$(rustc -Vv | awk '/^host:/ {print $2}').tar.gz.sha256
```

macOS bundles are unsigned developer artifacts, not Apple-notarized apps.
Operators must review configuration and network exposure before use.

## Repository map

- `connectors/` — Bitcoin Core, Reth/consensus, BSC, and Yellowstone adapters
- `crates/` — chain-native domains, envelopes, WAL, archive, storage ports
- `processors/` — canonicality, mempool, normalization, decoders, alerts
- `services/` — archive, fixture, query, and streaming services
- `schemas/` — Protobuf, ClickHouse, and PostgreSQL contracts
- `infra/` — local Compose topology and operational runbooks
- `tests/` — recorded fixtures, integration, fault, and end-to-end tests
- `docs/` — architecture decisions, data catalogs, and evidence

## Security and support

Do not report vulnerabilities in a public issue. Follow
[SECURITY.md](SECURITY.md) and send confidential reports to
[info@rsitech.ai](mailto:info@rsitech.ai).

General project questions may use GitHub issues or
[info@rsitech.ai](mailto:info@rsitech.ai). Contribution expectations are in
[CONTRIBUTING.md](CONTRIBUTING.md). Support boundaries are in
[SUPPORT.md](SUPPORT.md), and community participation follows the
[Code of Conduct](CODE_OF_CONDUCT.md).

## License

Copyright 2026 Rafal Sikora.

Maintained by [RSI Tech](https://rsitech.ai). Licensed under the
[Apache License 2.0](LICENSE). See [the licensing rationale](docs/licensing.md)
and [NOTICE](NOTICE).
