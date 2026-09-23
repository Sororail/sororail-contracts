# Contributing

Thanks for considering a contribution. This repository holds contracts that are
intended to move money, so the bar for merging is deliberately high — but the
path in is meant to be clear.

## Before you start

Issues labelled `good-first-issue` are scoped so that someone new to Soroban can
complete them: each states the problem, the affected file paths, the expected
behavior, acceptance criteria, and how to test the change locally. Comment to
claim one, then open a **draft PR early**. An in-progress PR with questions is
more useful than a perfect one that arrives three weeks late.

If you are unsure whether something is in scope, open an issue before writing
code. Scope discipline is the point of this project — see the non-goals in
[SPEC.md](SPEC.md).

## Setup

```bash
rustup target add wasm32v1-none
cargo install cargo-nextest
make ci          # fmt-check, lint, test, build
```

The toolchain is pinned in `rust-toolchain.toml`; rustup will honour it
automatically.

## What a complete PR looks like

- [ ] Tests added, including an authorization test for any new privileged entry
      point and edge cases for any new arithmetic
- [ ] `make ci` green locally
- [ ] Docs updated — a PR that changes public behavior without touching docs is
      incomplete
- [ ] No unrelated changes
- [ ] Breaking changes called out explicitly in the description

## Rules that will fail review

**Never renumber or remove a released `Error` variant.** The integers are public
ABI; clients decode failures by number. Append inside the contract's documented
range in `contracts/common/src/errors.rs` and leave retired numbers burned.

**No raw arithmetic on balances.** Use `sororail_common::math`. `a + b` on an
`i128` balance will be asked to change even where it happens to be safe, because
the invariant is easier to hold when it has no exceptions.

**Check membership before `require_auth`.** Use the guards in
`sororail_common::auth`. Calling `require_auth` first prompts an address that
was never permitted to act.

**Extend the TTL of anything you touch.** Soroban archives state that is not
extended. Use `sororail_common::storage`.

**Illegal state transitions must error, not panic.** A panic gives the caller a
useless failure; a typed error tells them what happened.

## Entry Point Anatomy

New contract entry points should follow the same order every time so review can focus on behavior instead of rediscovering authorization and state assumptions:

1. Load the minimal state needed to decide whether the caller is a member of the operation.
2. Check membership or role eligibility before prompting a signature.
3. Call `require_auth` only for an address that is already allowed to act.
4. Check operation-specific flags such as `cancellable` or `revocable` after the caller has been established as an eligible party.
5. Perform arithmetic through `sororail_common::math` and write state through the common storage helpers so TTL handling stays consistent.
6. Return typed errors for rejected state transitions instead of panicking.

This keeps stream, vesting, and escrow entry points aligned: membership first, authentication second, state-specific business rules third, and mutation last.
## Conventions

- **Commits:** [Conventional Commits](https://www.conventionalcommits.org),
  enforced in CI.
- **Branching:** trunk-based. Short-lived branches, PRs into `main`, squash
  merge, linear history.
- **CI must be green to merge.** No exceptions for maintainers.
- **Formatting:** `cargo fmt`. Lints: `cargo clippy`, warnings denied.

## Security

Do not report vulnerabilities through public issues. See [SECURITY.md](SECURITY.md).