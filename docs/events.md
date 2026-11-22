# Events reference

SoroRail contracts emit typed `#[contractevent]` structs. This is the
human-readable index of every event: the fixed topics an indexer filters on,
the address (or position) promoted to a topic, and the remaining data fields
carried in the event body.

The table below is checked against the pinned event list in
`tests/tests/event_topics.rs` by `scripts/check_event_docs.sh`, which CI runs.
If you add, rename or remove an event, update all three in the same PR.

## Topic scheme

Every event emits topics in this order:

```
[<contract>, <action>, <#[topic] fields in declaration order...>]
```

- `contract` is the crate's short name: `escrow`, `stream`, `vesting`,
  `recurring`, or `batch_payout`.
- `action` is a lower `snake_case` verb in the past tense.
- The `#[topic]` field(s) are the party the event is attributed to. Amounts are
  always data, never topics.
- The remaining fields form the data payload. A `-` in the table means the
  event has no data fields; an event with a single `#[topic]` and no data emits
  only its topics.

`contracts/common/src/events.rs` is the normative write-up of the convention;
`tests/tests/event_topics.rs` enforces the fixed-topic part mechanically.

## Table

| Contract | Event | Action topic | `#[topic]` field(s) | Data fields |
|---|---|---|---|---|
| `batch_payout` | `Executed` | `executed` | `funder` | `token`, `count`, `total`, `memo` |
| `batch_payout` | `ExecutedMany` | `executed_many` | `funder` | `count` |
| `escrow` | `Created` | `created` | `depositor` | `beneficiary`, `token`, `amount`, `deadline` |
| `escrow` | `Funded` | `funded` | `depositor` | `amount` |
| `escrow` | `Released` | `released` | `beneficiary` | `amount`, `released_by` |
| `escrow` | `Refunded` | `refunded` | `depositor` | `amount`, `refunded_by` |
| `escrow` | `Disputed` | `disputed` | `raised_by` | - |
| `escrow` | `Resolved` | `resolved` | `arbiter` | `split_bps`, `to_beneficiary`, `to_depositor` |
| `escrow` | `Cancelled` | `cancelled` | `depositor` | - |
| `stream` | `Created` | `created` | `sender` | `recipient`, `token`, `rate_per_second`, `start`, `stop`, `cancellable`, `deposited` |
| `stream` | `Withdrawn` | `withdrawn` | `recipient` | `amount`, `total_withdrawn` |
| `stream` | `Cancelled` | `cancelled` | `sender` | `settled_to_recipient`, `refunded_to_sender`, `cancelled_at` |
| `stream` | `ToppedUp` | `topped_up` | `sender` | `amount`, `new_stop`, `deposited` |
| `stream` | `ToppedUpFrom` | `topped_up_from` | `funder` | `sender`, `amount`, `new_stop`, `deposited` |
| `stream` | `Extended` | `extended` | `sender` | `added`, `old_stop`, `new_stop`, `deposited` |
| `vesting` | `Created` | `created` | `grantor` | `beneficiary`, `token`, `total`, `start`, `cliff`, `duration`, `revocable` |
| `vesting` | `BeneficiaryTransferred` | `beneficiary_transferred` | `from` | `to` |
| `vesting` | `Claimed` | `claimed` | `beneficiary` | `amount`, `total_claimed` |
| `vesting` | `Revoked` | `revoked` | `grantor` | `returned_to_grantor`, `still_claimable`, `revoked_at` |
| `recurring` | `Authorized` | `authorized` | `payer` | `payee`, `token`, `amount_per_period`, `period_seconds`, `max_periods`, `first_chargeable_at`, `valid_until` |
| `recurring` | `Charged` | `charged` | `payee` | `payer`, `amount`, `period_index`, `periods_charged`, `next_chargeable_at`, `remaining` |
| `recurring` | `Cancelled` | `cancelled` | `cancelled_by` | `periods_charged`, `cancelled_at` |
| `recurring` | `AmountUpdated` | `amount_updated` | - | `old_amount`, `new_amount`, `effective_from` |
| `recurring` | `Paused` | `paused` | `payer` | `paused_at` |
| `recurring` | `Resumed` | `resumed` | `payer` | `resumed_at`, `paused_seconds`, `next_chargeable_at` |

Every contract has at least one event; the five contracts account for 24
events in total.

`stream` `Created` carries `rate_per_second` and `deposited`, so an indexer can
rebuild the stream's economics from that event alone. `deposited` always equals
`rate_per_second * (stop - start)` at creation. The payload layout is pinned by
`stream_created_payload_lays_out_the_economics` in `tests/tests/event_topics.rs`.

## Notes for indexers

- **Escrow resolution.** There is no separate full-win event. `Resolved.split_bps`
  is the beneficiary's share: `0` is a full win for the depositor, `10000` a
  full win for the beneficiary. Treat the boundaries as the full-win cases.
- **Batch payout.** `Executed` covers `execute`, `execute_equal`,
  `execute_equal_split` and `execute_weighted`, and fires once per batch, not
  once per recipient. `memo` is the caller's optional 32-byte pay-run
  reference (`Option<BytesN<32>>`); it is data, not a topic, so the topics are
  the same whether it is `None` or set. `ExecutedMany` covers
  `execute_many_tokens`. Neither duplicates the per-recipient transfers: join
  against the token contract's `transfer` events in the same transaction to
  recover amounts. `ExecutedMany` has no `token` or `total` because a
  multi-token batch can touch incommensurable assets.
- **Recurring charges.** `Charged.period_index` is one-based;
  `periods_charged` is the cumulative count, and `remaining` is `null` when the
  authorization is uncapped.
- **Recurring pause and resume.** `Paused` and `Resumed` are attributed to the
  `payer`. Paused time shifts the schedule: `Resumed.next_chargeable_at` is the
  previous due date plus `paused_seconds`.
- **Recurring amount updates.** `AmountUpdated` is the one event with no
  `#[topic]` address; it is attributed only by its two fixed topics. Its new
  amount becomes effective at `effective_from`, not at the block that emitted
  the event.
- **Third-party stream top-ups.** `stream` `ToppedUpFrom` is attributed to the
  `funder`, who may be neither party to the stream. `sender` is data, not a
  topic, because the sender still receives any refund on cancellation. The
  sender's `ToppedUp` event is unchanged, so filter on `topped_up_from` to
  see third-party funding.
- **One position per instance.** Stream and vesting events carry no position-id
  topic because each deployed instance holds exactly one position (see
  [architecture.md](architecture.md)).
- **Event topics are ABI.** A changed action topic silently breaks every
  dashboard filtering on it. Only add rows; do not rename an existing action
  without a migration plan for downstream indexers.
