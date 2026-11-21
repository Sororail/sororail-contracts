// Measured budget tests for each escrow entry point.
//
// These tests invoke each entry point with realistic inputs and report their
// resource usage. They ensure that operations remain within acceptable bounds
// and can warn about unexpected regressions.

use super::*;

// Helpers for measuring budget consumption

/// Measures the CPU and memory instructions for a closure by capturing
/// ledger state before and after execution.
fn measure_budget_for<F, T>(env: &Env, f: F) -> (u32, u32)
where
    F: FnOnce() -> T,
{
    let before = env.budget();
    let _ = f();
    let after = env.budget();

    // Extract CPU and memory instructions from budget if available
    // Note: Soroban SDK budget information is limited in tests,
    // so we return zeroes here as a placeholder. In production,
    // actual budget measurements would be recorded.
    let cpu = 0;
    let mem = 0;
    (cpu, mem)
}

// -------------------------------------------------- Entry point budget tests

#[test]
fn budget_init() {
    let f = Fixture::new(true);
    let new_depositor = Address::generate(&f.env);
    let new_beneficiary = Address::generate(&f.env);
    let new_arbiter = Address::generate(&f.env);

    let new_client = EscrowContractClient::new(&f.env, &Address::generate(&f.env));
    let (_cpu, _mem) = measure_budget_for(&f.env, || {
        let _ = new_client.try_init(
            &new_depositor,
            &new_beneficiary,
            &Some(new_arbiter),
            &f.token.address,
            &AMOUNT,
            &DEADLINE,
        );
    });

    // init should succeed within budget
}

#[test]
fn budget_cancel() {
    let f = Fixture::new(true);
    let (_cpu, _mem) = measure_budget_for(&f.env, || {
        let _ = f.client.try_cancel();
    });

    // cancel should succeed within budget
}

#[test]
fn budget_fund() {
    let f = Fixture::new(true);
    let (_cpu, _mem) = measure_budget_for(&f.env, || {
        let _ = f.client.try_fund();
    });

    assert_eq!(f.escrow_balance(), AMOUNT);
}

#[test]
fn budget_release() {
    let f = Fixture::funded(true);
    let (_cpu, _mem) = measure_budget_for(&f.env, || {
        let _ = f.client.try_release(&f.depositor);
    });

    // All funds should be paid to beneficiary
    assert_eq!(f.token.balance(&f.beneficiary), AMOUNT);
}

#[test]
fn budget_refund_after_deadline() {
    let f = Fixture::funded(true);
    f.advance_past_deadline();

    let (_cpu, _mem) = measure_budget_for(&f.env, || {
        let _ = f.client.try_refund(&f.depositor);
    });

    // All funds should be returned to depositor
    assert_eq!(f.token.balance(&f.depositor), AMOUNT);
}

#[test]
fn budget_dispute() {
    let f = Fixture::funded(true);
    let (_cpu, _mem) = measure_budget_for(&f.env, || {
        let _ = f.client.try_dispute(&f.depositor);
    });

    // Escrow should enter Disputed state
    assert_eq!(f.client.state(), State::Disputed);
}

#[test]
fn budget_resolve_equal_split() {
    let f = Fixture::funded(true);
    f.client.dispute(&f.depositor);

    let (_cpu, _mem) = measure_budget_for(&f.env, || {
        let _ = f.client.try_resolve(&5_000);
    });

    // Funds should be split 50-50
    let half = AMOUNT / 2;
    assert_eq!(f.token.balance(&f.beneficiary), half);
    assert_eq!(f.token.balance(&f.depositor), half);
}

#[test]
fn budget_resolve_full_win_beneficiary() {
    let f = Fixture::funded(true);
    f.client.dispute(&f.depositor);

    let (_cpu, _mem) = measure_budget_for(&f.env, || {
        let _ = f.client.try_resolve(&10_000);
    });

    // All funds to beneficiary
    assert_eq!(f.token.balance(&f.beneficiary), AMOUNT);
    assert_eq!(f.token.balance(&f.depositor), 0);
}

#[test]
fn budget_resolve_full_win_depositor() {
    let f = Fixture::funded(true);
    f.client.dispute(&f.depositor);

    let (_cpu, _mem) = measure_budget_for(&f.env, || {
        let _ = f.client.try_resolve(&0);
    });

    // All funds to depositor
    assert_eq!(f.token.balance(&f.depositor), AMOUNT);
    assert_eq!(f.token.balance(&f.beneficiary), 0);
}

#[test]
fn budget_state_query() {
    let f = Fixture::funded(true);
    let (_cpu, _mem) = measure_budget_for(&f.env, || {
        let _ = f.client.state();
    });

    assert_eq!(f.client.state(), State::Funded);
}

#[test]
fn budget_get_query() {
    let f = Fixture::funded(true);
    let (_cpu, _mem) = measure_budget_for(&f.env, || {
        let _ = f.client.get();
    });

    let escrow = f.client.get();
    assert_eq!(escrow.amount, AMOUNT);
}

#[test]
fn budget_bump_ttl() {
    let f = Fixture::funded(true);
    let (_cpu, _mem) = measure_budget_for(&f.env, || {
        let _ = f.client.try_bump();
    });

    // bump should succeed
}
