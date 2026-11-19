use super::*;

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
