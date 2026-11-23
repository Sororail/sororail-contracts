# Error codes

The canonical mapping of `sororail_common::errors::Error` to stable ABI
numbers. The variants, owning contracts and descriptions below are generated
from `contracts/common/src/errors.rs` — do not edit the table by hand.

Run `make docs-errors` after modifying the enum or its documentation.
`make check-error-docs` verifies that this file matches the generated output.

| Code | Variant | Owner | Meaning |
|------|---------|-------|---------|
| 1 | `AlreadyInitialized` | generic | `init` called on an instance that already holds config. |
| 2 | `NotInitialized` | generic | An entry point was called before `init`. |
| 3 | `Unauthorized` | generic | The caller is not permitted to perform this action. |
| 4 | `InvalidAmount` | generic | An amount was zero or negative where a positive value is required. |
| 5 | `InvalidTimeRange` | generic | A time range was empty or inverted (e.g. `stop <= start`). |
| 6 | `Overflow` | generic | Checked arithmetic exceeded `i128::MAX`. |
| 7 | `Underflow` | generic | Checked arithmetic fell below `i128::MIN`. |
| 8 | `DivisionByZero` | generic | Division by zero. |
| 9 | `InvalidBasisPoints` | generic | Basis points outside the inclusive range 0..=10000. |
| 10 | `InvalidState` | generic | The action is not legal from the current state. |
| 11 | `DeadlineNotReached` | generic | The deadline has not yet been reached. |
| 12 | `DeadlinePassed` | generic | The deadline has already passed. |
| 13 | `InsufficientBalance` | generic | Not enough balance to satisfy the request. |
| 14 | `InvalidDuration` | generic | A duration was zero where a positive span is required. |
| 15 | `IdenticalParties` | generic | The two counterparties of a transfer must differ. |
| 20 | `EscrowNotFundable` | escrow | `fund` called on an escrow that is not in `Created`. |
| 21 | `EscrowNotFunded` | escrow | `release` or `refund` called on an escrow that is not in `Funded`. |
| 22 | `EscrowClosed` | escrow | The escrow has already reached a terminal state. |
| 23 | `EscrowNoArbiter` | escrow | The action requires an arbiter and none was configured. |
| 24 | `EscrowNotDisputed` | escrow | `resolve` called on an escrow that is not in `Disputed`. |
| 25 | `EscrowAlreadyDisputed` | escrow | `dispute` called on an escrow that is already disputed. |
| 26 | `EscrowNotCancellable` | escrow | `cancel` called on an escrow that is not in `Created`. |
| 27 | `EscrowArbiterNotNeutral` | escrow | `escrow::init` requires a neutral arbiter distinct from both participants. |
| 28 | `EscrowNoDisputeTimeout` | escrow | `resolve_timeout` called on an escrow whose `init` set no `dispute_timeout`. |
| 29 | `EscrowDisputeTimeoutNotReached` | escrow | `resolve_timeout` called before `disputed_at + dispute_timeout`. |
| 30 | `EscrowBalanceMismatch` | escrow | `fund` received a net transfer that did not increase the escrow balance by the requested amount. |
| 40 | `StreamNotFound` | stream | Reserved for the future id-keyed stream design. |
| 41 | `StreamCancelled` | stream | The stream has already been cancelled. |
| 42 | `StreamNotCancellable` | stream | `cancel` called on a stream created with `cancellable = false`. |
| 43 | `StreamInsufficientAccrued` | stream | Withdrawal requested exceeds the accrued, unwithdrawn balance. |
| 44 | `StreamNotExtendable` | stream | `extend` given a `new_stop` that is not after the current stop. |
| 60 | `VestingNotFound` | vesting | Reserved for the future id-keyed vesting design. |
| 61 | `VestingCliffNotReached` | vesting | Nothing is claimable yet -- the cliff has not been reached. |
| 62 | `VestingNotRevocable` | vesting | `revoke` called on a grant created with `revocable = false`. |
| 63 | `VestingRevoked` | vesting | The grant has already been revoked. |
| 64 | `VestingCliffAfterEnd` | vesting | The cliff falls after the end of the vesting period. |
| 65 | `VestingNothingToClaim` | vesting | Nothing vested-but-unclaimed remains to claim. |
| 66 | `VestingScheduleOverflow` | vesting | `start + duration` would overflow `u64`. |
| 67 | `VestingTooManyCheckpoints` | vesting | A schedule would report more checkpoints than the documented cap. |
| 80 | `RecurringNotFound` | recurring | Reserved for the future id-keyed recurring design. |
| 81 | `RecurringCancelled` | recurring | The authorization has been cancelled. |
| 82 | `RecurringPeriodNotElapsed` | recurring | `charge` called before `next_chargeable_at` (including the first charge). |
| 83 | `RecurringExhausted` | recurring | `max_periods` has been exhausted. |
| 100 | `BatchEmpty` | batch_payout | The recipient list was empty. |
| 101 | `BatchTooLarge` | batch_payout | The recipient list exceeds the documented maximum. |
| 103 | `BatchRecipientIsContract` | batch_payout | A recipient's address is a smart-contract address. Returned when the caller sets `only_eoa = true` on `execute`, `execute_equal`, `execute_many_tokens`, or `preview` and any recipient is a contract. |

`102` (`BatchDuplicateRecipient`) was removed before any release and stays
burned rather than reused; see the comment at the end of `errors.rs`.
