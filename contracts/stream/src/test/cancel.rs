use super::*;

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
#[test]
#[should_panic]
fn cancel_requires_the_senders_authorization() {
    let f = Fixture::new(true);
    f.at(START + 100);
    // Clear auths so require_auth bites.
    f.env.set_auths(&[]);
    f.client.cancel();
}
