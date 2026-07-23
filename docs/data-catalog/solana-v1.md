# Solana Native Facts v1

Solana identities are fork-qualified. An executed transaction key is
`(signature, slot, blockhash)`; signature alone is not a fact key.

`solana_transactions`, `solana_instructions`, `solana_logs`,
`solana_balance_changes`, `solana_token_balance_changes`, and
`solana_account_writes` are append-only `MergeTree` histories. Every row
retains source, observation, parser, revision, commitment, and fork context
where applicable. Current views use explicit `argMax(..., revision)`.

All transactions are S1 coverage. Account writes are only S2 selected-account
coverage and carry `coverage_tier = selected_accounts`; the schema and API do
not claim a full account firehose. Dead-fork account revisions remain in
history but are excluded from the current account projection.

Instruction bytes remain opaque native facts. SPL Token, Token-2022, and custom
program decoders append independent decoder revisions; unknown and failed
decodes retain raw bytes and never block native ingestion.
