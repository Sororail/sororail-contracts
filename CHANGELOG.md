# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Shared checked math helpers: `mul_div_ceil` for non-negative round-up arithmetic, `Rounding::{Down, Up, Nearest}` with `mul_div_with`, and `max`/`clamp` for bounds validation (#363, #364, #365).
- Shared `no_std` time helpers `end_of` (checked `u64` addition) and `elapsed` (zero for reversed timestamps), without migrating existing contract callers (#367).

- `Fuzz smoke` workflow (`.github/workflows/fuzz.yml`) runs every cargo-fuzz target for 60 seconds weekly and on manual dispatch, without gating pull requests, and uploads crash inputs as an artifact (#371). Its first run caught the `fuzz_mul_div` oracle expecting `DivisionByZero` for `d == 0` even when `a * b` overflows first; the oracle now follows `mul_div`'s documented multiply-then-divide order.

- `stream::withdraw_to(destination, amount)` withdraws accrued funds like `withdraw` but pays them to `destination` in the same transaction. It requires the recipient's authorization, shares validation, accrual and accounting with `withdraw` through one private helper, and rejects the stream contract itself as a destination with `IdenticalParties` (#391).

- `escrow::extend_deadline(new_deadline)` moves the deadline of a `Funded` escrow later. It requires authorization from both the depositor and the beneficiary, fails once the deadline has been reached, and emits `DeadlineExtended` with the old and new values (#377).
- `escrow::time_to_deadline()` is a read-only view returning the seconds left until the deadline on the ledger clock, `0` once it has been reached (#378).
- `escrow::resolve_timeout(caller)` refunds the depositor once the optional `dispute_timeout` has elapsed since `dispute`. Either party may call it. The arbiter's `resolve` remains available until then, so the first transaction to land decides the outcome (#379). The trade-off is documented in SECURITY.md.
- `sororail_common::storage` single-position macro gains `try_load(env) -> Option<T>`,
  a read-only single storage read that returns `None` when the value is absent.
  It does not extend the TTL, like `load_view` (#374).
- `sororail_common::testutils`: `TestEnv::advance_by(seconds)` (panics on `u64`
  overflow), `TestEnv::advance_to_end_of(ts)` (panics if `ts` is in the past), and
  `TestEnv::make_token_with_admin(supply)`, which returns the token client, the
  admin client and the funder. The token is revocable so tests can deauthorize a
  holder. `make_token` is unchanged (#372, #373).
- CI and `make build` now build `sororail-common` alone for `wasm32v1-none`, so
  a std-only dependency in `common` fails the build (#376).
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

- **Breaking (events):** `stream::Withdrawn` gains a `destination` data field: the recipient for `withdraw`, the chosen address for `withdraw_to`. Topics are unchanged (`["stream", "withdrawn", recipient]`), so filters keep working; decoders of the data map must add the field (#391).
- **Breaking (ABI):** the `escrow::Escrow` record returned by `get` gains `created_at: u64` (ledger timestamp set by `init`) and `funded_at: Option<u64>` (set by `fund`; `None` before funding and for an escrow cancelled unfunded). Neither changes after it is set. Clients decoding `Escrow` must add both fields; `init` and `fund` arguments are unchanged (#380).
- **Breaking (ABI):** `escrow::init` takes a new trailing `dispute_timeout: Option<u64>` argument, and the `Escrow` record gains `dispute_timeout` and `disputed_at` fields. Pass `None` to keep the previous behavior. `Some(0)` is rejected with `InvalidDuration`, and `Some(_)` without an arbiter with `EscrowNoArbiter`.
- `sororail_common::Error` gains `EscrowNoDisputeTimeout` (28) and `EscrowDisputeTimeoutNotReached` (29). Existing codes are unchanged.
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
