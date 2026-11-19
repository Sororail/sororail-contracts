use super::*;

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
#[should_panic]
fn extend_requires_the_senders_authorization() {
    let f = Fixture::new(true);
    f.at(START + 100);
    let s = f.client.get();
    // Clear auths so require_auth bites.
    f.env.set_auths(&[]);
    f.client.extend(&(s.stop + 100));
}
