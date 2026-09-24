//! Vesting error surface.
//!
//! Errors are shared org-wide -- see `sororail_common::errors`, where vesting
//! owns the numeric range **60–79**. This module re-exports the enum and
//! documents which variants this contract can return:
//!
//! | Variant | Raised by |
//! |---|---|
//! | [`Error::AlreadyInitialized`] | `create` on an initialized instance |
//! | [`Error::NotInitialized`] | any entry point before `create` |
//! | [`Error::IdenticalParties`] | `create` with `grantor == beneficiary` |
//! | [`Error::InvalidAmount`] | `create` with a non-positive total |
//! | [`Error::InvalidDuration`] | `create` with a zero duration |
//! | [`Error::Overflow`] | proportional vesting math exceeding `i128` |
//! | [`Error::VestingCliffAfterEnd`] | `create` with `cliff > duration` |
//! | [`Error::VestingCliffNotReached`] | `claim` before `start + cliff` |
//! | [`Error::VestingNothingToClaim`] | `claim` with nothing vested-but-unclaimed |
//! | [`Error::VestingNotRevocable`] | `revoke` on a grant created with `revocable = false` |
//! | [`Error::VestingRevoked`] | `revoke` on an already-revoked grant |
//!
//! As with `stream`, `Unauthorized` does not appear: each privileged entry
//! point acts on one fixed party -- the beneficiary for `claim`, the grantor
//! for `revoke` -- so a wrong caller fails at `require_auth` rather than at a
//! membership check.

pub use sororail_common::Error;
