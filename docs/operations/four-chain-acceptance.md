# Four-chain acceptance evidence

Date: 2026-07-23

This document separates repository-local proof from infrastructure-dependent
production validation. A missing live node, endpoint, or credential is
`blocked:external`, never a silent skip or a repository pass.

## Repository-local acceptance

| Network | Proven locally | Primary evidence |
| --- | --- | --- |
| Bitcoin mainnet model | Native parsing, proof-of-work/network binding, block DAG, ordered apply/revert, exact UTXO restoration, observer-local mempools, RBF/CPFP relationships, resumable backfill, facts, source-qualified APIs | `cargo test -p bitcoin-domain -p bitcoin-canonicality -p bitcoin-mempool -p bitcoin-core-connector -p integration-tests` |
| Ethereum mainnet | Chain ID 1 domain isolation, exact recorded Reth and consensus evidence, payload-hash join, ordered reorg revisions, safe/finalized ancestry, duplicate replay, critical finalized reversal, facts and decoder isolation | `cargo test -p evm-domain -p ethereum-reth-connector -p ethereum-consensus-connector -p evm-canonicality -p evm-decoder && cargo test -p integration-tests --test ethereum_recorded` |
| BNB Smart Chain mainnet | Chain ID 56 domain isolation, official-client configuration, recorded head plus native finalized tag, independent fast-finality state, stall detection, regression/nonancestor/finalized-reorg rejection, facts | `cargo test -p evm-domain -p bsc-connector -p evm-canonicality && cargo test -p integration-tests --test bsc_recorded` |
| Solana mainnet-beta | Fork-qualified signature identity, v0 messages, ALT/CPI/log/balance representation, exact Yellowstone protobuf capture, two-provider enforcement, gap/divergence evidence, reversible selected-account projection, facts, decoder revisions, coverage-qualified APIs | `cargo test -p solana-domain -p solana-yellowstone-connector -p solana-canonicality -p solana-decoder -p native-normalizer -p api-contract` |
| All four | Two independent executions produce identical logical fact and state hashes for every chain | `cargo test -p integration-tests --test four_chain_replay` |

The deterministic replay test uses a mined, proof-of-work-valid regtest chain
for Bitcoin state. The immutable parser corpus deliberately does not assert
proof of work and contains synthetic duplicate coinbase identities, so it is
not misrepresented as a canonical UTXO chain.

## Fault and correction coverage

| Failure or correction | Expected behavior | Evidence |
| --- | --- | --- |
| WAL/process restart and broker redelivery | Replay resumes without losing the exact observation or fabricating exactly-once semantics | `cargo test -p fault-tests --test phase0_restart_replay` |
| Broker or archive unavailable | WAL is retained until broker and verified archive coverage exist | `cargo test -p integration-tests --test wal_broker_archive` with `MULTICHAIN_REQUIRE_INFRA=1` |
| Bitcoin reorg | Old UTXOs revert before the heavier branch applies; resulting state hash is exact | `cargo test -p bitcoin-canonicality --test reorg --test apply_revert` |
| Bitcoin ZMQ sequence gap | Source-local membership becomes incomplete until RPC reconstruction closes the exact interval | `cargo test -p bitcoin-core-connector -p bitcoin-mempool` |
| Ethereum reorg/finality conflict | Reorg revisions are ordered; a finalized reversal fails as a critical error | `cargo test -p evm-canonicality --test ethereum` |
| BSC finalized regression or nonancestor | Observation is rejected atomically; finalized-tag stalls are explicit health state | `cargo test -p evm-canonicality --test bsc` |
| Yellowstone malformed/empty/oversized frame | Frame is rejected before interpretation | `cargo test -p solana-yellowstone-connector --test capture` |
| Yellowstone cursor gap or provider disagreement | Gap and divergence remain source-qualified and explicit | `cargo test -p solana-yellowstone-connector --test capture --test divergence` |
| Solana dead fork/account rollback | Selected-account writes roll back exactly; finalized descendants prevent ancestor death | `cargo test -p solana-canonicality --test accounts --test forks` |
| Unknown or failed Solana program decode | Raw instruction bytes remain replayable in an append-only unknown/failed revision | `cargo test -p solana-decoder --test registry` |

## Mainnet evidence already captured

- Ethereum and BSC immutable genesis semantic fixtures were captured from
  public mainnet JSON-RPC on 2026-07-23 and checksum-pinned in
  `tests/fixtures/evm/manifest.json`.
- The immutable Solana v0 transaction semantic fixture was captured from a
  finalized public mainnet-beta RPC response on 2026-07-23 and checksum-pinned
  in `tests/fixtures/solana/manifest.json`.
- Recorded Reth, consensus-checkpoint, and BSC native-finality fixtures exercise
  the owned-node adapter semantics without claiming a live owned deployment.

These fixtures prove protocol and replay compatibility at their documented
semantic scope. They do not prove continuous source availability, geographic
independence, production latency, or historical completeness.

## External live-validation matrix

The local capability audit on 2026-07-23 found none of `bitcoind`,
`bitcoin-cli`, `reth`, `geth`, `bsc`, or `solana-test-validator` on `PATH`, and
no environment-variable names prefixed with `BITCOIN_`, `BTC_`, `RETH_`,
`ETH_`, `BSC_`, `SOLANA_`, or `YELLOWSTONE_`.

| Network | Live production gate | Current status |
| --- | --- | --- |
| Bitcoin | Three independently located Bitcoin Core observers; ZMQ plus RPC sequence reconciliation; controlled disconnect/reconnect and reorg | `blocked:external` — nodes and RPC/ZMQ credentials are not provisioned |
| Ethereum | Owned Reth plus consensus client; controlled commit/reorg/revert; secondary head/finality reconciliation | `blocked:external` — execution/consensus nodes are not provisioned |
| BNB Smart Chain | Official `bnb-chain/bsc` node; native finalized-tag advancement/stall exercise | `blocked:external` — official node is not provisioned |
| Solana | Two independent Yellowstone providers; reconnect/gap/reconstruction exercise; selected-account comparison | `blocked:external` — endpoints and token environment variables are not provisioned |

Archive-state Ethereum queries, BSC archive operation, an owned Solana
validator, and a full Solana account firehose are explicitly outside the
initial acceptance scope.

## Full repository gate

Result on 2026-07-23: **passed**.

Command:

```text
just check
```

This checks formatting, workspace-wide Clippy with warnings denied, all
workspace tests, protobuf lint and compatibility, dependency policy,
secret scanning, and the production Compose configuration.

`cargo deny` reported duplicate-version warnings but completed with
advisories, bans, licenses, and sources all passing. The separately documented
temporary `paste` advisory exception remains narrow and unchanged.
