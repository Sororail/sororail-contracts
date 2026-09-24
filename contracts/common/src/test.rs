// Test fixtures do plain arithmetic on known-small constants; the checked-math
// rule is for contract code.
#![allow(clippy::arithmetic_side_effects)]

use soroban_sdk::{testutils::Address as _, Address, Env};

use crate::{
    auth,
    math::{self, BPS_DENOMINATOR, MAX_BPS},
    storage, Error,
};

// The protocol's ceiling on how far a TTL may be extended (~6 months).
const MAX_TTL: u32 = 3_110_400;

// ---------------------------------------------------------------- math

#[test]
fn add_sub_mul_are_checked() {
    assert_eq!(math::add(2, 3), Ok(5));
    assert_eq!(math::sub(3, 2), Ok(1));
    assert_eq!(math::mul(3, 4), Ok(12));

    assert_eq!(math::add(i128::MAX, 1), Err(Error::Overflow));
    assert_eq!(math::sub(i128::MIN, 1), Err(Error::Underflow));
    assert_eq!(math::mul(i128::MAX, 2), Err(Error::Overflow));
}

#[test]
fn div_rejects_zero_divisor() {
    assert_eq!(math::div(10, 2), Ok(5));
    assert_eq!(math::div(10, 0), Err(Error::DivisionByZero));
    // i128::MIN / -1 is the one division that overflows.
    assert_eq!(math::div(i128::MIN, -1), Err(Error::Overflow));
}

#[test]
fn mul_div_reports_overflow_rather_than_truncating() {
    assert_eq!(math::mul_div(100, 50, 200), Ok(25));
    // Rounds toward zero, never up.
    assert_eq!(math::mul_div(10, 1, 3), Ok(3));
    assert_eq!(math::mul_div(i128::MAX, 2, 2), Err(Error::Overflow));
}

#[test]
fn mul_bps_rejects_out_of_range() {
    assert_eq!(math::mul_bps(1_000, 0), Ok(0));
    assert_eq!(math::mul_bps(1_000, 2_500), Ok(250));
    assert_eq!(math::mul_bps(1_000, MAX_BPS), Ok(1_000));
    assert_eq!(
        math::mul_bps(1_000, MAX_BPS + 1),
        Err(Error::InvalidBasisPoints)
    );
}

#[test]
fn split_bps_conserves_the_total_exactly() {
    // The invariant escrow dispute resolution rests on: no rounding leakage,
    // at any split, for any amount -- including amounts that divide badly.
    let amounts = [
        0i128,
        1,
        2,
        3,
        7,
        99,
        100,
        101,
        9_999,
        10_001,
        i128::MAX / 10_000,
    ];
    let splits = [0u32, 1, 3, 50, 1_234, 5_000, 6_667, 9_999, 10_000];

    for amount in amounts {
        for bps in splits {
            let (a, b) = math::split_bps(amount, bps).unwrap();
            assert_eq!(
                math::add(a, b).unwrap(),
                amount,
                "split of {amount} at {bps}bps did not conserve"
            );
            assert!(
                a >= 0 && b >= 0,
                "split of {amount} at {bps}bps went negative"
            );
        }
    }
}

#[test]
fn split_bps_endpoints_are_whole() {
    assert_eq!(math::split_bps(777, 0), Ok((0, 777)));
    assert_eq!(math::split_bps(777, MAX_BPS), Ok((777, 0)));
}

#[test]
fn amount_guards() {
    assert_eq!(math::require_positive(1), Ok(()));
    assert_eq!(math::require_positive(0), Err(Error::InvalidAmount));
    assert_eq!(math::require_positive(-1), Err(Error::InvalidAmount));

    assert_eq!(math::require_non_negative(0), Ok(()));
    assert_eq!(math::require_non_negative(-1), Err(Error::InvalidAmount));
}

#[test]
fn min_picks_the_smaller() {
    assert_eq!(math::min(3, 9), 3);
    assert_eq!(math::min(9, 3), 3);
    assert_eq!(math::min(-1, 0), -1);
}

#[test]
fn bps_denominator_matches_max_bps() {
    assert_eq!(BPS_DENOMINATOR, MAX_BPS as i128);
}

// ---------------------------------------------------------------- storage

// These relationships hold between constants, so they are checked at compile
// time rather than at test time: a violation should fail the build, not wait
// for someone to run the suite.
const _: () = {
    assert!(storage::INSTANCE_THRESHOLD < storage::INSTANCE_BUMP);
    assert!(storage::PERSISTENT_THRESHOLD < storage::PERSISTENT_BUMP);
    // Asking for more than the protocol ceiling is an error at runtime.
    assert!(storage::INSTANCE_BUMP <= MAX_TTL);
    assert!(storage::PERSISTENT_BUMP <= MAX_TTL);
    // Positions should outlive config refreshes.
    assert!(storage::PERSISTENT_BUMP > storage::INSTANCE_BUMP);
};

// ---------------------------------------------------------------- auth
//
// Only the rejection paths are unit-tested here. The accept path calls
// `require_auth`, which needs a real contract invocation context, so it is
// covered in each contract's own tests.

#[test]
fn require_reports_the_given_error() {
    assert_eq!(auth::require(true, Error::Unauthorized), Ok(()));
    assert_eq!(
        auth::require(false, Error::InvalidState),
        Err(Error::InvalidState)
    );
}

#[test]
fn require_auth_as_rejects_a_different_address() {
    let env = Env::default();
    let caller = Address::generate(&env);
    let expected = Address::generate(&env);
    assert_eq!(
        auth::require_auth_as(&caller, &expected),
        Err(Error::Unauthorized)
    );
}

#[test]
fn require_auth_either_rejects_an_outsider() {
    let env = Env::default();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let outsider = Address::generate(&env);
    assert_eq!(
        auth::require_auth_either(&outsider, &a, &b),
        Err(Error::Unauthorized)
    );
}

#[test]
fn require_auth_either_opt_rejects_when_optional_party_absent() {
    let env = Env::default();
    let a = Address::generate(&env);
    let other = Address::generate(&env);
    assert_eq!(
        auth::require_auth_either_opt(&other, &a, &None),
        Err(Error::Unauthorized)
    );
}

#[test]
fn require_auth_one_of_rejects_an_outsider() {
    let env = Env::default();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let outsider = Address::generate(&env);
    let allowed = auth::allow_list(&env, &[&a, &b]);
    assert_eq!(
        auth::require_auth_one_of(&outsider, &allowed),
        Err(Error::Unauthorized)
    );
}
