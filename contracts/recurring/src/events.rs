//! Recurring events.
//!
//! Follows the org convention in `sororail_common::events`.

use soroban_sdk::{contractevent, Address};

/// A payer authorized a recurring pull.
#[contractevent(topics = ["recurring", "authorized"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Authorized {
    #[topic]
    pub payer: Address,
    pub payee: Address,
    pub token: Address,
    pub amount_per_period: i128,
    pub period_seconds: u64,
    pub max_periods: Option<u32>,
    pub first_chargeable_at: u64,
    pub valid_until: Option<u64>,
}

/// The payee pulled one period's payment.
#[contractevent(topics = ["recurring", "charged"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Charged {
    #[topic]
    pub payee: Address,
    pub payer: Address,
    pub amount: i128,
    /// One-based ordinal of this successful charge within the authorization.
    pub period_index: u32,
    /// How many charges have now been taken.
    pub periods_charged: u32,
    /// When the next charge becomes available.
    pub next_chargeable_at: u64,
    /// Charges still permitted under the authorization cap, if capped.
    pub remaining: Option<u32>,
}

/// Either party cancelled the authorization.
#[contractevent(topics = ["recurring", "cancelled"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cancelled {
    #[topic]
    pub cancelled_by: Address,
    pub periods_charged: u32,
    pub cancelled_at: u64,
}

/// The payer updated the amount per period.
#[contractevent(topics = ["recurring", "amount_updated"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AmountUpdated {
    pub old_amount: i128,
    pub new_amount: i128,
    /// The timestamp at which the new amount becomes effective.
    pub effective_from: u64,
}

/// The payer paused the authorization; no charge can be taken until `resume`.
#[contractevent(topics = ["recurring", "paused"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Paused {
    #[topic]
    pub payer: Address,
    pub paused_at: u64,
}

/// The payer resumed a paused authorization.
#[contractevent(topics = ["recurring", "resumed"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Resumed {
    #[topic]
    pub payer: Address,
    pub resumed_at: u64,
    /// How long the authorization was paused, in seconds.
    pub paused_seconds: u64,
    /// The schedule after shifting by `paused_seconds`.
    pub next_chargeable_at: u64,
}
