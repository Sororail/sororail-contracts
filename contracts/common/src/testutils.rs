//! Shared test-environment helpers for all SoroRail contract test suites.
//!
//! Gated behind the `testutils` feature so it is never compiled into
//! production or WASM builds. Each contract's `[dev-dependencies]` section
//! must include:
//!
//! ```toml
//! sororail-common = { workspace = true, features = ["testutils"] }
//! ```
//!
//! # What is here
//!
//! [`TestEnv`] wraps an [`Env`] that has `mock_all_auths` enabled, with
//! convenience methods for the three things every fixture repeats:
//!
//! * creating a Stellar Asset Contract token (SAC) and minting an initial
//!   supply to a named funder,
//! * generating fresh [`Address`] values without repeating the
//!   `Address::generate(&env)` call everywhere, and
//! * advancing ledger time to a known timestamp.
//!
//! Per-contract `Fixture` structs register their own contracts and drive the
//! protocol — those stay in each test file because they are contract-specific.
//! Only the plumbing that is literally identical across all of them lives here.
//!
//! [`assert_conserved`] is the workspace-wide token supply check: pass every
//! address that may hold the token and the supply minted at the start, and it
//! asserts no funds were created or destroyed.

// Test fixtures do plain arithmetic on known-small constants; the checked-math
// rule is for contract code.
#![allow(clippy::arithmetic_side_effects)]

use soroban_sdk::{
    testutils::{Address as _, IssuerFlags, Ledger as _},
    token::{StellarAssetClient, TokenClient},
    Address, Env,
};

use crate::Error;

/// A pre-configured test environment with `mock_all_auths` already enabled.
///
/// Create one per test (or per `Fixture::new` call), then use the helper
/// methods rather than repeating the same boilerplate in each contract's test
/// file.
///
/// ```rust,ignore
/// let te = TestEnv::at(START);
/// let (token, funder) = te.make_token(MINT);
/// let alice = te.make_address();
/// te.advance_to(START + 1_000);
/// ```
pub struct TestEnv {
    env: Env,
}

impl TestEnv {
    /// Creates a default [`Env`] with `mock_all_auths` enabled.
    ///
    /// The ledger timestamp is **not** set here — call [`TestEnv::at`] or
    /// [`TestEnv::advance_to`] immediately if the test needs a specific time.
    pub fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        Self { env }
    }

    /// Creates a default [`Env`] with `mock_all_auths` enabled and the ledger
    /// timestamp initialised to `timestamp`.
    pub fn at(timestamp: u64) -> Self {
        let te = Self::new();
        te.env.ledger().with_mut(|l| l.timestamp = timestamp);
        te
    }

    /// Generates a fresh, random [`Address`] in this environment.
    pub fn make_address(&self) -> Address {
        Address::generate(&self.env)
    }

    /// Borrows the wrapped Soroban environment for contract setup and calls.
    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Registers a Stellar Asset Contract, mints `initial_supply` tokens to a
    /// freshly generated funder address, and returns `(TokenClient, funder)`.
    ///
    /// The returned [`TokenClient`] borrows from `self`, so it lives as long as
    /// the `TestEnv` does.
    pub fn make_token(&self, initial_supply: i128) -> (TokenClient<'_>, Address) {
        let token_address = self.register_token();
        let funder = Address::generate(&self.env);
        StellarAssetClient::new(&self.env, &token_address).mint(&funder, &initial_supply);
        let client = TokenClient::new(&self.env, &token_address);
        (client, funder)
    }

    /// Like [`Self::make_token`], but also returns the token's admin client.
    ///
    /// Returns `(TokenClient, StellarAssetClient, funder)`. The admin client can
    /// mint, set authorization and claw back, none of which [`TokenClient`]
    /// exposes. The token is registered as revocable, so a test can deauthorize
    /// a holder with `admin.set_authorized(&holder, &false)`.
    ///
    /// [`Self::make_token`] is unchanged and still registers a non-revocable
    /// token. Use this variant only when the test needs the admin.
    pub fn make_token_with_admin(
        &self,
        initial_supply: i128,
    ) -> (TokenClient<'_>, StellarAssetClient<'_>, Address) {
        let issuer = Address::generate(&self.env);
        let sac = self.env.register_stellar_asset_contract_v2(issuer);
        sac.issuer().set_flag(IssuerFlags::RevocableFlag);
        let token_address = sac.address();
        let admin = StellarAssetClient::new(&self.env, &token_address);
        let funder = Address::generate(&self.env);
        admin.mint(&funder, &initial_supply);
        let client = TokenClient::new(&self.env, &token_address);
        (client, admin, funder)
    }

    /// Mints `amount` tokens of `token` to `recipient`.
    ///
    /// Useful when a test needs a single token but multiple funded accounts.
    pub fn mint(&self, token: &soroban_sdk::Address, recipient: &Address, amount: i128) {
        StellarAssetClient::new(&self.env, token).mint(recipient, &amount);
    }

    /// Sets the ledger timestamp to `ts`.
    pub fn advance_to(&self, ts: u64) {
        self.env.ledger().with_mut(|l| l.timestamp = ts);
    }

    /// Advances the ledger timestamp by `delta` seconds.
    ///
    /// Prefer this over `advance_to(start + N)`: the test states the step, not
    /// the absolute time, so it survives a change to the start time.
    ///
    /// # Panics
    ///
    /// Panics if the new timestamp would overflow `u64`.
    pub fn advance_by(&self, delta: u64) {
        let ts = self
            .timestamp()
            .checked_add(delta)
            .expect("advance_by: ledger timestamp would overflow u64");
        self.advance_to(ts);
    }

    /// Advances the ledger to `ts`, the boundary at which a window ends.
    ///
    /// Use this where a test means "run the clock to the end of a schedule"
    /// (for example a stream's `stop`), so the boundary is named rather than
    /// hand-computed. The ledger is set to `ts` itself.
    ///
    /// # Panics
    ///
    /// Panics if `ts` is earlier than the current ledger time. The clock only
    /// moves forward through this helper.
    pub fn advance_to_end_of(&self, ts: u64) {
        let now = self.timestamp();
        assert!(
            ts >= now,
            "advance_to_end_of: {ts} is before the current ledger time {now}"
        );
        self.advance_to(ts);
    }

    /// Returns the environment's ledger time.
    pub fn timestamp(&self) -> u64 {
        self.env.ledger().timestamp()
    }

    /// Registers an unfunded Stellar Asset Contract and returns its address.
    ///
    /// Use [`Self::make_token`] when a named funder should receive an initial
    /// supply as part of setup.
    pub fn register_token(&self) -> Address {
        let issuer = Address::generate(&self.env);
        self.env
            .register_stellar_asset_contract_v2(issuer)
            .address()
    }
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

/// Asserts that the token balances of `participants` sum to `expected_total`.
///
/// This is the whole-supply conservation check: list every address that can
/// hold the token — both parties, any fee sink, and the contract itself when it
/// can retain funds — and pass the supply minted at the start of the test. A
/// flow that creates or destroys tokens will make the sum drift away from
/// `expected_total` and fail here.
///
/// Balances are summed with checked arithmetic, so an over-large fixture cannot
/// wrap the check and hide a leak.
///
/// # Panics
///
/// Panics when the balances do not sum to `expected_total`, naming the
/// participants so a failing test points at the address that holds the wrong
/// amount.
///
/// ```rust,ignore
/// let (token, funder) = te.make_token(MINT);
/// // ... run the flow ...
/// assert_conserved(&token, &[recipient, sender, contract], MINT);
/// ```
pub fn assert_conserved(token: &TokenClient<'_>, participants: &[Address], expected_total: i128) {
    let actual = participants.iter().fold(0i128, |sum, who| {
        sum.checked_add(token.balance(who))
            .expect("conservation sum overflowed i128")
    });
    assert_eq!(
        actual, expected_total,
        "token balances for {participants:?} do not sum to {expected_total}"
    );
}

/// Asserts that a generated client's `try_*` call failed with `expected`.
///
/// Contract clients return `Result<Result<T, _>, Result<Error, _>>` from
/// `try_*` calls. A plain `assert_eq!` on that shape prints only numeric
/// codes; this names both errors via [`Error::as_str`] (#368):
///
/// ```text
/// expected Error::IdenticalParties (15), got Error::InvalidAmount (4)
/// ```
///
/// ```rust,ignore
/// assert_contract_error(client.try_create(&a, &a, ...), Error::IdenticalParties);
/// ```
pub fn assert_contract_error<T: core::fmt::Debug, C: core::fmt::Debug, I: core::fmt::Debug>(
    result: Result<Result<T, C>, Result<Error, I>>,
    expected: Error,
) {
    match result {
        Err(Ok(actual)) if actual == expected => {}
        Err(Ok(actual)) => panic!(
            "expected Error::{} ({}), got Error::{} ({})",
            expected.as_str(),
            expected as u32,
            actual.as_str(),
            actual as u32
        ),
        Err(Err(invoke)) => panic!(
            "expected Error::{} ({}), but the call failed outside the contract error type: {invoke:?}",
            expected.as_str(),
            expected as u32
        ),
        Ok(Ok(value)) => panic!(
            "expected Error::{} ({}), but the call succeeded with {value:?}",
            expected.as_str(),
            expected as u32
        ),
        Ok(Err(conversion)) => panic!(
            "expected Error::{} ({}), but the call succeeded and its result failed to convert: {conversion:?}",
            expected.as_str(),
            expected as u32
        ),
    }
}

// ─── Fee-on-transfer mock token (#438) ──────────────────────────────────────
//
// Several SoroRail tests need a token that does not conserve value on
// transfer — bridged assets with inner-fee hooks, deflationary tokens, LPs
// that charge on swap. Rather than each test re-inventing the shape, this
// module ships a reusable mock behind the `testutils` feature. The contract
// implements the `token::TokenInterface` methods the SoroRail contracts call
// (`balance`, `transfer`, `transfer_from`, `approve`, `allowance`,
// `decimals`, `name`, `symbol`); on every `transfer`/`transfer_from` it
// skims `fee_bps` basis points to a configured sink and credits the net
// amount to the recipient. The caller is debited the gross amount, matching
// how real FOT tokens work from the sender's perspective.
//
// The mock is intentionally minimal: no supply cap, no pause, no admin
// gating. `mint_for_tests` adds balance from thin air; production code must
// never reach it because the whole module is gated by the `testutils`
// feature.

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, String, Symbol};

/// Storage keys used by {@link FeeOnTransferToken}. One instance per
/// deployed contract address.
#[contracttype]
#[derive(Clone)]
pub enum FeeTokenKey {
    /// `fee_bps` (u32), basis points skimmed on every transfer.
    FeeBps,
    /// `fee_sink` (Address), where skimmed fees land.
    FeeSink,
    /// Metadata the Soroban `token::TokenInterface` requires: decimals.
    Decimals,
    /// Token name metadata.
    Name,
    /// Token symbol metadata.
    Symbol,
    /// Per-address balance.
    Balance(Address),
    /// `(from, spender)` allowance pair with its expiration.
    Allowance(Address, Address),
}

/// Allowance a holder has granted a spender on [`FeeOnTransferToken`].
#[contracttype]
#[derive(Clone)]
pub struct FeeTokenAllowance {
    /// Remaining amount the spender may move.
    pub amount: i128,
    /// Ledger sequence number recorded with the allowance.
    pub expiration_ledger: u32,
}

/// Mock token that skims `fee_bps` basis points on every transfer.
///
/// Register it with a `TestEnv`, initialize it with `init`, and use it from
/// any contract that accepts a `token::TokenInterface`. See
/// {@link FeeOnTransferTokenHelper} below for the one-call setup most tests
/// want.
#[contract]
pub struct FeeOnTransferToken;

/// Event topic emitted on every skim, so tests can assert fees were taken.
const TOPIC_FEE_SKIMMED: Symbol = symbol_short!("fee_skim");

#[contractimpl]
impl FeeOnTransferToken {
    /// One-time setup. `fee_bps` is clamped to `[0, 10_000]` by assertion
    /// so a mis-configured test fails loudly rather than silently reducing
    /// every transfer to zero.
    pub fn init(
        env: Env,
        fee_sink: Address,
        fee_bps: u32,
        decimals: u32,
        name: String,
        symbol: String,
    ) {
        assert!(fee_bps <= 10_000, "fee_bps must be <= 10_000 (100%)");
        let storage = env.storage().instance();
        storage.set(&FeeTokenKey::FeeBps, &fee_bps);
        storage.set(&FeeTokenKey::FeeSink, &fee_sink);
        storage.set(&FeeTokenKey::Decimals, &decimals);
        storage.set(&FeeTokenKey::Name, &name);
        storage.set(&FeeTokenKey::Symbol, &symbol);
    }

    /// Test-only: bump the fee without re-initialising. Not part of the
    /// token interface.
    pub fn set_fee_bps(env: Env, fee_bps: u32) {
        assert!(fee_bps <= 10_000);
        env.storage().instance().set(&FeeTokenKey::FeeBps, &fee_bps);
    }

    /// Test-only: credit `amount` to `to` with no counterparty debit. The
    /// real production path is minting via a SAC; this is the test seam.
    pub fn mint_for_tests(env: Env, to: Address, amount: i128) {
        assert!(amount >= 0, "mint_for_tests cannot mint negative");
        let current = read_balance(&env, &to);
        write_balance(
            &env,
            &to,
            current.checked_add(amount).expect("balance overflow"),
        );
    }

    // ── token::TokenInterface ─────────────────────────────────────────────

    /// Returns the balance held by `id`.
    pub fn balance(env: Env, id: Address) -> i128 {
        read_balance(&env, &id)
    }

    /// Moves `amount` from `from` to `to`, skimming the configured fee into the
    /// fee sink. Requires `from`'s authorization.
    #[allow(deprecated)]
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        assert!(amount >= 0, "transfer amount must be non-negative");
        let fee = fee_on(&env, amount);
        let net = amount.checked_sub(fee).expect("fee > amount");

        let from_balance = read_balance(&env, &from);
        assert!(from_balance >= amount, "insufficient balance");
        write_balance(&env, &from, from_balance - amount);

        let to_balance = read_balance(&env, &to);
        write_balance(
            &env,
            &to,
            to_balance.checked_add(net).expect("balance overflow"),
        );

        if fee > 0 {
            let sink: Address = env.storage().instance().get(&FeeTokenKey::FeeSink).unwrap();
            let sink_balance = read_balance(&env, &sink);
            write_balance(
                &env,
                &sink,
                sink_balance.checked_add(fee).expect("sink overflow"),
            );
            env.events()
                .publish((TOPIC_FEE_SKIMMED, from.clone(), to.clone()), (amount, fee));
        }
    }

    /// Moves `amount` from `from` to `to` on behalf of `spender`, spending the
    /// allowance and skimming the configured fee. Requires `spender`'s
    /// authorization.
    #[allow(deprecated)]
    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        assert!(amount >= 0, "transfer amount must be non-negative");
        spend_allowance(&env, &from, &spender, amount);

        let fee = fee_on(&env, amount);
        let net = amount.checked_sub(fee).expect("fee > amount");

        let from_balance = read_balance(&env, &from);
        assert!(from_balance >= amount, "insufficient balance");
        write_balance(&env, &from, from_balance - amount);

        let to_balance = read_balance(&env, &to);
        write_balance(
            &env,
            &to,
            to_balance.checked_add(net).expect("balance overflow"),
        );

        if fee > 0 {
            let sink: Address = env.storage().instance().get(&FeeTokenKey::FeeSink).unwrap();
            let sink_balance = read_balance(&env, &sink);
            write_balance(
                &env,
                &sink,
                sink_balance.checked_add(fee).expect("sink overflow"),
            );
            env.events()
                .publish((TOPIC_FEE_SKIMMED, from.clone(), to.clone()), (amount, fee));
        }
    }

    /// Sets `spender`'s allowance over `from`'s balance to `amount`, recorded
    /// with `expiration_ledger`. Requires `from`'s authorization.
    pub fn approve(
        env: Env,
        from: Address,
        spender: Address,
        amount: i128,
        expiration_ledger: u32,
    ) {
        from.require_auth();
        assert!(amount >= 0, "allowance cannot be negative");
        env.storage().temporary().set(
            &FeeTokenKey::Allowance(from, spender),
            &FeeTokenAllowance {
                amount,
                expiration_ledger,
            },
        );
    }

    /// Returns the remaining allowance `spender` holds over `from`'s balance.
    pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        read_allowance(&env, &from, &spender).amount
    }

    /// Returns the configured decimals, defaulting to `7`.
    pub fn decimals(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&FeeTokenKey::Decimals)
            .unwrap_or(7)
    }

    /// Returns the configured token name, defaulting to `FeeMock`.
    pub fn name(env: Env) -> String {
        env.storage()
            .instance()
            .get(&FeeTokenKey::Name)
            .unwrap_or_else(|| String::from_str(&env, "FeeMock"))
    }

    /// Returns the configured token symbol, defaulting to `FEE`.
    pub fn symbol(env: Env) -> String {
        env.storage()
            .instance()
            .get(&FeeTokenKey::Symbol)
            .unwrap_or_else(|| String::from_str(&env, "FEE"))
    }

    /// Destroys `amount` of `from`'s balance. Requires `from`'s authorization.
    pub fn burn(env: Env, from: Address, amount: i128) {
        from.require_auth();
        assert!(amount >= 0);
        let b = read_balance(&env, &from);
        assert!(b >= amount, "insufficient balance");
        write_balance(&env, &from, b - amount);
    }

    /// Destroys `amount` of `from`'s balance on behalf of `spender`, spending
    /// the allowance. Requires `spender`'s authorization.
    pub fn burn_from(env: Env, spender: Address, from: Address, amount: i128) {
        spender.require_auth();
        assert!(amount >= 0);
        spend_allowance(&env, &from, &spender, amount);
        let b = read_balance(&env, &from);
        assert!(b >= amount, "insufficient balance");
        write_balance(&env, &from, b - amount);
    }
}

fn read_balance(env: &Env, id: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&FeeTokenKey::Balance(id.clone()))
        .unwrap_or(0)
}

fn write_balance(env: &Env, id: &Address, amount: i128) {
    env.storage()
        .persistent()
        .set(&FeeTokenKey::Balance(id.clone()), &amount);
}

fn read_allowance(env: &Env, from: &Address, spender: &Address) -> FeeTokenAllowance {
    env.storage()
        .temporary()
        .get(&FeeTokenKey::Allowance(from.clone(), spender.clone()))
        .unwrap_or(FeeTokenAllowance {
            amount: 0,
            expiration_ledger: 0,
        })
}

fn spend_allowance(env: &Env, from: &Address, spender: &Address, amount: i128) {
    let current = read_allowance(env, from, spender);
    assert!(current.amount >= amount, "insufficient allowance");
    env.storage().temporary().set(
        &FeeTokenKey::Allowance(from.clone(), spender.clone()),
        &FeeTokenAllowance {
            amount: current.amount - amount,
            expiration_ledger: current.expiration_ledger,
        },
    );
}

fn fee_on(env: &Env, amount: i128) -> i128 {
    let bps: u32 = env
        .storage()
        .instance()
        .get(&FeeTokenKey::FeeBps)
        .unwrap_or(0);
    if bps == 0 || amount == 0 {
        return 0;
    }
    // Round the fee down so `net = amount - fee` is always >= 0.
    (amount * i128::from(bps)) / 10_000
}

/// One-call helper the test suites expect: registers a {@link FeeOnTransferToken}
/// in the given env, initialises it with `fee_bps` basis points (e.g. `25`
/// for 0.25%), and returns `(address, fee_sink)`. Mint balances with
/// `FeeOnTransferTokenClient::new(env, &address).mint_for_tests(&who, &amount)`.
pub fn deploy_fee_on_transfer_token(env: &Env, fee_bps: u32) -> (Address, Address) {
    let contract_id = env.register(FeeOnTransferToken, ());
    let sink = Address::generate(env);
    FeeOnTransferTokenClient::new(env, &contract_id).init(
        &sink,
        &fee_bps,
        &7u32,
        &String::from_str(env, "FeeMockToken"),
        &String::from_str(env, "FEE"),
    );
    (contract_id, sink)
}

#[cfg(test)]
mod helper_smoke {
    use super::*;

    #[test]
    fn assert_contract_error_accepts_the_expected_error() {
        let result: Result<Result<(), ()>, Result<Error, ()>> = Err(Ok(Error::IdenticalParties));
        assert_contract_error(result, Error::IdenticalParties);
    }

    #[test]
    #[should_panic(
        expected = "expected Error::IdenticalParties (15), got Error::InvalidAmount (4)"
    )]
    fn assert_contract_error_names_both_variants_on_mismatch() {
        let result: Result<Result<(), ()>, Result<Error, ()>> = Err(Ok(Error::InvalidAmount));
        assert_contract_error(result, Error::IdenticalParties);
    }

    #[test]
    #[should_panic(expected = "expected Error::Overflow (6), but the call succeeded with 7")]
    fn assert_contract_error_reports_unexpected_success() {
        let result: Result<Result<i32, ()>, Result<Error, ()>> = Ok(Ok(7));
        assert_contract_error(result, Error::Overflow);
    }

    #[test]
    fn advance_by_moves_the_clock_forward_by_delta() {
        let te = TestEnv::at(1_000);
        te.advance_by(250);
        assert_eq!(te.timestamp(), 1_250);
    }

    #[test]
    #[should_panic(expected = "would overflow u64")]
    fn advance_by_panics_instead_of_wrapping_past_u64_max() {
        let te = TestEnv::at(u64::MAX - 1);
        te.advance_by(2);
    }

    #[test]
    fn advance_to_end_of_sets_the_clock_to_the_boundary() {
        let te = TestEnv::at(1_000);
        te.advance_to_end_of(5_000);
        assert_eq!(te.timestamp(), 5_000);
    }

    #[test]
    #[should_panic(expected = "before the current ledger time")]
    fn advance_to_end_of_refuses_to_move_backwards() {
        let te = TestEnv::at(5_000);
        te.advance_to_end_of(1_000);
    }

    /// Issue #373: the admin can deauthorize a holder, the holder's transfer
    /// is rejected, and restoring authorization lets it succeed again.
    #[test]
    fn make_token_with_admin_lets_the_admin_deauthorize_a_holder() {
        let te = TestEnv::new();
        let (token, admin, funder) = te.make_token_with_admin(1_000);
        let other = te.make_address();

        admin.set_authorized(&funder, &false);
        assert!(
            token.try_transfer(&funder, &other, &100).is_err(),
            "a deauthorized holder must not be able to transfer"
        );
        assert_eq!(
            token.balance(&funder),
            1_000,
            "a rejected transfer moves nothing"
        );

        admin.set_authorized(&funder, &true);
        token.transfer(&funder, &other, &100);
        assert_eq!(token.balance(&other), 100);
    }
}

#[cfg(test)]
mod fee_token_smoke {
    use super::*;

    #[test]
    fn skims_the_configured_basis_points_on_transfer() {
        let env = Env::default();
        env.mock_all_auths();
        let (token_addr, sink) = deploy_fee_on_transfer_token(&env, 100); // 1%
        let client = FeeOnTransferTokenClient::new(&env, &token_addr);

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        client.mint_for_tests(&alice, &10_000);

        client.transfer(&alice, &bob, &10_000);

        assert_eq!(client.balance(&alice), 0);
        // 1% skimmed to the sink, 99% delivered.
        assert_eq!(client.balance(&bob), 9_900);
        assert_eq!(client.balance(&sink), 100);
    }

    #[test]
    fn zero_fee_behaves_like_an_ordinary_token() {
        let env = Env::default();
        env.mock_all_auths();
        let (token_addr, _sink) = deploy_fee_on_transfer_token(&env, 0);
        let client = FeeOnTransferTokenClient::new(&env, &token_addr);

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        client.mint_for_tests(&alice, &1_000);

        client.transfer(&alice, &bob, &250);

        assert_eq!(client.balance(&alice), 750);
        assert_eq!(client.balance(&bob), 250);
    }

    #[test]
    fn transfer_from_respects_allowance_and_skims_the_fee() {
        let env = Env::default();
        env.mock_all_auths();
        let (token_addr, sink) = deploy_fee_on_transfer_token(&env, 50); // 0.5%
        let client = FeeOnTransferTokenClient::new(&env, &token_addr);

        let owner = Address::generate(&env);
        let spender = Address::generate(&env);
        let dest = Address::generate(&env);
        client.mint_for_tests(&owner, &2_000);
        client.approve(&owner, &spender, &1_000, &(env.ledger().sequence() + 100));

        client.transfer_from(&spender, &owner, &dest, &1_000);

        assert_eq!(client.balance(&owner), 1_000);
        assert_eq!(client.balance(&dest), 995);
        assert_eq!(client.balance(&sink), 5);
        assert_eq!(client.allowance(&owner, &spender), 0);
    }
}
