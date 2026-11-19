use super::*;

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
fn withdraw_event_amount_matches_the_token_transfer() {
    let f = Fixture::new(true);
    f.at(START + 300);
    let recipient_before = f.token.balance(&f.recipient);
    let withdrawn = f.client.withdraw(&Some(RATE * 100));

    // Capture immediately: subsequent token balance invocations replace the
    // events returned by `events().all()`.
    // `withdraw` emits the token's own transfer event before its own
    // `Withdrawn`, so take the last event rather than assuming an index.
    let all = f.env.events().all();
    let events = all.events();
    let last = events.last().expect("no events emitted");
    let transferred = f.token.balance(&f.recipient) - recipient_before;

    assert_eq!(transferred, RATE * 100);
    assert_eq!(withdrawn, transferred);

    let expected = Withdrawn {
        recipient: f.recipient.clone(),
        amount: transferred,
        total_withdrawn: f.client.get().withdrawn,
    };
    assert_eq!(last, &expected.to_xdr(&f.env, &f.client.address));
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
