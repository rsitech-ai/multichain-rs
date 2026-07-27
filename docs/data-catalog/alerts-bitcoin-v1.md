# Bitcoin alert facts v1

## `bitcoin_mempool_quorum_vbytes_above`

This deterministic alert evaluates aggregate virtual bytes at or above one
integer sat/vB fee-band floor. It consumes a sequence of source-qualified
quorum snapshots; it does not infer an authoritative global Bitcoin mempool.

### Definition

| Field | Type | Meaning |
| --- | --- | --- |
| `alert_id` | ASCII string | Stable definition identity |
| `min_fee_rate_sat_vb` | `u64` | Inclusive fee-band floor |
| `threshold_vbytes` | positive `u64` | Strict trigger threshold |
| `quorum_required` | positive `u16` | Minimum healthy observers |
| `for_evaluations` | positive `u16` | Consecutive true revisions required |
| `cooldown_seconds` | bounded `u64` | Minimum interval between trigger deliveries |
| `degraded_policy` | enum | `suppress` or `evaluate_healthy_quorum` |

### Input evidence

Every revision retains:

- Bitcoin network and monotonic revision;
- supplied evaluation time;
- fee band, vbytes, and quorum;
- sorted healthy and unavailable source IDs;
- `complete`, `known_incomplete`, or `recovered` coverage; and
- `observed`, `recovered`, or `correction` provenance.

The evaluator rejects fee-band or quorum mismatches, revision regression,
conflicting reuse of one revision, source-set overlap, contradictory complete
claims, and evaluation-time regression.

### Transitions

| Transition | Active afterward | Delivery |
| --- | ---: | ---: |
| `pending` | no | no |
| `triggered` | yes | yes |
| `confirmed` | yes | no |
| `corrected` | no | yes |
| `retracted` | no | yes |
| `degraded_source` | unchanged | no |
| `below_threshold` | no | no |
| `cooldown_suppressed` | no | no |
| `duplicate_revision` | unchanged | no |

Insufficient healthy quorum always produces `degraded_source`. With
`degraded_policy=suppress`, any named unavailable source or
`known_incomplete` coverage also suppresses evaluation. A degraded revision
never retracts an active alert because absence is not proven.

Evaluation IDs hash the normalized definition/evidence transition. Trigger,
correction, and retraction deliveries receive separate deterministic outbox
idempotency keys. Durable PostgreSQL definitions and transactional outbox
delivery are not implemented in this preview slice.

### HTTP preview

`POST /v1/alerts/preview` accepts one definition and 1–1,000 ordered snapshots.
It returns every evaluation and `final_active`. The route performs no storage
mutation and returns:

```json
{"error":"invalid_alert_preview"}
```

with HTTP 400 for malformed or inconsistent input. Persistent alert creation,
pause, notification delivery, and organization authorization remain deferred
until their control-plane contracts are implemented.
