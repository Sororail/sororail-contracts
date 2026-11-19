use super::*;

// ------------------------------------------------ accrual (pure, no ledger)

#[test]
fn nothing_accrues_before_start() {
    let env = Env::default();
    let s = bare(&env);
    assert_eq!(s.accrued_at(0), Ok(0));
    assert_eq!(s.accrued_at(START - 1), Ok(0));
    assert_eq!(s.accrued_at(START), Ok(0));
}

#[test]
fn accrual_is_linear_between_start_and_stop() {
    let env = Env::default();
    let s = bare(&env);
    assert_eq!(s.accrued_at(START + 1), Ok(RATE));
    assert_eq!(s.accrued_at(START + 250), Ok(RATE * 250));
    assert_eq!(s.accrued_at(START + DURATION / 2), Ok(DEPOSITED / 2));
}

#[test]
fn accrual_is_capped_at_stop() {
    let env = Env::default();
    let s = bare(&env);
    assert_eq!(s.accrued_at(STOP), Ok(DEPOSITED));
    assert_eq!(s.accrued_at(STOP + 1), Ok(DEPOSITED));
    assert_eq!(s.accrued_at(u64::MAX), Ok(DEPOSITED));
}

#[test]
fn accrual_halts_at_cancellation() {
    let env = Env::default();
    let mut s = bare(&env);
    s.cancelled_at = Some(START + 100);
    assert_eq!(s.accrued_at(START + 100), Ok(RATE * 100));
    // Time moving on does not accrue any more.
    assert_eq!(s.accrued_at(START + 500), Ok(RATE * 100));
    assert_eq!(s.accrued_at(STOP + 5_000), Ok(RATE * 100));
}

#[test]
fn a_one_second_stream_accrues_exactly_one_tick() {
    let env = Env::default();
    let mut s = bare(&env);
    s.stop = START + 1;
    s.deposited = RATE;
    assert_eq!(s.accrued_at(START), Ok(0));
    assert_eq!(s.accrued_at(START + 1), Ok(RATE));
    assert_eq!(s.accrued_at(START + 2), Ok(RATE));
}

#[test]
fn accrual_reports_overflow_rather_than_wrapping() {
    let env = Env::default();
    let mut s = bare(&env);
    s.rate_per_second = i128::MAX;
    s.deposited = i128::MAX;
    assert_eq!(s.accrued_at(START + 2), Err(Error::Overflow));
}

#[test]
fn accrual_is_exact_at_a_rate_and_elapsed_that_divide_badly() {
    // Unlike vesting's `total * elapsed / duration`, accrual never divides,
    // so there is no rounding direction to get wrong: it is always exactly
    // `rate * elapsed`, even for a rate/elapsed pair that would leave a
    // remainder under any proportional (divide-then-multiply) formulation.
    let env = Env::default();
    let mut s = bare(&env);
    s.rate_per_second = 7; // prime; divides unevenly into most spans
    s.stop = START + 3; // an odd, non-multiple-of-7 span
    s.deposited = 7 * 3;

    assert_eq!(s.accrued_at(START + 1), Ok(7));
    assert_eq!(s.accrued_at(START + 2), Ok(14));
    assert_eq!(s.accrued_at(START + 3), Ok(21));
    // Capped at `stop`, not truncated to some lower rounded figure.
    assert_eq!(s.accrued_at(START + 3), Ok(s.deposited));
}

/// Documents that `available_at` degrades to `Err(Underflow)` if `withdrawn`
/// ever exceeds accrued (e.g. a future code path bug). Today's `withdraw`
/// rejects `requested > available`, so this state is unreachable through the
/// contract entry points; the pure helper must still not wrap to a negative
/// `i128` if it is reached.
#[test]
fn available_at_errors_if_withdrawn_exceeds_accrued() {
    let env = Env::default();
    let mut s = bare(&env);
    // Accrued at START+10 is RATE*10; force withdrawn past that.
    s.withdrawn = RATE * 10 + 1;
    assert_eq!(
        s.available_at(START + 10),
        Err(Error::Underflow),
        "available_at must error rather than wrap when withdrawn > accrued"
    );
    // Even after full accrual, an over-withdrawn book still errors.
    s.withdrawn = DEPOSITED + 1;
    assert_eq!(s.available_at(STOP), Err(Error::Underflow));
}
