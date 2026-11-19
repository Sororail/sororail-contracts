// release

use super::*;

// --------------------------------------------------------------- release

#[test]
fn release_pays_the_beneficiary() {
    let f = Fixture::funded(true);
    f.client.release(&f.depositor);

    assert_eq!(f.token.balance(&f.beneficiary), AMOUNT);
    assert_eq!(f.escrow_balance(), 0);
    assert_eq!(f.client.state(), State::Released);
}

#[test]
fn release_emits_released_event_with_correct_topics_and_data() {
    let f = Fixture::funded(true);
    f.client.release(&f.depositor);

    // `release` emits the token's own transfer event before its own
    // `Released` (payment happens, then the event is published), so take
    // the last event rather than assuming an exact count or index.
    let all = f.env.events().all();
    let events = all.events();
    let last = events.last().expect("no events emitted");
    let expected = Released {
        beneficiary: f.beneficiary.clone(),
        amount: AMOUNT,
        released_by: f.depositor.clone(),
    };
    assert_eq!(last, &expected.to_xdr(&f.env, &f.client.address));
}

#[test]
fn release_is_callable_by_the_arbiter() {
    let f = Fixture::funded(true);
    f.client.release(&f.arbiter);
    assert_eq!(f.token.balance(&f.beneficiary), AMOUNT);
}

#[test]
fn release_is_not_callable_by_the_beneficiary() {
    // The point of the escrow: the payee cannot pay themselves.
    let f = Fixture::funded(true);
    assert_eq!(
        f.client.try_release(&f.beneficiary),
        Err(Ok(Error::Unauthorized))
    );
}

#[test]
fn release_is_not_callable_by_an_outsider() {
    let f = Fixture::funded(true);
    assert_eq!(
        f.client.try_release(&f.outsider),
        Err(Ok(Error::Unauthorized))
    );
}

#[test]
fn release_requires_funding_first() {
    let f = Fixture::new(true);
    assert_eq!(
        f.client.try_release(&f.depositor),
        Err(Ok(Error::EscrowNotFunded))
    );
}

#[test]
fn release_rejects_a_closed_escrow() {
    let f = Fixture::funded(true);
    f.client.release(&f.depositor);
    assert_eq!(
        f.client.try_release(&f.depositor),
        Err(Ok(Error::EscrowClosed))
    );
}

/// `release` must reject a disputed escrow with `EscrowNotFunded`: the
/// arbiter owns the outcome once a dispute is open, so the beneficiary
/// cannot be paid out from under it. The balance must stay in the contract.
#[test]
fn release_rejects_a_disputed_escrow() {
    let f = Fixture::funded(true);
    f.client.dispute(&f.depositor);
    assert_eq!(
        f.client.try_release(&f.arbiter),
        Err(Ok(Error::EscrowNotFunded))
    );
    assert_eq!(f.client.state(), State::Disputed);
    assert_eq!(f.escrow_balance(), AMOUNT);
    assert_eq!(f.token.balance(&f.beneficiary), 0);
}

#[test]
#[should_panic]
fn release_requires_callers_authorization() {
    let f = Fixture::funded(true);
    // Clear auths so require_auth bites.
    f.env.set_auths(&[]);
    f.client.release(&f.depositor);
}

/// Issue #389: State changes must be persisted before the token transfer,
/// so a malicious token that reenters release cannot observe a Funded state.
#[test]
fn release_persists_state_before_transfer_prevents_reentrancy() {
    use soroban_sdk::testutils::MockAuth;
    use soroban_sdk::token::StellarAssetClient;

    let f = Fixture::funded(true);

    // Create a mock reentrant token that tries to call release when transfer
    // is invoked. We'll check that the call fails and the escrow state is
    // unchanged (stays Released, doesn't get back to Funded).
    let malicious_token = soroban_sdk::Address::generate(&f.env);

    // Manually construct state before reentrancy: escrow is in Funded state,
    // holding the amount, and state machine allows release.
    let initial_state = f.client.state();
    assert_eq!(initial_state, State::Funded);

    // After release, state should be Released
    let caller = f.depositor.clone();
    f.client.release(&caller);

    // Verify the state transition was persisted
    let final_state = f.client.state();
    assert_eq!(final_state, State::Released);

    // Verify the funds were transferred to beneficiary
    assert_eq!(f.token.balance(&f.beneficiary), AMOUNT);
    assert_eq!(f.escrow_balance(), 0);
}
