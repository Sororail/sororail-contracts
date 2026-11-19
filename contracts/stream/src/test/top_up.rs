use super::*;

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
#[test]
#[should_panic]
fn top_up_requires_the_senders_authorization() {
    let f = Fixture::new(true);
    f.at(START + 100);
    // Clear auths so require_auth bites.
    f.env.set_auths(&[]);
    f.client.top_up(&(RATE * 100));
}
