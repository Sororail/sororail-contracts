//! Stream error surface.
//!
//! Errors are shared org-wide -- see `sororail_common::errors`, where stream
//! owns the numeric range **40–59**. This module re-exports the enum and
//! documents which variants this contract can return:
//!
//! | Variant | Raised by |
//! |---|---|
//! | [`Error::AlreadyInitialized`] | `create` on an initialized instance |
//! | [`Error::NotInitialized`] | any entry point before `create` |
//! | [`Error::IdenticalParties`] | `create` with `sender == recipient` |
//! | [`Error::InvalidAmount`] | non-positive rate or withdrawal; `top_up` not a multiple of the rate |
//! | [`Error::InvalidTimeRange`] | `create` with `stop <= start`; `top_up` overflowing `stop` |
//! | [`Error::Overflow`] | funding or accrual exceeding `i128` |
//! | [`Error::StreamCancelled`] | any mutation after cancellation |
//! | [`Error::StreamNotCancellable`] | `cancel` on a stream created with `cancellable = false` |
//! | [`Error::StreamInsufficientAccrued`] | `withdraw` for more than has accrued |
//! | [`Error::StreamNotExtendable`] | `extend` with `new_stop` not after the current stop |
//!
//! Note that `Unauthorized` does not appear. Stream's privileged entry points
//! act on a single fixed party each -- the sender for `cancel`, `top_up` and
//! `extend`, the recipient for `withdraw` -- so they call `require_auth`
//! directly on that party rather than checking a caller argument against an
//! allow-list. A wrong caller fails at authorization, not at a membership
//! check.

pub use sororail_common::Error;
