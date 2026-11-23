use soroban_sdk::{contracttype, Address};
use sororail_common::math;

/// A recurring pull authorization. One per deployed contract instance.
///
/// # How the pull works
///
/// This contract holds no funds. The payer authorizes it as a **spender** on
/// the token (`approve`), and `charge` then calls `transfer_from` to move
/// `amount_per_period` straight from the payer to the payee. The payer's
/// allowance is the real cap, and revoking that allowance on the token stops
/// charges even without touching this contract.
///
/// # Periods never accrue retroactively
///
/// `charge` sets `next_chargeable_at` to `now + period_seconds`, forfeiting
/// skipped periods. `charge_up_to` instead advances from the stored due date
/// for each elapsed period, with an explicit per-call limit. Neither entry
/// point allows a payment before the first full period after authorization.
///
/// This is a deliberate consumer-protection choice, not an oversight. It costs
/// the payee the revenue of periods they did not collect, and it guarantees
/// the payer can never be surprised by a bundled back-charge.
///
/// # The pull invariant
///
/// Because this contract holds no funds, there is nothing for it to conserve
/// internally; the invariant is on what it can move. Every successful `charge`
/// transfers exactly `amount_per_period` from the payer to the payee and
/// increments `periods_charged` by one, and a failed transfer reverts the
/// increment with it. So the total ever pulled is always exactly:
///
/// ```text
/// periods_charged * amount_per_period
/// ```
///
/// and, when `max_periods` is set, never more than `max_periods *
/// amount_per_period`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Authorization {
    /// The subscriber, whose funds are pulled.
    pub payer: Address,
    /// The merchant, who initiates each charge.
    pub payee: Address,
    /// The token being charged.
    pub token: Address,
    /// The most that may be pulled in any one period, in the token's smallest
    /// unit (e.g., stroops for XLM, or the base unit defined by the token's
    /// decimals).
    pub amount_per_period: i128,
    /// The cadence, in seconds. Each charge advances `next_chargeable_at` by
    /// this duration.
    pub period_seconds: u64,
    /// Cap on the number of charges, if any. `None` means no cap: charges may
    /// continue indefinitely as long as the authorization is not cancelled and
    /// the payer's token allowance remains sufficient.
    pub max_periods: Option<u32>,
    /// How many charges have been taken.
    pub periods_charged: u32,
    /// The earliest ledger timestamp at which the next charge may be taken, in
    /// seconds since the Unix epoch.
    pub next_chargeable_at: u64,
    /// Whether either party has cancelled.
    ///
    /// A plain flag rather than a `State` enum or a timestamp: the lifecycle
    /// is binary and nothing is computed from when it ended. See "Lifecycle
    /// modeling" in SPEC.md.
    pub cancelled: bool,
    /// The latest ledger timestamp at which the authorization may be charged,
    /// in seconds since the Unix epoch. `None` means no expiry.
    pub valid_until: Option<u64>,
    /// Optional cap on the amount per period. `None` means no cap.
    pub max_amount: Option<i128>,
    /// The new amount to take effect starting from the next period after a
    /// successful charge. When a charge completes, this becomes
    /// `amount_per_period` if set.
    pub pending_amount_per_period: Option<i128>,
    /// Cumulative amount pulled across every successful charge, in the token's
    /// smallest unit. Accumulated with `math::add`, so it always equals the sum
    /// of the per-charge amounts even when `update_amount` changed the amount
    /// part-way through.
    pub total_charged: i128,
    /// When the payer paused the authorization, if it is currently paused.
    /// `None` means active. While `Some`, no charge can be taken.
    pub paused_at: Option<u64>,
}

impl Authorization {
    /// Whether the cap on charges has been reached.
    pub fn is_exhausted(&self) -> bool {
        self.remaining_periods() == Some(0)
    }

    /// Whether the payer has paused charging.
    pub fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }

    /// Whether a charge may be taken at `at`.
    pub fn is_chargeable_at(&self, at: u64) -> bool {
        !self.cancelled
            && !self.is_paused()
            && !self.is_exhausted()
            && at >= self.next_chargeable_at
            && self.valid_until.map_or(true, |until| at < until)
    }

    /// Charges still permitted under the cap, if there is one.
    pub fn remaining_periods(&self) -> Option<u32> {
        math::remaining_count(self.max_periods, self.periods_charged)
    }
}

#[cfg(test)]
mod tests {
    use soroban_sdk::{testutils::Address as _, Address, Env};

    use super::Authorization;

    #[test]
    fn cap_checks_match_remaining_period_count() {
        let env = Env::default();
        let authorization = |max_periods, periods_charged| Authorization {
            payer: Address::generate(&env),
            payee: Address::generate(&env),
            token: Address::generate(&env),
            amount_per_period: 1,
            period_seconds: 1,
            max_periods,
            periods_charged,
            next_chargeable_at: 0,
            cancelled: false,
            valid_until: None,
            max_amount: None,
            pending_amount_per_period: None,
            total_charged: 0,
            paused_at: None,
        };

        let below = authorization(Some(3), 2);
        assert_eq!(below.remaining_periods(), Some(1));
        assert!(!below.is_exhausted());

        let at_cap = authorization(Some(3), 3);
        assert_eq!(at_cap.remaining_periods(), Some(0));
        assert!(at_cap.is_exhausted());

        let above_cap = authorization(Some(3), 4);
        assert_eq!(above_cap.remaining_periods(), Some(0));
        assert!(above_cap.is_exhausted());

        let uncapped = authorization(None, u32::MAX);
        assert_eq!(uncapped.remaining_periods(), None);
        assert!(!uncapped.is_exhausted());
    }

    #[test]
    fn is_exhausted_is_true_exactly_when_periods_charged_equals_the_cap() {
        let env = Env::default();
        let authorization = |max_periods, periods_charged| Authorization {
            payer: Address::generate(&env),
            payee: Address::generate(&env),
            token: Address::generate(&env),
            amount_per_period: 1,
            period_seconds: 1,
            max_periods,
            periods_charged,
            next_chargeable_at: 0,
            cancelled: false,
            valid_until: None,
            max_amount: None,
            pending_amount_per_period: None,
            total_charged: 0,
            paused_at: None,
        };

        // Not exhausted when periods_charged < max_periods
        let before_cap = authorization(Some(5), 4);
        assert!(!before_cap.is_exhausted());

        // Exhausted exactly when periods_charged == max_periods
        let at_cap = authorization(Some(5), 5);
        assert!(at_cap.is_exhausted());

        // Exhausted when periods_charged > max_periods (should not happen in practice)
        let over_cap = authorization(Some(5), 6);
        assert!(over_cap.is_exhausted());

        // Not exhausted when uncapped (None)
        let uncapped = authorization(None, 100);
        assert!(!uncapped.is_exhausted());
    }

    #[test]
    fn is_chargeable_at_is_false_before_the_first_period() {
        let env = Env::default();
        let first_chargeable_at = 1000u64;
        let authorization = Authorization {
            payer: Address::generate(&env),
            payee: Address::generate(&env),
            token: Address::generate(&env),
            amount_per_period: 100,
            period_seconds: 100,
            max_periods: None,
            periods_charged: 0,
            next_chargeable_at: first_chargeable_at,
            cancelled: false,
            valid_until: None,
            max_amount: None,
            pending_amount_per_period: None,
            total_charged: 0,
            paused_at: None,
        };

        // Before next_chargeable_at: not chargeable
        assert!(!authorization.is_chargeable_at(first_chargeable_at - 1));
        assert!(!authorization.is_chargeable_at(0));

        // At next_chargeable_at: chargeable
        assert!(authorization.is_chargeable_at(first_chargeable_at));

        // After next_chargeable_at: chargeable
        assert!(authorization.is_chargeable_at(first_chargeable_at + 1));
    }

    #[test]
    fn is_chargeable_at_respects_valid_until() {
        let env = Env::default();
        let valid_until = 2000u64;
        let next_chargeable_at = 1000u64;
        let authorization = Authorization {
            payer: Address::generate(&env),
            payee: Address::generate(&env),
            token: Address::generate(&env),
            amount_per_period: 100,
            period_seconds: 100,
            max_periods: None,
            periods_charged: 0,
            next_chargeable_at,
            cancelled: false,
            valid_until: Some(valid_until),
            max_amount: None,
            pending_amount_per_period: None,
            total_charged: 0,
            paused_at: None,
        };

        // Before next_chargeable_at: not chargeable
        assert!(!authorization.is_chargeable_at(next_chargeable_at - 1));

        // At next_chargeable_at and before valid_until: chargeable
        assert!(authorization.is_chargeable_at(next_chargeable_at));
        assert!(authorization.is_chargeable_at(valid_until - 1));

        // At or after valid_until: not chargeable
        assert!(!authorization.is_chargeable_at(valid_until));
        assert!(!authorization.is_chargeable_at(valid_until + 1));
    }

    #[test]
    fn is_chargeable_at_allows_no_expiry() {
        let env = Env::default();
        let next_chargeable_at = 1000u64;
        let authorization = Authorization {
            payer: Address::generate(&env),
            payee: Address::generate(&env),
            token: Address::generate(&env),
            amount_per_period: 100,
            period_seconds: 100,
            max_periods: None,
            periods_charged: 0,
            next_chargeable_at,
            cancelled: false,
            valid_until: None,
            max_amount: None,
            pending_amount_per_period: None,
            total_charged: 0,
            paused_at: None,
        };

        assert!(authorization.is_chargeable_at(next_chargeable_at));
        assert!(authorization.is_chargeable_at(next_chargeable_at + 1_000_000));
        assert!(authorization.is_chargeable_at(u64::MAX));
    }
}
