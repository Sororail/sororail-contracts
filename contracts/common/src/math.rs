//! Checked money math.
//!
//! Every operation here returns [`Error`] rather than wrapping, saturating or
//! panicking. Saturation is the dangerous default for balances: it silently
//! converts an overflow into a wrong-but-plausible number, and a payment
//! contract that reports a wrong balance is worse than one that refuses to act.
//!
//! # Basis-point math: single-consumer functions in a shared crate
//!
//! [`mul_bps`] and [`split_bps`] are currently used only by `escrow::resolve`,
//! but they live here rather than in `contracts/escrow/src/` for two reasons:
//!
//! 1. **Correctness by proximity**: The no-rounding-leakage invariant that
//!    `split_bps` provides is subtle and security-critical for dispute
//!    resolution. Keeping it alongside the other checked math makes that
//!    property visible in the shared test suite and documents it as a
//!    general-purpose building block, not an escrow implementation detail.
//!
//! 2. **Future reuse without duplication**: Any future contract that needs
//!    proportional splits (fee distribution, profit sharing, multi-party
//!    settlements) can use these immediately without rediscovering the
//!    remainder-by-subtraction pattern or reintroducing the tests.
//!
//! If no other contract adopts them by the time the codebase ships 1.0, we may
//! revisit this and move them into escrow. Until then, the cost of a few extra
//! lines in a shared crate is lower than the risk of reimplementing bps math
//! incorrectly in a future contract.

use crate::Error;

/// Denominator for basis-point math. 10000 bps == 100%.
pub const BPS_DENOMINATOR: i128 = 10_000;

/// Returns the uses left under an optional cap, saturating at zero.
///
/// `None` represents an unbounded count. Shared by types that expose remaining
/// uses so capped behavior stays consistent across contracts.
pub const fn remaining_count(cap: Option<u32>, used: u32) -> Option<u32> {
    match cap {
        Some(cap) => Some(cap.saturating_sub(used)),
        None => None,
    }
}

/// Maximum valid basis points.
pub const MAX_BPS: u32 = 10_000;

/// Checked addition. Errors with [`Error::Overflow`] when the sum falls
/// outside the `i128` range.
#[inline]
pub fn add(a: i128, b: i128) -> Result<i128, Error> {
    a.checked_add(b).ok_or(Error::Overflow)
}

/// Checked subtraction. Errors with [`Error::Underflow`] when the difference
/// falls below `i128::MIN`.
#[inline]
pub fn sub(a: i128, b: i128) -> Result<i128, Error> {
    a.checked_sub(b).ok_or(Error::Underflow)
}

/// Checked multiplication. Errors with [`Error::Overflow`] when the product
/// falls outside the `i128` range.
#[inline]
pub fn mul(a: i128, b: i128) -> Result<i128, Error> {
    a.checked_mul(b).ok_or(Error::Overflow)
}

/// Checked division, truncating toward zero. Errors with
/// [`Error::DivisionByZero`] when `b` is zero, and with [`Error::Overflow`]
/// when the quotient does not fit in `i128` (only `i128::MIN / -1`).
#[inline]
pub fn div(a: i128, b: i128) -> Result<i128, Error> {
    if b == 0 {
        return Err(Error::DivisionByZero);
    }
    a.checked_div(b).ok_or(Error::Overflow)
}

/// `a * b / d`, rounded toward zero.
///
/// The intermediate product is computed in `i128` and errors on overflow
/// rather than widening. Callers doing proportional math over very large
/// totals (`total * elapsed / duration`) should be aware that an overflow is
/// reported as [`Error::Overflow`], not silently truncated.
///
/// # Examples
///
/// ```
/// use sororail_common::math;
///
/// assert_eq!(math::mul_div(1_000, 25, 100), Ok(250));
/// ```
#[inline]
pub fn mul_div(a: i128, b: i128, d: i128) -> Result<i128, Error> {
    mul_div_with(Rounding::Down, a, b, d)
}

/// Determines how a quotient is rounded by [`mul_div_with`].
///
/// `Down` preserves the existing truncation toward zero (including signed
/// inputs). `Up` rounds toward positive infinity for non-negative inputs and
/// rejects negative operands with [`Error::InvalidAmount`]. `Nearest`
/// rounds to the closest integer; exact halves round **away from zero**.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rounding {
    /// Truncate the quotient toward zero, preserving [`mul_div`] behavior.
    Down,
    /// Round a non-negative quotient upward when a remainder exists.
    Up,
    /// Round to the closest integer, with exact halves away from zero.
    Nearest,
}

/// Computes `a * b / d` with explicit rounding and checked `i128` arithmetic.
///
/// The product is checked *before* division, so a product that overflows
/// returns [`Error::Overflow`] even when the mathematical quotient would fit.
/// Division by zero returns [`Error::DivisionByZero`].
///
/// `Down` truncates toward zero for both positive and negative operands.
/// `Up` is intended for unsigned amounts represented as `i128`: it rejects
/// negative `a`, `b`, or `d` with [`Error::InvalidAmount`]. This keeps
/// the existing signed behavior of [`mul_div`] intact while making the
/// protocol-favouring [`mul_div_ceil`] safe for non-negative money amounts.
/// `Nearest` supports signed values and rounds exact halves away from zero.
///
/// No `product + denominator - 1` or absolute-value conversion to `i128`
/// is performed, including for `i128::MIN`.
///
/// # Examples
///
/// ```
/// use sororail_common::math::{mul_div_with, Rounding};
///
/// assert_eq!(mul_div_with(Rounding::Down, 7, 1, 2), Ok(3));
/// assert_eq!(mul_div_with(Rounding::Up, 7, 1, 2), Ok(4));
/// assert_eq!(mul_div_with(Rounding::Nearest, -7, 1, 2), Ok(-4));
/// ```
pub fn mul_div_with(rounding: Rounding, a: i128, b: i128, d: i128) -> Result<i128, Error> {
    if rounding == Rounding::Up && (a < 0 || b < 0 || d < 0) {
        return Err(Error::InvalidAmount);
    }

    // This preserves the evaluation order of mul_div: product overflow wins
    // over a simultaneous zero denominator (also required by the fuzz oracle).
    let product = mul(a, b)?;
    let quotient = div(product, d)?;
    if rounding == Rounding::Down {
        return Ok(quotient);
    }

    // `div` above has already excluded zero denominators and MIN / -1.
    let remainder = product.checked_rem(d).ok_or(Error::Overflow)?;
    if remainder == 0 {
        return Ok(quotient);
    }

    match rounding {
        Rounding::Down => Ok(quotient),
        Rounding::Up => quotient.checked_add(1).ok_or(Error::Overflow),
        Rounding::Nearest => {
            // unsigned_abs handles i128::MIN without overflowing.
            let half = d.unsigned_abs().div_ceil(2);
            if remainder.unsigned_abs() < half {
                return Ok(quotient);
            }
            // The mathematical quotient is negative when the signs differ.
            if (product < 0) != (d < 0) {
                quotient.checked_sub(1).ok_or(Error::Overflow)
            } else {
                quotient.checked_add(1).ok_or(Error::Overflow)
            }
        }
    }
}

/// `a * b / d` rounded upward for non-negative amounts.
///
/// Unlike [`mul_div`], negative operands are rejected with
/// [`Error::InvalidAmount`]. A zero denominator yields
/// [`Error::DivisionByZero`], and an overflowing intermediate product
/// yields [`Error::Overflow`]. Uses checked arithmetic without the
/// overflow-prone `(a * b + d - 1) / d` idiom.
///
/// # Examples
///
/// ```
/// use sororail_common::math;
///
/// assert_eq!(math::mul_div(7, 1, 2), Ok(3));
/// assert_eq!(math::mul_div_ceil(7, 1, 2), Ok(4));
/// ```
#[inline]
pub fn mul_div_ceil(a: i128, b: i128, d: i128) -> Result<i128, Error> {
    mul_div_with(Rounding::Up, a, b, d)
}

/// `ceil(a * b / d)`, rounded up (toward positive infinity) for non-negative
/// operands and a strictly positive divisor.
///
/// Unlike [`mul_div`], which truncates toward zero, `mul_div_ceil` rounds any
/// non-zero remainder up by one unit. Uses checked arithmetic throughout and
/// computes the ceiling via quotient and remainder (`prod / d + (prod % d != 0)`)
/// so `a * b` near `i128::MAX` does not spuriously overflow on `+ (d - 1)`.
///
/// Rejects negative operands (`a < 0`, `b < 0`, or `d < 0`) with
/// [`Error::InvalidAmount`] and `d == 0` with [`Error::DivisionByZero`].
///
/// # Examples
///
/// ```
/// use sororail_common::math;
///
/// assert_eq!(math::mul_div_ceil(7, 1, 2), Ok(4));
/// assert_eq!(math::mul_div(7, 1, 2), Ok(3));
/// ```
pub fn mul_div_ceil(a: i128, b: i128, d: i128) -> Result<i128, Error> {
    if d == 0 {
        return Err(Error::DivisionByZero);
    }
    require_non_negative(a)?;
    require_non_negative(b)?;
    require_positive(d)?;
    let prod = mul(a, b)?;
    let q = div(prod, d)?;
    let rem = prod.checked_rem(d).ok_or(Error::Overflow)?;
    if rem != 0 {
        add(q, 1)
    } else {
        Ok(q)
    }
}

/// `amount * bps / 10000`, rounded toward zero.
///
/// Currently used only by `escrow::resolve`. See module docs for rationale.
///
/// # Examples
///
/// ```
/// use sororail_common::math;
///
/// assert_eq!(math::mul_bps(1_000_000, 250), Ok(25_000));
/// ```
pub fn mul_bps(amount: i128, bps: u32) -> Result<i128, Error> {
    if bps > MAX_BPS {
        return Err(Error::InvalidBasisPoints);
    }
    mul_div(amount, i128::from(bps), BPS_DENOMINATOR)
}

/// Splits `amount` into `(first, second)` where `first` is `bps` of the total.
///
/// `first` is `mul_bps(amount, bps)`, truncated toward zero (see
/// [`mul_bps`]). `second` is computed as `amount - first` rather than as
/// `mul_bps(amount, 10000 - bps)`, so any remainder left over by that
/// truncation always lands on `second` and the two parts always sum to
/// exactly `amount` with no rounding leakage. Callers can rely on `second`
/// being `>= amount * (10000 - bps) / 10000`, never less. This is the
/// property escrow dispute resolution depends on.
///
/// Currently used only by `escrow::resolve`. See module docs for rationale.
///
/// # Examples
///
/// ```
/// use sororail_common::math;
///
/// let (fee, remainder) = math::split_bps(101, 3_333)?;
/// assert_eq!(fee + remainder, 101);
/// # Ok::<(), sororail_common::Error>(())
/// ```
pub fn split_bps(amount: i128, bps: u32) -> Result<(i128, i128), Error> {
    let first = mul_bps(amount, bps)?;
    let second = sub(amount, first)?;
    Ok((first, second))
}

/// Errors unless `amount` is strictly positive.
#[inline]
pub fn require_positive(amount: i128) -> Result<(), Error> {
    if amount <= 0 {
        return Err(Error::InvalidAmount);
    }
    Ok(())
}

/// Errors unless `amount` is zero or positive.
#[inline]
pub fn require_non_negative(amount: i128) -> Result<(), Error> {
    if amount < 0 {
        return Err(Error::InvalidAmount);
    }
    Ok(())
}

/// Returns the smaller of two values.
#[inline]
pub fn min(a: i128, b: i128) -> i128 {
    if a < b {
        a
    } else {
        b
    }
}

/// Returns the larger of two values.
#[inline]
pub fn max(a: i128, b: i128) -> i128 {
    if a > b {
        a
    } else {
        b
    }
}

/// Clamps `value` to the inclusive range `lo..=hi`.
///
/// Returns [`Error::InvalidAmount`] if the bounds are inverted, even when
/// `value` happens to equal one of them.
///
/// # Examples
///
/// ```
/// use sororail_common::math;
///
/// assert_eq!(math::clamp(12, 0, 10), Ok(10));
/// assert_eq!(math::clamp(-3, 0, 10), Ok(0));
/// ```
#[inline]
pub fn clamp(value: i128, lo: i128, hi: i128) -> Result<i128, Error> {
    if lo > hi {
        return Err(Error::InvalidAmount);
    }
    Ok(min(max(value, lo), hi))
}

#[cfg(test)]
mod rounding_and_bounds_tests {
    // Test fixtures use known-safe constants for boundary comparisons.
    #![allow(clippy::arithmetic_side_effects)]

    use super::*;

    #[test]
    fn rounding_modes_cover_below_exact_and_above_half() {
        // 5/4 = 1.25, 6/4 = 1.5, 7/4 = 1.75.
        for (numerator, down, up, nearest) in [
            (5, 1, 2, 1),
            (6, 1, 2, 2),
            (7, 1, 2, 2),
            (8, 2, 2, 2),
            (0, 0, 0, 0),
        ] {
            assert_eq!(mul_div_with(Rounding::Down, numerator, 1, 4), Ok(down));
            assert_eq!(mul_div_with(Rounding::Up, numerator, 1, 4), Ok(up));
            assert_eq!(
                mul_div_with(Rounding::Nearest, numerator, 1, 4),
                Ok(nearest)
            );
        }
    }

    #[test]
    fn nearest_half_away_from_zero_applies_to_negative_inputs() {
        for (numerator, rounded) in [(-5, -1), (-6, -2), (-7, -2), (-8, -2)] {
            assert_eq!(
                mul_div_with(Rounding::Nearest, numerator, 1, 4),
                Ok(rounded)
            );
        }
        assert_eq!(mul_div_with(Rounding::Nearest, 7, 1, -4), Ok(-2));
        assert_eq!(mul_div_with(Rounding::Nearest, -7, 1, -4), Ok(2));
        assert_eq!(mul_div_with(Rounding::Nearest, i128::MIN, 1, i128::MIN), Ok(1));
    }

    #[test]
    fn rounding_up_matches_ceil_without_leaking_an_extra_unit() {
        assert_eq!(mul_div_ceil(7, 1, 2), Ok(4));
        assert_eq!(mul_div(7, 1, 2), Ok(3));
        assert_eq!(mul_div_ceil(6, 1, 2), Ok(3));
        assert_eq!(mul_div_ceil(0, 19, 3), Ok(0));
        assert_eq!(mul_div_ceil(i128::MAX, 1, 1), Ok(i128::MAX));
        assert_eq!(mul_div_ceil(i128::MAX, 1, 2), Ok(i128::MAX / 2 + 1));
    }

    #[test]
    fn ceiling_rejects_negative_inputs_and_invalid_divisors() {
        for (a, b, d) in [(-1, 2, 3), (1, -2, 3), (1, 2, -3)] {
            assert_eq!(mul_div_ceil(a, b, d), Err(Error::InvalidAmount));
        }
        assert_eq!(mul_div_ceil(7, 1, 0), Err(Error::DivisionByZero));
        assert_eq!(mul_div_with(Rounding::Nearest, 7, 1, 0), Err(Error::DivisionByZero));
        // Keep the documented multiply-then-divide error order.
        assert_eq!(mul_div(i128::MAX, 2, 0), Err(Error::Overflow));
        assert_eq!(mul_div_ceil(i128::MAX, 2, 0), Err(Error::Overflow));
        assert_eq!(mul_div_ceil(i128::MAX, 2, 2), Err(Error::Overflow));
        assert_eq!(
            mul_div_with(Rounding::Nearest, i128::MIN, 1, -1),
            Err(Error::Overflow)
        );
        // Preserve the original signed mul_div semantics, including i128::MIN.
        assert_eq!(mul_div(-7, 1, 2), Ok(-3));
    }

    #[test]
    fn max_is_table_driven_for_equal_and_boundary_values() {
        for (a, b, expected) in [
            (3, 9, 9),
            (9, 3, 9),
            (5, 5, 5),
            (-2, -9, -2),
            (i128::MIN, i128::MAX, i128::MAX),
            (i128::MAX, i128::MIN, i128::MAX),
        ] {
            assert_eq!(max(a, b), expected, "max({a}, {b})");
        }
    }

    #[test]
    fn clamp_is_table_driven_for_equal_boundary_and_inverted_ranges() {
        for (value, lo, hi, expected) in [
            (5, 0, 10, Ok(5)),
            (-1, 0, 10, Ok(0)),
            (11, 0, 10, Ok(10)),
            (0, 0, 10, Ok(0)),
            (10, 0, 10, Ok(10)),
            (5, 5, 5, Ok(5)),
            (i128::MIN, i128::MIN, i128::MAX, Ok(i128::MIN)),
            (i128::MAX, i128::MIN, i128::MAX, Ok(i128::MAX)),
            (5, 6, 4, Err(Error::InvalidAmount)),
            (5, i128::MAX, i128::MIN, Err(Error::InvalidAmount)),
        ] {
            assert_eq!(clamp(value, lo, hi), expected);
        }
    }
}

#[cfg(test)]
mod remaining_count_tests {
    use super::remaining_count;

    #[test]
    fn cap_remaining_count_saturates_and_supports_no_cap() {
        assert_eq!(remaining_count(Some(3), 1), Some(2));
        assert_eq!(remaining_count(Some(3), 3), Some(0));
        assert_eq!(remaining_count(Some(3), 4), Some(0));
        assert_eq!(remaining_count(None, u32::MAX), None);
    }
}

#[cfg(test)]
mod tests {
    // This seeded property-test generator intentionally uses wrapping,
    // truncating PRNG arithmetic; contract arithmetic remains checked above.
    #![allow(clippy::arithmetic_side_effects)]

    use super::*;

    /// A small xorshift PRNG. `rand`/`proptest` aren't workspace dependencies
    /// (see `CONTRIBUTING.md`'s supply-chain policy), and this is plenty for
    /// a deterministic, seeded sweep over `split_bps`'s input space.
    struct Xorshift(u64);

    impl Xorshift {
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        /// Uniform in `[lo, hi]`.
        fn next_i128_in(&mut self, lo: i128, hi: i128) -> i128 {
            let span = hi
                .checked_sub(lo)
                .and_then(|difference| difference.checked_add(1))
                .and_then(|difference| u128::try_from(difference).ok())
                .expect("range should be positive and fit in u128");
            let offset = i128::try_from(u128::from(self.next_u64()) % span)
                .expect("offset should fit in i128");
            lo.checked_add(offset)
                .expect("generated value should remain within the requested range")
        }

        fn next_bps(&mut self) -> u32 {
            u32::try_from(self.next_u64() % u64::from(MAX_BPS).saturating_add(1))
                .expect("generated value should not exceed MAX_BPS")
        }
    }

    /// `split_bps`'s whole contract is that the two shares it returns always
    /// sum back to exactly the input, with no rounding leakage -- escrow
    /// dispute resolution depends on this holding for every valid `amount`
    /// and `bps`, not just the hand-picked doctest example.
    #[test]
    fn split_bps_sums_to_input_across_random_inputs() {
        let mut rng = Xorshift(0x5eed_u64.wrapping_add(1));

        for _ in 0..10_000 {
            let amount = rng.next_i128_in(0, 1_000_000_000_000);
            let bps = rng.next_bps();

            let (first, second) = split_bps(amount, bps)
                .unwrap_or_else(|err| panic!("split_bps({amount}, {bps}) failed: {err:?}"));

            assert_eq!(
                first + second,
                amount,
                "split_bps({amount}, {bps}) = ({first}, {second}), which does not sum to {amount}"
            );
            assert!(first >= 0, "first share went negative: {first}");
            assert!(second >= 0, "second share went negative: {second}");
        }
    }

    #[test]
    fn split_bps_rejects_bps_over_max() {
        assert_eq!(split_bps(100, MAX_BPS + 1), Err(Error::InvalidBasisPoints));
    }
}
