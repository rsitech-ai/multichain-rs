# Bitcoin ZMQ Gap Recovery

## Trigger

Treat any discontinuity in a topic-local ZMQ transport sequence, any
discontinuity in the `sequence` topic's mempool sequence, or a subscriber
reconnect as a known source gap. Bitcoin Core ZMQ is a notification surface,
not a durable queue.

## Immediate response

1. Mark only the affected observer/session `gapped`; do not degrade the other
   observers.
2. Open a `known_incomplete` coverage interval with the first detected time and
   the topic/sequence evidence.
3. Keep capturing exact live frames to the local WAL. Do not wait for
   transaction or block decoding.
4. For mempool gaps, call `getrawmempool false true`, persist the exact snapshot
   result as `recovered_by_rpc`, replace that observer's current projection,
   and wait for a subsequent live `sequence` transition before returning health
   to `healthy`.
5. For block gaps, call `getbestblockhash`, then walk `getblock HASH 0` parent
   links back to the last durable ancestor. Persist every recovered raw block
   with RPC recovery provenance.

State convergence does not prove transition-history recovery. Keep the closed
interval `known_incomplete` unless another durable observer supplies every
missing transition.

## Irrecoverable transient drill

On an isolated regtest node:

1. Start the connector and record its current session and mempool sequence.
2. Disconnect the `sequence` subscriber.
3. Submit a valid transaction, then remove it by mining it, replacing it, or
   restarting regtest with an empty mempool before reconnecting.
4. Reconnect and obtain `getrawmempool false true`.
5. Verify the connector returns to `healthy` after live-sequence alignment and
   its current mempool equals Core.
6. Verify one closed `known_incomplete` interval remains and that no synthetic
   add/remove observation was created for the unseen transaction.

## Verification

```bash
cargo test -p bitcoin-core-connector --test mempool_gap -- --nocapture
BITCOIN_REGTEST_RPC_URL=http://127.0.0.1:18443 \
BITCOIN_REGTEST_COOKIE=/absolute/path/to/regtest/.cookie \
cargo test -p integration-tests --test bitcoin_connector_regtest -- --nocapture
```

If the live environment variables are absent, the integration test reports a
skip; the deterministic state-machine test remains mandatory.
