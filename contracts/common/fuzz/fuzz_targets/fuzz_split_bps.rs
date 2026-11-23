//! Fuzz target for `sororail_common::math::split_bps` (#370).
//!
//! `split_bps` underpins escrow dispute resolution, so it must never create or
//! lose value. For every input this target checks that:
//!
//! - `bps > MAX_BPS` is rejected with `Error::InvalidBasisPoints`;
//! - an overflowing `amount * bps` is reported as `Error::Overflow`, never a
//!   wrong split;
//! - otherwise the split is exact: `first + second == amount`, `first` equals
//!   the `BigInt` reference `amount * bps / 10000` truncated toward zero, and
//!   for a non-negative `amount` both halves are non-negative and neither
//!   exceeds `amount`.
//!
//! Half of the inputs fold `bps` into `0..=MAX_BPS` so the valid range gets
//! most of the coverage instead of being one in 400 000 random `u32`s.
#![no_main]

use libfuzzer_sys::fuzz_target;
use num_bigint::BigInt;
use sororail_common::{
    math::{self, BPS_DENOMINATOR, MAX_BPS},
    Error,
};

fuzz_target!(|data: (i128, u32, bool)| {
    let (amount, raw_bps, fold) = data;
    let bps = if fold {
        raw_bps % (MAX_BPS + 1)
    } else {
        raw_bps
    };
    let result = math::split_bps(amount, bps);

    if bps > MAX_BPS {
        assert_eq!(
            result,
            Err(Error::InvalidBasisPoints),
            "split_bps({amount}, {bps})"
        );
        return;
    }

    if amount.checked_mul(i128::from(bps)).is_none() {
        assert_eq!(result, Err(Error::Overflow), "split_bps({amount}, {bps})");
        return;
    }

    let (first, second) = match result {
        Ok(parts) => parts,
        Err(err) => panic!("split_bps({amount}, {bps}) returned {err:?} for a valid input"),
    };

    // Sum invariant, checked without risking overflow in the check itself.
    assert_eq!(
        BigInt::from(first) + BigInt::from(second),
        BigInt::from(amount),
        "split_bps({amount}, {bps}) = ({first}, {second}) does not sum to the amount",
    );

    // `first` is exactly bps of the amount, truncated toward zero.
    let reference = BigInt::from(amount) * BigInt::from(bps) / BigInt::from(BPS_DENOMINATOR);
    assert_eq!(
        BigInt::from(first),
        reference,
        "split_bps({amount}, {bps}).0 = {first}, but the bigint reference is {reference}",
    );

    if amount >= 0 {
        assert!(
            (0..=amount).contains(&first) && (0..=amount).contains(&second),
            "split_bps({amount}, {bps}) = ({first}, {second}) has a half outside 0..=amount",
        );
    }
});
