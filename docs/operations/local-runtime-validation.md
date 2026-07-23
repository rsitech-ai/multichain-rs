# Local runtime validation

Date: 2026-07-23

The local validation gate downloads checksum-pinned native chain clients,
starts disposable loopback runtimes, exercises real RPC and storage paths, and
writes structured evidence without requiring production credentials.

Run every scope sequentially:

```bash
just validate-local
```

Run one scope:

```bash
just validate-local bitcoin
just validate-local ethereum
just validate-local bsc
just validate-local solana
just validate-local platform
```

The latest aggregate pass is
`artifacts/local-validation/20260723T171946Z/summary.json`. The stable
`artifacts/local-validation/latest` symlink points to the most recent run.
Artifacts are intentionally ignored by Git.

## Proven local scope

| Gate | Runtime proof | Deliberately not claimed |
| --- | --- | --- |
| Bitcoin | Bitcoin Core 31.1; three source IDs and datadirs; distinct RPC, P2P, and ZMQ endpoints; observer-local mempool divergence; reconciliation; valid heavier-branch reorg; live connector test | Geographic or network-provider independence; mainnet propagation measurements |
| Ethereum | Reth 2.2.0 dev chain; funded signed transaction; successful receipt; block advancement; state retained across node restart; recorded execution/consensus canonicality tests | Owned mainnet execution plus consensus deployment; secondary-source reconciliation; mainnet finality |
| BNB Smart Chain | Official BSC 1.7.3 arm64 client; generated one-validator Parlia genesis; funded signed transaction; successful receipt; recorded chain-ID-56 native-finality tests | Local chain ID 56; a validator quorum; live BSC fast finality |
| Solana | Agave 4.1.2 local validator; healthy RPC; generated signer; airdrop; signed transfer; confirmed transaction and slot evidence; recorded fork and Yellowstone contracts | Mainnet-beta runtime; two independent Yellowstone providers; account firehose |
| Platform | Task-scoped Redpanda, ClickHouse, PostgreSQL, and MinIO; verified buckets; WAL/broker/archive round trip; raw replay; fact idempotence; durable checkpoints; REST/WebSocket proof; restart replay and gap repair | Replication factor three; multi-zone operation; production HA or capacity |

The aggregate evidence preserves exact client versions, archive checksums,
transaction identities, block or slot identities, image IDs, readiness
responses, and cleanup results.

## Isolation and cleanup

- Every native runtime is placed under a
  `mktemp` directory named `multichain-local-validation.*`.
- Every process is registered by PID before it can satisfy a gate.
- The platform uses a unique Compose project beginning
  `multichain-validation-`.
- Ports are checked before startup. An occupied port fails closed and reports
  the owner.
- Runtime cleanup targets only registered PIDs and the exact Compose project.
- Connector WALs, generated keys, and ledgers are removed after the evidence
  is written unless `--keep-runtime` is requested.
- No unrelated image, Docker cache, volume, network, container, or user file
  is pruned.

Each successful chain directory contains `cleanup.json`. The aggregate run
left all validation ports free and removed its platform containers, volumes,
and network.

## Supply-chain pins

| Component | Version | Verification |
| --- | --- | --- |
| Bitcoin Core | 31.1 | Official arm64 macOS archive and published SHA-256 |
| Reth | 2.2.0 | Official arm64 macOS release archive and published SHA-256 |
| BSC | 1.7.3 | Official arm64 macOS release binary and published SHA-256 |
| Agave | 4.1.2 | Official arm64 macOS release archive and published SHA-256 |
| Redpanda | 26.1.1 | Pinned Compose image reference plus runtime image ID |
| ClickHouse | 26.3 | Pinned Compose image reference plus runtime image ID |
| PostgreSQL | 18.3 | Pinned Compose image reference plus runtime image ID |
| MinIO | 2025-04-22 | Pinned Compose image reference plus runtime image ID |

Downloads fail on checksum mismatch. Container evidence records both the
configured reference and the exact local image ID used by the passing run.

## Resource and failure behavior

The runner requires at least 6 GiB free before starting each scope and executes
scopes sequentially. RPC readiness, receipt confirmation, transaction lookup,
and query API readiness are bounded. JSON-RPC error objects are rejected
explicitly rather than being treated as empty successful results.

Failures still write a per-scope result and cleanup record. The top-level
summary passes only when every selected scope reports `passed`.

This local gate is repeatable engineering evidence. It does not replace the
production source, topology, soak, latency, and incident-response gates in the
implementation specification.
