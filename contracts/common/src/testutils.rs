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
    testutils::{Address as _, Ledger as _},
    token::{StellarAssetClient, TokenClient},
    Address, Env,
};

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
