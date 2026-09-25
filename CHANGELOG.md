# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-09-07

### Fixed

- **Batch payout contract spec bug**: The `batch_payout` contract's `execute` and `preview` functions took a `Payments` type alias for `Vec<Payment>`. The Soroban contract code generator (`#[contractimpl]`) reads argument types syntactically and cannot resolve type aliases, causing the emitted contract spec to reference a non-existent user-defined type named `Payments`. This caused contract spec loading to fail entirely — `stellar contract invoke` would fail with `Missing Entry Payments` on any function call, including argument-less ones. The fix was to use the concrete `Vec<Payment>` type in the function signatures rather than the alias. The type alias has been removed, and the reasoning is documented in `contracts/batch_payout/src/types.rs` to prevent reintroduction.
  - **Impact**: This was discovered only after mainnet deployment when a real client attempted to parse the contract spec. Local Rust tests were unaffected because they call the generated client directly and never parse the spec.
  - **Deployment**: Fixed in commit `0e558578f188c139e007a80914ec4707e265840f` on 2026-09-07.
