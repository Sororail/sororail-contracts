WASM_TARGET := wasm32v1-none
WASM_DIR    := target/$(WASM_TARGET)/release
CONTRACTS   := escrow stream vesting recurring batch_payout

.PHONY: all build test test-all test-contracts fmt fmt-check lint audit optimize wasm-size specs clean ci coverage docs-errors check-error-docs check-event-docs verify-deployments check-deployments check-wasm-sizes check-storage-footprint changelog check-changelog update-spec-snapshots check-spec-snapshots

all: build test

# The integration-test crate is host-only (it needs std), so it is excluded
# from the wasm build rather than failing it.
build:
	cargo build --workspace --exclude sororail-integration-tests \
		--target $(WASM_TARGET) --release
	cargo build -p sororail-common --target $(WASM_TARGET) --release

# Tests run on the host, not on wasm -- soroban_sdk::testutils needs std.
test:
	$(MAKE) test-all

test-all:
	cargo nextest run --workspace

# Run every contract and shared-library test, excluding only the host-only
# integration crate which has its own deployment-dependent test environment.
test-contracts:
	cargo test --workspace --exclude sororail-integration-tests

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

wasm-size: build
	./scripts/check_wasm_sizes.sh $(WASM_DIR)

verify-deployments:
	./scripts/check_deployments.sh

audit:
	cargo audit

# Strips and shrinks each contract wasm. Requires the `stellar` CLI.
optimize: build
	@for c in $(CONTRACTS); do \
		echo "optimizing $$c"; \
		stellar contract optimize --wasm $(WASM_DIR)/sororail_$$c.wasm || exit 1; \
	done

# Contract specs as JSON, attached to the GitHub Release. Requires `stellar`
# and `jq`.
#
# Emitting a spec is not enough: a spec can reference a user-defined type it
# never defines (the `Payments` alias in DEPLOYMENTS.md), and `stellar
# contract info` prints it without complaint while every client that loads it
# fails with `Missing Entry`. So each spec must parse as JSON and every `udt`
# it references must be defined in it.
SPEC_CHECK := ([.[] | to_entries[] | select(.key | startswith("udt_")) | .value.name]) as $$defined \
	| ([.. | objects | select(has("udt")) | .udt.name] | unique) - $$defined \
	| if length > 0 then error("references undefined types: \(join(", "))") else true end

specs: build
	@mkdir -p dist/specs
	@for c in $(CONTRACTS); do \
		stellar contract info interface --output json \
			--wasm $(WASM_DIR)/sororail_$$c.wasm > dist/specs/$$c.json || exit 1; \
		jq -e '$(SPEC_CHECK)' dist/specs/$$c.json > /dev/null \
			|| { echo "invalid spec: $$c"; exit 1; }; \
	done
	@echo "specs written to dist/specs/"

docs-errors:
	bash scripts/gen_error_docs.sh

check-error-docs:
	bash scripts/check_error_docs.sh

check-event-docs:
	./scripts/check_event_docs.sh

ci: fmt-check lint test build check-error-docs check-event-docs check-deployments check-wasm-sizes check-storage-footprint

check-deployments:
	./scripts/check_deployments.sh

check-wasm-sizes:
	./scripts/check_wasm_sizes.sh $(WASM_DIR)

check-storage-footprint:
	./scripts/check_storage_footprint.sh

# Matches .github/workflows/ci.yml coverage job (lines / regions / functions).
coverage:
	cargo llvm-cov nextest --workspace \
		--fail-under-lines 90 \
		--fail-under-regions 75 \
		--fail-under-functions 80
	bash scripts/audit-error-variant-coverage.sh

# Regenerate CHANGELOG.md from the git log via git-cliff (#444). Commits
# already follow Conventional Commits; the mapping to Keep-a-Changelog
# sections lives in cliff.toml. `check-changelog` is the read-only form
# used in CI: it prints the generated file to stdout and fails if the
# committed file is stale.
changelog:
	@command -v git-cliff >/dev/null || { \
		echo "git-cliff is required. Install with 'cargo install git-cliff --locked' or 'brew install git-cliff'." >&2; \
		exit 1; \
	}
	git-cliff --config cliff.toml --output CHANGELOG.md
	@echo "CHANGELOG.md regenerated."

check-changelog:
	@command -v git-cliff >/dev/null || { \
		echo "git-cliff is required. Install with 'cargo install git-cliff --locked' or 'brew install git-cliff'." >&2; \
		exit 1; \
	}
	@tmp="$$(mktemp)"; \
	git-cliff --config cliff.toml --output "$$tmp" >/dev/null; \
	if ! diff -u CHANGELOG.md "$$tmp"; then \
		echo "CHANGELOG.md is out of date — run 'make changelog' and commit the result." >&2; \
		rm -f "$$tmp"; \
		exit 1; \
	fi; \
	rm -f "$$tmp"; \
	echo "CHANGELOG.md is up to date."

# Generate spec snapshot JSON files for every contract and commit the results.
# Requires: stellar CLI  (cargo install --locked stellar-cli --features opt)
# Run this whenever the ABI of any contract changes, then commit the updated
# spec_snapshot.json files in contracts/<name>/.
update-spec-snapshots: build
	@command -v stellar >/dev/null || { \
		echo "stellar CLI required: cargo install --locked stellar-cli --features opt" >&2; \
		exit 1; \
	}
	@for c in $(CONTRACTS); do \
		echo "updating spec snapshot for $$c"; \
		stellar contract info interface --output json \
			--wasm $(WASM_DIR)/sororail_$$c.wasm \
			> contracts/$$c/spec_snapshot.json || exit 1; \
	done
	@echo "snapshots updated — commit the spec_snapshot.json files in contracts/"

# Diff each contract's current spec against its committed snapshot and fail on
# any drift. Silently skips if stellar CLI is not installed.
check-spec-snapshots:
	@bash scripts/check_spec_snapshots.sh

clean:
	cargo clean
	rm -rf dist
