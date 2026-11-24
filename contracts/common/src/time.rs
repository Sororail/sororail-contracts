//! Checked `u64` timestamp arithmetic for SoroRail contracts.

use crate::Error;

/// Computes `start + duration` using checked arithmetic, returning
/// [`Error::Overflow`] if the sum exceeds `u64::MAX`.
///
/// # Examples
///
/// ```
/// use sororail_common::time;
///
/// assert_eq!(time::end_of(1_000, 500), Ok(1_500));
/// ```
#[inline]
pub fn end_of(start: u64, duration: u64) -> Result<u64, Error> {
    start.checked_add(duration).ok_or(Error::Overflow)
}

/// Returns the elapsed seconds between `from` and `to`, saturating at `0`
/// when `to <= from`.
///
/// # Examples
///
/// ```
/// use sororail_common::time;
///
/// assert_eq!(time::elapsed(100, 250), 150);
/// assert_eq!(time::elapsed(250, 100), 0);
/// ```
#[inline]
pub const fn elapsed(from: u64, to: u64) -> u64 {
    to.saturating_sub(from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_of_checks_u64_max_boundaries() {
        assert_eq!(end_of(0, 0), Ok(0));
        assert_eq!(end_of(1_000, 2_500), Ok(3_500));
        assert_eq!(end_of(u64::MAX, 0), Ok(u64::MAX));
        assert_eq!(end_of(0, u64::MAX), Ok(u64::MAX));
        assert_eq!(end_of(u64::MAX - 1, 1), Ok(u64::MAX));
        assert_eq!(end_of(u64::MAX, 1), Err(Error::Overflow));
        assert_eq!(end_of(u64::MAX, u64::MAX), Err(Error::Overflow));
    }

    #[test]
    fn elapsed_handles_ordered_equal_and_u64_max_boundaries() {
        assert_eq!(elapsed(0, 0), 0);
        assert_eq!(elapsed(100, 100), 0);
        assert_eq!(elapsed(200, 100), 0);
        assert_eq!(elapsed(100, 250), 150);
        assert_eq!(elapsed(0, u64::MAX), u64::MAX);
        assert_eq!(elapsed(u64::MAX, 0), 0);
        assert_eq!(elapsed(u64::MAX, u64::MAX), 0);
    }
}
