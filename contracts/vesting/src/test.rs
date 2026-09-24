// Test fixtures do plain arithmetic on known-small constants; the checked-math
// rule is for contract code.
#![allow(clippy::arithmetic_side_effects)]

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token::TokenClient,
    Address, Env,
};
use sororail_common::{testutils::TestEnv, Error};

use crate::{
    contract::{VestingContract, VestingContractClient},
    types::Grant,
};

const TOTAL: i128 = 1_000_000;
const START: u64 = 1_000_000;
const CLIFF: u64 = 100;
const DURATION: u64 = 1_000;
const MINT: i128 = TOTAL * 10;

struct Fixture<'a> {
    env: Env,
    client: VestingContractClient<'a>,
    token: TokenClient<'a>,
    grantor: Address,
    beneficiary: Address,
}

impl Fixture<'_> {
    fn new(revocable: bool) -> Self {
        Fixture::with_schedule(revocable, CLIFF, DURATION)
    }

    fn with_schedule(revocable: bool, cliff: u64, duration: u64) -> Self {
        let te = TestEnv::at(START);
        let (token, grantor) = te.make_token(MINT);
        let token_address = token.address.clone();
        let beneficiary = te.make_address();
        let env = te.env;

        let client = VestingContractClient::new(&env, &env.register(VestingContract, ()));
        client.create(
            &grantor,
            &beneficiary,
            &token_address,
            &TOTAL,
            &START,
            &cliff,
            &duration,
            &revocable,
        );

        Fixture {
            token: TokenClient::new(&env, &token_address),
            env,
            client,
            grantor,
            beneficiary,
        }
    }

    fn at(&self, ts: u64) {
        self.env.ledger().with_mut(|l| l.timestamp = ts);
    }

    fn held(&self) -> i128 {
        self.token.balance(&self.client.address)
    }

    fn assert_conserved(&self) {
        let g = self.client.get();
        assert_eq!(
            g.claimed + g.returned + self.client.remaining(),
            g.total,
            "conservation violated: {g:?}"
        );
        assert_eq!(
            self.held(),
            self.client.remaining(),
            "held balance diverged from remaining: {g:?}"
        );
    }
}

/// A bare grant for testing the schedule in isolation.
fn bare(env: &Env, cliff: u64, duration: u64) -> Grant {
    Grant {
        grantor: Address::generate(env),
        beneficiary: Address::generate(env),
        token: Address::generate(env),
        total: TOTAL,
        start: START,
        cliff,
        duration,
        revocable: true,
        claimed: 0,
        returned: 0,
        revoked_at: None,
    }
}

// ----------------------------------------- the schedule (pure, no ledger)

#[test]
fn nothing_vests_before_the_cliff() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    assert_eq!(g.vested_amount(0), Ok(0));
    assert_eq!(g.vested_amount(START), Ok(0));
    assert_eq!(g.vested_amount(START + CLIFF - 1), Ok(0));
}

#[test]
fn the_cliff_vests_the_elapsed_proportion_in_one_step() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    // At the cliff, 100/1000 of the schedule has elapsed.
    assert_eq!(g.vested_amount(START + CLIFF), Ok(TOTAL / 10));
}

#[test]
fn vesting_is_linear_after_the_cliff() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    assert_eq!(g.vested_amount(START + 250), Ok(TOTAL / 4));
    assert_eq!(g.vested_amount(START + 500), Ok(TOTAL / 2));
    assert_eq!(g.vested_amount(START + 750), Ok(TOTAL * 3 / 4));
}

#[test]
fn fully_vested_at_start_plus_duration() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    assert_eq!(g.vested_amount(START + DURATION), Ok(TOTAL));
    assert_eq!(g.vested_amount(START + DURATION + 1), Ok(TOTAL));
    assert_eq!(g.vested_amount(u64::MAX), Ok(TOTAL));
}

#[test]
fn a_cliff_equal_to_duration_is_all_or_nothing() {
    let env = Env::default();
    let g = bare(&env, DURATION, DURATION);
    assert_eq!(g.vested_amount(START + DURATION - 1), Ok(0));
    assert_eq!(g.vested_amount(START + DURATION), Ok(TOTAL));
}

#[test]
fn a_zero_cliff_vests_linearly_from_the_start() {
    let env = Env::default();
    let g = bare(&env, 0, DURATION);
    assert_eq!(g.vested_amount(START), Ok(0));
    assert_eq!(g.vested_amount(START + 1), Ok(TOTAL / DURATION as i128));
    assert_eq!(g.vested_amount(START + 500), Ok(TOTAL / 2));
}

#[test]
fn a_one_second_schedule_vests_in_one_tick() {
    let env = Env::default();
    let g = bare(&env, 0, 1);
    assert_eq!(g.vested_amount(START), Ok(0));
    assert_eq!(g.vested_amount(START + 1), Ok(TOTAL));
}

#[test]
fn revocation_freezes_the_schedule() {
    let env = Env::default();
    let mut g = bare(&env, CLIFF, DURATION);
    g.revoked_at = Some(START + 300);
    assert_eq!(g.vested_amount(START + 300), Ok(TOTAL * 3 / 10));
    assert_eq!(g.vested_amount(START + 900), Ok(TOTAL * 3 / 10));
    assert_eq!(g.vested_amount(u64::MAX), Ok(TOTAL * 3 / 10));
}

#[test]
fn vesting_reports_overflow_rather_than_wrapping() {
    let env = Env::default();
    let mut g = bare(&env, 0, DURATION);
    g.total = i128::MAX;
    assert_eq!(g.vested_amount(START + 500), Err(Error::Overflow));
}

#[test]
fn long_duration_large_total_stays_inside_documented_safe_range() {
    let env = Env::default();
    let ten_years = 10 * 365 * 24 * 60 * 60;
    let mut g = bare(&env, 0, ten_years);

    // One billion 18-decimal tokens over ten years stays well below i128::MAX
    // during the checked `total * elapsed / duration` calculation.
    g.total = 1_000_000_000_i128 * 1_000_000_000_000_000_000_i128;

    assert_eq!(g.vested_amount(START + ten_years / 2), Ok(g.total / 2));
    assert!(g.vested_amount(START + ten_years - 1).is_ok());
    assert_eq!(g.vested_amount(START + ten_years), Ok(g.total));
}

// ------------------------------------------------------------------ create

#[test]
fn create_funds_the_whole_grant_up_front() {
    let f = Fixture::new(true);
    assert_eq!(f.held(), TOTAL);
    assert_eq!(f.token.balance(&f.grantor), MINT - TOTAL);
    f.assert_conserved();
}

#[test]
fn create_rejects_a_non_positive_total() {
    let env = Env::default();
    env.mock_all_auths();
    let c = VestingContractClient::new(&env, &env.register(VestingContract, ()));
    let (a, b, t) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );
    for bad in [0i128, -1, i128::MIN] {
        assert_eq!(
            c.try_create(&a, &b, &t, &bad, &START, &CLIFF, &DURATION, &true),
            Err(Ok(Error::InvalidAmount)),
            "total {bad} was accepted"
        );
    }
}

#[test]
fn create_rejects_a_zero_duration() {
    let env = Env::default();
    env.mock_all_auths();
    let c = VestingContractClient::new(&env, &env.register(VestingContract, ()));
    let (a, b, t) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );
    assert_eq!(
        c.try_create(&a, &b, &t, &TOTAL, &START, &0, &0, &true),
        Err(Ok(Error::InvalidDuration))
    );
}

#[test]
fn create_rejects_a_cliff_after_the_end() {
    let env = Env::default();
    env.mock_all_auths();
    let c = VestingContractClient::new(&env, &env.register(VestingContract, ()));
    let (a, b, t) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );
    assert_eq!(
        c.try_create(
            &a,
            &b,
            &t,
            &TOTAL,
            &START,
            &(DURATION + 1),
            &DURATION,
            &true
        ),
        Err(Ok(Error::VestingCliffAfterEnd))
    );
}

#[test]
fn create_rejects_a_second_call() {
    let f = Fixture::new(true);
    assert_eq!(
        f.client.try_create(
            &f.grantor,
            &f.beneficiary,
            &f.token.address,
            &TOTAL,
            &START,
            &CLIFF,
            &DURATION,
            &true
        ),
        Err(Ok(Error::AlreadyInitialized))
    );
}

/// Pins the wire shape indexers decode (SPEC.md `indexed_events`). The
/// expected value is spelled out literally rather than built from
/// `events::Created`, so renaming a topic or a field fails here.
#[test]
fn create_emits_the_created_event() {
    let te = TestEnv::at(START);
    let (token_client, grantor) = te.make_token(MINT);
    let token_address = token_client.address.clone();
    let beneficiary = te.make_address();
    let env = te.env;

    let client = VestingContractClient::new(&env, &env.register(VestingContract, ()));
    client.create(
        &grantor,
        &beneficiary,
        &token_address,
        &TOTAL,
        &START,
        &CLIFF,
        &DURATION,
        &true,
    );

    let total: Val = TOTAL.into_val(&env);
    let start: Val = START.into_val(&env);
    let cliff: Val = CLIFF.into_val(&env);
    let duration: Val = DURATION.into_val(&env);
    let revocable: Val = true.into_val(&env);
    // Map data: keys are the field names, serialized in sorted order.
    let data: Val = map![
        env,
        (Symbol::new(&env, "beneficiary"), beneficiary.into_val(&env)),
        (Symbol::new(&env, "cliff"), cliff),
        (Symbol::new(&env, "duration"), duration),
        (Symbol::new(&env, "revocable"), revocable),
        (Symbol::new(&env, "start"), start),
        (Symbol::new(&env, "token"), token_address.into_val(&env)),
        (Symbol::new(&env, "total"), total),
    ]
    .into_val(&env);
    assert_eq!(
        env.events().all().filter_by_contract(&client.address),
        vec![
            env,
            (
                client.address.clone(),
                vec![
                    env,
                    Symbol::new(&env, "vesting").into_val(&env),
                    Symbol::new(&env, "created").into_val(&env),
                    grantor.into_val(&env),
                ],
                data,
            ),
        ]
    );
}

#[test]
fn entry_points_error_before_create() {
    let env = Env::default();
    env.mock_all_auths();
    let c = VestingContractClient::new(&env, &env.register(VestingContract, ()));
    assert_eq!(c.try_claim(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_revoke(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_claimable(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_remaining(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_vested_amount(&START), Err(Ok(Error::NotInitialized)));
}

#[test]
#[should_panic]
fn create_requires_the_grantors_authorization() {
    let env = Env::default();
    let c = VestingContractClient::new(&env, &env.register(VestingContract, ()));
    c.create(
        &Address::generate(&env),
        &Address::generate(&env),
        &Address::generate(&env),
        &TOTAL,
        &START,
        &CLIFF,
        &DURATION,
        &true,
    );
}

// ------------------------------------------------------------------- claim

#[test]
fn claim_is_blocked_before_the_cliff() {
    let f = Fixture::new(true);
    f.at(START + CLIFF - 1);
    assert_eq!(f.client.try_claim(), Err(Ok(Error::VestingCliffNotReached)));
    assert_eq!(f.held(), TOTAL);
}

#[test]
fn claim_exactly_at_and_one_second_before_cliff() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    // One second before cliff_at(): nothing vested yet.
    assert_eq!(g.claimable_at(START + CLIFF - 1), Ok(0));
    // Exactly at cliff_at(): cliff vests its proportion in one step.
    assert_eq!(g.claimable_at(START + CLIFF), Ok(TOTAL / 10));
}

#[test]
fn claim_fails_one_second_before_cliff_but_succeeds_at_cliff() {
    let f = Fixture::new(true);
    f.at(START + CLIFF - 1);
    assert_eq!(f.client.try_claim(), Err(Ok(Error::VestingCliffNotReached)));
    f.at(START + CLIFF);
    assert_eq!(f.client.claim(), TOTAL / 10);
    assert_eq!(f.token.balance(&f.beneficiary), TOTAL / 10);
    f.assert_conserved();
}

#[test]
fn claim_pays_the_vested_portion() {
    let f = Fixture::new(true);
    f.at(START + 500);
    assert_eq!(f.client.claim(), TOTAL / 2);
    assert_eq!(f.token.balance(&f.beneficiary), TOTAL / 2);
    f.assert_conserved();
}

#[test]
fn claim_twice_in_the_same_second_has_nothing_left() {
    let f = Fixture::new(true);
    f.at(START + 500);
    f.client.claim();
    assert_eq!(f.client.try_claim(), Err(Ok(Error::VestingNothingToClaim)));
    f.assert_conserved();
}

#[test]
fn incremental_claims_sum_to_the_total() {
    let f = Fixture::new(true);
    for step in 1..=10u64 {
        f.at(START + step * 100);
        f.client.claim();
        f.assert_conserved();
    }
    assert_eq!(f.token.balance(&f.beneficiary), TOTAL);
    assert_eq!(f.held(), 0);
}

#[test]
fn claim_after_the_end_takes_everything() {
    let f = Fixture::new(true);
    f.at(START + DURATION + 10_000);
    assert_eq!(f.client.claim(), TOTAL);
    assert_eq!(f.held(), 0);
    f.assert_conserved();
}

#[test]
#[should_panic]
fn claim_requires_the_beneficiarys_authorization() {
    let f = Fixture::new(true);
    f.at(START + 500);
    f.env.set_auths(&[]);
    f.client.claim();
}

// ------------------------------------------------------------------ revoke

#[test]
fn revoke_returns_only_the_unvested_portion() {
    let f = Fixture::new(true);
    let grantor_before = f.token.balance(&f.grantor);
    f.at(START + 300);
    assert_eq!(f.client.revoke(), TOTAL * 7 / 10);

    assert_eq!(f.token.balance(&f.grantor), grantor_before + TOTAL * 7 / 10);
    // The vested 30% stays in the contract for the beneficiary.
    assert_eq!(f.held(), TOTAL * 3 / 10);
    f.assert_conserved();
}

#[test]
fn the_vested_portion_stays_claimable_after_revocation() {
    let f = Fixture::new(true);
    f.at(START + 300);
    f.client.revoke();
    f.at(START + 900);
    assert_eq!(f.client.claim(), TOTAL * 3 / 10);
    assert_eq!(f.token.balance(&f.beneficiary), TOTAL * 3 / 10);
    assert_eq!(f.held(), 0);
    f.assert_conserved();
}

#[test]
fn revoke_accounts_for_what_was_already_claimed() {
    let f = Fixture::new(true);
    f.at(START + 200);
    f.client.claim();
    f.at(START + 600);
    f.client.revoke();

    // 60% vested overall, 20% already taken, 40% still claimable, 40% back.
    assert_eq!(f.client.claimable(), TOTAL * 4 / 10);
    f.assert_conserved();
    f.client.claim();
    assert_eq!(f.token.balance(&f.beneficiary), TOTAL * 6 / 10);
    assert_eq!(f.held(), 0);
    f.assert_conserved();
}

#[test]
fn revoke_before_the_cliff_returns_everything() {
    let f = Fixture::new(true);
    let grantor_before = f.token.balance(&f.grantor);
    assert_eq!(f.client.revoke(), TOTAL);
    assert_eq!(f.token.balance(&f.grantor), grantor_before + TOTAL);
    assert_eq!(f.held(), 0);
    // Nothing ever vested, so there is nothing to claim.
    assert_eq!(f.client.try_claim(), Err(Ok(Error::VestingCliffNotReached)));
    f.assert_conserved();
}

#[test]
fn revoke_before_the_cliff_then_advance_time_and_claim() {
    let f = Fixture::new(true);
    // Revoke before the cliff.
    f.client.revoke();
    // Advance time past the cliff -- but revoked_at freezes effective time.
    f.at(START + CLIFF + 1_000);
    // After revocation before the cliff, claim still reports CliffNotReached
    // because effective time is frozen at revoked_at (before the cliff).
    // Nothing was ever vested, so nothing is claimable.
    assert_eq!(f.client.try_claim(), Err(Ok(Error::VestingCliffNotReached)));
    f.assert_conserved();
}

#[test]
fn revoke_after_the_end_returns_nothing() {
    let f = Fixture::new(true);
    f.at(START + DURATION + 500);
    assert_eq!(f.client.revoke(), 0);
    assert_eq!(f.client.claimable(), TOTAL);
    f.assert_conserved();
}

#[test]
fn revoke_is_rejected_on_a_non_revocable_grant() {
    let f = Fixture::new(false);
    f.at(START + 300);
    assert_eq!(f.client.try_revoke(), Err(Ok(Error::VestingNotRevocable)));
    assert_eq!(f.held(), TOTAL);
}

#[test]
fn revoke_rejects_a_second_call() {
    let f = Fixture::new(true);
    f.at(START + 300);
    f.client.revoke();
    assert_eq!(f.client.try_revoke(), Err(Ok(Error::VestingRevoked)));
}

#[test]
#[should_panic]
fn revoke_requires_the_grantors_authorization() {
    let f = Fixture::new(true);
    f.at(START + 300);
    f.env.set_auths(&[]);
    f.client.revoke();
}

// ------------------------------------------------------- conservation

#[test]
fn conservation_holds_across_claim_then_revoke_then_claim() {
    let f = Fixture::new(true);
    f.assert_conserved();

    f.at(START + CLIFF);
    f.client.claim();
    f.assert_conserved();

    f.at(START + 450);
    f.client.claim();
    f.assert_conserved();

    f.at(START + 700);
    f.client.revoke();
    f.assert_conserved();

    f.client.claim();
    f.assert_conserved();

    let g = f.client.get();
    assert_eq!(f.held(), 0);
    assert_eq!(g.claimed + g.returned, g.total);
    assert_eq!(
        f.token.balance(&f.beneficiary) + f.token.balance(&f.grantor),
        MINT
    );
}

#[test]
fn a_schedule_that_divides_badly_still_conserves() {
    // 7 into 3 -- deliberately lossy proportions.
    let f = Fixture::with_schedule(true, 0, 3);
    f.at(START + 1);
    f.client.claim();
    f.assert_conserved();
    f.at(START + 2);
    f.client.claim();
    f.assert_conserved();
    f.at(START + 3);
    f.client.claim();
    f.assert_conserved();
    assert_eq!(f.token.balance(&f.beneficiary), TOTAL);
    assert_eq!(f.held(), 0);
}