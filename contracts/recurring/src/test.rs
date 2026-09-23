use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    token::{StellarAssetClient, TokenClient},
    Address, Env, Event,
};
use sororail_common::Error;

use crate::{
    contract::{RecurringContract, RecurringContractClient},
    events::Authorized,
};

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
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.timestamp = START);

        let payer = Address::generate(&env);
        let payee = Address::generate(&env);
        let outsider = Address::generate(&env);
        let issuer = Address::generate(&env);
        let token_address = env.register_stellar_asset_contract_v2(issuer).address();
        StellarAssetClient::new(&env, &token_address).mint(&payer, &MINT);

        let client = RecurringContractClient::new(&env, &env.register(RecurringContract, ()));
        client.authorize(
            &payer,
            &payee,
            &token_address,
            &AMOUNT,
            &PERIOD,
            &max_periods,
        );

        let token = TokenClient::new(&env, &token_address);
        // The payer grants the contract an allowance -- without this, no charge
        // can move anything, however valid the authorization record is.
        let expiry = env.ledger().sequence() + 100_000;
        token.approve(&payer, &client.address, &MINT, &expiry);

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
            c.try_authorize(&a, &b, &t, &bad, &PERIOD, &None),
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
        c.try_authorize(&a, &b, &t, &AMOUNT, &0, &None),
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
        c.try_authorize(&a, &b, &t, &AMOUNT, &PERIOD, &Some(0)),
        Err(Ok(Error::InvalidAmount))
    );
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
    assert_eq!(c.try_cancel(&who), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_next_chargeable_at(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_is_chargeable(), Err(Ok(Error::NotInitialized)));
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
    client.authorize(&payer, &payee, &token, &AMOUNT, &PERIOD, &Some(12));

    let expected = Authorized {
        payer: payer.clone(),
        payee: payee.clone(),
        token: token.clone(),
        amount_per_period: AMOUNT,
        period_seconds: PERIOD,
        max_periods: Some(12),
        first_chargeable_at: START + PERIOD,
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
fn charging_on_schedule_bills_once_per_period() {
    let f = Fixture::new(None);
    for period in 1..=6u64 {
        f.at(START + PERIOD * period);
        f.client.charge();
    }
    assert_eq!(f.token.balance(&f.payee), AMOUNT * 6);
    assert_eq!(f.client.get().periods_charged, 6);
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
#[should_panic]
fn charge_requires_the_payees_authorization() {
    let f = Fixture::new(None);
    f.at(START + PERIOD);
    f.env.set_auths(&[]);
    f.client.charge();
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

#[test]
fn the_payee_can_cancel() {
    let f = Fixture::new(None);
    f.client.cancel(&f.payee);
    assert!(f.client.get().cancelled);
}

#[test]
fn an_outsider_cannot_cancel() {
    let f = Fixture::new(None);
    assert_eq!(
        f.client.try_cancel(&f.outsider),
        Err(Ok(Error::Unauthorized))
    );
    assert!(!f.client.get().cancelled);
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

#[test]
fn is_chargeable_tracks_the_schedule_and_the_cancellation() {
    let f = Fixture::new(None);
    assert!(!f.client.is_chargeable());
    f.at(START + PERIOD);
    assert!(f.client.is_chargeable());
    f.client.cancel(&f.payer);
    assert!(!f.client.is_chargeable());
}
