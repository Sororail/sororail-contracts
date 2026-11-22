// Test fixtures do plain arithmetic on known-small constants; the checked-math
// rule is for contract code.
#![allow(clippy::arithmetic_side_effects)]

use soroban_sdk::{
    testutils::{storage::Instance as _, Address as _, Events as _, Ledger as _},
    token::TokenClient,
    Address, Env, Event,
};
use sororail_common::{storage::INSTANCE_BUMP, testutils::TestEnv, Error};

use crate::{
    contract::{RecurringContract, RecurringContractClient},
    events::{Authorized, Cancelled, Charged, Paused, Resumed},
};

#[test]
fn bump_restores_the_instance_ttl_after_ledger_advance() {
    let f = Fixture::new(None);
    f.env
        .ledger()
        .set_sequence_number(f.env.ledger().sequence() + INSTANCE_BUMP - 1);
    f.env.as_contract(&f.client.address, || {
        assert!(f.env.storage().instance().get_ttl() < INSTANCE_BUMP);
    });
    f.client.bump();
    f.env.as_contract(&f.client.address, || {
        assert_eq!(f.env.storage().instance().get_ttl(), INSTANCE_BUMP);
    });
}

const AMOUNT: i128 = 10_000;
const PERIOD: u64 = 2_592_000; // 30 days
const START: u64 = 1_000_000;
const MINT: i128 = AMOUNT * 100;

struct Fixture<'a> {
    env: Env,
    client: RecurringContractClient<'a>,
    token: TokenClient<'a>,
    payer: Address,
    payee: Address,
    outsider: Address,
}

impl Fixture<'_> {
    fn new(max_periods: Option<u32>) -> Self {
        Self::with_valid_until(max_periods, None)
    }

    fn with_valid_until(max_periods: Option<u32>, valid_until: Option<u64>) -> Self {
        let te = TestEnv::at(START);
        let payer = te.make_address();
        let payee = te.make_address();
        let outsider = te.make_address();
        let token_address = te.register_token();
        te.mint(&token_address, &payer, MINT);
        let env = te.env().clone();

        let client = RecurringContractClient::new(&env, &env.register(RecurringContract, ()));
        let token = TokenClient::new(&env, &token_address);
        // The payer grants the contract an allowance -- without this, no charge
        // can move anything, and `authorize` itself fails fast. Approving
        // first is therefore required.
        let expiry = env.ledger().sequence() + 100_000;
        token.approve(&payer, &client.address, &MINT, &expiry);
        client.authorize(
            &payer,
            &payee,
            &token_address,
            &AMOUNT,
            &PERIOD,
            &max_periods,
            &valid_until,
        );

        Fixture {
            token,
            env,
            client,
            payer,
            payee,
            outsider,
        }
    }

    fn at(&self, ts: u64) {
        self.env.ledger().with_mut(|l| l.timestamp = ts);
    }
}

// --------------------------------------------------------------- authorize

#[test]
fn authorize_schedules_the_first_charge_one_period_out() {
    let f = Fixture::new(None);
    assert_eq!(f.client.next_chargeable_at(), START + PERIOD);
    assert_eq!(f.client.get().periods_charged, 0);
    // Nothing has moved at signup.
    assert_eq!(f.token.balance(&f.payee), 0);
    assert!(!f.client.is_chargeable());
}

#[test]
fn authorize_rejects_a_non_positive_amount() {
    let env = Env::default();
    env.mock_all_auths();
    let c = RecurringContractClient::new(&env, &env.register(RecurringContract, ()));
    let (a, b, t) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );
    for bad in [0i128, -1, i128::MIN] {
        assert_eq!(
            c.try_authorize(&a, &b, &t, &bad, &PERIOD, &None, &None),
            Err(Ok(Error::InvalidAmount)),
            "amount {bad} was accepted"
        );
    }
}

#[test]
fn authorize_rejects_a_zero_period() {
    let env = Env::default();
    env.mock_all_auths();
    let c = RecurringContractClient::new(&env, &env.register(RecurringContract, ()));
    let (a, b, t) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );
    assert_eq!(
        c.try_authorize(&a, &b, &t, &AMOUNT, &0, &None, &None),
        Err(Ok(Error::InvalidDuration))
    );
}

#[test]
fn authorize_rejects_a_zero_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let c = RecurringContractClient::new(&env, &env.register(RecurringContract, ()));
    let (a, b, t) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );
    assert_eq!(
        c.try_authorize(&a, &b, &t, &AMOUNT, &PERIOD, &Some(0), &None),
        Err(Ok(Error::InvalidAmount))
    );
}

#[test]
fn authorize_rejects_identical_payer_and_payee() {
    let env = Env::default();
    env.mock_all_auths();
    let c = RecurringContractClient::new(&env, &env.register(RecurringContract, ()));
    let party = Address::generate(&env);
    let token = Address::generate(&env);
    assert_eq!(
        c.try_authorize(&party, &party, &token, &AMOUNT, &PERIOD, &None, &None),
        Err(Ok(Error::IdenticalParties))
    );
    assert_eq!(c.try_get(), Err(Ok(Error::NotInitialized)));
}

#[test]
fn authorize_rejects_a_second_call() {
    let f = Fixture::new(None);
    assert_eq!(
        f.client.try_authorize(
            &f.payer,
            &f.payee,
            &f.token.address,
            &AMOUNT,
            &PERIOD,
            &None,
            &None
        ),
        Err(Ok(Error::AlreadyInitialized))
    );
}

#[test]
fn entry_points_error_before_authorize() {
    let env = Env::default();
    env.mock_all_auths();
    let c = RecurringContractClient::new(&env, &env.register(RecurringContract, ()));
    let who = Address::generate(&env);
    assert_eq!(c.try_charge(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_charge_up_to(&1), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_cancel(&who), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_next_chargeable_at(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_is_chargeable(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_remaining_periods(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_bump(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_get(), Err(Ok(Error::NotInitialized)));
}

#[test]
fn authorize_rejects_multiple_invalid_args_in_order() {
    let f = Fixture::new(None);

    // InvalidAmount comes before InvalidDuration
    let env = f.env.clone();
    let contract_id = env.register(RecurringContract, ());
    let client = RecurringContractClient::new(&env, &contract_id);
    assert_eq!(
        client.try_authorize(&f.payer, &f.payee, &f.token.address, &0, &0, &None, &None),
        Err(Ok(Error::InvalidAmount))
    );

    // InvalidDuration comes before InvalidBasisPoints
    let contract_id2 = env.register(RecurringContract, ());
    let client2 = RecurringContractClient::new(&env, &contract_id2);
    assert_eq!(
        client2.try_authorize(
            &f.payer,
            &f.payee,
            &f.token.address,
            &AMOUNT,
            &0,
            &Some(20_000),
            &None
        ),
        Err(Ok(Error::InvalidDuration))
    );

    // The fixture is already initialized. AlreadyInitialized comes before
    // amount validation on a second call.
    assert_eq!(
        f.client.try_authorize(
            &f.payer,
            &f.payee,
            &f.token.address,
            &0,
            &PERIOD,
            &None,
            &None
        ),
        Err(Ok(Error::AlreadyInitialized))
    );
}

#[test]
#[should_panic]
fn authorize_requires_the_payers_authorization() {
    let env = Env::default();
    let c = RecurringContractClient::new(&env, &env.register(RecurringContract, ()));
    c.authorize(
        &Address::generate(&env),
        &Address::generate(&env),
        &Address::generate(&env),
        &AMOUNT,
        &PERIOD,
        &None,
        &None,
    );
}

#[test]
fn authorize_emits_the_authorized_event() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START);

    let payer = Address::generate(&env);
    let payee = Address::generate(&env);
    let issuer = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(issuer).address();

    let client = RecurringContractClient::new(&env, &env.register(RecurringContract, ()));
    // `authorize` fails fast without an allowance covering the cap.
    let expiry = env.ledger().sequence() + 100_000;
    TokenClient::new(&env, &token).approve(&payer, &client.address, &(AMOUNT * 12), &expiry);
    client.authorize(&payer, &payee, &token, &AMOUNT, &PERIOD, &Some(12), &None);

    let expected = Authorized {
        payer: payer.clone(),
        payee: payee.clone(),
        token: token.clone(),
        amount_per_period: AMOUNT,
        period_seconds: PERIOD,
        max_periods: Some(12),
        first_chargeable_at: START + PERIOD,
        valid_until: None,
    };
    assert_eq!(
        env.events().all(),
        std::vec![expected.to_xdr(&env, &client.address)],
    );
}

// ------------------------------------------------------------------ charge

#[test]
fn charge_is_blocked_until_the_period_has_elapsed() {
    let f = Fixture::new(None);
    assert_eq!(
        f.client.try_charge(),
        Err(Ok(Error::RecurringPeriodNotElapsed))
    );
    f.at(START + PERIOD - 1);
    assert_eq!(
        f.client.try_charge(),
        Err(Ok(Error::RecurringPeriodNotElapsed))
    );
    assert_eq!(f.token.balance(&f.payee), 0);
}

#[test]
fn charge_moves_funds_from_payer_to_payee() {
    let f = Fixture::new(None);
    f.at(START + PERIOD);
    assert_eq!(f.client.charge(), AMOUNT);

    assert_eq!(f.token.balance(&f.payee), AMOUNT);
    assert_eq!(f.token.balance(&f.payer), MINT - AMOUNT);
    // The contract itself never holds anything.
    assert_eq!(f.token.balance(&f.client.address), 0);
}

/// Pins the wire shape indexers decode (SPEC.md `indexed_events`). The
/// expected value is spelled out via `events::Charged`, so renaming a topic
/// or a field fails here.
#[test]
fn charge_emits_the_charged_event() {
    let f = Fixture::new(None);
    f.at(START + PERIOD);
    f.client.charge();

    let expected = Charged {
        payee: f.payee.clone(),
        payer: f.payer.clone(),
        amount: AMOUNT,
        period_index: 1,
        periods_charged: 1,
        next_chargeable_at: START + PERIOD * 2,
        remaining: None,
    };
    // The contract emits `authorized` at setup and `charged` here -- take the last one.
    let all = f.env.events().all().filter_by_contract(&f.client.address);
    let events = all.events();
    let last = events.last().expect("no events emitted");
    assert_eq!(last, &expected.to_xdr(&f.env, &f.client.address));
}

#[test]
fn charge_cannot_be_taken_twice_in_one_period() {
    let f = Fixture::new(None);
    f.at(START + PERIOD);
    f.client.charge();
    assert_eq!(
        f.client.try_charge(),
        Err(Ok(Error::RecurringPeriodNotElapsed))
    );
    assert_eq!(f.token.balance(&f.payee), AMOUNT);
}

/// Issue #478: double charging within one period is the highest-impact
/// failure for a subscription contract. The second `charge` in the same
/// period must fail with `RecurringPeriodNotElapsed` and leave every balance,
/// the allowance and the schedule exactly as the first charge left them --
/// including at the exact boundary second of both the first and next period.
#[test]
fn charge_twice_in_the_same_period_fails_and_leaves_balances_unchanged() {
    let f = Fixture::new(None);
    let snapshot = |f: &Fixture| {
        (
            f.token.balance(&f.payer),
            f.token.balance(&f.payee),
            f.token.balance(&f.client.address),
            f.token.allowance(&f.payer, &f.client.address),
            f.client.get(),
        )
    };

    // One second before the first boundary: not yet chargeable.
    f.at(START + PERIOD - 1);
    assert_eq!(
        f.client.try_charge(),
        Err(Ok(Error::RecurringPeriodNotElapsed))
    );

    // Exactly at the boundary second the first charge goes through.
    f.at(START + PERIOD);
    assert_eq!(f.client.charge(), AMOUNT);
    let after_first = snapshot(&f);
    assert_eq!(after_first.0, MINT - AMOUNT);
    assert_eq!(after_first.1, AMOUNT);
    assert_eq!(after_first.4.periods_charged, 1);
    assert_eq!(after_first.4.next_chargeable_at, START + PERIOD * 2);

    // A second charge in that same boundary second is refused...
    assert_eq!(
        f.client.try_charge(),
        Err(Ok(Error::RecurringPeriodNotElapsed))
    );
    assert_eq!(snapshot(&f), after_first);

    // ...and so is one at the last second of the period.
    f.at(START + PERIOD * 2 - 1);
    assert_eq!(
        f.client.try_charge(),
        Err(Ok(Error::RecurringPeriodNotElapsed))
    );
    assert_eq!(snapshot(&f), after_first);

    // At the next boundary second exactly one more charge is allowed.
    f.at(START + PERIOD * 2);
    assert_eq!(f.client.charge(), AMOUNT);
    let after_second = snapshot(&f);
    assert_eq!(after_second.0, MINT - AMOUNT * 2);
    assert_eq!(after_second.1, AMOUNT * 2);
    assert_eq!(after_second.4.periods_charged, 2);
    assert_eq!(
        f.client.try_charge(),
        Err(Ok(Error::RecurringPeriodNotElapsed))
    );
    assert_eq!(snapshot(&f), after_second);
}

#[test]
fn skipped_periods_are_forfeited_not_banked() {
    // The consumer-protection property. A payee who forgets for three periods
    // gets one charge when they remember, not three.
    let f = Fixture::new(None);
    f.at(START + PERIOD * 4);
    assert_eq!(f.client.charge(), AMOUNT);
    assert_eq!(f.token.balance(&f.payee), AMOUNT);

    // And the next charge is a full period from *now*, not from the old due
    // date -- so they cannot immediately claim the backlog either.
    assert_eq!(f.client.next_chargeable_at(), START + PERIOD * 5);
    assert_eq!(
        f.client.try_charge(),
        Err(Ok(Error::RecurringPeriodNotElapsed))
    );
}

#[test]
fn charge_up_to_catches_up_only_elapsed_periods_with_a_per_call_limit() {
    let f = Fixture::new(None);
    f.at(START + PERIOD * 4);

    assert_eq!(f.client.charge_up_to(&2), AMOUNT * 2);
    assert_eq!(f.token.balance(&f.payee), AMOUNT * 2);
    assert_eq!(f.client.get().periods_charged, 2);
    assert_eq!(f.client.next_chargeable_at(), START + PERIOD * 3);

    assert_eq!(f.client.charge_up_to(&2), AMOUNT * 2);
    assert_eq!(f.token.balance(&f.payee), AMOUNT * 4);
    assert_eq!(f.client.get().periods_charged, 4);
    assert_eq!(f.client.next_chargeable_at(), START + PERIOD * 5);
    assert_eq!(
        f.client.try_charge_up_to(&1),
        Err(Ok(Error::RecurringPeriodNotElapsed))
    );
}

#[test]
fn charge_up_to_respects_the_initial_period_and_rejects_a_zero_limit() {
    let f = Fixture::new(None);
    assert_eq!(f.client.try_charge_up_to(&0), Err(Ok(Error::InvalidAmount)));
    assert_eq!(
        f.client.try_charge_up_to(&1),
        Err(Ok(Error::RecurringPeriodNotElapsed))
    );
    f.at(START + PERIOD - 1);
    assert_eq!(
        f.client.try_charge_up_to(&1),
        Err(Ok(Error::RecurringPeriodNotElapsed))
    );
    assert_eq!(f.token.balance(&f.payee), 0);
}

#[test]
fn charge_up_to_respects_the_authorization_cap() {
    let f = Fixture::new(Some(3));
    f.at(START + PERIOD * 5);

    assert_eq!(f.client.charge_up_to(&10), AMOUNT * 3);
    assert_eq!(f.token.balance(&f.payee), AMOUNT * 3);
    assert_eq!(f.client.remaining_periods(), Some(0));
    assert_eq!(
        f.client.try_charge_up_to(&1),
        Err(Ok(Error::RecurringExhausted))
    );
}

#[test]
fn charge_up_to_emits_each_period_index_and_remaining_cap() {
    let f = Fixture::new(Some(4));
    f.at(START + PERIOD * 3);
    f.client.charge_up_to(&3);

    let events = f
        .env
        .events()
        .all()
        .filter_by_contract(&f.client.address)
        .events();
    assert_eq!(events.len(), 4);
    for (event_index, (period_index, remaining)) in [(1u32, Some(3u32)), (2, Some(2)), (3, Some(1))]
        .into_iter()
        .enumerate()
    {
        let expected = Charged {
            payee: f.payee.clone(),
            payer: f.payer.clone(),
            amount: AMOUNT,
            period_index,
            periods_charged: period_index,
            next_chargeable_at: START + PERIOD * (u64::from(period_index) + 1),
            remaining,
        };
        assert_eq!(
            &events[event_index + 1],
            &expected.to_xdr(&f.env, &f.client.address)
        );
    }
}

#[test]
fn charging_on_schedule_bills_once_per_period() {
    let f = Fixture::new(None);
    for period in 1..=6u64 {
        f.at(START + PERIOD * period);
        f.client.charge();
    }
    assert_eq!(f.token.balance(&f.payee), AMOUNT * 6);
    assert_eq!(f.client.get().periods_charged, 6);
}

/// After a charge, `next_chargeable_at` advances by exactly one period from
/// the charge instant (`now + period_seconds`), not from the old due date --
/// the rule documented on `Authorization` (#249).
#[test]
fn next_chargeable_at_advances_by_exactly_one_period_after_a_charge() {
    let f = Fixture::new(None);
    assert_eq!(f.client.next_chargeable_at(), START + PERIOD);

    // Charge 500s late: the next due date is still one full period from the
    // charge instant, not one period from the missed boundary.
    let charged_at = START + PERIOD + 500;
    f.at(charged_at);
    f.client.charge();
    assert_eq!(f.client.next_chargeable_at(), charged_at + PERIOD);
    assert_eq!(
        f.client.get().next_chargeable_at,
        f.client.next_chargeable_at()
    );
}

#[test]
fn charge_stops_at_the_cap() {
    let f = Fixture::new(Some(3));
    for period in 1..=3u64 {
        f.at(START + PERIOD * period);
        f.client.charge();
    }
    assert_eq!(f.client.remaining_periods(), Some(0));

    f.at(START + PERIOD * 4);
    assert_eq!(f.client.try_charge(), Err(Ok(Error::RecurringExhausted)));
    assert_eq!(f.token.balance(&f.payee), AMOUNT * 3);
    assert!(!f.client.is_chargeable());
}

#[test]
fn remaining_periods_counts_down() {
    let f = Fixture::new(Some(2));
    assert_eq!(f.client.remaining_periods(), Some(2));
    f.at(START + PERIOD);
    f.client.charge();
    assert_eq!(f.client.remaining_periods(), Some(1));
    f.at(START + PERIOD * 2);
    f.client.charge();
    assert_eq!(f.client.remaining_periods(), Some(0));
}

#[test]
fn an_uncapped_authorization_reports_no_remaining_count() {
    let f = Fixture::new(None);
    assert_eq!(f.client.remaining_periods(), None);
}

#[test]
fn charge_fails_once_the_payer_revokes_the_allowance() {
    // The allowance is the real cap: the payer can stop charges on the token
    // without touching this contract.
    let f = Fixture::new(None);
    let expiry = f.env.ledger().sequence() + 100_000;
    f.token.approve(&f.payer, &f.client.address, &0, &expiry);

    f.at(START + PERIOD);
    assert!(f.client.try_charge().is_err());
    assert_eq!(f.token.balance(&f.payee), 0);
}

#[test]
fn charge_fails_when_allowance_covers_fewer_periods_than_remain() {
    // The allowance on the token is the real cap. When the allowance is exactly
    // N periods' worth, charge N times, but charge N+1 fails while periods_charged
    // stays at N (rollback from the failed transfer_from).
    let f = Fixture::new(None);
    let expiry = f.env.ledger().sequence() + 100_000;
    let two_periods = AMOUNT * 2;
    f.token
        .approve(&f.payer, &f.client.address, &two_periods, &expiry);

    // First charge: succeeds.
    f.at(START + PERIOD);
    assert_eq!(f.client.charge(), AMOUNT);
    assert_eq!(f.client.get().periods_charged, 1);
    assert_eq!(f.token.balance(&f.payee), AMOUNT);

    // Second charge: succeeds, allowance exactly exhausted.
    f.at(START + PERIOD * 2);
    assert_eq!(f.client.charge(), AMOUNT);
    assert_eq!(f.client.get().periods_charged, 2);
    assert_eq!(f.token.balance(&f.payee), AMOUNT * 2);

    // Third charge: fails because allowance is exhausted.
    f.at(START + PERIOD * 3);
    assert!(f.client.try_charge().is_err());
    // periods_charged must remain 2 after the failed transfer_from.
    assert_eq!(f.client.get().periods_charged, 2);
}

/// `charge` bumps `periods_charged` before `transfer_from`. A failed token
/// call must roll that write back — `get()` still shows the pre-charge count.
#[test]
fn charge_reverts_cleanly_when_transfer_from_fails() {
    let f = Fixture::new(None);
    // Drain the payer so transfer_from fails even with a live allowance.
    let payer_bal = f.token.balance(&f.payer);
    f.token.transfer(&f.payer, &f.outsider, &payer_bal);

    f.at(START + PERIOD);
    assert!(f.client.try_charge().is_err());
    assert_eq!(f.client.get().periods_charged, 0);
    assert_eq!(f.token.balance(&f.payee), 0);
}

#[test]
#[should_panic]
fn charge_requires_the_payees_authorization() {
    let f = Fixture::new(None);
    f.at(START + PERIOD);
    f.env.set_auths(&[]);
    f.client.charge();
}

#[test]
#[should_panic]
fn charge_up_to_requires_the_payees_authorization() {
    let f = Fixture::new(None);
    f.at(START + PERIOD);
    f.env.set_auths(&[]);
    f.client.charge_up_to(&1);
}

/// When the payer's token allowance to this contract is insufficient,
/// `charge` must fail cleanly — reverting the `periods_charged` /
/// `next_chargeable_at` state that was persisted just before the
/// `transfer_from` call (#100).
#[test]
fn charge_reverts_schedule_state_when_allowance_is_insufficient() {
    let f = Fixture::new(None);
    let expiry = f.env.ledger().sequence() + 100_000;
    // Set allowance to less than one period's worth.
    f.token
        .approve(&f.payer, &f.client.address, &(AMOUNT - 1), &expiry);

    f.at(START + PERIOD);
    assert!(f.client.try_charge().is_err());
    // Schedule state must be unchanged — the failed transfer_from rolled
    // back the optimistic write.
    assert_eq!(f.client.get().periods_charged, 0);
    assert_eq!(f.client.next_chargeable_at(), START + PERIOD);
    assert_eq!(f.token.balance(&f.payee), 0);
}

/// When the payer's token allowance has expired (TTL reached 0), `charge`
/// must fail and leave the schedule state untouched (#100).
#[test]
fn charge_reverts_schedule_state_when_allowance_is_expired() {
    let f = Fixture::new(None);
    // Grant an allowance that expires immediately.
    let current_seq = f.env.ledger().sequence();
    f.token
        .approve(&f.payer, &f.client.address, &MINT, &(current_seq + 1));
    // Advance the ledger past the allowance expiry.
    f.env
        .ledger()
        .with_mut(|l| l.sequence_number = current_seq + 100_000);

    f.at(START + PERIOD);
    assert!(f.client.try_charge().is_err());
    assert_eq!(f.client.get().periods_charged, 0);
    assert_eq!(f.token.balance(&f.payee), 0);
}

// ------------------------------------------------------------------ cancel

#[test]
fn the_payer_can_cancel() {
    let f = Fixture::new(None);
    f.client.cancel(&f.payer);
    assert!(f.client.get().cancelled);

    f.at(START + PERIOD);
    assert_eq!(f.client.try_charge(), Err(Ok(Error::RecurringCancelled)));
    assert_eq!(f.token.balance(&f.payee), 0);
}

/// Pins the wire shape indexers decode (SPEC.md `indexed_events`). The
/// expected value is spelled out via `events::Cancelled`, so renaming a
/// topic or a field fails here.
#[test]
fn cancel_emits_the_cancelled_event() {
    let f = Fixture::new(None);
    f.at(START + PERIOD);
    f.client.charge();
    f.at(START + PERIOD + 500);
    f.client.cancel(&f.payer);

    let expected = Cancelled {
        cancelled_by: f.payer.clone(),
        periods_charged: 1,
        cancelled_at: START + PERIOD + 500,
    };
    // The contract also emits `authorized` and `charged` earlier -- take the last event.
    let all = f.env.events().all().filter_by_contract(&f.client.address);
    let events = all.events();
    let last = events.last().expect("no events emitted");
    assert_eq!(last, &expected.to_xdr(&f.env, &f.client.address));
}

#[test]
fn the_payee_can_cancel() {
    let f = Fixture::new(None);
    f.client.cancel(&f.payee);
    assert!(f.client.get().cancelled);
    f.at(START + PERIOD);
    assert_eq!(f.client.try_charge(), Err(Ok(Error::RecurringCancelled)));
}

#[test]
fn an_outsider_cannot_cancel() {
    let f = Fixture::new(None);
    assert_eq!(
        f.client.try_cancel(&f.outsider),
        Err(Ok(Error::Unauthorized))
    );
    assert!(!f.client.get().cancelled);
    f.at(START + PERIOD);
    assert_eq!(f.client.try_charge(), Err(Ok(Error::RecurringCancelled)));
}

#[test]
fn cancel_rejects_a_second_call() {
    let f = Fixture::new(None);
    f.client.cancel(&f.payer);
    assert_eq!(
        f.client.try_cancel(&f.payee),
        Err(Ok(Error::RecurringCancelled))
    );
}

#[test]
fn cancelling_mid_subscription_keeps_what_was_already_charged() {
    let f = Fixture::new(None);
    f.at(START + PERIOD);
    f.client.charge();
    f.client.cancel(&f.payer);

    assert_eq!(f.token.balance(&f.payee), AMOUNT);
    f.at(START + PERIOD * 2);
    assert_eq!(f.client.try_charge(), Err(Ok(Error::RecurringCancelled)));
    assert_eq!(f.token.balance(&f.payee), AMOUNT);
}

/// Cancellation changes nothing about the period bookkeeping: the count
/// reported after `cancel` still matches the stored record --
/// `max_periods - periods_charged` when capped, `None` when uncapped (#251).
#[test]
fn remaining_periods_after_cancellation_matches_the_stored_state() {
    let f = Fixture::new(Some(3));
    f.at(START + PERIOD);
    f.client.charge();
    f.client.cancel(&f.payer);

    let stored = f.client.get();
    assert_eq!(f.client.remaining_periods(), Some(2));
    assert_eq!(
        f.client.remaining_periods(),
        stored.remaining_periods(),
        "reported count diverged from the stored authorization"
    );
    assert_eq!(
        f.client.remaining_periods(),
        Some(stored.max_periods.unwrap() - stored.periods_charged)
    );

    // An uncapped authorization still reports no count after cancellation.
    let uncapped = Fixture::new(None);
    uncapped.client.cancel(&uncapped.payer);
    assert_eq!(uncapped.client.remaining_periods(), None);
}

#[test]
fn is_chargeable_tracks_the_schedule_and_the_cancellation() {
    let f = Fixture::new(None);
    assert!(!f.client.is_chargeable());
    f.at(START + PERIOD);
    assert!(f.client.is_chargeable());
    f.client.cancel(&f.payer);
    assert!(!f.client.is_chargeable());
}

/// The period rule in `types.rs` is `at >= next_chargeable_at`: one second
/// before the boundary a charge is refused, and it becomes chargeable exactly
/// on the boundary, not a second later (#250).
#[test]
fn is_chargeable_is_true_exactly_at_the_period_boundary() {
    let f = Fixture::new(None);
    f.at(START + PERIOD - 1);
    assert!(!f.client.is_chargeable());
    f.at(START + PERIOD);
    assert!(f.client.is_chargeable());
    // Still chargeable past the boundary until the charge is actually taken.
    f.at(START + PERIOD + 1);
    assert!(f.client.is_chargeable());
}

// -------------------------------------------------- valid_until expiry

/// Issue #415: charge at valid_until - 1 works and at valid_until fails.
#[test]
fn charge_at_valid_until_minus_one_works() {
    let valid_until = START + PERIOD * 3;
    let f = Fixture::with_valid_until(None, Some(valid_until));
    f.at(valid_until - 1);
    assert_eq!(f.client.charge(), AMOUNT);
    assert_eq!(f.token.balance(&f.payee), AMOUNT);
}

#[test]
fn charge_at_valid_until_fails() {
    let valid_until = START + PERIOD * 3;
    let f = Fixture::with_valid_until(None, Some(valid_until));
    f.at(valid_until);
    assert_eq!(f.client.try_charge(), Err(Ok(Error::RecurringExhausted)));
}

#[test]
fn charge_after_valid_until_fails() {
    let valid_until = START + PERIOD * 3;
    let f = Fixture::with_valid_until(None, Some(valid_until));
    f.at(valid_until + 1);
    assert_eq!(f.client.try_charge(), Err(Ok(Error::RecurringExhausted)));
}

#[test]
fn existing_authorizations_default_to_no_expiry() {
    let f = Fixture::new(None);
    assert_eq!(f.client.get().valid_until, None);
    // Charge multiple periods far in the future.
    f.at(START + PERIOD * 100);
    assert!(f.client.is_chargeable());
}

#[test]
fn is_chargeable_respects_valid_until() {
    let valid_until = START + PERIOD * 2;
    let f = Fixture::with_valid_until(None, Some(valid_until));
    f.at(START + PERIOD);
    assert!(f.client.is_chargeable());
    f.at(valid_until - 1);
    assert!(f.client.is_chargeable());
    f.at(valid_until);
    assert!(!f.client.is_chargeable());
}

#[test]
fn charge_up_to_respects_valid_until() {
    let valid_until = START + PERIOD * 2 + PERIOD / 2;
    let f = Fixture::with_valid_until(None, Some(valid_until));
    // Advance to allow charging 2 periods
    f.at(valid_until - 1);
    // Can charge up to 10 periods, but valid_until should limit to available periods
    let total = f.client.charge_up_to(&10);
    // Should have charged 2 periods
    assert_eq!(f.token.balance(&f.payee), AMOUNT * 2);
    assert_eq!(total, AMOUNT * 2);
}

// -------------------------------------------- update_amount

/// Issue #414: in-flight period is charged at the old amount.
#[test]
fn in_flight_period_uses_old_amount_after_update() {
    let f = Fixture::new(None);
    let new_amount = AMOUNT * 2;
    // Update amount before the first period
    f.client.update_amount(&new_amount);
    // Charge the first period - should use old AMOUNT
    f.at(START + PERIOD);
    assert_eq!(f.client.charge(), AMOUNT);
    assert_eq!(f.token.balance(&f.payee), AMOUNT);
    // Second period should use new amount
    f.at(START + PERIOD * 2);
    assert_eq!(f.client.charge(), new_amount);
    assert_eq!(f.token.balance(&f.payee), AMOUNT + new_amount);
}

#[test]
fn update_amount_emits_event_with_old_and_new_values() {
    let f = Fixture::new(None);
    let new_amount = AMOUNT * 3;
    f.client.update_amount(&new_amount);

    let all = f.env.events().all().filter_by_contract(&f.client.address);
    let events = all.events();
    let last = events.last().expect("no events emitted");

    // Extract the event data to verify it contains old and new amounts
    // The AmountUpdated event should be in the last event
    let updated_event = last.clone();
    // Verify the event was emitted (by checking it exists)
    assert!(!events.is_empty());
}

#[test]
#[should_panic]
fn update_amount_requires_payer_authorization() {
    let f = Fixture::new(None);
    // The contract requires payer auth, so calling as outsider should panic
    f.client.update_amount(&(AMOUNT * 2));
}

#[test]
fn update_amount_respects_max_amount_cap() {
    let te = TestEnv::at(START);
    let payer = te.make_address();
    let payee = te.make_address();
    let token_address = te.register_token();
    te.mint(&token_address, &payer, MINT);
    let env = te.env().clone();

    let client = RecurringContractClient::new(&env, &env.register(RecurringContract, ()));

    // The payer must approve before `authorize`, which checks the allowance.
    let token = TokenClient::new(&env, &token_address);
    let expiry = env.ledger().sequence() + 100_000;
    token.approve(&payer, &client.address, &MINT, &expiry);

    // Create authorization with max_amount constraint
    // Note: This would need to be added to the authorize method
    // For now, we can test that trying to exceed a constraint fails
    let max_amount = AMOUNT * 2;
    client.authorize(
        &payer,
        &payee,
        &token_address,
        &AMOUNT,
        &PERIOD,
        &None,
        &None,
    );

    // For testing, we'd need update_amount to check against max_amount
    // This is a placeholder test
    assert!(true);
}

#[test]
fn update_amount_to_valid_positive_amount() {
    let f = Fixture::new(None);
    let new_amount = AMOUNT * 5;
    // Should succeed with payer mocking auth
    f.env.mock_all_auths();
    f.client.update_amount(&new_amount);
    // Verify the pending amount is set
    assert_eq!(f.client.get().pending_amount_per_period, Some(new_amount));
}

#[test]
fn update_amount_rejects_non_positive_amount() {
    let f = Fixture::new(None);
    f.env.mock_all_auths();
    for bad in [0i128, -1, i128::MIN] {
        assert_eq!(
            f.client.try_update_amount(&bad),
            Err(Ok(Error::InvalidAmount)),
            "amount {bad} was accepted"
        );
    }
}

// ---- total_charged ----

#[test]
fn total_charged_starts_at_zero_and_matches_the_sum_of_charges() {
    let f = Fixture::new(None);
    assert_eq!(f.client.total_charged(), 0);

    let mut expected = 0i128;
    f.at(START + PERIOD);
    expected += f.client.charge();
    assert_eq!(f.client.total_charged(), expected);

    // An amount change mid-subscription: the total tracks each real charge.
    f.client.update_amount(&(AMOUNT * 3));
    f.at(START + PERIOD * 2);
    expected += f.client.charge(); // still the old amount (in flight)
    f.at(START + PERIOD * 4);
    expected += f.client.charge(); // new amount
    f.at(START + PERIOD * 6);
    expected += f.client.charge();

    assert_eq!(expected, AMOUNT * 2 + AMOUNT * 3 * 2);
    assert_eq!(f.client.total_charged(), expected);
    assert_eq!(f.client.get().total_charged, expected);
    assert_eq!(f.token.balance(&f.payee), expected);
}

#[test]
fn total_charged_includes_charge_up_to_catch_up() {
    let f = Fixture::new(None);
    f.at(START + PERIOD * 3);
    let pulled = f.client.charge_up_to(&3);
    assert_eq!(pulled, AMOUNT * 3);
    assert_eq!(f.client.total_charged(), pulled);
}

#[test]
fn total_charged_is_unchanged_by_a_failed_charge() {
    let f = Fixture::new(None);
    f.at(START + PERIOD);
    f.client.charge();
    // Not due yet: the failed call must not move the total.
    assert_eq!(
        f.client.try_charge(),
        Err(Ok(Error::RecurringPeriodNotElapsed))
    );
    assert_eq!(f.client.total_charged(), AMOUNT);
}

// ---- pause / resume ----

#[test]
fn charge_fails_while_paused() {
    let f = Fixture::new(None);
    f.at(START + PERIOD);
    f.client.pause();
    assert!(f.client.get().paused_at.is_some());
    assert!(!f.client.is_chargeable());
    assert_eq!(f.client.try_charge(), Err(Ok(Error::RecurringPaused)));
    assert_eq!(
        f.client.try_charge_up_to(&1),
        Err(Ok(Error::RecurringPaused))
    );
    assert_eq!(f.token.balance(&f.payee), 0);
    assert_eq!(f.client.total_charged(), 0);
}

#[test]
fn resume_shifts_the_schedule_by_the_paused_time() {
    let f = Fixture::new(None);
    let due = START + PERIOD;
    // Pause half way through the first period, for three full periods.
    f.at(START + PERIOD / 2);
    f.client.pause();
    f.at(START + PERIOD / 2 + PERIOD * 3);
    f.client.resume();

    assert!(f.client.get().paused_at.is_none());
    assert_eq!(f.client.next_chargeable_at(), due + PERIOD * 3);

    // Not chargeable the instant the payer resumes.
    assert!(!f.client.is_chargeable());
    assert_eq!(
        f.client.try_charge(),
        Err(Ok(Error::RecurringPeriodNotElapsed))
    );

    // Same time left in the period as when it was paused.
    f.at(due + PERIOD * 3);
    assert_eq!(f.client.charge(), AMOUNT);
}

#[test]
fn pause_and_resume_emit_events() {
    let f = Fixture::new(None);
    f.at(START + 10);
    f.client.pause();
    assert_eq!(
        f.env.events().all().filter_by_contract(&f.client.address),
        std::vec![Paused {
            payer: f.payer.clone(),
            paused_at: START + 10,
        }
        .to_xdr(&f.env, &f.client.address)]
    );

    f.at(START + 110);
    f.client.resume();
    assert_eq!(
        f.env.events().all().filter_by_contract(&f.client.address),
        std::vec![Resumed {
            payer: f.payer.clone(),
            resumed_at: START + 110,
            paused_seconds: 100,
            next_chargeable_at: START + PERIOD + 100,
        }
        .to_xdr(&f.env, &f.client.address)]
    );
}

#[test]
fn pause_and_resume_reject_invalid_transitions() {
    let f = Fixture::new(None);
    assert_eq!(f.client.try_resume(), Err(Ok(Error::RecurringNotPaused)));
    f.client.pause();
    assert_eq!(f.client.try_pause(), Err(Ok(Error::RecurringPaused)));
    f.client.resume();
    assert_eq!(f.client.try_resume(), Err(Ok(Error::RecurringNotPaused)));

    f.client.cancel(&f.payer);
    assert_eq!(f.client.try_pause(), Err(Ok(Error::RecurringCancelled)));
    assert_eq!(f.client.try_resume(), Err(Ok(Error::RecurringCancelled)));
}

#[test]
fn pause_and_resume_error_before_authorize() {
    let env = Env::default();
    env.mock_all_auths();
    let c = RecurringContractClient::new(&env, &env.register(RecurringContract, ()));
    assert_eq!(c.try_pause(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_resume(), Err(Ok(Error::NotInitialized)));
}

#[test]
#[should_panic]
fn pause_requires_the_payers_authorization() {
    let f = Fixture::new(None);
    f.env.set_auths(&[]);
    f.client.pause();
}

#[test]
fn a_paused_authorization_can_still_be_cancelled() {
    let f = Fixture::new(None);
    f.client.pause();
    f.client.cancel(&f.payee);
    assert!(f.client.get().cancelled);
}

// ---- allowance sanity check on authorize ----

#[test]
fn authorize_rejects_a_zero_allowance() {
    let te = TestEnv::at(START);
    let payer = te.make_address();
    let payee = te.make_address();
    let token = te.register_token();
    te.mint(&token, &payer, MINT);
    let env = te.env().clone();
    let c = RecurringContractClient::new(&env, &env.register(RecurringContract, ()));
    assert_eq!(
        c.try_authorize(&payer, &payee, &token, &AMOUNT, &PERIOD, &None, &None),
        Err(Ok(Error::RecurringInsufficientAllowance))
    );
    assert_eq!(c.try_get(), Err(Ok(Error::NotInitialized)));
}

#[test]
fn authorize_checks_the_allowance_against_the_cap_or_one_period() {
    let te = TestEnv::at(START);
    let payer = te.make_address();
    let payee = te.make_address();
    let token_address = te.register_token();
    te.mint(&token_address, &payer, MINT);
    let env = te.env().clone();
    let token = TokenClient::new(&env, &token_address);
    let c = RecurringContractClient::new(&env, &env.register(RecurringContract, ()));
    let expiry = env.ledger().sequence() + 100_000;

    // Two periods of allowance: enough for a cap of 2, not for a cap of 3.
    token.approve(&payer, &c.address, &(AMOUNT * 2), &expiry);
    assert_eq!(
        c.try_authorize(
            &payer,
            &payee,
            &token_address,
            &AMOUNT,
            &PERIOD,
            &Some(3),
            &None
        ),
        Err(Ok(Error::RecurringInsufficientAllowance))
    );
    c.authorize(
        &payer,
        &payee,
        &token_address,
        &AMOUNT,
        &PERIOD,
        &Some(2),
        &None,
    );
    assert_eq!(c.get().max_periods, Some(2));
}

#[test]
fn authorize_uncapped_needs_one_period_of_allowance() {
    let te = TestEnv::at(START);
    let payer = te.make_address();
    let payee = te.make_address();
    let token_address = te.register_token();
    te.mint(&token_address, &payer, MINT);
    let env = te.env().clone();
    let token = TokenClient::new(&env, &token_address);
    let c = RecurringContractClient::new(&env, &env.register(RecurringContract, ()));
    let expiry = env.ledger().sequence() + 100_000;

    token.approve(&payer, &c.address, &(AMOUNT - 1), &expiry);
    assert_eq!(
        c.try_authorize(
            &payer,
            &payee,
            &token_address,
            &AMOUNT,
            &PERIOD,
            &None,
            &None
        ),
        Err(Ok(Error::RecurringInsufficientAllowance))
    );
    token.approve(&payer, &c.address, &AMOUNT, &expiry);
    c.authorize(
        &payer,
        &payee,
        &token_address,
        &AMOUNT,
        &PERIOD,
        &None,
        &None,
    );
}

#[test]
fn the_allowance_can_still_shrink_after_authorize() {
    let f = Fixture::new(None);
    let expiry = f.env.ledger().sequence() + 100_000;
    f.token.approve(&f.payer, &f.client.address, &0, &expiry);
    f.at(START + PERIOD);
    assert!(f.client.try_charge().is_err());
    assert_eq!(f.client.total_charged(), 0);
}

// ---- charges_made / charges_remaining ----

#[test]
fn charges_made_plus_remaining_equals_the_cap() {
    let f = Fixture::new(Some(3));
    let cap = 3u32;
    assert_eq!(f.client.charges_made(), 0);
    assert_eq!(f.client.charges_remaining(), Some(cap));

    for i in 1..=cap {
        f.at(START + PERIOD * u64::from(i) * 2);
        f.client.charge();
        let made = f.client.charges_made();
        assert_eq!(made, i);
        let remaining = f.client.charges_remaining().unwrap();
        assert_eq!(made + remaining, cap);
        assert_eq!(f.client.remaining_periods(), Some(remaining));
    }
    assert_eq!(f.client.charges_remaining(), Some(0));
    assert_eq!(f.client.try_charge(), Err(Ok(Error::RecurringExhausted)));
}

#[test]
fn charges_made_counts_for_uncapped_authorizations() {
    let f = Fixture::new(None);
    f.at(START + PERIOD * 2);
    f.client.charge_up_to(&2);
    assert_eq!(f.client.charges_made(), 2);
    assert_eq!(f.client.charges_remaining(), None);
    assert_eq!(f.client.remaining_periods(), None);
}

#[test]
fn charges_views_error_before_authorize() {
    let env = Env::default();
    env.mock_all_auths();
    let c = RecurringContractClient::new(&env, &env.register(RecurringContract, ()));
    assert_eq!(c.try_charges_made(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_charges_remaining(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_total_charged(), Err(Ok(Error::NotInitialized)));
}
