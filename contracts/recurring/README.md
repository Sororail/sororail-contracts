# sororail-recurring

## Purpose

Recurring is pull-based authorization for subscriptions. The payer authorizes a
cap and a cadence; the payee pulls within it. One authorization is stored per
deployed instance.

The contract never holds funds. The payer grants it a token allowance with
`approve`, and each `charge` moves money directly from payer to payee via
`transfer_from`. Revoking that allowance stops charges immediately, with or
without cancelling here.

`charge` pulls one payment and schedules the next at `now + period_seconds`, so
skipped periods are forfeited by default. `charge_up_to(max_periods)` is an
explicit catch-up option: it pulls only periods whose scheduled timestamps have
elapsed, up to the caller's limit and the authorization's remaining cap. The
first payment is never available until one full period after authorization.

## Entry points

| Entry point | Caller | Effect |
| --- | --- | --- |
| `authorize` | payer | Records the cap and cadence. Requires payer auth. |
| `charge` | payee | Pulls one period's amount from payer to payee via `transfer_from`. |
| `charge_up_to` | payee | Pulls up to the requested number of elapsed periods. |
| `pause`, `resume` | payer | Pauses charging; `charge` fails with `RecurringPaused`. Paused time shifts the schedule later by the length of the pause. |
| `cancel` | payer or payee | Stops future charges. Revoking the token allowance also stops them. |
| `next_chargeable_at`, `is_chargeable`, `remaining_periods`, `charges_remaining`, `charges_made`, `total_charged` | anyone | Reads the schedule, what is still allowed, how many charges were taken and the cumulative amount charged. |
| `bump`, `get` | anyone | Extends TTL; reads the full authorization record. |

## Example flow

```text
1. The payer approves the recurring contract address on the token for
   amount_per_period * number_of_periods. This must happen first:
   `authorize` returns RecurringInsufficientAllowance if the allowance is
   below the cap (or one period when uncapped). The allowance can still
   change later, so the check is a fail-fast, not a guarantee.
2. client.authorize(&payer, &payee, &token, &amount_per_period,
                    &period_seconds, &Some(max_periods))
3. Once per period, client.charge() pulls one payment. Before the next due
   time it returns RecurringPeriodNotElapsed; after max_periods it returns
   RecurringExhausted. If periods were skipped, client.charge_up_to(&n) can
   pull up to `n` of the elapsed periods, subject to the remaining cap.
4. Either party may call client.cancel(&caller, &caller). The payer can also
   revoke the token allowance directly, which stops charges immediately.
```

Full CLI commands are in
[docs/CONTRACT_INVOKE.md](../../docs/CONTRACT_INVOKE.md), and the token movement
diagram is in [docs/fund-flows.md](../../docs/fund-flows.md).

## Errors

Failures use the shared `Error` enum in
[contracts/common/src/errors.rs](../common/src/errors.rs), re-exported as
`sororail_recurring::Error`. Recurring owns codes 80–99; the canonical table is
[docs/ERRORS.md](../../docs/ERRORS.md).

| Variant | Raised by |
| --- | --- |
| `AlreadyInitialized` | `authorize` on an initialized instance |
| `NotInitialized` | any entry point before `authorize` |
| `IdenticalParties` | `authorize` with `payer == payee` |
| `Unauthorized` | `cancel` by someone who is neither party |
| `InvalidAmount` | non-positive `amount_per_period`, zero `max_periods` at authorization, or zero `max_periods` passed to `charge_up_to` |
| `InvalidDuration` | zero `period_seconds` |
| `InvalidTimeRange` | `authorize`, `charge`, or `charge_up_to` scheduling the next charge past `u64::MAX` |
| `Overflow` | `periods_charged` exceeding `u32` |
| `RecurringCancelled` | `charge`, `charge_up_to`, or `cancel` after cancellation |
| `RecurringPeriodNotElapsed` | `charge` or `charge_up_to` before `next_chargeable_at` |
| `RecurringExhausted` | `charge` or `charge_up_to` once `max_periods` is reached |
