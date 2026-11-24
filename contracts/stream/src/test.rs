// Test fixtures do plain arithmetic on known-small constants; the checked-math
// rule is for contract code.
#![allow(clippy::arithmetic_side_effects)]

use soroban_sdk::{
    map,
    testutils::{
        storage::Instance as _, Address as _, Events as _, IssuerFlags, Ledger as _, MockAuth,
        MockAuthInvoke,
    },
    token::{StellarAssetClient, TokenClient},
    vec,
    xdr::{ScErrorCode, ScErrorType},
    Address, Env, Event, IntoVal, Symbol, Val,
};
use sororail_common::{storage::INSTANCE_BUMP, testutils::TestEnv, Error};

use crate::{
    contract::{StreamContract, StreamContractClient},
    events::{Cancelled, Created, Extended, ToppedUp, ToppedUpFrom, Withdrawn},
    types::Stream,
};

mod accrual;
mod cancel;
mod create;
mod events;
mod extend;
mod fee_on_transfer;
mod lifecycle;
mod properties;
mod top_up;
mod views;
mod withdraw;

const RATE: i128 = 100;
const START: u64 = 1_000_000;
const DURATION: u64 = 1_000;
const STOP: u64 = START + DURATION;
const DEPOSITED: i128 = RATE * DURATION as i128;
const MINT: i128 = DEPOSITED * 10;

struct Fixture<'a> {
    env: Env,
    client: StreamContractClient<'a>,
    token: TokenClient<'a>,
    sender: Address,
    recipient: Address,
    outsider: Address,
}

impl Fixture<'_> {
    fn new(cancellable: bool) -> Self {
        Fixture::with_span(cancellable, START, START, STOP)
    }

    /// Creates the stream while the ledger reads `now`, which need not equal
    /// the stream's `start` (e.g. a stream that starts in the future).
    fn with_span(cancellable: bool, now: u64, start: u64, stop: u64) -> Self {
        let te = TestEnv::at(now);
        let (token, sender) = te.make_token(MINT);
        let token_address = token.address.clone();
        let recipient = te.make_address();
        let outsider = te.make_address();
        let env = te.env().clone();

        let client = StreamContractClient::new(&env, &env.register(StreamContract, ()));
        client.create(
            &sender,
            &recipient,
            &token_address,
            &RATE,
            &start,
            &stop,
            &cancellable,
        );

        Fixture {
            token: TokenClient::new(&env, &token_address),
            env,
            client,
            sender,
            recipient,
            outsider,
        }
    }

    fn at(&self, ts: u64) {
        self.env.ledger().with_mut(|l| l.timestamp = ts);
    }

    fn held(&self) -> i128 {
        self.token.balance(&self.client.address)
    }

    /// The property the whole contract exists to preserve.
    fn assert_conserved(&self) {
        let s = self.client.get();
        assert_eq!(
            s.withdrawn + s.refunded + self.client.remaining(),
            s.deposited,
            "conservation violated: {s:?}"
        );
        assert_eq!(
            self.held(),
            self.client.remaining(),
            "held balance diverged from remaining: {s:?}"
        );
    }
}

/// A bare struct for testing accrual in isolation, with no contract or ledger.
fn bare(env: &Env) -> Stream {
    Stream {
        sender: Address::generate(env),
        recipient: Address::generate(env),
        token: Address::generate(env),
        rate_per_second: RATE,
        start: START,
        stop: STOP,
        cancellable: true,
        deposited: DEPOSITED,
        withdrawn: 0,
        refunded: 0,
        cancelled_at: None,
        rate_per_second_scaled: None,
        rate_scale: None,
    }
}

// ------------------------------------------------ accrual (pure, no ledger)

#[test]
fn nothing_accrues_before_start() {
    let env = Env::default();
    let s = bare(&env);
    assert_eq!(s.accrued_at(0), Ok(0));
    assert_eq!(s.accrued_at(START - 1), Ok(0));
    assert_eq!(s.accrued_at(START), Ok(0));
}

#[test]
fn accrual_is_linear_between_start_and_stop() {
    let env = Env::default();
    let s = bare(&env);
    assert_eq!(s.accrued_at(START + 1), Ok(RATE));
    assert_eq!(s.accrued_at(START + 250), Ok(RATE * 250));
    assert_eq!(s.accrued_at(START + DURATION / 2), Ok(DEPOSITED / 2));
}

#[test]
fn accrual_is_capped_at_stop() {
    let env = Env::default();
    let s = bare(&env);
    assert_eq!(s.accrued_at(STOP), Ok(DEPOSITED));
    assert_eq!(s.accrued_at(STOP + 1), Ok(DEPOSITED));
    assert_eq!(s.accrued_at(u64::MAX), Ok(DEPOSITED));
}

#[test]
fn accrual_halts_at_cancellation() {
    let env = Env::default();
    let mut s = bare(&env);
    s.cancelled_at = Some(START + 100);
    assert_eq!(s.accrued_at(START + 100), Ok(RATE * 100));
    // Time moving on does not accrue any more.
    assert_eq!(s.accrued_at(START + 500), Ok(RATE * 100));
    assert_eq!(s.accrued_at(STOP + 5_000), Ok(RATE * 100));
}

#[test]
fn a_one_second_stream_accrues_exactly_one_tick() {
    let env = Env::default();
    let mut s = bare(&env);
    s.stop = START + 1;
    s.deposited = RATE;
    assert_eq!(s.accrued_at(START), Ok(0));
    assert_eq!(s.accrued_at(START + 1), Ok(RATE));
    assert_eq!(s.accrued_at(START + 2), Ok(RATE));
}

#[test]
fn accrual_reports_overflow_rather_than_wrapping() {
    let env = Env::default();
    let mut s = bare(&env);
    s.rate_per_second = i128::MAX;
    s.deposited = i128::MAX;
    assert_eq!(s.accrued_at(START + 2), Err(Error::Overflow));
}

#[test]
fn accrual_is_exact_at_a_rate_and_elapsed_that_divide_badly() {
    // Unlike vesting's `total * elapsed / duration`, accrual never divides,
    // so there is no rounding direction to get wrong: it is always exactly
    // `rate * elapsed`, even for a rate/elapsed pair that would leave a
    // remainder under any proportional (divide-then-multiply) formulation.
    let env = Env::default();
    let mut s = bare(&env);
    s.rate_per_second = 7; // prime; divides unevenly into most spans
    s.stop = START + 3; // an odd, non-multiple-of-7 span
    s.deposited = 7 * 3;

    assert_eq!(s.accrued_at(START + 1), Ok(7));
    assert_eq!(s.accrued_at(START + 2), Ok(14));
    assert_eq!(s.accrued_at(START + 3), Ok(21));
    // Capped at `stop`, not truncated to some lower rounded figure.
    assert_eq!(s.accrued_at(START + 3), Ok(s.deposited));
}

/// Documents that `available_at` degrades to `Err(Underflow)` if `withdrawn`
/// ever exceeds accrued (e.g. a future code path bug). Today's `withdraw`
/// rejects `requested > available`, so this state is unreachable through the
/// contract entry points; the pure helper must still not wrap to a negative
/// `i128` if it is reached.
#[test]
fn available_at_errors_if_withdrawn_exceeds_accrued() {
    let env = Env::default();
    let mut s = bare(&env);
    // Accrued at START+10 is RATE*10; force withdrawn past that.
    s.withdrawn = RATE * 10 + 1;
    assert_eq!(
        s.available_at(START + 10),
        Err(Error::Underflow),
        "available_at must error rather than wrap when withdrawn > accrued"
    );
    // Even after full accrual, an over-withdrawn book still errors.
    s.withdrawn = DEPOSITED + 1;
    assert_eq!(s.available_at(STOP), Err(Error::Underflow));
}

// ------------------------------------------------------------------ create

#[test]
fn create_funds_the_whole_span_up_front() {
    let f = Fixture::new(true);
    assert_eq!(f.held(), DEPOSITED);
    assert_eq!(f.token.balance(&f.sender), MINT - DEPOSITED);
    let s = f.client.get();
    assert_eq!(s.deposited, DEPOSITED);
    assert_eq!(s.withdrawn, 0);
    assert_eq!(s.refunded, 0);
    assert_eq!(s.cancelled_at, None);
    f.assert_conserved();
}

/// Pins Created topics and data fields (SPEC.md `indexed_events`).
#[test]
fn create_emits_the_created_event() {
    let f = Fixture::new(true);
    // `create` emits the token's own transfer event before its own
    // `Created`, so take the last event rather than assuming an index.
    let all = f.env.events().all();
    let events = all.events();
    let last = events.last().expect("no events emitted");
    let expected = Created {
        sender: f.sender.clone(),
        recipient: f.recipient.clone(),
        token: f.token.address.clone(),
        rate_per_second: RATE,
        start: START,
        stop: STOP,
        cancellable: true,
        deposited: DEPOSITED,
    };
    assert_eq!(last, &expected.to_xdr(&f.env, &f.client.address));
}

#[test]
fn create_emits_the_created_event_with_rate_and_span() {
    let f = Fixture::new(true);
    let all = f.env.events().all();
    let events = all.events();
    let last = events.last().expect("no events emitted");
    let expected = Created {
        sender: f.sender.clone(),
        recipient: f.recipient.clone(),
        token: f.token.address.clone(),
        rate_per_second: RATE,
        start: START,
        stop: STOP,
        cancellable: true,
        deposited: DEPOSITED,
    };
    assert_eq!(last, &expected.to_xdr(&f.env, &f.client.address));
    // The event's rate and span describe the stream that was stored.
    let s = f.client.get();
    assert_eq!(s.rate_per_second, RATE);
    assert_eq!(s.stop - s.start, DURATION);
}

#[test]
fn create_rejects_a_non_positive_rate() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START);
    let c = StreamContractClient::new(&env, &env.register(StreamContract, ()));
    let (a, b, t) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );
    for bad in [0i128, -1, i128::MIN] {
        assert_eq!(
            c.try_create(&a, &b, &t, &bad, &START, &STOP, &true),
            Err(Ok(Error::InvalidAmount)),
            "rate {bad} was accepted"
        );
    }
}

#[test]
fn create_rejects_an_empty_or_inverted_span() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START);
    let c = StreamContractClient::new(&env, &env.register(StreamContract, ()));
    let (a, b, t) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );
    for bad_stop in [START, START - 1, 0] {
        assert_eq!(
            c.try_create(&a, &b, &t, &RATE, &START, &bad_stop, &true),
            Err(Ok(Error::InvalidTimeRange)),
            "stop {bad_stop} was accepted"
        );
    }
}

#[test]
fn create_rejects_a_stop_before_start() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START);
    let c = StreamContractClient::new(&env, &env.register(StreamContract, ()));
    let (a, b, t) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );
    assert_eq!(
        c.try_create(&a, &b, &t, &RATE, &START, &(START - 1), &true),
        Err(Ok(Error::InvalidTimeRange))
    );
    // The rejected call must not have persisted a stream.
    assert_eq!(c.try_get(), Err(Ok(Error::NotInitialized)));
}

#[test]
fn create_rejects_a_zero_length_span() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START);
    let c = StreamContractClient::new(&env, &env.register(StreamContract, ()));
    let (a, b, t) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );
    assert_eq!(
        c.try_create(&a, &b, &t, &RATE, &START, &START, &true),
        Err(Ok(Error::InvalidTimeRange))
    );
    // The rejected call must not have persisted a stream.
    assert_eq!(c.try_get(), Err(Ok(Error::NotInitialized)));
}

#[test]
fn create_rejects_funding_that_overflows() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START);
    let c = StreamContractClient::new(&env, &env.register(StreamContract, ()));
    let (a, b, t) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );
    assert_eq!(
        c.try_create(&a, &b, &t, &i128::MAX, &START, &(START + 2), &true),
        Err(Ok(Error::Overflow))
    );
}

#[test]
fn create_allows_the_exact_max_funding_boundary() {
    // `rate_per_second * (stop - start) == i128::MAX` is the largest a stream
    // can be funded: it must succeed. `i128::MAX` is prime (Mersenne prime
    // M127), so the only single-second-span way to land exactly on it is
    // `rate = i128::MAX`, `duration = 1`; one more second overflows (asserted
    // just above, in `create_rejects_funding_that_overflows`).
    let te = TestEnv::at(START);
    let (token_client, sender) = te.make_token(i128::MAX);
    let token = token_client.address.clone();
    let recipient = te.make_address();
    let env = te.env().clone();
    let c = StreamContractClient::new(&env, &env.register(StreamContract, ()));

    c.create(
        &sender,
        &recipient,
        &token,
        &i128::MAX,
        &START,
        &(START + 1),
        &true,
    );

    let s = c.get();
    assert_eq!(s.rate_per_second, i128::MAX);
    assert_eq!(s.stop - s.start, 1);
    assert_eq!(s.deposited, i128::MAX);
    assert_eq!(s.withdrawn, 0);
    assert_eq!(c.remaining(), i128::MAX);
    assert_eq!(token_client.balance(&c.address), i128::MAX);
    assert_eq!(token_client.balance(&sender), 0);
}

#[test]
fn create_rejects_a_second_call() {
    let f = Fixture::new(true);
    assert_eq!(
        f.client.try_create(
            &f.sender,
            &f.recipient,
            &f.token.address,
            &RATE,
            &START,
            &STOP,
            &true
        ),
        Err(Ok(Error::AlreadyInitialized))
    );
}

#[test]
fn create_rejects_identical_sender_and_recipient() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START);
    let c = StreamContractClient::new(&env, &env.register(StreamContract, ()));
    let party = Address::generate(&env);
    let token = Address::generate(&env);
    assert_eq!(
        c.try_create(&party, &party, &token, &RATE, &START, &STOP, &true),
        Err(Ok(Error::IdenticalParties))
    );
}

/// A self-stream is refused before any funds move, and leaves no stream behind.
#[test]
fn self_stream_is_refused_without_moving_funds() {
    let te = TestEnv::at(START);
    let (token, sender) = te.make_token(MINT);
    let env = te.env().clone();
    let client = StreamContractClient::new(&env, &env.register(StreamContract, ()));
    assert_eq!(
        client.try_create(
            &sender,
            &sender,
            &token.address,
            &RATE,
            &START,
            &STOP,
            &true
        ),
        Err(Ok(Error::IdenticalParties))
    );
    assert_eq!(token.balance(&sender), MINT);
    assert_eq!(token.balance(&client.address), 0);
    assert_eq!(client.try_get(), Err(Ok(Error::NotInitialized)));
}

/// `create` saves the stream before pulling funds. If the token transfer
/// fails, the host reverts the whole invocation 鈥� including that save 鈥� so a
/// later `get` still sees `NotInitialized`.
#[test]
fn create_reverts_cleanly_when_sender_cannot_fund() {
    let te = TestEnv::at(START);
    // Mint far less than the deposit the span requires.
    let (token_client, sender) = te.make_token(RATE);
    let token = token_client.address.clone();
    let recipient = te.make_address();
    let env = te.env().clone();
    let c = StreamContractClient::new(&env, &env.register(StreamContract, ()));

    assert!(c
        .try_create(&sender, &recipient, &token, &RATE, &START, &STOP, &true)
        .is_err());
    assert_eq!(c.try_get(), Err(Ok(Error::NotInitialized)));
    assert_eq!(token_client.balance(&c.address), 0);
    assert_eq!(token_client.balance(&sender), RATE);
}
#[test]
fn entry_points_error_before_create() {
    let env = Env::default();
    env.mock_all_auths();
    let c = StreamContractClient::new(&env, &env.register(StreamContract, ()));
    let who = Address::generate(&env);
    assert_eq!(c.try_withdraw(&None), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_cancel(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_top_up(&RATE), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_extend(&STOP), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_balance_of(&who), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_remaining(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_bump(), Err(Ok(Error::NotInitialized)));
    assert_eq!(c.try_get(), Err(Ok(Error::NotInitialized)));
}

#[test]
fn create_with_contract_as_recipient_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START);
    let sender = Address::generate(&env);
    let issuer = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(issuer).address();
    let contract_id = env.register(StreamContract, ());
    let client = StreamContractClient::new(&env, &contract_id);

    StellarAssetClient::new(&env, &token).mint(&sender, &(RATE * 1000));

    let result = client.try_create(&sender, &contract_id, &token, &RATE, &START, &STOP, &true);
    assert!(
        result.is_ok(),
        "create with contract as recipient should succeed"
    );
}

#[test]
fn create_rejects_multiple_invalid_args_in_order() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let issuer = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(issuer).address();
    let funding = RATE * i128::from(STOP - START);
    StellarAssetClient::new(&env, &token).mint(&sender, &funding);
    let contract_id = env.register(StreamContract, ());
    let client = StreamContractClient::new(&env, &contract_id);

    // AlreadyInitialized comes before InvalidTimeRange
    client.create(&sender, &recipient, &token, &RATE, &START, &STOP, &true);
    assert_eq!(
        client.try_create(&sender, &recipient, &token, &RATE, &START, &START, &true),
        Err(Ok(Error::AlreadyInitialized))
    );

    // IdenticalParties check comes before rate validation
    let contract_id2 = env.register(StreamContract, ());
    let client2 = StreamContractClient::new(&env, &contract_id2);
    assert_eq!(
        client2.try_create(&sender, &sender, &token, &0, &START, &STOP, &true),
        Err(Ok(Error::IdenticalParties))
    );

    // InvalidTimeRange check comes before overflow check
    let contract_id3 = env.register(StreamContract, ());
    let client3 = StreamContractClient::new(&env, &contract_id3);
    assert_eq!(
        client3.try_create(&sender, &recipient, &token, &RATE, &STOP, &START, &true),
        Err(Ok(Error::InvalidTimeRange))
    );
}

#[test]
#[should_panic]
fn create_requires_the_senders_authorization() {
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = START);
    let c = StreamContractClient::new(&env, &env.register(StreamContract, ()));
    c.create(
        &Address::generate(&env),
        &Address::generate(&env),
        &Address::generate(&env),
        &RATE,
        &START,
        &STOP,
        &true,
    );
}

// ---------------------------------------------------------------- withdraw

#[test]
fn withdraw_none_takes_everything_available() {
    let f = Fixture::new(true);
    f.at(START + 300);
    assert_eq!(f.client.withdraw(&None), RATE * 300);
    assert_eq!(f.token.balance(&f.recipient), RATE * 300);
    f.assert_conserved();
}

/// Issue #232: `withdraw(None)` with no unwithdrawn accrual left is rejected;
/// the None path must not silently no-op.
#[test]
fn withdraw_none_drains_accrued_then_reports_nothing_left() {
    let f = Fixture::new(true);
    f.at(START + 250);
    assert_eq!(f.client.withdraw(&None), RATE * 250);
    assert_eq!(f.token.balance(&f.recipient), RATE * 250);
    assert_eq!(f.client.balance_of(&f.recipient), 0);
    assert_eq!(f.client.remaining(), DEPOSITED - RATE * 250);
    f.assert_conserved();

    // In the same instant nothing new accrued, so a second None has nothing
    // to take and errors rather than transferring zero.
    assert_eq!(f.client.try_withdraw(&None), Err(Ok(Error::InvalidAmount)));
    assert_eq!(f.client.remaining(), DEPOSITED - RATE * 250);
    f.assert_conserved();
}

#[test]
fn withdraw_event_amount_matches_the_token_transfer() {
    let f = Fixture::new(true);
    f.at(START + 300);
    let recipient_before = f.token.balance(&f.recipient);
    let withdrawn = f.client.withdraw(&Some(RATE * 100));
    // Capture immediately: subsequent token balance invocations replace the
    // events returned by `events().all()`.
    let all = f.env.events().all();
    let transferred = f.token.balance(&f.recipient) - recipient_before;

    assert_eq!(transferred, RATE * 100);
    assert_eq!(withdrawn, transferred);

    // `withdraw` emits the token's own transfer event before its own
    // `Withdrawn`, so take the last event rather than assuming an index.
    let events = all.events();
    let last = events.last().expect("no events emitted");
    let expected = Withdrawn {
        recipient: f.recipient.clone(),
        amount: transferred,
        total_withdrawn: f.client.get().withdrawn,
        // Plain `withdraw` pays the recipient itself.
        destination: f.recipient.clone(),
    };
    assert_eq!(last, &expected.to_xdr(&f.env, &f.client.address));
}

#[test]
fn withdraw_takes_a_partial_amount() {
    let f = Fixture::new(true);
    f.at(START + 300);
    assert_eq!(f.client.withdraw(&Some(RATE * 100)), RATE * 100);
    assert_eq!(f.client.balance_of(&f.recipient), RATE * 200);
    f.assert_conserved();
}

#[test]
fn withdraw_rejects_more_than_has_accrued() {
    let f = Fixture::new(true);
    f.at(START + 300);

    let available_before = f.client.balance_of(&f.recipient);
    let stream_before = f.client.get();
    let held_before = f.held();
    let recipient_balance_before = f.token.balance(&f.recipient);
    let sender_balance_before = f.token.balance(&f.sender);

    assert_eq!(available_before, RATE * 300);
    assert_eq!(
        f.client.try_withdraw(&Some(available_before + 1)),
        Err(Ok(Error::StreamInsufficientAccrued))
    );

    // A rejected withdrawal must not change accounting or transfer any tokens.
    assert_eq!(f.client.get(), stream_before);
    assert_eq!(f.client.balance_of(&f.recipient), available_before);
    assert_eq!(f.client.remaining(), held_before);
    assert_eq!(f.held(), held_before);
    assert_eq!(f.token.balance(&f.recipient), recipient_balance_before);
    assert_eq!(f.token.balance(&f.sender), sender_balance_before);
    f.assert_conserved();

    // The same accrued balance must remain withdrawable after the failed call.
    assert_eq!(f.client.withdraw(&Some(available_before)), available_before);
    assert_eq!(
        f.token.balance(&f.recipient),
        recipient_balance_before + available_before
    );
    f.assert_conserved();
}

/// Issue #439: a SAC with clawback and authorization flags enabled can revoke
/// the recipient mid-stream. The failed token transfer must not corrupt the
/// stream: balances and accounting are unchanged and the accrued amount stays
/// withdrawable.
///
/// Recovery: the token admin calls `set_authorized` with `true` again, after
/// which `withdraw` succeeds. If authorization cannot be restored, the sender's
/// `cancel` settles the accrued amount to the recipient and refunds the rest.
#[test]
fn withdraw_fails_without_state_change_when_recipient_is_deauthorized() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = START);

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = sac.address();
    let sac_client = StellarAssetClient::new(&env, &token);
    sac.issuer().set_flag(IssuerFlags::RequiredFlag);
    sac.issuer().set_flag(IssuerFlags::RevocableFlag);
    sac.issuer().set_flag(IssuerFlags::ClawbackEnabledFlag);

    let contract_id = env.register(StreamContract, ());
    let client = StreamContractClient::new(&env, &contract_id);

    // AUTH_REQUIRED makes every contract address unauthorized by default, so
    // the sender, the stream contract and the recipient must all be authorized
    // before any balance can be received or spent.
    sac_client.set_authorized(&sender, &true);
    sac_client.set_authorized(&contract_id, &true);
    sac_client.set_authorized(&recipient, &true);
    sac_client.mint(&sender, &MINT);

    client.create(&sender, &recipient, &token, &RATE, &START, &STOP, &true);
    let token_client = TokenClient::new(&env, &token);
    assert_eq!(token_client.balance(&contract_id), DEPOSITED);

    env.ledger().with_mut(|l| l.timestamp = START + 300);
    let available_before = client.balance_of(&recipient);
    assert_eq!(available_before, RATE * 300);
    let stream_before = client.get();
    let held_before = token_client.balance(&contract_id);
    let recipient_before = token_client.balance(&recipient);
    let sender_before = token_client.balance(&sender);

    // Revoke the recipient mid-stream; AUTH_REVOCABLE makes this legal.
    sac_client.set_authorized(&recipient, &false);
    assert!(!sac_client.authorized(&recipient));

    // The token rejects the incoming transfer, so withdraw fails as a host
    // error (not a typed contract error) and the host rolls back every write.
    assert!(matches!(client.try_withdraw(&None), Err(Err(_))));
    assert_eq!(client.get(), stream_before);
    assert_eq!(client.balance_of(&recipient), available_before);
    assert_eq!(client.remaining(), held_before);
    assert_eq!(token_client.balance(&contract_id), held_before);
    assert_eq!(token_client.balance(&recipient), recipient_before);
    assert_eq!(token_client.balance(&sender), sender_before);

    // Recovery: re-authorizing the recipient restores the same withdrawable
    // amount; nothing was lost while the balance was frozen.
    sac_client.set_authorized(&recipient, &true);
    assert!(sac_client.authorized(&recipient));
    assert_eq!(client.withdraw(&None), available_before);
    assert_eq!(
        token_client.balance(&recipient),
        recipient_before + available_before
    );
    assert_eq!(
        token_client.balance(&contract_id),
        held_before - available_before
    );
    assert_eq!(client.remaining(), held_before - available_before);
    assert_eq!(client.get().withdrawn, available_before);
}

#[test]
fn withdraw_rejects_when_nothing_has_accrued() {
    let f = Fixture::new(true);
    // Still at `start`; nothing accrued.
    assert_eq!(f.client.try_withdraw(&None), Err(Ok(Error::InvalidAmount)));
    assert_eq!(
        f.client.try_withdraw(&Some(0)),
        Err(Ok(Error::InvalidAmount))
    );
    assert_eq!(
        f.client.try_withdraw(&Some(-1)),
        Err(Ok(Error::InvalidAmount))
    );
}

#[test]
fn withdraw_some_zero_distinctly_from_none_with_zero_accrued() {
    let f = Fixture::new(true);
    // Explicit Some(0) flows through unwrap_or untouched into require_positive.
    // Both None and Some(0) should fail with InvalidAmount when nothing accrued.
    let none_result = f.client.try_withdraw(&None);
    let some_zero_result = f.client.try_withdraw(&Some(0));
    assert_eq!(none_result, Err(Ok(Error::InvalidAmount)));
    assert_eq!(some_zero_result, Err(Ok(Error::InvalidAmount)));
}

/// `None` and `Some(0)` reach the same `InvalidAmount` error from *different*
/// paths even once the stream has accrued: `None` substitutes the (positive)
/// available amount and would succeed, while `Some(0)` is passed straight to
/// `require_positive` and must be rejected on its own. Zero accrued is not the
/// only case that distinguishes them.
#[test]
fn withdraw_some_zero_fails_even_when_accrual_is_positive() {
    let f = Fixture::new(true);
    f.at(START + 300);

    // Sanity: with positive accrual, `None` withdraws everything.
    let accrued = f.client.balance_of(&f.recipient);
    assert!(accrued > 0);

    // `Some(0)` is validated directly and rejected regardless of accrual.
    assert_eq!(
        f.client.try_withdraw(&Some(0)),
        Err(Ok(Error::InvalidAmount))
    );
    assert_eq!(f.client.balance_of(&f.recipient), accrued);

    // `None`, by contrast, succeeds and drains exactly the accrued amount.
    assert_eq!(f.client.withdraw(&None), accrued);
    assert_eq!(f.client.balance_of(&f.recipient), 0);
}

#[test]
fn withdrawing_the_whole_stream_after_stop_empties_the_contract() {
    let f = Fixture::new(true);
    f.at(STOP + 10_000);
    assert_eq!(f.client.withdraw(&None), DEPOSITED);
    assert_eq!(f.held(), 0);
    assert_eq!(f.client.remaining(), 0);
    f.assert_conserved();
}

#[test]
fn repeated_withdrawals_never_exceed_the_deposit() {
    let f = Fixture::new(true);
    for step in 1..=10u64 {
        f.at(START + step * 100);
        f.client.withdraw(&None);
        f.assert_conserved();
    }
    f.at(STOP + 1);
    assert_eq!(f.client.get().withdrawn, DEPOSITED);
    assert_eq!(f.held(), 0);
}

#[test]
#[should_panic]
fn withdraw_requires_the_recipients_authorization() {
    let te = TestEnv::at(START);
    let (token_client, sender) = te.make_token(MINT);
    let token = token_client.address.clone();
    let recipient = te.make_address();
    let env = te.env().clone();
    let c = StreamContractClient::new(&env, &env.register(StreamContract, ()));
    c.create(&sender, &recipient, &token, &RATE, &START, &STOP, &true);

    // Drop the mocks, then withdraw with nobody authorized.
    env.set_auths(&[]);
    te.advance_by(100);
    c.withdraw(&None);
}

// ------------------------------------------------------------------ cancel

#[test]
fn cancel_settles_the_recipient_and_refunds_the_sender() {
    let f = Fixture::new(true);
    let sender_before = f.token.balance(&f.sender);
    f.at(START + 400);
    f.client.cancel();

    assert_eq!(f.token.balance(&f.recipient), RATE * 400);
    assert_eq!(
        f.token.balance(&f.sender),
        sender_before + DEPOSITED - RATE * 400
    );
    assert_eq!(f.held(), 0);
    assert_eq!(f.client.remaining(), 0);
    f.assert_conserved();
}

/// Issue #235: `cancel` splits the deposit exactly — the accrued portion is
/// settled to the recipient and the unaccrued remainder is refunded to the
/// sender, with the recorded totals matching the transfers.
#[test]
fn cancel_splits_accrued_to_recipient_and_unaccrued_to_sender() {
    let f = Fixture::new(true);
    let sender_before = f.token.balance(&f.sender);
    f.at(START + 600);
    f.client.cancel();

    let accrued = RATE * 600;
    let unaccrued = DEPOSITED - accrued;
    assert_eq!(f.token.balance(&f.recipient), accrued);
    assert_eq!(f.token.balance(&f.sender), sender_before + unaccrued);

    let s = f.client.get();
    assert_eq!(s.withdrawn, accrued);
    assert_eq!(s.refunded, unaccrued);
    assert_eq!(f.client.remaining(), 0);
    assert_eq!(f.held(), 0);
    f.assert_conserved();
}

#[test]
fn cancel_emits_the_cancelled_event() {
    let f = Fixture::new(true);
    f.at(START + 400);
    f.client.cancel();

    // `cancel` pays out up to two token transfers before its own
    // `Cancelled`, so take the last event rather than assuming an index.
    let all = f.env.events().all();
    let events = all.events();
    let last = events.last().expect("no events emitted");
    let expected = Cancelled {
        sender: f.sender.clone(),
        settled_to_recipient: RATE * 400,
        refunded_to_sender: DEPOSITED - RATE * 400,
        cancelled_at: START + 400,
    };
    assert_eq!(last, &expected.to_xdr(&f.env, &f.client.address));
}

#[test]
fn cancel_is_rejected_when_the_stream_is_not_cancellable() {
    let f = Fixture::new(false);
    f.at(START + 400);
    assert_eq!(f.client.try_cancel(), Err(Ok(Error::StreamNotCancellable)));
    assert_eq!(f.held(), DEPOSITED);
}

#[test]
fn cancel_rejects_a_second_call() {
    let f = Fixture::new(true);
    f.at(START + 400);
    f.client.cancel();
    assert_eq!(f.client.try_cancel(), Err(Ok(Error::StreamCancelled)));
}

/// Only the sender may cancel: a caller authorizing as anyone else hits the
/// host's auth check, not a contract error, and the stream is left intact.
///
/// The host narrows every non-`Contract` host error to
/// `Context`/`InvalidAction` before it crosses back to the guest, so a failed
/// `require_auth` surfaces as that pair rather than `Auth`/`InvalidAction`.
#[test]
fn cancel_is_rejected_for_a_non_sender_caller() {
    let f = Fixture::new(true);
    f.at(START + 400);
    let held = f.held();
    let sender_balance = f.token.balance(&f.sender);
    let recipient_balance = f.token.balance(&f.recipient);

    for caller in [&f.outsider, &f.recipient] {
        f.env.mock_auths(&[MockAuth {
            address: caller,
            invoke: &MockAuthInvoke {
                contract: &f.client.address,
                fn_name: "cancel",
                args: ().into_val(&f.env),
                sub_invokes: &[],
            },
        }]);
        let res = f.env.try_invoke_contract::<(), soroban_sdk::Error>(
            &f.client.address,
            &Symbol::new(&f.env, "cancel"),
            vec![&f.env],
        );
        assert_eq!(
            res,
            Err(Ok(soroban_sdk::Error::from_type_and_code(
                ScErrorType::Context,
                ScErrorCode::InvalidAction,
            )))
        );
    }

    assert_eq!(f.client.get().cancelled_at, None);
    assert_eq!(f.held(), held);
    assert_eq!(f.token.balance(&f.sender), sender_balance);
    assert_eq!(f.token.balance(&f.recipient), recipient_balance);
}

#[test]
fn second_cancel_errors_without_moving_tokens() {
    let f = Fixture::new(true);
    f.at(START + 400);
    f.client.cancel();
    let after_first = f.client.get();
    let held = f.held();
    let sender_balance = f.token.balance(&f.sender);
    let recipient_balance = f.token.balance(&f.recipient);

    f.at(START + 800);
    assert_eq!(f.client.try_cancel(), Err(Ok(Error::StreamCancelled)));

    assert_eq!(f.client.get(), after_first);
    assert_eq!(f.held(), held);
    assert_eq!(f.token.balance(&f.sender), sender_balance);
    assert_eq!(f.token.balance(&f.recipient), recipient_balance);
}

#[test]
fn cancel_after_stop_refunds_nothing_and_settles_everything() {
    let f = Fixture::new(true);
    f.at(STOP + 500);
    f.client.cancel();
    assert_eq!(f.token.balance(&f.recipient), DEPOSITED);
    assert_eq!(f.client.get().refunded, 0);
    f.assert_conserved();
}

#[test]
fn cancel_after_stop_with_prior_withdrawal_refunds_nothing_and_pays_the_remainder() {
    let f = Fixture::new(true);
    f.at(START + 400);
    assert_eq!(f.client.withdraw(&None), RATE * 400);

    f.at(STOP + 500);
    f.client.cancel();

    let stream = f.client.get();
    assert_eq!(f.token.balance(&f.recipient), DEPOSITED);
    assert_eq!(stream.withdrawn, DEPOSITED);
    assert_eq!(stream.refunded, 0);
    assert_eq!(f.held(), 0);
    f.assert_conserved();
}

#[test]
fn cancel_before_start_refunds_the_whole_deposit() {
    let f = Fixture::new(true);
    let sender_before = f.token.balance(&f.sender);
    // Still at `start`; nothing has accrued.
    f.client.cancel();
    assert_eq!(f.token.balance(&f.recipient), 0);
    assert_eq!(f.token.balance(&f.sender), sender_before + DEPOSITED);
    f.assert_conserved();
}

#[test]
fn cancel_settles_only_what_was_not_already_withdrawn() {
    let f = Fixture::new(true);
    f.at(START + 200);
    f.client.withdraw(&None);
    f.at(START + 500);
    f.client.cancel();

    assert_eq!(f.token.balance(&f.recipient), RATE * 500);
    assert_eq!(f.client.get().withdrawn, RATE * 500);
    f.assert_conserved();
}

#[test]
fn nothing_accrues_after_cancellation() {
    let f = Fixture::new(true);
    f.at(START + 300);
    f.client.cancel();
    f.at(STOP + 10_000);
    assert_eq!(f.client.balance_of(&f.recipient), 0);
    assert_eq!(f.client.try_withdraw(&None), Err(Ok(Error::InvalidAmount)));
    f.assert_conserved();
}

// ----------------------------------------------------------------- top_up

#[test]
fn top_up_extends_the_stop_by_the_span_it_buys() {
    let f = Fixture::new(true);
    f.client.top_up(&(RATE * 500));
    let s = f.client.get();
    assert_eq!(s.stop, STOP + 500);
    assert_eq!(s.deposited, DEPOSITED + RATE * 500);
    assert_eq!(f.held(), DEPOSITED + RATE * 500);
    f.assert_conserved();
}

/// Pins the wire shape indexers decode (SPEC.md `indexed_events`). The
/// expected value is spelled out literally rather than built from
/// `events::ToppedUp`, so renaming a topic or a field fails here.
#[test]
fn top_up_emits_the_topped_up_event() {
    let f = Fixture::new(true);
    f.client.top_up(&(RATE * 500));

    let env = &f.env;
    let amount: Val = (RATE * 500).into_val(env);
    let deposited: Val = (DEPOSITED + RATE * 500).into_val(env);
    let new_stop: Val = (STOP + 500).into_val(env);
    // Map data: keys are the field names, serialized in sorted order.
    let data: Val = map![
        env,
        (Symbol::new(env, "amount"), amount),
        (Symbol::new(env, "deposited"), deposited),
        (Symbol::new(env, "new_stop"), new_stop),
    ]
    .into_val(env);
    assert_eq!(
        env.events().all().filter_by_contract(&f.client.address),
        vec![
            env,
            (
                f.client.address.clone(),
                vec![
                    env,
                    Symbol::new(env, "stream").into_val(env),
                    Symbol::new(env, "topped_up").into_val(env),
                    f.sender.into_val(env),
                ],
                data,
            ),
        ]
    );
}

/// Across repeated top-ups the event carries only the amount just added,
/// while `deposited` and `new_stop` report the cumulative totals.
#[test]
fn top_up_event_carries_the_added_amount_and_the_new_total() {
    let f = Fixture::new(true);
    f.client.top_up(&(RATE * 500));
    f.client.top_up(&(RATE * 250));

    // `top_up` emits the token's transfer event before its own `ToppedUp`,
    // so take the last event rather than assuming an index.
    let all = f.env.events().all();
    let events = all.events();
    let last = events.last().expect("no events emitted");
    let expected = ToppedUp {
        sender: f.sender.clone(),
        amount: RATE * 250,
        new_stop: STOP + 750,
        deposited: DEPOSITED + RATE * 750,
    };
    assert_eq!(last, &expected.to_xdr(&f.env, &f.client.address));
    assert_eq!(f.client.get().deposited, DEPOSITED + RATE * 750);
}

#[test]
fn top_up_rejects_an_amount_that_is_not_a_whole_number_of_seconds() {
    let f = Fixture::new(true);
    assert_eq!(
        f.client.try_top_up(&(RATE + 1)),
        Err(Ok(Error::InvalidAmount))
    );
    assert_eq!(f.client.try_top_up(&1), Err(Ok(Error::InvalidAmount)));
}

#[test]
fn top_up_rejects_non_positive_amounts() {
    let f = Fixture::new(true);
    assert_eq!(f.client.try_top_up(&0), Err(Ok(Error::InvalidAmount)));
    assert_eq!(f.client.try_top_up(&(-RATE)), Err(Ok(Error::InvalidAmount)));
}

#[test]
fn top_up_rejects_a_span_that_overflows_the_stop_timestamp() {
    // rate = 1, duration = 1 keeps `deposited` tiny, so this isolates the
    // `amount / rate` -> u64 conversion: `2^64` seconds is beyond `u64::MAX`,
    // so extending `stop` by it could not even be represented. The contract
    // must reject rather than silently truncate the seconds (which would
    // break the funding invariant).
    let te = TestEnv::at(START);
    let (token_client, sender) = te.make_token(i128::MAX);
    let token = token_client.address.clone();
    let recipient = te.make_address();
    let env = te.env().clone();
    let c = StreamContractClient::new(&env, &env.register(StreamContract, ()));
    c.create(&sender, &recipient, &token, &1, &START, &(START + 1), &true);

    assert_eq!(
        c.try_top_up(&(1_i128 << 64)),
        Err(Ok(Error::InvalidTimeRange))
    );

    // The failed call changed nothing.
    let s = c.get();
    assert_eq!(s.stop, START + 1);
    assert_eq!(s.deposited, 1);
}

#[test]
fn top_up_rejects_seconds_that_fit_u64_but_overflow_the_stop_timestamp() {
    // rate = 1, so `amount` is exactly the seconds added. Unlike the `2^64`
    // case above, `seconds` here is a valid `u64`, so the `u64::try_from` step
    // passes and only the `stop.checked_add(seconds)` step can reject it:
    // `(START + 1) + (u64::MAX - START)` is `u64::MAX + 1`.
    let te = TestEnv::at(START);
    let (token_client, sender) = te.make_token(i128::MAX);
    let token = token_client.address.clone();
    let recipient = te.make_address();
    let env = te.env().clone();
    let c = StreamContractClient::new(&env, &env.register(StreamContract, ()));
    c.create(&sender, &recipient, &token, &1, &START, &(START + 1), &true);

    assert_eq!(
        c.try_top_up(&i128::from(u64::MAX - START)),
        Err(Ok(Error::InvalidTimeRange))
    );

    // The failed call changed nothing.
    let s = c.get();
    assert_eq!(s.stop, START + 1);
    assert_eq!(s.deposited, 1);

    // One second less lands exactly on `u64::MAX`, the largest `stop` that
    // can be represented, and must be accepted.
    c.top_up(&i128::from(u64::MAX - START - 1));
    assert_eq!(c.get().stop, u64::MAX);
}

#[test]
fn top_up_rejects_a_deposit_total_that_overflows() {
    // rate = i128::MAX / 2 over a 1-second span funds `rate`. Topping up
    // 2 seconds' worth (`2 * rate == i128::MAX - 1`) is itself representable and
    // a whole number of seconds, but `deposited + amount` is not.
    let rate = i128::MAX / 2;
    let te = TestEnv::at(START);
    let (token_client, sender) = te.make_token(i128::MAX);
    let token = token_client.address.clone();
    let recipient = te.make_address();
    let env = te.env().clone();
    let c = StreamContractClient::new(&env, &env.register(StreamContract, ()));
    c.create(
        &sender,
        &recipient,
        &token,
        &rate,
        &START,
        &(START + 1),
        &true,
    );

    assert_eq!(c.try_top_up(&(rate * 2)), Err(Ok(Error::Overflow)));

    // The failed call changed nothing.
    let s = c.get();
    assert_eq!(s.stop, START + 1);
    assert_eq!(s.deposited, rate);
}

#[test]
fn top_up_is_rejected_after_cancellation() {
    let f = Fixture::new(true);
    f.client.cancel();
    assert_eq!(
        f.client.try_top_up(&(RATE * 10)),
        Err(Ok(Error::StreamCancelled))
    );
}

#[test]
fn top_up_after_cancellation_moves_no_tokens_and_leaves_the_stream_unchanged() {
    let f = Fixture::new(true);
    f.at(START + 400);
    f.client.cancel();
    let before = f.client.get();
    let held = f.held();
    let sender_balance = f.token.balance(&f.sender);

    assert_eq!(
        f.client.try_top_up(&(RATE * 100)),
        Err(Ok(Error::StreamCancelled))
    );

    assert_eq!(f.client.get(), before);
    assert_eq!(f.held(), held);
    assert_eq!(f.token.balance(&f.sender), sender_balance);
}

// ------------------------------------------------------------- top_up_from

/// Mints `amount` of the fixture's token to a fresh address that is neither
/// the sender nor the recipient, and returns it.
fn funded_third_party(f: &Fixture<'_>, amount: i128) -> Address {
    let funder = Address::generate(&f.env);
    StellarAssetClient::new(&f.env, &f.token.address).mint(&funder, &amount);
    funder
}

#[test]
fn top_up_from_funds_the_stream_from_a_third_party() {
    let f = Fixture::new(true);
    let funder = funded_third_party(&f, RATE * 500);
    let sender_before = f.token.balance(&f.sender);

    f.client.top_up_from(&funder, &(RATE * 500));

    let s = f.client.get();
    assert_eq!(s.stop, STOP + 500);
    assert_eq!(s.deposited, DEPOSITED + RATE * 500);
    assert_eq!(f.token.balance(&funder), 0);
    assert_eq!(f.token.balance(&f.sender), sender_before);
    assert_eq!(f.held(), DEPOSITED + RATE * 500);
    f.assert_conserved();
}

/// The funder gains no rights: on cancellation the sender is refunded the
/// unaccrued remainder, including the funder's contribution.
#[test]
fn top_up_from_refunds_the_sender_not_the_funder_on_cancel() {
    let f = Fixture::new(true);
    let funder = funded_third_party(&f, RATE * 500);
    f.client.top_up_from(&funder, &(RATE * 500));
    let sender_before = f.token.balance(&f.sender);

    f.at(START + 400);
    f.client.cancel();

    let total = DEPOSITED + RATE * 500;
    let settled = RATE * 400;
    let refund = total - settled;
    assert_eq!(f.token.balance(&f.recipient), settled);
    assert_eq!(f.token.balance(&f.sender), sender_before + refund);
    assert_eq!(f.token.balance(&funder), 0);
    f.assert_conserved();
}

/// Pins the `ToppedUpFrom` wire shape: attributed to the funder, with the
/// sender carried as data because it receives any refund.
#[test]
fn top_up_from_emits_the_topped_up_from_event() {
    let f = Fixture::new(true);
    let funder = funded_third_party(&f, RATE * 500);
    f.client.top_up_from(&funder, &(RATE * 500));

    // The token transfer event is emitted first, so check the last event.
    let all = f.env.events().all();
    let events = all.events();
    let last = events.last().expect("no events emitted");
    let expected = ToppedUpFrom {
        funder: funder.clone(),
        sender: f.sender.clone(),
        amount: RATE * 500,
        new_stop: STOP + 500,
        deposited: DEPOSITED + RATE * 500,
    };
    assert_eq!(last, &expected.to_xdr(&f.env, &f.client.address));
}

#[test]
fn top_up_from_rejects_an_amount_that_is_not_a_whole_number_of_seconds() {
    let f = Fixture::new(true);
    let funder = funded_third_party(&f, RATE * 500);
    let before = f.client.get();

    assert_eq!(
        f.client.try_top_up_from(&funder, &(RATE * 5 + 1)),
        Err(Ok(Error::InvalidAmount))
    );
    assert_eq!(f.client.get(), before);
    assert_eq!(f.token.balance(&funder), RATE * 500);
}

#[test]
fn top_up_from_is_rejected_after_cancellation_and_moves_no_tokens() {
    let f = Fixture::new(true);
    f.client.cancel();
    let funder = funded_third_party(&f, RATE * 100);
    let held = f.held();

    assert_eq!(
        f.client.try_top_up_from(&funder, &(RATE * 100)),
        Err(Ok(Error::StreamCancelled))
    );
    assert_eq!(f.held(), held);
    assert_eq!(f.token.balance(&funder), RATE * 100);
}

// ----------------------------------------------------------------- has_ended

#[test]
fn has_ended_view_flips_exactly_at_stop() {
    let f = Fixture::new(true);
    f.at(STOP - 1);
    assert!(!f.client.has_ended());
    f.at(STOP);
    assert!(f.client.has_ended());
    f.at(STOP + 1);
    assert!(f.client.has_ended());
}

/// `has_ended` is a clock predicate, so cancelling before `stop` does not
/// make the stream "ended".
#[test]
fn has_ended_view_ignores_cancellation() {
    let f = Fixture::new(true);
    f.at(START + 100);
    f.client.cancel();
    assert!(!f.client.has_ended());
}

#[test]
fn has_ended_view_moves_with_a_top_up() {
    let f = Fixture::new(true);
    f.client.top_up(&(RATE * 500));
    f.at(STOP);
    assert!(!f.client.has_ended());
    f.at(STOP + 500);
    assert!(f.client.has_ended());
}

// ----------------------------------------------------------------- extend

#[test]
fn extend_pulls_the_funds_the_longer_span_requires() {
    let f = Fixture::new(true);
    let sender_before = f.token.balance(&f.sender);
    f.client.extend(&(STOP + 300));

    let s = f.client.get();
    assert_eq!(s.stop, STOP + 300);
    assert_eq!(s.deposited, DEPOSITED + RATE * 300);
    assert_eq!(f.token.balance(&f.sender), sender_before - RATE * 300);
    f.assert_conserved();
}

#[test]
fn extend_rejects_a_stop_that_is_not_later() {
    let f = Fixture::new(true);
    for bad in [STOP, STOP - 1, 0] {
        assert_eq!(
            f.client.try_extend(&bad),
            Err(Ok(Error::StreamNotExtendable)),
            "new_stop {bad} was accepted"
        );
    }
}

#[test]
fn extend_rejects_funding_that_overflows() {
    // rate = i128::MAX over a 1-second span funds exactly i128::MAX (the
    // boundary pinned by `create_allows_the_exact_max_funding_boundary`).
    // Extending by 2 more seconds needs MAX * 2, which overflows i128, so
    // `extend` must report Overflow rather than wrap.
    let te = TestEnv::at(START);
    let (token_client, sender) = te.make_token(i128::MAX);
    let token = token_client.address.clone();
    let recipient = te.make_address();
    let c = StreamContractClient::new(te.env(), &te.env().register(StreamContract, ()));
    c.create(
        &sender,
        &recipient,
        &token,
        &i128::MAX,
        &START,
        &(START + 1),
        &true,
    );

    assert_eq!(c.try_extend(&(START + 3)), Err(Ok(Error::Overflow)));

    // The failed call changed nothing.
    let s = c.get();
    assert_eq!(s.stop, START + 1);
    assert_eq!(s.deposited, i128::MAX);
}

#[test]
fn extend_rejects_a_deposit_total_that_overflows() {
    // rate = i128::MAX / 2 over a 1-second span funds `rate`. Extending by 2
    // more seconds needs `2 * rate == i128::MAX - 1`, which is representable,
    // so the multiplication passes and only `deposited + added` overflows.
    let rate = i128::MAX / 2;
    let te = TestEnv::at(START);
    let (token_client, sender) = te.make_token(i128::MAX);
    let token = token_client.address.clone();
    let recipient = te.make_address();
    let c = StreamContractClient::new(te.env(), &te.env().register(StreamContract, ()));
    c.create(
        &sender,
        &recipient,
        &token,
        &rate,
        &START,
        &(START + 1),
        &true,
    );

    assert_eq!(c.try_extend(&(START + 3)), Err(Ok(Error::Overflow)));

    // The failed call changed nothing.
    let s = c.get();
    assert_eq!(s.stop, START + 1);
    assert_eq!(s.deposited, rate);
}

#[test]
fn extend_is_rejected_after_cancellation() {
    let f = Fixture::new(true);
    f.client.cancel();
    assert_eq!(
        f.client.try_extend(&(STOP + 100)),
        Err(Ok(Error::StreamCancelled))
    );
}

/// Pins the wire shape indexers decode (SPEC.md `indexed_events`). The
/// expected value is spelled out literally rather than built from
/// `events::Extended`, so renaming a topic or a field fails here.
#[test]
fn extend_emits_the_extended_event() {
    let f = Fixture::new(true);
    let new_stop_val = STOP + 300;
    f.client.extend(&new_stop_val);

    let env = &f.env;
    let added: Val = (RATE * 300).into_val(env);
    let deposited: Val = (DEPOSITED + RATE * 300).into_val(env);
    let old_stop: Val = STOP.into_val(env);
    let new_stop: Val = new_stop_val.into_val(env);
    // Map data: keys are the field names, serialized in sorted order.
    let data: Val = map![
        env,
        (Symbol::new(env, "added"), added),
        (Symbol::new(env, "deposited"), deposited),
        (Symbol::new(env, "new_stop"), new_stop),
        (Symbol::new(env, "old_stop"), old_stop),
    ]
    .into_val(env);
    assert_eq!(
        env.events().all().filter_by_contract(&f.client.address),
        vec![
            env,
            (
                f.client.address.clone(),
                vec![
                    env,
                    Symbol::new(env, "stream").into_val(env),
                    Symbol::new(env, "extended").into_val(env),
                    f.sender.into_val(env),
                ],
                data,
            ),
        ]
    );
}

#[test]
fn extend_moves_the_stop_later_and_preserves_the_rate_and_conservation() {
    let f = Fixture::new(true);
    let before = f.client.get();
    let new_stop = before.stop + 400;
    f.client.extend(&new_stop);

    let after = f.client.get();
    // The stop moved later, by exactly the requested amount.
    assert_eq!(after.stop, new_stop);
    assert!(after.stop > before.stop);
    // The rate is a property of the stream, not the span: it is unchanged, so
    // `deposited` is recomputable as `rate * (stop - start)`.
    assert_eq!(after.rate_per_second, before.rate_per_second);
    assert_eq!(
        after.deposited,
        after.rate_per_second * i128::from(after.stop - after.start)
    );
    // ...and the conservation invariant still holds after the extension.
    f.assert_conserved();
}

#[test]
fn extend_refuses_to_shorten_the_stream_and_changes_nothing() {
    let f = Fixture::new(true);
    f.at(START + 100);
    let before = f.client.get();
    let held = f.held();
    let sender_balance = f.token.balance(&f.sender);

    // Every stop at or before the current one is a shortening (or a no-op),
    // and `extend` only ever moves the stop later, so all are refused.
    for bad in [before.stop - 1, before.stop, before.stop - 100] {
        assert_eq!(
            f.client.try_extend(&bad),
            Err(Ok(Error::StreamNotExtendable)),
            "new_stop {bad} was accepted"
        );
    }

    // A refused extension applies nothing: no state change, no tokens pulled.
    assert_eq!(f.client.get(), before);
    assert_eq!(f.held(), held);
    assert_eq!(f.token.balance(&f.sender), sender_balance);
}

#[test]
fn extend_event_carries_the_old_and_new_stop_times() {
    let f = Fixture::new(true);
    let old_stop = f.client.get().stop;
    let new_stop = old_stop + 300;
    f.client.extend(&new_stop);

    // `extend` emits the token's transfer event before its own `Extended`,
    // so take the last event rather than assuming an index.
    let all = f.env.events().all();
    let events = all.events();
    let last = events.last().expect("no events emitted");
    let expected = Extended {
        sender: f.sender.clone(),
        added: RATE * 300,
        old_stop,
        new_stop,
        deposited: DEPOSITED + RATE * 300,
    };
    assert_eq!(last, &expected.to_xdr(&f.env, &f.client.address));
}

// -------------------------------------------------------------- balance_of

#[test]
fn balance_of_reports_each_partys_position() {
    let f = Fixture::new(true);
    f.at(START + 250);
    assert_eq!(f.client.balance_of(&f.recipient), RATE * 250);
    assert_eq!(f.client.balance_of(&f.sender), DEPOSITED - RATE * 250);
    assert_eq!(f.client.balance_of(&f.outsider), 0);
}

#[test]
fn balance_of_sender_is_zero_once_cancelled() {
    let f = Fixture::new(true);
    f.at(START + 250);
    f.client.cancel();
    assert_eq!(f.client.balance_of(&f.sender), 0);
    assert_eq!(f.client.balance_of(&f.recipient), 0);
}

#[test]
fn balance_of_shows_the_recipient_accrued_minus_withdrawn_and_the_sender_the_unstreamed_remainder()
{
    let f = Fixture::new(true);
    f.at(START + 300);
    f.client.withdraw(&Some(RATE * 100));
    f.at(START + 400);

    // Recipient: accrued so far (RATE * 400) minus what was already withdrawn.
    assert_eq!(f.client.balance_of(&f.recipient), RATE * 400 - RATE * 100);
    // Sender: the part of the deposit that has not accrued at all yet.
    assert_eq!(f.client.balance_of(&f.sender), DEPOSITED - RATE * 400);
    // Anyone else sees nothing.
    assert_eq!(f.client.balance_of(&f.outsider), 0);
    f.assert_conserved();
}

// ---------------------------------------------------- conservation, mixed

#[test]
fn conservation_holds_across_a_full_mixed_lifecycle() {
    let f = Fixture::new(true);
    f.assert_conserved();

    f.at(START + 100);
    f.client.withdraw(&Some(RATE * 50));
    f.assert_conserved();

    f.client.top_up(&(RATE * 200));
    f.assert_conserved();

    f.at(START + 400);
    f.client.extend(&(STOP + 500));
    f.assert_conserved();

    f.client.withdraw(&None);
    f.assert_conserved();

    f.at(START + 900);
    f.client.cancel();
    f.assert_conserved();

    // Everything is out, and the two sides sum to what went in.
    let s = f.client.get();
    assert_eq!(f.held(), 0);
    assert_eq!(s.withdrawn + s.refunded, s.deposited);
    assert_eq!(
        f.token.balance(&f.recipient) + f.token.balance(&f.sender),
        MINT
    );
}

#[test]
fn deposited_calculation_matches_across_top_ups_and_extends() {
    let f = Fixture::new(true);
    let mut s = f.client.get();
    assert_eq!(s.deposited, RATE * i128::from(s.stop - s.start));

    // First top_up: add 100 seconds of funding
    f.client.top_up(&(RATE * 100));
    s = f.client.get();
    assert_eq!(s.deposited, RATE * i128::from(s.stop - s.start));

    // First extend: move stop forward by 50 seconds
    f.client.extend(&(s.stop + 50));
    s = f.client.get();
    assert_eq!(s.deposited, RATE * i128::from(s.stop - s.start));

    // Second top_up: add 200 seconds
    f.client.top_up(&(RATE * 200));
    s = f.client.get();
    assert_eq!(s.deposited, RATE * i128::from(s.stop - s.start));

    // Second extend: move stop forward by 75 seconds
    f.client.extend(&(s.stop + 75));
    s = f.client.get();
    assert_eq!(s.deposited, RATE * i128::from(s.stop - s.start));

    // Third top_up: add 150 seconds
    f.client.top_up(&(RATE * 150));
    s = f.client.get();
    assert_eq!(s.deposited, RATE * i128::from(s.stop - s.start));

    // Third extend: move stop forward by 300 seconds
    f.client.extend(&(s.stop + 300));
    s = f.client.get();
    assert_eq!(s.deposited, RATE * i128::from(s.stop - s.start));

    // Verify conservation still holds after all operations
    f.assert_conserved();
}

// ----------------------------------------------- negative auth tests (#109)

#[test]
#[should_panic]
fn cancel_requires_the_senders_authorization() {
    let f = Fixture::new(true);
    f.at(START + 100);
    // Clear auths so require_auth bites.
    f.env.set_auths(&[]);
    f.client.cancel();
}

#[test]
#[should_panic]
fn top_up_requires_the_senders_authorization() {
    let f = Fixture::new(true);
    f.at(START + 100);
    // Clear auths so require_auth bites.
    f.env.set_auths(&[]);
    f.client.top_up(&(RATE * 100));
}

#[test]
#[should_panic]
fn extend_requires_the_senders_authorization() {
    let f = Fixture::new(true);
    f.at(START + 100);
    let s = f.client.get();
    // Clear auths so require_auth bites.
    f.env.set_auths(&[]);
    f.client.extend(&(s.stop + 100));
}

#[test]
#[should_panic]
fn top_up_from_requires_the_funders_authorization() {
    let f = Fixture::new(true);
    let funder = funded_third_party(&f, RATE * 100);
    // Clear auths after minting, so only the funder's require_auth can fail.
    f.env.set_auths(&[]);
    f.client.top_up_from(&funder, &(RATE * 100));
}

// ---------------------------------------------------------------- property

mod accrued_at_properties {
    use proptest::prelude::*;

    use super::*;

    fn arb_stream(env: &Env) -> Stream {
        Stream {
            sender: Address::generate(env),
            recipient: Address::generate(env),
            token: Address::generate(env),
            rate_per_second: 0,
            start: 0,
            stop: 1,
            cancellable: false,
            deposited: 0,
            withdrawn: 0,
            refunded: 0,
            cancelled_at: None,
        }
    }

    proptest! {
        /// `accrued_at` can never exceed `deposited`, no matter how far past
        /// `stop` -- or how large a rate -- it is asked about. The stream is
        /// only ever funded for `rate_per_second * (stop - start)`, so
        /// accrual cannot pay out more than was ever put in.
        #[test]
        fn accrued_at_never_exceeds_deposit(
            rate_per_second in 0i128..=1_000_000_000,
            start in 0u64..=1_000_000_000,
            span in 1u64..=10_000_000,
            at_offset in 0u64..=20_000_000,
        ) {
            let env = Env::default();
            let stop = start + span;
            let deposited = rate_per_second * i128::from(span);

            let stream = Stream {
                rate_per_second,
                start,
                stop,
                deposited,
                ..arb_stream(&env)
            };

            let at = start.saturating_add(at_offset);
            let accrued = stream
                .accrued_at(at)
                .expect("accrued_at must not error for these bounded inputs");

            prop_assert!(
                accrued <= deposited,
                "accrued_at({at}) = {accrued} exceeds deposited = {deposited} (rate={rate_per_second}, start={start}, stop={stop})"
            );
            prop_assert!(accrued >= 0, "accrued_at({at}) = {accrued} is negative");
        }

        /// Cancellation freezes accrual at the cancellation timestamp, same
        /// as `stop` would -- and the bound against `deposited` still holds.
        #[test]
        fn accrued_at_respects_cancellation_and_the_deposit_bound(
            rate_per_second in 0i128..=1_000_000_000,
            start in 0u64..=1_000_000_000,
            span in 1u64..=10_000_000,
            cancel_offset in 0u64..=10_000_000,
            at_offset in 0u64..=20_000_000,
        ) {
            let env = Env::default();
            let stop = start + span;
            let deposited = rate_per_second * i128::from(span);
            let cancelled_at = start.saturating_add(cancel_offset).min(stop);

            let stream = Stream {
                rate_per_second,
                start,
                stop,
                deposited,
                cancelled_at: Some(cancelled_at),
                ..arb_stream(&env)
            };

            let at = start.saturating_add(at_offset);
            let accrued = stream
                .accrued_at(at)
                .expect("accrued_at must not error for these bounded inputs");

            prop_assert!(accrued <= deposited);

            let at_cancellation = stream
                .accrued_at(cancelled_at)
                .expect("accrued_at(cancelled_at) must not error");
            let long_after = stream
                .accrued_at(cancelled_at.saturating_add(at_offset))
                .expect("accrued_at(long after cancellation) must not error");
            prop_assert_eq!(at_cancellation, long_after);
        }
    }
}

mod conservation_properties {
    use proptest::prelude::*;

    use super::*;

    /// One step of a random valid-or-invalid call sequence. Invalid calls are
    /// included on purpose: a rejected call must roll back and leave the
    /// conservation invariant untouched just as a valid one does.
    #[derive(Clone, Copy, Debug)]
    enum Op {
        Advance(u64),
        WithdrawAll,
        WithdrawSome(i128),
        TopUp(i128),
        Extend(u64),
        Cancel,
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        prop_oneof![
            (1u64..=2_000).prop_map(Op::Advance),
            Just(Op::WithdrawAll),
            (-5i128..=500).prop_map(Op::WithdrawSome),
            (0i128..=500).prop_map(Op::TopUp),
            (0u64..=2_000).prop_map(Op::Extend),
            Just(Op::Cancel),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// The stream's conservation invariant
        /// `withdrawn + refunded + remaining == deposited` holds after every
        /// call of an arbitrary sequence of withdraw / cancel / top_up /
        /// extend calls, and the contract's real token balance always equals
        /// `remaining`.
        #[test]
        fn conservation_holds_after_any_call_sequence(
            rate in 1i128..=1_000,
            span in 1u64..=1_000,
            ops in proptest::collection::vec(op_strategy(), 1..=24),
        ) {
            let start = 1_000_000u64;
            let stop = start + span;
            let te = TestEnv::at(start);
            let (token, sender) = te.make_token(1_000_000_000_000_000);
            let token_address = token.address.clone();
            let recipient = te.make_address();
            let env = te.env().clone();

            let client = StreamContractClient::new(&env, &env.register(StreamContract, ()));
            client.create(
                &sender,
                &recipient,
                &token_address,
                &rate,
                &start,
                &stop,
                &true,
            );

            for op in ops {
                match op {
                    Op::Advance(delta) => {
                        te.advance_by(delta);
                    }
                    Op::WithdrawAll => {
                        let _ = client.try_withdraw(&None);
                    }
                    Op::WithdrawSome(amount) => {
                        let _ = client.try_withdraw(&Some(amount));
                    }
                    Op::TopUp(periods) => {
                        let _ = client.try_top_up(&periods.saturating_mul(rate));
                    }
                    Op::Extend(extra) => {
                        let current_stop = client.get().stop;
                        let _ = client.try_extend(&current_stop.saturating_add(extra));
                    }
                    Op::Cancel => {
                        let _ = client.try_cancel();
                    }
                }

                let s = client.get();
                let remaining = client.remaining();
                prop_assert!(
                    s.withdrawn >= 0 && s.refunded >= 0 && remaining >= 0,
                    "negative component after {:?}: {:?}", op, s
                );
                prop_assert_eq!(
                    s.withdrawn + s.refunded + remaining,
                    s.deposited,
                    "conservation violated after {:?}: {:?}", op, s
                );
                prop_assert_eq!(
                    token.balance(&client.address),
                    remaining,
                    "held balance diverged after {:?}: {:?}", op, s
                );
            }
        }
    }
}

// ------------------------------------------------- withdraw at zero accrual

/// Issue #402: a stream whose `start` is the current ledger time has accrued
/// nothing. `withdraw(None)` resolves to the zero available amount and is
/// rejected as `InvalidAmount`; an explicit amount is rejected as
/// `StreamInsufficientAccrued`. Neither moves funds.
#[test]
fn withdraw_none_at_start_equal_to_now_fails_cleanly() {
    let f = Fixture::new(true);
    assert_eq!(f.client.balance_of(&f.recipient), 0);

    assert_eq!(f.client.try_withdraw(&None), Err(Ok(Error::InvalidAmount)));
    assert_eq!(
        f.client.try_withdraw(&Some(1)),
        Err(Ok(Error::StreamInsufficientAccrued))
    );

    assert_eq!(f.token.balance(&f.recipient), 0);
    assert_eq!(f.held(), DEPOSITED);
    assert_eq!(f.client.get().withdrawn, 0);
    f.assert_conserved();
}

/// Issue #402, mirror case: `start` is still in the future, so the same two
/// withdrawals fail the same way and nothing moves.
#[test]
fn withdraw_none_before_a_future_start_fails_cleanly() {
    let f = Fixture::with_span(true, START, START + 500, START + 500 + DURATION);
    assert_eq!(f.client.balance_of(&f.recipient), 0);

    assert_eq!(f.client.try_withdraw(&None), Err(Ok(Error::InvalidAmount)));
    assert_eq!(
        f.client.try_withdraw(&Some(1)),
        Err(Ok(Error::StreamInsufficientAccrued))
    );

    assert_eq!(f.token.balance(&f.recipient), 0);
    assert_eq!(f.held(), DEPOSITED);
    assert_eq!(f.client.get().withdrawn, 0);
    f.assert_conserved();
}

// ------------------------------------------- cancel worked examples (#400)
//
// Each test backs one worked example in docs/stream-cancel.md. The numbers in
// the doc are the numbers asserted here; change both together.

/// Worked example 1: cancel mid-stream with no prior withdrawals.
///
/// Rate 100/s over 1000 s, so the deposit is 100_000. At `start + 400`,
/// 40_000 has accrued: 40_000 is settled to the recipient and 60_000 is
/// refunded to the sender.
#[test]
fn cancel_worked_example_1_mid_stream_without_withdrawals() {
    let f = Fixture::new(true);
    let sender_before = f.token.balance(&f.sender);
    f.at(START + 400);
    f.client.cancel();

    assert_eq!(f.token.balance(&f.recipient), 40_000);
    assert_eq!(f.token.balance(&f.sender), sender_before + 60_000);
    let s = f.client.get();
    assert_eq!(s.withdrawn, 40_000);
    assert_eq!(s.refunded, 60_000);
    assert_eq!(s.cancelled_at, Some(START + 400));
    assert_eq!(f.held(), 0);

    let all = f.env.events().all();
    let events = all.events();
    let last = events.last().expect("no events emitted");
    let expected = Cancelled {
        sender: f.sender.clone(),
        settled_to_recipient: 40_000,
        refunded_to_sender: 60_000,
        cancelled_at: START + 400,
    };
    assert_eq!(last, &expected.to_xdr(&f.env, &f.client.address));
    f.assert_conserved();
}

/// Worked example 2: cancel after a partial withdrawal.
///
/// At `start + 200`, 20_000 has accrued and the recipient withdraws 10_000.
/// At `start + 500`, 50_000 has accrued in total, so 40_000 is still
/// unwithdrawn and is settled by the cancel. Together the recipient receives
/// 50_000, and the 50_000 that has not accrued is refunded to the sender.
#[test]
fn cancel_worked_example_2_after_a_partial_withdrawal() {
    let f = Fixture::new(true);
    let sender_before = f.token.balance(&f.sender);
    f.at(START + 200);
    assert_eq!(f.client.withdraw(&Some(10_000)), 10_000);

    f.at(START + 500);
    f.client.cancel();

    assert_eq!(f.token.balance(&f.recipient), 50_000);
    assert_eq!(f.token.balance(&f.sender), sender_before + 50_000);
    let s = f.client.get();
    assert_eq!(s.withdrawn, 50_000);
    assert_eq!(s.refunded, 50_000);

    let all = f.env.events().all();
    let events = all.events();
    let last = events.last().expect("no events emitted");
    let expected = Cancelled {
        sender: f.sender.clone(),
        settled_to_recipient: 40_000,
        refunded_to_sender: 50_000,
        cancelled_at: START + 500,
    };
    assert_eq!(last, &expected.to_xdr(&f.env, &f.client.address));
    f.assert_conserved();
}

/// Worked example 3: cancel after `stop`, with a prior withdrawal.
///
/// Accrual halts at `stop`, so the full 100_000 has accrued by `stop + 100`.
/// 10_000 was withdrawn at `start + 300`, so 90_000 is settled now. Nothing
/// is left unaccrued, so the sender's refund is zero and the sender's balance
/// does not change.
#[test]
fn cancel_worked_example_3_after_stop_refunds_nothing() {
    let f = Fixture::new(true);
    let sender_before = f.token.balance(&f.sender);
    f.at(START + 300);
    assert_eq!(f.client.withdraw(&Some(10_000)), 10_000);

    f.at(STOP + 100);
    f.client.cancel();

    assert_eq!(f.token.balance(&f.recipient), DEPOSITED);
    assert_eq!(f.token.balance(&f.sender), sender_before);
    let s = f.client.get();
    assert_eq!(s.withdrawn, DEPOSITED);
    assert_eq!(s.refunded, 0);
    assert_eq!(s.cancelled_at, Some(STOP + 100));

    let all = f.env.events().all();
    let events = all.events();
    let last = events.last().expect("no events emitted");
    let expected = Cancelled {
        sender: f.sender.clone(),
        settled_to_recipient: 90_000,
        refunded_to_sender: 0,
        cancelled_at: STOP + 100,
    };
    assert_eq!(last, &expected.to_xdr(&f.env, &f.client.address));
    f.assert_conserved();
}

// ------------------------------------------------------- budget across length

/// Ten years of 365-day years, in seconds.
const TEN_YEARS: u64 = 10 * 365 * 24 * 60 * 60;

/// The metered `(cpu_instructions, memory_bytes)` of the last top-level call
/// made in `env`. The SDK resets metering before each top-level invocation, so
/// this is the cost of that call alone.
fn last_call_cost(env: &Env) -> (u64, u64) {
    let budget = env.cost_estimate().budget();
    (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
}

/// Metered costs of `withdraw` and `cancel` on a stream of `duration` seconds,
/// as `(withdraw, cancel)`.
///
/// Both calls run at `stop`, after the whole span has accrued. Each one then
/// settles the full stream and has nothing left to refund, so a one second
/// stream and a ten year stream take the same code path and differ only in the
/// length they accrue over. Each call is made on its own instance.
fn withdraw_and_cancel_costs(duration: u64) -> ((u64, u64), (u64, u64)) {
    let te = TestEnv::at(START);
    let stop = START + duration;
    let deposited = RATE * i128::from(duration);
    // Two streams are created from the same sender, so fund both.
    let (token, sender) = te.make_token(deposited * 2);
    let token_address = token.address.clone();
    let recipient = te.make_address();
    let env = te.env().clone();

    let withdraw_client = StreamContractClient::new(&env, &env.register(StreamContract, ()));
    withdraw_client.create(
        &sender,
        &recipient,
        &token_address,
        &RATE,
        &START,
        &stop,
        &true,
    );
    let cancel_client = StreamContractClient::new(&env, &env.register(StreamContract, ()));
    cancel_client.create(
        &sender,
        &recipient,
        &token_address,
        &RATE,
        &START,
        &stop,
        &true,
    );

    env.ledger().with_mut(|l| l.timestamp = stop);
    withdraw_client.withdraw(&None);
    let withdraw = last_call_cost(&env);
    cancel_client.cancel();
    let cancel = last_call_cost(&env);
    (withdraw, cancel)
}

/// Fails unless `long` is within 10% of `short`.
fn assert_within_tolerance(what: &str, short: u64, long: u64) {
    assert!(
        short.abs_diff(long) * 10 <= short,
        "{what} grew with stream length: 1 second = {short}, 10 years = {long}"
    );
}

/// Accrual is arithmetic, so `withdraw` and `cancel` must not cost more on a
/// long stream. A loop over time would make the ten year stream about
/// 3 * 10^8 times as expensive as the one second stream, far past a 10%
/// tolerance, so this catches that with a wide margin.
///
/// The measured numbers are not recorded in this file. Run the test, and a
/// failure prints them, or read them from the CI log.
#[test]
fn withdraw_and_cancel_cost_does_not_grow_with_stream_length() {
    let (short_withdraw, short_cancel) = withdraw_and_cancel_costs(1);
    let (long_withdraw, long_cancel) = withdraw_and_cancel_costs(TEN_YEARS);

    assert_within_tolerance("withdraw cpu", short_withdraw.0, long_withdraw.0);
    assert_within_tolerance("withdraw mem", short_withdraw.1, long_withdraw.1);
    assert_within_tolerance("cancel cpu", short_cancel.0, long_cancel.0);
    assert_within_tolerance("cancel mem", short_cancel.1, long_cancel.1);
// ------------------------------------------------ scaled rate (sub-unit accrual)

#[test]
fn scaled_rate_accrues_sub_unit_amounts() {
    let env = Env::default();
    let mut s = bare(&env);
    // Rate of 0.5 units per second: 500_000 / 1_000_000
    s.rate_per_second_scaled = Some(500_000);
    s.rate_scale = Some(1_000_000);
    // Update deposited to reflect the new rate
    s.deposited = 500_000; // 0.5 * 1000 seconds

    // After 1 second: 500_000 / 1_000_000 = 0 (rounds toward zero)
    assert_eq!(s.accrued_at(START + 1), Ok(0));

    // After 2 seconds: 500_000 * 2 / 1_000_000 = 1
    assert_eq!(s.accrued_at(START + 2), Ok(1));

    // After 3 seconds: 500_000 * 3 / 1_000_000 = 1 (1500000 / 1000000)
    assert_eq!(s.accrued_at(START + 3), Ok(1));

    // After 4 seconds: 500_000 * 4 / 1_000_000 = 2 (2000000 / 1000000)
    assert_eq!(s.accrued_at(START + 4), Ok(2));
}

#[test]
fn scaled_rate_respects_deposited_cap() {
    let env = Env::default();
    let mut s = bare(&env);
    // Tiny rate: 1 / 1_000_000 per second, but very long stream
    s.rate_per_second_scaled = Some(1);
    s.rate_scale = Some(1_000_000);
    s.deposited = 100; // Only 100 units total

    // After 1 million seconds: 1 * 1_000_000 / 1_000_000 = 1
    assert_eq!(s.accrued_at(START + 1_000_000), Ok(1));

    // Way past stop: should not exceed deposited
    assert_eq!(s.accrued_at(STOP + 1_000_000), Ok(DEPOSITED));
}

#[test]
fn scaled_rate_frozen_at_cancellation() {
    let env = Env::default();
    let mut s = bare(&env);
    s.rate_per_second_scaled = Some(500_000);
    s.rate_scale = Some(1_000_000);
    let cancel_time = START + 5;
    s.cancelled_at = Some(cancel_time);

    let accrued_at_cancel = s.accrued_at(cancel_time).unwrap();
    // Further into the future should be the same
    assert_eq!(s.accrued_at(cancel_time + 1_000), Ok(accrued_at_cancel));
}

// ---------------------------------------------------------------- views: streamed_so_far, time_remaining

#[test]
fn streamed_so_far_matches_accrued_at() {
    let f = Fixture::new(true);
    f.at(START + 100);
    assert_eq!(
        f.client.streamed_so_far(),
        f.client.get().accrued_at(START + 100).unwrap()
    );
}

#[test]
fn streamed_so_far_before_start_is_zero() {
    let f = Fixture::new(true);
    f.at(START - 100);
    assert_eq!(f.client.streamed_so_far(), 0);
}

#[test]
fn streamed_so_far_after_stop_is_full_deposit() {
    let f = Fixture::new(true);
    f.at(STOP + 100);
    assert_eq!(f.client.streamed_so_far(), DEPOSITED);
}

#[test]
fn streamed_so_far_frozen_at_cancellation() {
    let f = Fixture::new(true);
    let cancel_time = START + 300;
    f.at(cancel_time);
    f.client.cancel();
    let streamed_at_cancel = f.client.streamed_so_far();

    // Advance time and verify it doesn't change
    f.at(STOP + 1000);
    assert_eq!(f.client.streamed_so_far(), streamed_at_cancel);
}

#[test]
fn time_remaining_decreases_over_time() {
    let f = Fixture::new(true);
    f.at(START);
    assert_eq!(f.client.time_remaining(), DURATION);

    f.at(START + 100);
    assert_eq!(f.client.time_remaining(), DURATION - 100);

    f.at(START + 500);
    assert_eq!(f.client.time_remaining(), DURATION - 500);
}

#[test]
fn time_remaining_clamps_at_zero() {
    let f = Fixture::new(true);
    f.at(STOP + 1000);
    assert_eq!(f.client.time_remaining(), 0);
}

#[test]
fn time_remaining_frozen_at_cancellation() {
    let f = Fixture::new(true);
    let cancel_time = START + 300;
    f.at(cancel_time);
    f.client.cancel();
    let remaining_at_cancel = f.client.time_remaining();

    // Advance past stop and verify it doesn't change
    f.at(STOP + 1000);
    assert_eq!(f.client.time_remaining(), remaining_at_cancel);
}
}
