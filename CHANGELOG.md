# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `recurring`: `total_charged()` view backed by a cumulative `total_charged`
  field on `Authorization` (accumulated with `math::add`); `charges_made()` and
  `charges_remaining()` views that satisfy `charges_made + remaining ==
  max_periods` for capped subscriptions; payer-only `pause()` / `resume()` with
  `Paused` and `Resumed` events (paused time shifts the schedule, `charge`
  fails with `RecurringPaused`); and an allowance check on `authorize` that
  returns `RecurringInsufficientAllowance`. The payer must now `approve` the
  contract before `authorize` (#416, #417, #418, #419).
- Per-contract crate READMEs rendered through rustdoc, a per-contract threat
  model, `source_repo` contract metadata, and a regression test proving that a
  deauthorized recipient cannot corrupt a stream on a flagged Stellar Asset
  Contract (#439, #442, #450, #451).
- CLI invocation examples, TTL operator guidance, and a CI check requiring an
  Unreleased changelog entry for contract-affecting pull requests (#469, #470,
  #471).
- Storage-footprint regression check: stateful contracts retain exactly one
  instance-storage position and batch payout remains stateless (#468).
- `stream::top_up_from(funder, amount)` lets a third party fund an existing
  stream. The funder authorizes the transfer, the funds count toward
  `deposited`, and cancellation still refunds the sender. It emits
  `ToppedUpFrom` (#397).
- `stream::has_ended()` view, true once the ledger time reaches `stop`, with
  boundary tests at `stop - 1`, `stop` and `stop + 1` (#396).
- Property tests for `Stream::accrued_at`: accrual never decreases over time,
  never goes negative or exceeds `deposited`, and is zero at `start` (#399).
- Budget test pinning the CPU and memory cost of `withdraw` and `cancel`
  within 10% between a one second and a ten year stream, so a loop over time
  fails CI (#398).

- Workspace test `tests/tests/event_topics.rs` asserting every `#[contractevent]` declares exactly `[<crate name>, <snake_case action>]` topics (#64).
- SPEC.md "Cross-contract design conventions": when an entry point takes a `caller` argument vs. calling `require_auth` on a fixed party (#63), and when a lifecycle is a `State` enum vs. an optional timestamp (#66).
- CONTRIBUTING.md "Choosing an authorization pattern" (#63).
- `vesting::schedule()` view returning the schedule as a list of `Checkpoint { at, vested }` corners (start, cliff, end), so clients can render it without recomputing (#315).
- `vesting::transfer_beneficiary(new_beneficiary)` for key rotation. Only the current beneficiary may call it. The schedule and `claimed` are unchanged, later claims pay the new address, and it emits `BeneficiaryTransferred`. Rejected on a revoked grant (#403).
- `docs/stream-cancel.md` with three worked cancel examples. Each example has a test that asserts its numbers (#400).
- Tests pinning `withdraw` at zero accrual: `None` fails with `InvalidAmount` and an explicit amount with `StreamInsufficientAccrued`, both when `start` equals now and when `start` is in the future (#402).
- The `stream::Created` payload layout (including `rate_per_second` and `deposited`) is pinned by `tests/tests/event_topics.rs` and documented in `docs/events.md` (#401).

### Changed

- **Breaking (Rust API):** removed the unused `sororail_common::auth::allow_list`
  and `require_auth_one_of` helpers (#28). Use `require_auth_either` or
  `require_auth_either_opt` for the supported two-party authorization patterns.
  Contract entry points, authorization behavior and error codes are unchanged.
- `recurring::authorize` rejects `payer == payee` with `IdenticalParties` (#314). Previously a self-subscription was accepted; it moved no value but is now refused up front.
- Regression tests confirm `vesting::create` and `stream::create` refuse identical parties before any funds move and leave no record behind (#312, #313). The checks themselves predate this change.

- Every contract's `errors.rs` now documents which shared variants it never returns, and why (#67). The `recurring` note wrongly claimed `Unauthorized` never appears and has been corrected. `escrow` now lists `DeadlinePassed`, `Overflow` and `EscrowNotCancellable`, `recurring` lists `InvalidTimeRange` and `Overflow`, and `batch_payout` lists `IdenticalParties`, all of which were raised but undocumented.

## [0.1.0] - 2026-09-07

### Fixed

- **Batch payout contract spec bug**: The `batch_payout` contract's `execute` and `preview` functions took a `Payments` type alias for `Vec<Payment>`. The Soroban contract code generator (`#[contractimpl]`) reads argument types syntactically and cannot resolve type aliases, causing the emitted contract spec to reference a non-existent user-defined type named `Payments`. This caused contract spec loading to fail entirely — `stellar contract invoke` would fail with `Missing Entry Payments` on any function call, including argument-less ones. The fix was to use the concrete `Vec<Payment>` type in the function signatures rather than the alias. The type alias has been removed, and the reasoning is documented in `contracts/batch_payout/src/types.rs` to prevent reintroduction.
  - **Impact**: This was discovered only after mainnet deployment when a real client attempted to parse the contract spec. Local Rust tests were unaffected because they call the generated client directly and never parse the spec.
  - **Deployment**: Fixed in commit `0e558578f188c139e007a80914ec4707e265840f` on 2026-09-07.
