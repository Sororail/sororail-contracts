// Test fixtures do plain arithmetic on known-small constants; the checked-math
// rule is for contract code.
#![allow(clippy::arithmetic_side_effects)]

use soroban_sdk::{
    testutils::{Address as _, AuthorizedFunction, AuthorizedInvocation, Auths as _, Events as _},
    token::{StellarAssetClient, TokenClient},
    vec, Address, Env, Event, IntoVal, Vec,
};
use sororail_common::{testutils::TestEnv, Error};

use crate::{
    contract::{BatchPayoutContract, BatchPayoutContractClient},
    events::{Executed, ExecutedMany},
    types::{MultiTokenPayment, Payment, Receipt, MAX_RECIPIENTS},
};

const MINT: i128 = 1_000_000_000;

struct Fixture<'a> {
    env: Env,
    client: BatchPayoutContractClient<'a>,
    token: TokenClient<'a>,
    funder: Address,
}

impl Fixture<'_> {
    fn new() -> Self {
        let te = TestEnv::new();
        let (token, funder) = te.make_token(MINT);
        let token_address = token.address.clone();
        let env = te.env().clone();

        let client = BatchPayoutContractClient::new(&env, &env.register(BatchPayoutContract, ()));
        Fixture {
            token: TokenClient::new(&env, &token_address),
            env,
            client,
            funder,
        }
    }

    fn addresses(&self, n: u32) -> Vec<Address> {
        let mut v = Vec::new(&self.env);
        for _ in 0..n {
            v.push_back(Address::generate(&self.env));
        }
        v
    }

    fn payments(&self, n: u32, amount: i128) -> Vec<Payment> {
        let mut v = Vec::new(&self.env);
        for _ in 0..n {
            v.push_back(Payment {
                to: Address::generate(&self.env),
                amount,
            });
        }
        v
    }
}

// ----------------------------------------------------------------- execute

#[test]
fn execute_pays_each_recipient_their_own_amount() {
    let f = Fixture::new();
    let a = Address::generate(&f.env);
    let b = Address::generate(&f.env);
    let c = Address::generate(&f.env);
    let batch = vec![
        &f.env,
        Payment {
            to: a.clone(),
            amount: 100,
        },
        Payment {
            to: b.clone(),
            amount: 250,
        },
        Payment {
            to: c.clone(),
            amount: 7,
        },
    ];

    let receipt = f
        .client
        .execute(&f.funder, &f.token.address, &batch, &false);
    // Capture immediately: subsequent token balance invocations replace the
    // events returned by `events().all()`.
    let all = f.env.events().all();
    let events = all.events();
    assert_eq!(
        receipt,
        Receipt {
            count: 3,
            total: 357
        }
    );
    assert_eq!(f.token.balance(&a), 100);
    assert_eq!(f.token.balance(&b), 250);
    assert_eq!(f.token.balance(&c), 7);
    assert_eq!(f.token.balance(&f.funder), MINT - 357);
    // The contract is a conduit; it holds nothing.
    assert_eq!(f.token.balance(&f.client.address), 0);

    // `execute` emits one token transfer per recipient before its own
    // `Executed`, so take the last event rather than assuming an index.
    let executed_event = events.last().expect("no events emitted");
    let expected = Executed {
        funder: f.funder.clone(),
        token: f.token.address.clone(),
        count: 3,
        total: 357,
    };
    assert_eq!(executed_event, &expected.to_xdr(&f.env, &f.client.address));
}

#[test]
fn execute_emits_the_total_and_recipient_count() {
    let f = Fixture::new();
    let recipients = f.addresses(2);
    let payments = vec![
        &f.env,
        Payment {
            to: recipients.get(0).unwrap(),
            amount: 125,
        },
        Payment {
            to: recipients.get(1).unwrap(),
            amount: 875,
        },
    ];

    f.client
        .execute(&f.funder, &f.token.address, &payments, &false);

    let expected = Executed {
        funder: f.funder,
        token: f.token.address,
        count: 2,
        total: 1_000,
    };
    let all = f.env.events().all();
    let events = all.events();
    assert_eq!(
        events.last().expect("no events emitted"),
        &expected.to_xdr(&f.env, &f.client.address)
    );
}

#[test]
fn execute_leaves_no_token_balance_on_the_contract() {
    let f = Fixture::new();
    let payments = f.payments(3, 37);

    f.client
        .execute(&f.funder, &f.token.address, &payments, &false);

    assert_eq!(f.token.balance(&f.client.address), 0);
}

#[test]
fn execute_rejects_an_empty_batch() {
    let f = Fixture::new();
    let empty: Vec<Payment> = Vec::new(&f.env);
    assert_eq!(
        f.client
            .try_execute(&f.funder, &f.token.address, &empty, &false),
        Err(Ok(Error::BatchEmpty))
    );
}

#[test]
fn execute_rejects_a_batch_over_the_cap() {
    let f = Fixture::new();
    let batch = f.payments(MAX_RECIPIENTS + 1, 1);
    assert_eq!(
        f.client
            .try_execute(&f.funder, &f.token.address, &batch, &false),
        Err(Ok(Error::BatchTooLarge))
    );
}

#[test]
fn execute_rejects_non_positive_amounts_before_paying_anyone() {
    let f = Fixture::new();
    let good = Address::generate(&f.env);
    let bad = Address::generate(&f.env);
    for bad_amount in [0i128, -1, i128::MIN] {
        let batch = vec![
            &f.env,
            Payment {
                to: good.clone(),
                amount: 100,
            },
            Payment {
                to: bad.clone(),
                amount: bad_amount,
            },
        ];
        assert_eq!(
            f.client
                .try_execute(&f.funder, &f.token.address, &batch, &false),
            Err(Ok(Error::InvalidAmount)),
            "amount {bad_amount} was accepted"
        );
        // Validation happens before any transfer, so the valid line in the
        // same batch was never paid either.
        assert_eq!(f.token.balance(&good), 0);
        assert_eq!(f.token.balance(&f.funder), MINT);
    }
}

#[test]
fn execute_rejects_a_zero_amount_payment_inside_a_larger_batch() {
    // The other non-positive-amount test only uses a 2-line batch; this
    // checks the same all-or-nothing validation holds with several valid
    // lines around the bad one, not just a single good line next to it (#264).
    let f = Fixture::new();
    let recipients: std::vec::Vec<Address> = (0..5).map(|_| Address::generate(&f.env)).collect();
    let mut batch = Vec::new(&f.env);
    for (i, to) in recipients.iter().enumerate() {
        batch.push_back(Payment {
            to: to.clone(),
            // The third of five payments is the zero amount.
            amount: if i == 2 { 0 } else { 100 },
        });
    }

    assert_eq!(
        f.client
            .try_execute(&f.funder, &f.token.address, &batch, &false),
        Err(Ok(Error::InvalidAmount))
    );
    // All-or-nothing: none of the four otherwise-valid lines were paid either.
    for to in &recipients {
        assert_eq!(f.token.balance(to), 0);
    }
    assert_eq!(f.token.balance(&f.funder), MINT);
}

#[test]
fn preview_rejects_a_zero_amount_payment_inside_a_larger_batch() {
    // `preview` runs the same validation as `execute` (#264); nothing to
    // assert about balances since preview never moves funds.
    let f = Fixture::new();
    let mut batch = Vec::new(&f.env);
    for i in 0..5 {
        batch.push_back(Payment {
            to: Address::generate(&f.env),
            amount: if i == 2 { 0 } else { 100 },
        });
    }

    assert_eq!(
        f.client.try_preview(&batch, &false),
        Err(Ok(Error::InvalidAmount))
    );
}

#[test]
fn execute_rejects_a_total_that_overflows() {
    let f = Fixture::new();
    let batch = vec![
        &f.env,
        Payment {
            to: Address::generate(&f.env),
            amount: i128::MAX,
        },
        Payment {
            to: Address::generate(&f.env),
            amount: 1,
        },
    ];
    assert_eq!(
        f.client
            .try_execute(&f.funder, &f.token.address, &batch, &false),
        Err(Ok(Error::Overflow))
    );
}

#[test]
fn a_batch_the_funder_cannot_afford_pays_nobody() {
    // All-or-nothing: the host reverts the transfers already made when a later
    // one fails on insufficient balance.
    let f = Fixture::new();
    let a = Address::generate(&f.env);
    let b = Address::generate(&f.env);
    let batch = vec![
        &f.env,
        Payment {
            to: a.clone(),
            amount: 100,
        },
        Payment {
            to: b.clone(),
            amount: MINT,
        },
    ];

    assert!(f
        .client
        .try_execute(&f.funder, &f.token.address, &batch, &false)
        .is_err());
    assert_eq!(f.token.balance(&a), 0, "a partial payout survived");
    assert_eq!(f.token.balance(&b), 0);
    assert_eq!(f.token.balance(&f.funder), MINT);
}

/// Same host-revert guarantee as the other contracts: a failed mid-batch
/// transfer leaves no partial mutation (batch_payout is otherwise stateless).
#[test]
fn execute_reverts_cleanly_when_a_transfer_fails() {
    a_batch_the_funder_cannot_afford_pays_nobody();
}

#[test]
fn paying_the_same_address_twice_is_allowed() {
    // Duplicates are legitimate -- two invoices for one contractor -- and are
    // deliberately not rejected on-chain.
    let f = Fixture::new();
    let a = Address::generate(&f.env);
    let batch = vec![
        &f.env,
        Payment {
            to: a.clone(),
            amount: 100,
        },
        Payment {
            to: a.clone(),
            amount: 50,
        },
    ];
    let receipt = f
        .client
        .execute(&f.funder, &f.token.address, &batch, &false);
    assert_eq!(receipt.total, 150);
    assert_eq!(f.token.balance(&a), 150);
}

/// Issue #479: pins the behaviour left by removing duplicate detection.
///
/// Error 102 was `BatchDuplicateRecipient`; it was removed before release
/// (see the "What is absent, and why" note in `errors.rs` and the reserved
/// slot in `sororail_common::errors`) because an on-chain duplicate check
/// costs a quadratic scan and paying one address twice is legitimate. The
/// consequence is that both transfers occur: the recipient receives the sum
/// and the receipt counts each line separately.
#[test]
fn duplicate_recipients_are_each_paid_and_counted_in_the_receipt() {
    let f = Fixture::new();
    let dup = Address::generate(&f.env);
    let other = Address::generate(&f.env);
    let batch = vec![
        &f.env,
        Payment {
            to: dup.clone(),
            amount: 100,
        },
        Payment {
            to: other.clone(),
            amount: 25,
        },
        Payment {
            to: dup.clone(),
            amount: 50,
        },
    ];

    let receipt = f
        .client
        .execute(&f.funder, &f.token.address, &batch, &false);
    assert_eq!(
        receipt,
        Receipt {
            count: 3,
            total: 175
        }
    );
    // Both lines for the duplicate address arrived, summed.
    assert_eq!(f.token.balance(&dup), 150);
    assert_eq!(f.token.balance(&other), 25);
    assert_eq!(f.token.balance(&f.funder), MINT - 175);
    assert_eq!(f.token.balance(&f.client.address), 0);
}

/// Same as above for `execute_equal`: the same address listed twice is paid
/// twice and counted twice (no `BatchDuplicateRecipient`, see error 102).
#[test]
fn execute_equal_pays_a_duplicate_recipient_once_per_line() {
    let f = Fixture::new();
    let dup = Address::generate(&f.env);
    let recipients = vec![&f.env, dup.clone(), dup.clone()];

    let receipt =
        f.client
            .execute_equal(&f.funder, &f.token.address, &recipients, &40, &false);
    assert_eq!(
        receipt,
        Receipt {
            count: 2,
            total: 80
        }
    );
    assert_eq!(f.token.balance(&dup), 80);
    assert_eq!(f.token.balance(&f.funder), MINT - 80);
}

#[test]
#[should_panic]
fn execute_requires_the_funders_authorization() {
    let env = Env::default();
    let funder = Address::generate(&env);
    let issuer = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(issuer).address();
    let client = BatchPayoutContractClient::new(&env, &env.register(BatchPayoutContract, ()));
    let batch = vec![
        &env,
        Payment {
            to: Address::generate(&env),
            amount: 100,
        },
    ];
    client.execute(&funder, &token, &batch, &false);
}

#[test]
fn execute_rejects_payment_to_contract_address() {
    let f = Fixture::new();
    let contract = f.client.address.clone();
    let batch = vec![
        &f.env,
        Payment {
            to: contract,
            amount: 100,
        },
    ];
    assert_eq!(
        f.client
            .try_execute(&f.funder, &f.token.address, &batch, &false),
        Err(Ok(Error::IdenticalParties))
    );
}

#[test]
fn preview_rejects_payment_to_contract_address() {
    let f = Fixture::new();
    let contract = f.client.address.clone();
    let batch = vec![
        &f.env,
        Payment {
            to: contract,
            amount: 100,
        },
    ];
    assert_eq!(
        f.client.try_preview(&batch, &false),
        Err(Ok(Error::IdenticalParties))
    );
}

#[test]
fn execute_rejects_multiple_invalid_args_in_order() {
    let f = Fixture::new();
    let contract = f.client.address.clone();
    // Both empty batch AND self-payment: size check comes first
    let empty_batch: Vec<Payment> = Vec::new(&f.env);
    assert_eq!(
        f.client
            .try_execute(&f.funder, &f.token.address, &empty_batch, &false),
        Err(Ok(Error::BatchEmpty))
    );

    // Self-payment AND zero amount: self-payment check comes first
    let batch = vec![
        &f.env,
        Payment {
            to: contract,
            amount: 0,
        },
    ];
    assert_eq!(
        f.client
            .try_execute(&f.funder, &f.token.address, &batch, &false),
        Err(Ok(Error::IdenticalParties))
    );
}

// ----------------------------------------------------------- execute_equal

#[test]
fn execute_equal_pays_everyone_the_same() {
    let f = Fixture::new();
    let recipients = f.addresses(5);
    let receipt =
        f.client
            .execute_equal(&f.funder, &f.token.address, &recipients, &200, &false);

    assert_eq!(
        receipt,
        Receipt {
            count: 5,
            total: 1_000
        }
    );
    for r in recipients.iter() {
        assert_eq!(f.token.balance(&r), 200);
    }
    assert_eq!(f.token.balance(&f.funder), MINT - 1_000);
}

#[test]
fn execute_equal_applies_the_exact_amount_without_rounding() {
    let f = Fixture::new();
    let recipients = f.addresses(3);

    let receipt =
        f.client
            .execute_equal(&f.funder, &f.token.address, &recipients, &7, &false);

    assert_eq!(
        receipt,
        Receipt {
            count: 3,
            total: 21
        }
    );
    for recipient in recipients.iter() {
        assert_eq!(f.token.balance(&recipient), 7);
    }
}

#[test]
fn execute_equal_rejects_a_non_positive_amount() {
    let f = Fixture::new();
    let recipients = f.addresses(3);
    for bad in [0i128, -1, i128::MIN] {
        assert_eq!(
            f.client
                .try_execute_equal(&f.funder, &f.token.address, &recipients, &bad, &false),
            Err(Ok(Error::InvalidAmount)),
            "amount {bad} was accepted"
        );
    }
}

#[test]
fn execute_equal_rejects_an_empty_batch() {
    let f = Fixture::new();
    let empty: Vec<Address> = Vec::new(&f.env);
    assert_eq!(
        f.client
            .try_execute_equal(&f.funder, &f.token.address, &empty, &100, &false),
        Err(Ok(Error::BatchEmpty))
    );
}

#[test]
fn execute_equal_rejects_a_batch_over_the_cap() {
    let f = Fixture::new();
    let recipients = f.addresses(MAX_RECIPIENTS + 1);
    assert_eq!(
        f.client
            .try_execute_equal(&f.funder, &f.token.address, &recipients, &1, &false),
        Err(Ok(Error::BatchTooLarge))
    );
}

#[test]
fn execute_equal_rejects_contract_address_in_recipients() {
    let f = Fixture::new();
    let contract = f.client.address.clone();
    let mut recipients = f.addresses(2);
    recipients.push_back(contract);
    assert_eq!(
        f.client
            .try_execute_equal(&f.funder, &f.token.address, &recipients, &100, &false),
        Err(Ok(Error::IdenticalParties))
    );
}

#[test]
fn execute_equal_rejects_multiple_invalid_args_in_order() {
    let f = Fixture::new();
    let contract = f.client.address.clone();
    // Non-positive amount comes before size check
    assert_eq!(
        f.client
            .try_execute_equal(&f.funder, &f.token.address, &f.addresses(0), &0, &false),
        Err(Ok(Error::InvalidAmount))
    );

    // Size check comes before self-payment check
    let mut oversized = f.addresses(MAX_RECIPIENTS + 1);
    oversized.push_back(contract.clone());
    assert_eq!(
        f.client
            .try_execute_equal(&f.funder, &f.token.address, &oversized, &100, &false),
        Err(Ok(Error::BatchTooLarge))
    );

    // Self-payment check comes before total calculation
    let recipients = vec![&f.env, contract];
    assert_eq!(
        f.client
            .try_execute_equal(&f.funder, &f.token.address, &recipients, &i128::MAX, &false),
        Err(Ok(Error::IdenticalParties))
    );
}

#[test]
fn execute_equal_rejects_a_total_that_overflows() {
    let f = Fixture::new();
    let recipients = f.addresses(2);
    assert_eq!(
        f.client
            .try_execute_equal(&f.funder, &f.token.address, &recipients, &i128::MAX, &false),
        Err(Ok(Error::Overflow))
    );
}

// ----------------------------------------------------------------- preview

#[test]
fn preview_totals_without_moving_anything() {
    let f = Fixture::new();
    let batch = f.payments(4, 25);
    let receipt = f.client.preview(&batch, &false);
    assert_eq!(
        receipt,
        Receipt {
            count: 4,
            total: 100
        }
    );
    assert_eq!(f.token.balance(&f.funder), MINT);
}

#[test]
fn preview_returns_the_correct_receipt_and_moves_nothing() {
    // The existing uniform-amount preview test can't catch a `Receipt` whose
    // `total` happens to equal `count * amount` by coincidence; this uses
    // distinct amounts so `count` and `total` are independently verified,
    // and checks every balance plus the event log, not just the funder's
    // (#265).
    let f = Fixture::new();
    let a = Address::generate(&f.env);
    let b = Address::generate(&f.env);
    let c = Address::generate(&f.env);
    let batch = vec![
        &f.env,
        Payment {
            to: a.clone(),
            amount: 11,
        },
        Payment {
            to: b.clone(),
            amount: 22,
        },
        Payment {
            to: c.clone(),
            amount: 33,
        },
    ];

    let receipt = f.client.preview(&batch, &false);
    assert_eq!(
        receipt,
        Receipt {
            count: 3,
            total: 66
        }
    );

    assert_eq!(f.token.balance(&a), 0);
    assert_eq!(f.token.balance(&b), 0);
    assert_eq!(f.token.balance(&c), 0);
    assert_eq!(f.token.balance(&f.funder), MINT);
    assert_eq!(f.token.balance(&f.client.address), 0);
    // Unlike `execute`, `preview` never publishes an `Executed` event.
    assert!(f.env.events().all().events().is_empty());
}

#[test]
fn preview_rejects_exactly_what_execute_rejects() {
    let f = Fixture::new();
    let empty: Vec<Payment> = Vec::new(&f.env);
    assert_eq!(
        f.client.try_preview(&empty, &false),
        Err(Ok(Error::BatchEmpty))
    );

    let too_many = f.payments(MAX_RECIPIENTS + 1, 1);
    assert_eq!(
        f.client.try_preview(&too_many, &false),
        Err(Ok(Error::BatchTooLarge))
    );

    let bad = vec![
        &f.env,
        Payment {
            to: Address::generate(&f.env),
            amount: 0,
        },
    ];
    assert_eq!(
        f.client.try_preview(&bad, &false),
        Err(Ok(Error::InvalidAmount))
    );
}

// -------------------------------------------------------------- the cap

#[test]
fn max_recipients_is_reported_to_clients() {
    let f = Fixture::new();
    assert_eq!(f.client.max_recipients(), MAX_RECIPIENTS);
}

#[test]
fn a_full_size_batch_executes() {
    let f = Fixture::new();
    let recipients = f.addresses(MAX_RECIPIENTS);
    let receipt =
        f.client
            .execute_equal(&f.funder, &f.token.address, &recipients, &10, &false);
    assert_eq!(receipt.count, MAX_RECIPIENTS);
    assert_eq!(receipt.total, 10 * i128::from(MAX_RECIPIENTS));
}

/// Largest batch that executes under the test environment's budget.
///
/// Returns `None` if even the smallest probe fails. Resource exhaustion
/// surfaces as a host panic rather than a returnable error, so the probe has
/// to catch unwinds; the panic hook is silenced for the duration so the
/// expected failures do not fill the test log.
fn largest_batch_that_executes(candidates: &[u32]) -> Option<u32> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(std::boxed::Box::new(|_| {}));

    let mut highest = None;
    for &n in candidates {
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let env = Env::default();
            env.mock_all_auths();
            let funder = Address::generate(&env);
            let issuer = Address::generate(&env);
            let token = env.register_stellar_asset_contract_v2(issuer).address();
            StellarAssetClient::new(&env, &token).mint(&funder, &MINT);
            let client =
                BatchPayoutContractClient::new(&env, &env.register(BatchPayoutContract, ()));

            let mut recipients = Vec::new(&env);
            for _ in 0..n {
                recipients.push_back(Address::generate(&env));
            }
            // Call the entry point directly, bypassing MAX_RECIPIENTS, so the
            // probe measures resources rather than our own cap.
            client.execute_equal(&funder, &token, &recipients, &1, &false);
        }))
        .is_ok();

        if ok {
            highest = Some(n);
        } else {
            break;
        }
    }

    std::panic::set_hook(previous);
    highest
}

#[test]
fn max_recipients_is_within_what_actually_executes() {
    // SPEC.md requires the cap be discovered empirically rather than guessed.
    // An earlier guess of 100 exceeded the budget, which is why this exists.
    let ceiling = largest_batch_that_executes(&[10, 20, 30, 40, 50, 60, 70, 80, 90, 100])
        .expect("even a 10-recipient batch failed to execute");
    std::println!("measured batch ceiling in the test environment: {ceiling} recipients");
    assert!(
        MAX_RECIPIENTS <= ceiling,
        "MAX_RECIPIENTS ({MAX_RECIPIENTS}) exceeds what executes ({ceiling})"
    );
}

#[test]
#[ignore = "reports the measured ceiling; run with --ignored --nocapture"]
fn report_batch_ceiling() {
    // Finer-grained sweep, for when the cap is being re-derived.
    //
    // Caveat that keeps MAX_RECIPIENTS conservative: the local test
    // environment models the CPU/memory budget but NOT the transaction size
    // limit a real network applies to the submitted envelope, and its
    // `mock_all_auths` builds a separate authorization entry per transfer
    // where a real submission signs one tree. So this measures a budget
    // ceiling, not the on-chain ceiling. Re-run against testnet before v0.2.
    let candidates: std::vec::Vec<u32> = (1..=20).map(|i| i * 5).collect();
    let ceiling = largest_batch_that_executes(&candidates);
    std::println!("measured batch ceiling in the test environment: {ceiling:?}");
}

// ----------------------------------------- #431: per-recipient budget cost

/// CPU instructions and memory bytes one `execute` of `n` recipients consumes.
///
/// Each size gets a fresh environment so set-up (token registration, minting,
/// address generation) is excluded: the budget is reset to unlimited
/// immediately before the call, which both zeroes the counters and lets sizes
/// above the default-budget ceiling (e.g. 50) run to completion.
fn measure_execute_cost(n: u32) -> (u64, u64) {
    let env = Env::default();
    env.mock_all_auths();
    let funder = Address::generate(&env);
    let issuer = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(issuer).address();
    StellarAssetClient::new(&env, &token).mint(&funder, &MINT);
    let client = BatchPayoutContractClient::new(&env, &env.register(BatchPayoutContract, ()));

    let mut recipients = Vec::new(&env);
    for _ in 0..n {
        recipients.push_back(Payment {
            to: Address::generate(&env),
            amount: 1,
        });
    }

    let mut budget = env.cost_estimate().budget();
    budget.reset_unlimited();
    budget.reset_tracker();
    let receipt = client.execute(&funder, &token, &recipients, &false);
    assert_eq!(receipt.count, n);

    let budget = env.cost_estimate().budget();
    (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
}

/// Relative difference between two marginal costs, as a fraction of the larger.
fn relative_spread(a: f64, b: f64) -> f64 {
    (a - b).abs() / a.max(b)
}

#[test]
fn per_recipient_cost_grows_linearly() {
    // #431: MAX_RECIPIENTS is derived from the budget, so record how the
    // per-recipient cost was reached. Measures 10, 50 and MAX_RECIPIENTS and
    // asserts that the marginal cost of each extra recipient is the same
    // (within tolerance) across both intervals, i.e. cost is linear in n.
    // Run with `--nocapture` to see the table this prints.
    const TOLERANCE: f64 = 0.10;

    let mut sizes = [10, MAX_RECIPIENTS, 50];
    sizes.sort_unstable();
    assert!(
        sizes[0] < sizes[1] && sizes[1] < sizes[2],
        "sizes must be distinct to measure two intervals: {sizes:?}"
    );

    let costs: std::vec::Vec<(u32, u64, u64)> = sizes
        .iter()
        .map(|&n| {
            let (cpu, mem) = measure_execute_cost(n);
            (n, cpu, mem)
        })
        .collect();

    std::println!("batch_payout::execute budget cost (MAX_RECIPIENTS = {MAX_RECIPIENTS})");
    std::println!(
        "{:>10} {:>14} {:>12} {:>14} {:>12}",
        "recipients",
        "cpu_insns",
        "mem_bytes",
        "cpu/recipient",
        "mem/recipient"
    );
    for &(n, cpu, mem) in &costs {
        std::println!(
            "{:>10} {:>14} {:>12} {:>14} {:>12}",
            n,
            cpu,
            mem,
            cpu / u64::from(n),
            mem / u64::from(n)
        );
    }

    let marginal = |lo: (u32, u64, u64), hi: (u32, u64, u64)| {
        let dn = f64::from(hi.0 - lo.0);
        (
            (hi.1 as f64 - lo.1 as f64) / dn,
            (hi.2 as f64 - lo.2 as f64) / dn,
        )
    };
    let (cpu_a, mem_a) = marginal(costs[0], costs[1]);
    let (cpu_b, mem_b) = marginal(costs[1], costs[2]);
    std::println!(
        "marginal cpu/recipient: {cpu_a:.0} ({}..{}) vs {cpu_b:.0} ({}..{})",
        costs[0].0,
        costs[1].0,
        costs[1].0,
        costs[2].0
    );
    std::println!(
        "marginal mem/recipient: {mem_a:.0} ({}..{}) vs {mem_b:.0} ({}..{})",
        costs[0].0,
        costs[1].0,
        costs[1].0,
        costs[2].0
    );

    assert!(cpu_a > 0.0 && mem_a > 0.0, "cost must grow with recipients");
    assert!(
        relative_spread(cpu_a, cpu_b) <= TOLERANCE,
        "cpu cost per recipient is not linear: {cpu_a:.0} vs {cpu_b:.0}"
    );
    assert!(
        relative_spread(mem_a, mem_b) <= TOLERANCE,
        "memory cost per recipient is not linear: {mem_a:.0} vs {mem_b:.0}"
    );
}

// ------------------------------------------------ #435: env.auths() assertions

#[test]
fn execute_records_the_exact_authorised_invocation() {
    // #435: every state-changing entry point must be covered by an env.auths()
    // assertion verifying that the funder authorised the specific call.
    let f = Fixture::new();
    let to = Address::generate(&f.env);
    let batch = vec![
        &f.env,
        Payment {
            to: to.clone(),
            amount: 100,
        },
    ];

    f.client
        .execute(&f.funder, &f.token.address, &batch, &false);

    assert_eq!(
        f.env.auths(),
        std::vec![(
            f.funder.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    f.client.address.clone(),
                    soroban_sdk::symbol_short!("execute"),
                    (&f.funder, &f.token.address, &batch, &false).into_val(&f.env),
                )),
                sub_invocations: std::vec![AuthorizedInvocation {
                    function: AuthorizedFunction::Contract((
                        f.token.address.clone(),
                        soroban_sdk::symbol_short!("transfer"),
                        (&f.funder, &to, &100_i128).into_val(&f.env),
                    )),
                    sub_invocations: std::vec![],
                }],
            }
        )]
    );
}

#[test]
fn execute_equal_records_the_exact_authorised_invocation() {
    // #435: execute_equal must record an auth entry for the funder.
    let f = Fixture::new();
    let to = Address::generate(&f.env);
    let recipients = vec![&f.env, to.clone()];

    f.client
        .execute_equal(&f.funder, &f.token.address, &recipients, &200, &false);

    assert_eq!(
        f.env.auths(),
        std::vec![(
            f.funder.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    f.client.address.clone(),
                    soroban_sdk::symbol_short!("execute_equal"),
                    (&f.funder, &f.token.address, &recipients, &200_i128, &false)
                        .into_val(&f.env),
                )),
                sub_invocations: std::vec![AuthorizedInvocation {
                    function: AuthorizedFunction::Contract((
                        f.token.address.clone(),
                        soroban_sdk::symbol_short!("transfer"),
                        (&f.funder, &to, &200_i128).into_val(&f.env),
                    )),
                    sub_invocations: std::vec![],
                }],
            }
        )]
    );
}

#[test]
fn execute_many_tokens_records_the_exact_authorised_invocation() {
    // #435: execute_many_tokens must record an auth entry for the funder.
    let te = TestEnv::new();
    let (token_a, funder) = te.make_token(MINT);
    let (token_b, _) = te.make_token(0);
    te.mint(&token_b, &funder, MINT);
    let env = te.env().clone();
    let client = BatchPayoutContractClient::new(&env, &env.register(BatchPayoutContract, ()));

    let to_a = Address::generate(&env);
    let to_b = Address::generate(&env);
    let payments = vec![
        &env,
        MultiTokenPayment {
            token: token_a.address.clone(),
            to: to_a.clone(),
            amount: 100,
        },
        MultiTokenPayment {
            token: token_b.address.clone(),
            to: to_b.clone(),
            amount: 200,
        },
    ];

    client.execute_many_tokens(&funder, &payments, &false);

    let auths = env.auths();
    assert_eq!(auths.len(), 1, "expected one top-level auth entry");
    let (auth_addr, auth_inv) = &auths[0];
    assert_eq!(auth_addr, &funder);
    assert_eq!(
        auth_inv.function,
        AuthorizedFunction::Contract((
            client.address.clone(),
            soroban_sdk::symbol_short!("execute_many_tokens"),
            (&funder, &payments, &false).into_val(&env),
        ))
    );
    // Two sub-invocations: one token transfer per payment line.
    assert_eq!(auth_inv.sub_invocations.len(), 2);
}

// ---------------------------------------- #433: only_eoa gate

#[test]
fn execute_rejects_contract_recipient_when_only_eoa_is_set() {
    // A registered contract address should be rejected when only_eoa=true.
    let f = Fixture::new();
    let contract_recipient = f.client.address.clone();
    // Use a different contract so we don't hit IdenticalParties first.
    let other_contract = f.env.register(BatchPayoutContract, ());
    let batch = vec![
        &f.env,
        Payment {
            to: other_contract,
            amount: 100,
        },
    ];
    assert_eq!(
        f.client
            .try_execute(&f.funder, &f.token.address, &batch, &true),
        Err(Ok(Error::BatchRecipientIsContract))
    );
    // The same batch succeeds when only_eoa is false.
    // (Payment will fail due to insufficient balance on the other contract, but
    // the validation itself passes; we only check the error type here.)
    let _ = f
        .client
        .try_execute(&f.funder, &f.token.address, &batch, &false);
    _ = contract_recipient; // silence unused warning
}

#[test]
fn execute_allows_account_recipients_when_only_eoa_is_set() {
    let f = Fixture::new();
    let batch = f.payments(2, 50);
    // With only_eoa=true, normal account recipients should still work.
    let receipt = f
        .client
        .execute(&f.funder, &f.token.address, &batch, &true);
    assert_eq!(receipt.count, 2);
    assert_eq!(receipt.total, 100);
}

#[test]
fn execute_equal_rejects_contract_recipient_when_only_eoa_is_set() {
    let f = Fixture::new();
    let other_contract = f.env.register(BatchPayoutContract, ());
    let recipients = vec![&f.env, other_contract];
    assert_eq!(
        f.client
            .try_execute_equal(&f.funder, &f.token.address, &recipients, &100, &true),
        Err(Ok(Error::BatchRecipientIsContract))
    );
}

#[test]
fn execute_equal_allows_account_recipients_when_only_eoa_is_set() {
    let f = Fixture::new();
    let recipients = f.addresses(3);
    let receipt = f
        .client
        .execute_equal(&f.funder, &f.token.address, &recipients, &10, &true);
    assert_eq!(receipt.count, 3);
}

#[test]
fn preview_rejects_contract_recipient_when_only_eoa_is_set() {
    let f = Fixture::new();
    let other_contract = f.env.register(BatchPayoutContract, ());
    let batch = vec![
        &f.env,
        Payment {
            to: other_contract,
            amount: 100,
        },
    ];
    assert_eq!(
        f.client.try_preview(&batch, &true),
        Err(Ok(Error::BatchRecipientIsContract))
    );
    // Without only_eoa: passes validation (no token check in preview).
    assert!(f.client.try_preview(&batch, &false).is_ok());
}

#[test]
fn only_eoa_check_comes_after_identical_parties_check() {
    // BatchPayoutContract's own address is both a contract AND the same as
    // env.current_contract_address(). The IdenticalParties check runs first
    // so the error is IdenticalParties, not BatchRecipientIsContract.
    let f = Fixture::new();
    let batch = vec![
        &f.env,
        Payment {
            to: f.client.address.clone(),
            amount: 100,
        },
    ];
    assert_eq!(
        f.client
            .try_execute(&f.funder, &f.token.address, &batch, &true),
        Err(Ok(Error::IdenticalParties))
    );
}

// ---------------------------------------- #432: execute_many_tokens

#[test]
fn execute_many_tokens_pays_each_recipient_from_their_own_token() {
    let te = TestEnv::new();
    let (token_a, funder) = te.make_token(MINT);
    let (token_b, _) = te.make_token(0);
    te.mint(&token_b, &funder, MINT);
    let env = te.env().clone();
    let client = BatchPayoutContractClient::new(&env, &env.register(BatchPayoutContract, ()));
    let token_a_client = TokenClient::new(&env, &token_a.address);
    let token_b_client = TokenClient::new(&env, &token_b.address);

    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let payments = vec![
        &env,
        MultiTokenPayment {
            token: token_a.address.clone(),
            to: a.clone(),
            amount: 300,
        },
        MultiTokenPayment {
            token: token_b.address.clone(),
            to: b.clone(),
            amount: 500,
        },
    ];

    let receipt = client.execute_many_tokens(&funder, &payments, &false);

    assert_eq!(receipt, Receipt { count: 2, total: 800 });
    assert_eq!(token_a_client.balance(&a), 300);
    assert_eq!(token_b_client.balance(&b), 500);
    assert_eq!(token_a_client.balance(&funder), MINT - 300);
    assert_eq!(token_b_client.balance(&funder), MINT - 500);
    // The contract holds nothing.
    assert_eq!(token_a_client.balance(&client.address), 0);
    assert_eq!(token_b_client.balance(&client.address), 0);
}

#[test]
fn execute_many_tokens_emits_executed_many_event() {
    let te = TestEnv::new();
    let (token_a, funder) = te.make_token(MINT);
    let env = te.env().clone();
    let client = BatchPayoutContractClient::new(&env, &env.register(BatchPayoutContract, ()));

    let payments = vec![
        &env,
        MultiTokenPayment {
            token: token_a.address.clone(),
            to: Address::generate(&env),
            amount: 100,
        },
    ];

    client.execute_many_tokens(&funder, &payments, &false);

    let all = env.events().all();
    let events = all.events();
    let last = events.last().expect("no events emitted");
    let expected = ExecutedMany {
        funder: funder.clone(),
        count: 1,
    };
    assert_eq!(last, &expected.to_xdr(&env, &client.address));
}

#[test]
fn execute_many_tokens_rejects_an_empty_batch() {
    let te = TestEnv::new();
    let (_, funder) = te.make_token(MINT);
    let env = te.env().clone();
    let client = BatchPayoutContractClient::new(&env, &env.register(BatchPayoutContract, ()));
    let empty: Vec<MultiTokenPayment> = Vec::new(&env);
    assert_eq!(
        client.try_execute_many_tokens(&funder, &empty, &false),
        Err(Ok(Error::BatchEmpty))
    );
}

#[test]
fn execute_many_tokens_rejects_a_batch_over_the_cap() {
    let te = TestEnv::new();
    let (token_a, funder) = te.make_token(MINT);
    let env = te.env().clone();
    let client = BatchPayoutContractClient::new(&env, &env.register(BatchPayoutContract, ()));

    let mut payments = Vec::new(&env);
    for _ in 0..MAX_RECIPIENTS + 1 {
        payments.push_back(MultiTokenPayment {
            token: token_a.address.clone(),
            to: Address::generate(&env),
            amount: 1,
        });
    }
    assert_eq!(
        client.try_execute_many_tokens(&funder, &payments, &false),
        Err(Ok(Error::BatchTooLarge))
    );
}

#[test]
fn execute_many_tokens_rejects_non_positive_amount() {
    let te = TestEnv::new();
    let (token_a, funder) = te.make_token(MINT);
    let env = te.env().clone();
    let client = BatchPayoutContractClient::new(&env, &env.register(BatchPayoutContract, ()));

    let payments = vec![
        &env,
        MultiTokenPayment {
            token: token_a.address.clone(),
            to: Address::generate(&env),
            amount: 0,
        },
    ];
    assert_eq!(
        client.try_execute_many_tokens(&funder, &payments, &false),
        Err(Ok(Error::InvalidAmount))
    );
}

#[test]
fn execute_many_tokens_rejects_payment_to_contract_address() {
    let te = TestEnv::new();
    let (token_a, funder) = te.make_token(MINT);
    let env = te.env().clone();
    let client = BatchPayoutContractClient::new(&env, &env.register(BatchPayoutContract, ()));

    let payments = vec![
        &env,
        MultiTokenPayment {
            token: token_a.address.clone(),
            to: client.address.clone(),
            amount: 100,
        },
    ];
    assert_eq!(
        client.try_execute_many_tokens(&funder, &payments, &false),
        Err(Ok(Error::IdenticalParties))
    );
}

#[test]
fn execute_many_tokens_rejects_contract_recipient_when_only_eoa_is_set() {
    let te = TestEnv::new();
    let (token_a, funder) = te.make_token(MINT);
    let env = te.env().clone();
    let client = BatchPayoutContractClient::new(&env, &env.register(BatchPayoutContract, ()));
    let other_contract = env.register(BatchPayoutContract, ());

    let payments = vec![
        &env,
        MultiTokenPayment {
            token: token_a.address.clone(),
            to: other_contract,
            amount: 100,
        },
    ];
    assert_eq!(
        client.try_execute_many_tokens(&funder, &payments, &true),
        Err(Ok(Error::BatchRecipientIsContract))
    );
}

#[test]
fn execute_many_tokens_uses_all_or_nothing_semantics() {
    // If a later transfer fails (e.g. insufficient balance), no earlier
    // transfer survives.
    let te = TestEnv::new();
    let (token_a, funder) = te.make_token(MINT);
    // token_b has no balance for funder
    let (token_b, _) = te.make_token(0);
    let env = te.env().clone();
    let client = BatchPayoutContractClient::new(&env, &env.register(BatchPayoutContract, ()));
    let token_a_client = TokenClient::new(&env, &token_a.address);

    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let payments = vec![
        &env,
        MultiTokenPayment {
            token: token_a.address.clone(),
            to: a.clone(),
            amount: 100,
        },
        MultiTokenPayment {
            token: token_b.address.clone(),
            to: b.clone(),
            amount: 1, // funder has 0 of token_b → will fail
        },
    ];

    assert!(client
        .try_execute_many_tokens(&funder, &payments, &false)
        .is_err());
    // The first transfer was reverted along with the failed second one.
    assert_eq!(token_a_client.balance(&a), 0, "partial payout survived");
    assert_eq!(token_a_client.balance(&funder), MINT);
}
