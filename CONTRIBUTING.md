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
make check       # fmt-check, lint, test, build, docs and deployment checks
```

`make check` mirrors the required CI jobs. Run it before opening a PR.

### Error code documentation

The error reference in `docs/ERRORS.md` is generated from the variants and
Rust documentation in `contracts/common/src/errors.rs`. After changing the
shared error enum or its descriptions, run:

```bash
make docs-errors        # regenerate docs/ERRORS.md
make check-error-docs  # diff committed docs against generated output
```

Commit the regenerated reference together with any error changes. CI checks
that the generated output matches the committed file.
`make ci` is an alias for `make check` and is what CI itself executes.

The toolchain is pinned in `rust-toolchain.toml`; rustup will honour it
automatically.

### Pre-commit hooks (recommended)

A `.pre-commit-config.yaml` ships at the repo root. Install the hooks once
so every commit is checked with `cargo fmt`, `cargo clippy` and the
standard hygiene checks (trailing whitespace, newline-at-EOF, line endings,
merge markers, large files):

```bash
# One-time install of the pre-commit framework itself.
pip install pre-commit        # or: brew install pre-commit

# Register the Git hooks in this repo.
pre-commit install

# One-off: run all hooks against every tracked file.
pre-commit run --all-files
```

On a warm cache the hook suite finishes in well under 30 seconds. A
failing fmt hook prints the diff and refuses the commit — fix with
`cargo fmt --all` and commit again. A failing clippy hook exits on any
new warning; fix the warning rather than suppress it.

## What a complete PR looks like

- [ ] Tests added, including an authorization test for any new privileged entry
      point and edge cases for any new arithmetic
- [ ] `make check` green locally
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

**No `unsafe` code.** Every crate root, including the test and build-script
roots, carries `#![forbid(unsafe_code)]`, so an `unsafe` block anywhere in the
workspace fails compilation. `sororail-common` also sets
`#![warn(missing_docs)]`, and `make lint` runs clippy with `-D warnings`, so
any undocumented public item there fails CI. Document new public items in
`common` when you add them.

## Entry Point Anatomy

New contract entry points should follow the same order every time so review can focus on behavior instead of rediscovering authorization and state assumptions:

1. Load the minimal state needed to decide whether the caller is a member of the operation.
2. Check membership or role eligibility before prompting a signature.
3. Call `require_auth` only for an address that is already allowed to act.
4. Check operation-specific flags such as `cancellable` or `revocable` after the caller has been established as an eligible party.
5. Perform arithmetic through `sororail_common::math` and write state through the common storage helpers so TTL handling stays consistent.
6. Return typed errors for rejected state transitions instead of panicking.

This keeps stream, vesting, and escrow entry points aligned: membership first, authentication second, state-specific business rules third, and mutation last.

## Choosing an authorization pattern

Reading the `contract.rs` files side by side, you will see two calling
conventions for what is conceptually the same "is this the right party?"
check. Both are deliberate. Which one an entry point uses depends on a single
question: **how many parties may call it?**

**Exactly one fixed party → no `caller` argument; call `require_auth` on the
stored field.**

```rust
pub fn cancel(env: Env) -> Result<(), Error> {
    let mut stream = storage::load(&env)?;
    // ...state checks...
    stream.sender.require_auth();
```

There is nothing to choose between, so a `caller` argument would only be a
second copy of an address the contract already knows, and one that has to be
checked against it. A wrong caller fails at authorization. `Unauthorized` is
never returned, which is why it is missing from the error tables in `stream`,
`vesting` and `batch_payout`.

Used by `stream` (`withdraw`, `cancel`, `top_up`, `extend`), `vesting`
(`claim`, `revoke`), `recurring::charge`, `escrow` (`init`, `fund`, `cancel`,
`resolve`) and `batch_payout` (`execute`, `execute_equal`).

**More than one permitted party → take `caller: Address`, check membership
with a `sororail_common::auth` guard, which then calls `require_auth`.**

```rust
pub fn release(env: Env, caller: Address) -> Result<(), Error> {
    let mut escrow = storage::load(&env)?;
    // ...state checks...
    auth::require_auth_either_opt(&caller, &escrow.depositor, &escrow.arbiter)?;
```

With several eligible signers the contract cannot know which one is acting, so
the caller has to say, and it needs to be recorded for the event
(`released_by`, `cancelled_by`, `raised_by`). The guard rejects an outsider
with `Unauthorized` _before_ any signature is requested, which is the
membership-before-auth rule above.

Used by `escrow` (`release`, `refund`, `dispute`) and `recurring::cancel`.

Use `require_auth_either` for two fixed parties, or `require_auth_either_opt`
for a fixed party and an optional second party. Production code standardizes
on these guards; the unused `allow_list` and `require_auth_one_of` helpers
have been removed (#28).

Rules of thumb:

- Do not add a `caller` argument to a single-party entry point "for
  consistency". It widens the ABI, adds a check that can only ever compare an
  address with itself, and makes `Unauthorized` reachable for no benefit.
- When a single-party entry point gains a second permitted party, switch it to
  the `caller` pattern, use the matching `auth::require_auth_*` guard, and add
  `Unauthorized` to that contract's error table in its `errors.rs`. This is an
  ABI change: call it out as breaking.
- Never call `caller.require_auth()` on an unchecked `caller`. That proves the
  caller signed, not that it is allowed to act.
- Either way, add an authorization test proving a wrong party is rejected.

## Conventions

- **Commits:** [Conventional Commits](https://www.conventionalcommits.org),
  enforced in CI.
- **Branching:** trunk-based. Short-lived branches, PRs into `main`, squash
  merge, linear history.
- **CI must be green to merge.** No exceptions for maintainers. Branch protection
  requires the `all-checks` summary job to pass rather than listing every
  individual job. This summary job depends on all other CI jobs and fails if any
  dependency fails or is cancelled.
- **Formatting:** `cargo fmt`. Lints: `cargo clippy`, warnings denied.

### Dependency hygiene checks

Two required CI jobs guard the dependency graph:

- `unused-deps` runs `cargo machete`, a fast textual scan for dependencies no
  source file references. Because it does not compile anything, code generated
  by a macro or a build script can look unused. If you hit one of those, confirm
  the crate really is used, then add its name to
  `[workspace.metadata.cargo-machete].ignored` in the workspace `Cargo.toml`
  with a comment explaining why. A genuinely unused dependency must be
  removed, not ignored.
- `cargo-deny` runs `cargo deny check licenses bans` against `deny.toml`,
  enforcing the SPDX allow-list and reporting duplicate versions and
  non-crates.io sources. Vulnerability advisories are owned by the separate
  `cargo audit` job — never add an advisory to `deny.toml` just to turn a red
  job green. Run `cargo deny check` locally for the full policy, which also
  rejects yanked crate versions.

## Fuzzing

`contracts/common/fuzz` holds differential fuzz targets for
`sororail_common::math`, run with [`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz)
on nightly (it is intentionally outside the main workspace — see that
crate's `Cargo.toml`):

```bash
cargo install cargo-fuzz
cd contracts/common/fuzz
cargo +nightly fuzz run fuzz_mul_div
cargo +nightly fuzz run fuzz_split_bps -- -max_total_time=30
```

| Target | Property |
| --- | --- |
| `fuzz_mul_div` | `mul_div` matches a `BigInt` reference and only returns its documented errors |
| `fuzz_split_bps` | `split_bps` halves always sum to `amount`, the first half is exactly `amount * bps / 10000` truncated, both halves are non-negative for a non-negative `amount`, and out-of-range `bps` or an overflowing product is reported as an error |

Fuzzing is open-ended, so it never gates a pull request. Instead the
[`Fuzz smoke`](.github/workflows/fuzz.yml) workflow runs every target that
`cargo fuzz list` reports for 60 seconds each, weekly and on manual dispatch
(Actions → Fuzz smoke → Run workflow, where the duration can be changed). If a
target finds a crash the run fails and uploads the crashing input as the
`fuzz-artifacts` artifact; reproduce it with
`cargo +nightly fuzz run <target> <crash-file>`. Still run the targets locally
after touching `math.rs`.

## Code of Conduct

All participation in this project is governed by our
[Code of Conduct](CODE_OF_CONDUCT.md). By contributing, you agree to abide by
its terms. Reports of violations go to rubelu2017@gmail.com (see
[SECURITY.md](SECURITY.md) for the same contact and reporting context).

## Security

Do not report vulnerabilities through public issues. See [SECURITY.md](SECURITY.md).
