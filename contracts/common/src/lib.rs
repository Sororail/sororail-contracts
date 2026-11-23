// Unsafe code is denied, not forbidden: soroban-sdk's `#[contractimpl]` emits
// an `#[allow(unsafe_code)]` constructor-registration shim whenever its
// `testutils` feature is enabled (unit tests, and any test crate depending on
// this one), and `forbid` cannot be overridden (E0453). `deny` still rejects
// any hand-written `unsafe`.
#![deny(unsafe_code)]
#![no_std]
#![warn(missing_docs)]
#![doc = r#"
Shared primitives for the SoroRail payment contracts.

This crate is not a contract. It holds the pieces that must be identical
across `escrow`, `stream`, `vesting`, `recurring` and `batch_payout`:

- [`errors`] -- one `Error` enum with a stable, documented numeric mapping
- [`storage`] -- TTL constants and bump helpers, because Soroban state expires
- [`auth`] -- authorization guards that check membership before `require_auth`
- [`events`] -- a single topic scheme every contract emits under
- [`math`] -- checked money math with explicit rounding and clamping
- [`time`] -- checked ledger time arithmetic without a network dependency
"#]

#[cfg(test)]
extern crate std;

pub mod auth;
// soroban-sdk's `#[contracterror]`/`#[contracttype]`/`#[contract]` emit public
// spec statics and helper functions without doc comments, which `missing_docs`
// (an error under the workspace's `warnings = "deny"`) rejects. The lint is
// relaxed only for the modules that hold such macros; every hand-written item
// in them is still documented.
#[allow(missing_docs)]
pub mod errors;
pub mod events;
pub mod math;
pub mod storage;
pub mod time;

pub use errors::Error;

// Available when the crate is used as a test dependency (feature = "testutils")
// or inside the crate's own test suite.
#[cfg(any(test, feature = "testutils"))]
#[allow(missing_docs)] // soroban macro output; see `errors` above
pub mod testutils;

#[cfg(any(test, feature = "testutils"))]
pub mod strategies;

#[cfg(test)]
mod test;
