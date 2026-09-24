//! Escrow error surface.
//!
//! Errors are not defined per crate. Every SoroRail contract shares the single
//! [`Error`] enum in `sororail_common::errors`, which assigns each contract a
//! documented numeric range so that one integer never means two things across
//! the org. Escrow owns **20–39**.
//!
//! This module re-exports the shared enum so callers can write
//! `sororail_escrow::Error`, and documents which variants this contract can
//! actually return:
//!
//! | Variant | Raised by |
//! |---|---|
//! | [`Error::AlreadyInitialized`] | `init` on an initialized instance |
//! | [`Error::NotInitialized`] | any entry point before `init` |
//! | [`Error::Unauthorized`] | a caller outside the permitted set |
//! | [`Error::InvalidAmount`] | `init` with a non-positive amount |
//! | [`Error::InvalidTimeRange`] | `init` with a deadline at or before now |
//! | [`Error::InvalidBasisPoints`] | `resolve` with `split_bps > 10000` |
//! | [`Error::DeadlineNotReached`] | depositor `refund` before the deadline |
//! | [`Error::EscrowNotFundable`] | `fund` when not in `Created` |
//! | [`Error::EscrowNotFunded`] | `release`/`refund`/`dispute` when not in `Funded` |
//! | [`Error::EscrowClosed`] | any transition from a terminal state |
//! | [`Error::EscrowNoArbiter`] | `dispute` with no arbiter configured (`resolve` is unreachable — see invariant in `contract.rs`) |
//! | [`Error::EscrowNotDisputed`] | `resolve` when not in `Disputed` |
//! | [`Error::EscrowAlreadyDisputed`] | `dispute` when already disputed |

pub use sororail_common::Error;
