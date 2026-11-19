use super::*;

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
    env.ledger().with_mut(|l| l.timestamp = START + 100);
    c.withdraw(&None);
}
