//! Recurring error surface.
//!
//! Errors are shared org-wide -- see `sororail_common::errors`, where recurring
//! owns the numeric range **80–99**. This module re-exports the enum and
//! documents which variants this contract can return:
//!
//! | Variant | Raised by |
//! |---|---|
//! | [`Error::AlreadyInitialized`] | `authorize` on an initialized instance |
//! | [`Error::NotInitialized`] | any entry point before `authorize` |
//! | [`Error::IdenticalParties`] | `authorize` with `payer == payee` |
//! | [`Error::Unauthorized`] | `cancel` by someone who is neither party |
//! | [`Error::InvalidAmount`] | non-positive `amount_per_period`, zero `max_periods` at authorization, or zero `max_periods` passed to `charge_up_to` |
//! | [`Error::InvalidDuration`] | zero `period_seconds` |
//! | [`Error::InvalidTimeRange`] | `authorize`, `charge`, or `charge_up_to` scheduling the next charge past `u64::MAX` |
//! | [`Error::Overflow`] | `periods_charged` exceeding `u32`, or `total_charged` exceeding `i128` |
//! | [`Error::RecurringCancelled`] | `charge`, `charge_up_to`, `pause`, `resume` or `cancel` after cancellation |
//! | [`Error::RecurringPeriodNotElapsed`] | `charge` or `charge_up_to` before `next_chargeable_at` |
//! | [`Error::RecurringExhausted`] | `charge` or `charge_up_to` once `max_periods` is reached |
//! | [`Error::RecurringPaused`] | `charge` or `charge_up_to` while paused, or `pause` when already paused |
//! | [`Error::RecurringNotPaused`] | `resume` when the authorization is not paused |
//! | [`Error::RecurringInsufficientAllowance`] | `authorize` when the payer's allowance to this contract is below the cap or one period amount |
//!
//! # What is absent, and why
//!
//! `Unauthorized` **does** appear here, unlike in `stream` and `vesting`.
//! `cancel` may be called by either of two parties, so it takes a `caller`
//! argument and checks it against the `{payer, payee}` allow-list before
//! `require_auth`; a third party fails that membership check. `charge` acts on
//! a single fixed party (the payee) and so calls `require_auth` directly --
//! a wrong caller there fails at authorization, not with `Unauthorized`.
//!
//! The following shared variants are never returned by this contract:
//!
//! - `RecurringNotFound` (80) -- each instance holds exactly one
//!   authorization, so a missing record is reported as `NotInitialized`. The
//!   number is reserved for an id-keyed design (see "Open design question" in
//!   SPEC.md).
//! - `InsufficientBalance` -- the contract holds no funds. A payer whose
//!   balance is too small, or whose allowance shrinks after `authorize`, makes
//!   the token's `transfer_from` fail with the token's own error, which aborts
//!   the whole `charge`. Only the allowance at `authorize` time is checked
//!   up front, with `RecurringInsufficientAllowance`.
//! - `DeadlineNotReached` / `DeadlinePassed` -- there is no deadline; timing is
//!   expressed as `next_chargeable_at` and reported as
//!   `RecurringPeriodNotElapsed`.
//! - `InvalidState` -- the lifecycle is a single `cancelled` flag plus the
//!   `max_periods` cap, each with its own specific variant
//!   (`RecurringCancelled`, `RecurringExhausted`).
//! - `Underflow`, `DivisionByZero`, `InvalidBasisPoints` -- no subtraction,
//!   division or basis-point math is performed on balances.

pub use sororail_common::Error;
