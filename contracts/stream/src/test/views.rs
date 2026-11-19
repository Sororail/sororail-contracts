use super::*;

#[test]
fn bump_restores_the_instance_ttl_after_ledger_advance() {
    let f = Fixture::new(true);
    f.env
        .ledger()
        .set_sequence_number(f.env.ledger().sequence() + INSTANCE_BUMP - 1);
    f.env.as_contract(&f.client.address, || {
        assert!(f.env.storage().instance().get_ttl() < INSTANCE_BUMP);
    });
    f.client.bump();
    f.env.as_contract(&f.client.address, || {
        assert_eq!(f.env.storage().instance().get_ttl(), INSTANCE_BUMP);
    });
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
