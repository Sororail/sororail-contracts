# sororail-stream

## Purpose

Stream is a continuous per-second transfer from sender to recipient. One stream
is stored per deployed instance and is fully funded up front for its whole
declared span. The recipient withdraws what has accrued; the sender may cancel,
if the stream was created cancellable, taking back only what has not yet
accrued.

There is no per-second state: accrual is a pure function of the stream record
and a ledger timestamp, evaluated at read time. The invariant is exact at every
point in the lifecycle, with no rounding leakage:

```text
withdrawn + refunded + remaining == deposited
```

## Entry points

| Entry point | Caller | Effect |
| --- | --- | --- |
| `create` | sender | Records the stream and pulls the full `deposited` amount in. |
| `preview_create` | anyone | Read-only estimate of the upfront deposit, using exactly the same validation and overflow checks as `create`; no token movement or state changes. |
| `withdraw` | recipient | Pays out accrued, unwithdrawn funds (all of them if `amount` is `None`). |
| `withdraw_to` | recipient | Same as `withdraw`, but pays `destination` (for example a treasury or exchange deposit address) in the same transaction. `destination` may not be the stream contract itself. |
| `cancel` | sender | Ends a cancellable stream; returns unaccrued funds to the sender. Worked examples: [docs/stream-cancel.md](../../docs/stream-cancel.md). |
| `top_up` | sender | Adds funds to the deposit. The amount must keep the per-second rate exact. |
| `top_up_from` | any funder | Adds funds on behalf of a third party, who authorizes the transfer. The same rate rule applies, and cancellation still refunds the sender. See the third-party top-ups section of [SECURITY.md](../../SECURITY.md). |
| `extend` | sender | Moves `stop` later and pulls the extra funds the longer span needs. |
| `balance_of`, `remaining` | anyone | Reads accrued-but-unwithdrawn or unstreamed balances. |
| `has_ended` | anyone | Whether accrual has reached `stop` as of the ledger time. Ignores cancellation. |
| `bump`, `get` | anyone | Extends TTL; reads the full stream record. |

## Example flow

```text
0. client.preview_create(&sender, &recipient, &token, &rate, &start, &stop, &true)
   -> returns the required deposit without authorizing, funding or storing a stream.
1. client.create(&sender, &recipient, &token, &rate, &start, &stop, &true)
   -> the contract pulls rate * (stop - start) from the sender.
2. Time passes. client.balance_of(&recipient) accrues with the ledger timestamp.
3. client.withdraw(&None) pays the recipient everything accrued so far.
4. The sender may call client.top_up(&amount) or client.extend(&new_stop) to
   keep the stream going, or client.cancel() to settle and refund the rest.
```

Copy-pasteable `stellar contract invoke` commands are in
[docs/CONTRACT_INVOKE.md](../../docs/CONTRACT_INVOKE.md), and the token movement
diagram is in [docs/fund-flows.md](../../docs/fund-flows.md).

## Errors

Failures use the shared `Error` enum in
[contracts/common/src/errors.rs](../common/src/errors.rs), re-exported as
`sororail_stream::Error`. Stream owns codes 40–59; the canonical table is
[docs/ERRORS.md](../../docs/ERRORS.md).

| Variant | Raised by |
| --- | --- |
| `AlreadyInitialized` | `create` or `preview_create` on an initialized instance |
| `NotInitialized` | any entry point before `create` |
| `IdenticalParties` | `create` or `preview_create` with `sender == recipient` |
| `IdenticalParties` | `create` with `token` set to the stream contract's own address |
| `InvalidAmount` | non-positive rate or withdrawal; `top_up` not a multiple of the rate |
| `IdenticalParties` | `withdraw_to` naming the stream contract itself as `destination` |
| `InvalidTimeRange` | `create` or `preview_create` with `stop <= start`; `top_up` overflowing `stop` |
| `Overflow` | funding or accrual exceeding `i128` |
| `StreamCancelled` | any mutation after cancellation |
| `StreamNotCancellable` | `cancel` on a stream created with `cancellable = false` |
| `StreamInsufficientAccrued` | `withdraw` or `withdraw_to` for more than has accrued |
| `StreamNotExtendable` | `extend` with `new_stop` not after the current stop |

Authorization failures are host errors, not typed variants: the privileged
entry points call `require_auth` directly on the one party allowed to invoke
them.
