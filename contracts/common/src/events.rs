//! The org-wide event convention and the shared publishing helper.
//!
//! # Why the helper is thin
//!
//! `env.events().publish(...)` is **deprecated as of soroban-sdk 27** in favour
//! of the `#[contractevent]` attribute macro, which generates a typed struct
//! per event, derives its topics, and gives the SDK and downstream indexers a
//! real schema to work from. A shared helper cannot construct those typed
//! events: the macro *is* the emission mechanism, applied at each event's own
//! definition site.
//!
//! [`publish`] therefore only forwards a typed event to the host. It exists so
//! every contract emits through the same call site, which keeps the convention
//! visible at the point of use and leaves one place to add cross-cutting
//! emission rules later. The convention itself is normative — review will ask
//! for changes if an event does not follow it.
//!
//! # The convention
//!
//! Every event declares its fixed topics as `[contract, action]`, then marks
//! the address it is attributed to — and the position key, where one exists —
//! as `#[topic]` fields:
//!
//! ```ignore
//! use soroban_sdk::{contractevent, Address};
//!
//! #[contractevent(topics = ["escrow", "released"])]
//! #[derive(Clone, Debug, Eq, PartialEq)]
//! pub struct Released {
//!     #[topic]
//!     pub beneficiary: Address,
//!     pub amount: i128,
//!     pub released_by: Address,
//! }
//! ```
//!
//! which emits topics `("escrow", "released", beneficiary)` and a data map of
//! the remaining fields.
//!
//! Rules:
//!
//! - `contract` is the crate's short name: `escrow`, `stream`, `vesting`,
//!   `recurring`, `batch_payout`.
//! - `action` is a verb in the past tense: `created`, `funded`, `released`.
//! - Exactly one address is a `#[topic]` -- the party the event is most
//!   naturally attributed to, which is what an indexer filters on.
//! - Contracts holding keyed positions (`stream`, `vesting`) additionally mark
//!   the position id as a `#[topic]`, declared **before** the address.
//! - Soroban permits at most four topics, which the keyed shape uses fully.
//!   Do not add a third `#[topic]` field to a keyed event.
//! - Amounts are emitted as data, never as topics. They are not useful to
//!   index and they bloat the topic list.
//!
//! # Enforcement
//!
//! The fixed-topic rules are checked mechanically by
//! `tests/tests/event_topics.rs`, which scans every `contracts/<crate>/src`
//! tree and fails CI if a `#[contractevent]` does not declare exactly
//! `[<crate directory name>, <snake_case action>]`. A new contract crate is
//! covered automatically, so its directory name must be the short name used
//! as the first topic.

use soroban_sdk::{Env, Event};

/// Publishes one typed `#[contractevent]` through the shared call site.
///
/// `event` is a struct declared with `#[contractevent(topics = [contract,
/// action])]` and its attributed address marked `#[topic]`, as described in the
/// module docs. The macro generates the [`Event`] implementation, so this
/// function never has to know the event's field layout.
///
/// Prefer this over calling the generated `event.publish(env)` directly: it
/// keeps every contract on one emission path and makes an org-wide change to
/// event publication a one-line edit here.
pub fn publish<E: Event + ?Sized>(env: &Env, event: &E) {
    env.events().publish_event(event);
}
