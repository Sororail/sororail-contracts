//! TTL constants and bump helpers.
//!
//! Soroban state expires. An entry that is not extended is archived, and a
//! contract whose config has been archived cannot run. Every entry point that
//! touches storage must therefore extend the TTL of what it touched, so that
//! logic lives here and is written once rather than forgotten in one contract
//! out of six.
//!
//! # Persistent storage (reserved)
//!
//! Today every contract stores its single position in **instance** storage, so
//! only [`extend_instance`] is used. Persistent TTL helpers
//! (`PERSISTENT_BUMP` / `extend_persistent`) are deliberately **not** shipped
//! here yet: they are reserved for the future id-keyed multi-position design
//! discussed in SPEC.md. Reintroduce them with that change rather than leaving
//! unused constants that drift from any real caller.

use soroban_sdk::Env;

/// Ledgers closed in a day, at the ~5s close time Soroban targets.
pub const DAY_IN_LEDGERS: u32 = 17_280;

/// Instance entries (contract config / the single position) are extended to
/// 30 days...
pub const INSTANCE_BUMP: u32 = 30 * DAY_IN_LEDGERS;
/// ...whenever they fall below 29 days remaining.
pub const INSTANCE_THRESHOLD: u32 = INSTANCE_BUMP - DAY_IN_LEDGERS;

/// Extends the TTL of the contract instance and its code.
pub fn extend_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_THRESHOLD, INSTANCE_BUMP);
}
