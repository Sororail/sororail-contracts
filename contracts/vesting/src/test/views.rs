use super::*;

// issue #404: unvested and percent_vested_bps views

// ------------------------------------------------------- issue #404: unvested and percent_vested_bps views

#[test]
fn unvested_before_cliff() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    let unvested = g.unvested(START + CLIFF - 1).expect("unvested before cliff");
    assert_eq!(unvested, TOTAL);
}

#[test]
fn unvested_at_cliff() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    let vested = g.vested_amount(START + CLIFF).expect("vested at cliff");
    let unvested = g.unvested(START + CLIFF).expect("unvested at cliff");
    assert_eq!(vested + unvested, TOTAL);
}

#[test]
fn unvested_after_end() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    let unvested = g.unvested(START + DURATION + 1).expect("unvested after end");
    assert_eq!(unvested, 0);
}

#[test]
fn unvested_via_contract_view() {
    let f = Fixture::new(true);
    f.at(START + 500);
    let vested = f.client.vested_amount(&(START + 500));
    let unvested = f.client.unvested(&(START + 500));
    assert_eq!(vested + unvested, TOTAL);
}

#[test]
fn percent_vested_bps_zero_before_cliff() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    let percent = g.percent_vested_bps(START + CLIFF - 1).expect("percent before cliff");
    assert_eq!(percent, 0);
}

#[test]
fn percent_vested_bps_full_after_end() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    let percent = g.percent_vested_bps(START + DURATION + 1).expect("percent after end");
    assert_eq!(percent, 10_000);
}

#[test]
fn percent_vested_bps_monotonic() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    let mut prev = 0;
    for step in 0..=(DURATION + 1) {
        let percent = g
            .percent_vested_bps(START + step)
            .expect("percent at every step");
        assert!(percent >= prev, "percent_vested_bps must be monotonic");
        prev = percent;
    }
}

#[test]
fn percent_vested_bps_at_mid_schedule() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    let percent = g.percent_vested_bps(START + CLIFF + (DURATION - CLIFF) / 2)
        .expect("percent at midpoint");
    assert!(percent > 0 && percent < 10_000, "percent should be between cliff and end");
}

#[test]
fn percent_vested_bps_via_contract_view() {
    let f = Fixture::new(true);
    f.at(START + 500);
    let percent = f.client.percent_vested_bps(&(START + 500));
    assert!(percent > 0 && percent <= 10_000, "percent should be in [0, 10000]");
}

// issue #405: next_unlock_at view

// ------------------------------------------------------- issue #405: next_unlock_at view

#[test]
fn next_unlock_at_before_cliff_returns_cliff() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    let next = g.next_unlock_at(START).expect("next_unlock_at before cliff");
    assert_eq!(next, Some(START + CLIFF));
}

#[test]
fn next_unlock_at_at_cliff_returns_next_time() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    let next = g.next_unlock_at(START + CLIFF).expect("next_unlock_at at cliff");
    // Between cliff and end, return at + 1 as a poll hint
    assert_eq!(next, Some(START + CLIFF + 1));
}

#[test]
fn next_unlock_at_between_cliff_and_end_returns_next_time() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    let mid = START + CLIFF + (DURATION - CLIFF) / 2;
    let next = g.next_unlock_at(mid).expect("next_unlock_at between cliff and end");
    assert_eq!(next, Some(mid + 1));
}

#[test]
fn next_unlock_at_after_end_returns_none() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    let next = g.next_unlock_at(START + DURATION + 1);
    assert_eq!(next, None);
}

#[test]
fn next_unlock_at_at_end_returns_none() {
    let env = Env::default();
    let g = bare(&env, CLIFF, DURATION);
    let next = g.next_unlock_at(START + DURATION);
    assert_eq!(next, None);
}

#[test]
fn next_unlock_at_via_contract_view() {
    let f = Fixture::new(true);
    f.at(START + 100);
    let next = f.client.next_unlock_at(&(START + 100));
    assert!(next.is_some());
}

#[test]
fn next_unlock_at_cliffless_schedule() {
    let env = Env::default();
    let g = bare(&env, 0, DURATION);
    // No cliff, so immediately returns at + 1
    let next = g.next_unlock_at(START).expect("next_unlock_at with no cliff");
    assert_eq!(next, Some(START + 1));
}

#[test]
fn next_unlock_at_cliff_equals_duration() {
    let env = Env::default();
    let g = bare(&env, DURATION, DURATION);
    // Cliff equals duration, so only one unlock at cliff/end
    let next = g.next_unlock_at(START).expect("next_unlock_at before all-or-nothing unlock");
    assert_eq!(next, Some(START + DURATION));
}
