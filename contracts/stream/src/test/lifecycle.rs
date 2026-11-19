use super::*;
use sororail_common::testutils::assert_conserved;

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

    // Everything is out, and every balance sums back to the minted supply.
    let s = f.client.get();
    assert_eq!(f.held(), 0);
    assert_eq!(s.withdrawn + s.refunded, s.deposited);
    assert_conserved(
        &f.token,
        &[
            f.sender.clone(),
            f.recipient.clone(),
            f.client.address.clone(),
        ],
        MINT,
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
