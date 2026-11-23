//! Checked, network-independent ledger timestamp arithmetic.
//!
//! These helpers are `no_std` and do not query the ledger. They are additive
//! utilities; existing stream and vesting callers can migrate separately.

use crate::Error;

/// Returns `start + duration`, or [`Error::Overflow`] on `u64` overflow.
///
/// A zero duration is permitted; validation of minimum duration belongs to
/// the caller.
///
/// # Examples
///
/// ```
/// use sororail_common::time;
///
/// assert_eq!(time::end_of(100, 30), Ok(130));
/// assert_eq!(time::end_of(u64::MAX, 1), Err(sororail_common::Error::Overflow));
/// ```
#[inline]
pub fn end_of(start: u64, duration: u64) -> Result<u64, Error> {
    start.checked_add(duration).ok_or(Error::Overflow)
}

/// Returns seconds elapsed between `from` and `to`, or zero when
/// `to <= from`. Never wraps on reversed timestamps.
///
/// # Examples
///
/// ```
/// use sororail_common::time;
///
/// assert_eq!(time::elapsed(100, 125), 25);
/// assert_eq!(time::elapsed(125, 100), 0);
/// ```
#[inline]
pub fn elapsed(from: u64, to: u64) -> u64 {
    to.saturating_sub(from)
}

#[cfg(test)]
mod tests {
    // Direct u64 boundary values are intentional test-only arithmetic.
    #![allow(clippy::arithmetic_side_effects)]

    use super::*;

    #[test]
    fn end_of_is_checked_at_u64_boundaries() {
        for (start, duration, expected) in [
            (0, 0, Ok(0)),
            (0, u64::MAX, Ok(u64::MAX)),
            (u64::MAX, 0, Ok(u64::MAX)),
            (u64::MAX - 1, 1, Ok(u64::MAX)),
            (u64::MAX, 1, Err(Error::Overflow)),
            (u64::MAX - 1, 2, Err(Error::Overflow)),
        ] {
            assert_eq!(end_of(start, duration), expected);
        }
    }

    #[test]
    fn elapsed_is_zero_for_equal_or_reversed_timestamps() {
        for (from, to, expected) in [
            (0, 0, 0),
            (0, 1, 1),
            (5, 5, 0),
            (5, 4, 0),
            (u64::MAX, 0, 0),
            (u64::MAX, u64::MAX, 0),
            (0, u64::MAX, u64::MAX),
            (u64::MAX - 1, u64::MAX, 1),
        ] {
            assert_eq!(elapsed(from, to), expected);
        }
    }
}
