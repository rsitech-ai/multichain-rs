# Contributing

Multichain accepts focused issues and pull requests that preserve source
identity, replayability, and explicit correction semantics.

## Before opening a change

1. Check whether the behavior belongs to a chain-native domain, connector,
   processor, storage adapter, or serving contract.
2. Keep exact raw observations separate from interpretation.
3. Make gaps, retries, duplicate delivery, reorgs, and incomplete intervals
   explicit.
4. Avoid adding a dependency when a small existing abstraction is sufficient.
5. Never include credentials, private endpoints, private keys, wallet
   material, or production data.

## Verification

Run the full repository gate:

```bash
just check
```

Changes to source ingestion, canonicality, or durability should also include a
focused fault or integration test. Changes to native client validation should
run the affected bounded scope:

```bash
just validate-local <bitcoin|ethereum|bsc|solana|platform>
```

Document what the evidence proves and what remains external. A skipped live
dependency is not a passing production gate.

## Pull requests

Keep the diff reviewable and explain:

- the failure mode or capability being addressed;
- the state transition and rollback behavior;
- exact commands run;
- compatibility or schema impact; and
- any production validation that remains unavailable.

Generated Protobuf files and the compatibility baseline must stay synchronized.
Public contributions are submitted under the repository's Apache-2.0 license,
as described by section 5 of that license.

Project contact: [info@rsitech.ai](mailto:info@rsitech.ai)
