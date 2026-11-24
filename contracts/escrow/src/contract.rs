use soroban_sdk::{contract, contractimpl, token, Address, BytesN, Env};
use sororail_common::{auth, math, Error};

use crate::{
    events, storage,
    types::{Escrow, State},
};

#[contract]
pub struct EscrowContract;

#[contractimpl]
impl EscrowContract {
    /// Configures the escrow. One agreement per deployed instance.
    ///
    /// Requires the depositor's authorization: `init` decides whose funds
    /// `fund` will later pull, so a third party must not be able to name
    /// someone else as depositor.
    ///
    /// `deadline` is a ledger timestamp and must be in the future. After it
    /// passes, the depositor may refund unilaterally.
    ///
    /// `arbiter` is optional. When it is `None`:
    ///
    /// - `dispute` returns [`Error::EscrowNoArbiter`], so the escrow can never
    ///   enter `Disputed`.
    /// - `resolve` is unreachable, because it is only callable from `Disputed`.
    /// - `release` is callable only by the depositor, and `refund` only by the
    ///   depositor after the deadline. Neither party can force a dispute, and
    ///   no third party can intervene.
    ///
    /// Choose `Some(arbiter)` whenever the parties may need a neutral split.
    /// An arbiter must be distinct from both depositor and beneficiary.
    /// `None` is only safe when the depositor is trusted to release or refund.
    ///
    /// `dispute_timeout` is optional and requires an arbiter. When `Some(s)`,
    /// either party may call `resolve_timeout` once `s` seconds have passed
    /// since the dispute was raised, which refunds the depositor if the
    /// arbiter has not acted. `Some(0)` is rejected with
    /// [`Error::InvalidDuration`], and `Some(_)` without an arbiter with
    /// [`Error::EscrowNoArbiter`]. `None` keeps the arbiter as the only way
    /// out of a dispute (apart from the grace-period refund in `refund`).
    pub fn init(
        env: Env,
        depositor: Address,
        beneficiary: Address,
        arbiter: Option<Address>,
        token: Address,
        amount: i128,
        deadline: u64,
        dispute_timeout: Option<u64>,
        reference: Option<BytesN<32>>,
        arbiter_fee_bps: u32,
    ) -> Result<(), Error> {
        if storage::is_initialized(&env) {
            return Err(Error::AlreadyInitialized);
        }
        math::require_positive(amount)?;
        if deadline <= env.ledger().timestamp() {
            return Err(Error::InvalidTimeRange);
        }
        // Reject conflicted arbitrators before persisting an agreement or
        // requesting a depositor signature. `None` retains the existing
        // explicit no-arbitration behavior.
        if let Some(ref neutral) = arbiter {
            if *neutral == depositor || *neutral == beneficiary {
                return Err(Error::EscrowArbiterNotNeutral);
            }
        }
        if dispute_timeout == Some(0) {
            return Err(Error::InvalidDuration);
        }
        // A timeout only ever applies to a dispute, and a dispute needs an
        // arbiter, so a timeout without one could never take effect.
        if dispute_timeout.is_some() && arbiter.is_none() {
            return Err(Error::EscrowNoArbiter);
        }
        if arbiter_fee_bps > 1_000 {
            return Err(Error::InvalidBasisPoints);
        }
        depositor.require_auth();

        let escrow = Escrow {
            depositor: depositor.clone(),
            beneficiary: beneficiary.clone(),
            arbiter,
            token: token.clone(),
            amount,
            deadline,
            dispute_timeout,
            reference: reference.clone(),
            arbiter_fee_bps,
            disputed_at: None,
            created_at: env.ledger().timestamp(),
            funded_at: None,
            state: State::Created,
        };
        storage::save(&env, &escrow);
        events::Created {
            depositor,
            beneficiary,
            token,
            amount,
            deadline,
            reference,
        }
        .publish(&env);
        Ok(())
    }

    /// Cancels a Created escrow, returning control of the instance to the
    /// deployer. Only the depositor may do this, and only before funding.
    ///
    /// This provides an exit from a mistaken `init` without burning the
    /// deployed instance.
    pub fn cancel(env: Env) -> Result<(), Error> {
        let mut escrow = storage::load(&env)?;
        auth::require(escrow.state == State::Created, Error::EscrowNotCancellable)?;
        escrow.depositor.require_auth();

        escrow.state = State::Refunded;
        storage::save(&env, &escrow);

        events::Cancelled {
            depositor: escrow.depositor.clone(),
        }
        .publish(&env);
        Ok(())
    }

    /// Pulls the agreed amount from the depositor into the contract.
    ///
    /// Rejected once `deadline` has passed (inclusive, matching the boundary
    /// at which the depositor's unilateral refund right begins): funding an
    /// agreement that has already lapsed would only let the depositor pull
    /// their own funds right back out via `refund`, so it is refused up
    /// front with [`Error::DeadlinePassed`] rather than the generic
    /// [`Error::EscrowClosed`], which describes a *terminal* state and would
    /// be misleading here -- a never-funded, lapsed escrow is still `Created`.
    pub fn fund(env: Env) -> Result<(), Error> {
        let mut escrow = storage::load(&env)?;
        if escrow.state.is_terminal() {
            return Err(Error::EscrowClosed);
        }
        let now = env.ledger().timestamp();
        if !Self::is_fundable(&escrow, now) {
            if escrow.state != State::Created {
                return Err(Error::EscrowNotFundable);
            }
            return Err(Error::DeadlinePassed);
        }
        escrow.depositor.require_auth();

        let token = token::TokenClient::new(&env, &escrow.token);
        let contract_address = env.current_contract_address();
        let balance_before = token.balance(&contract_address);
        escrow.state = State::Funded;
        escrow.funded_at = Some(now);
        storage::save(&env, &escrow);

        token.transfer(
            &escrow.depositor,
            &contract_address,
            &escrow.amount,
        );
        let balance_after = token.balance(&contract_address);
        let expected = balance_before
            .checked_add(escrow.amount)
            .ok_or(Error::Overflow)?;
        auth::require(
            balance_after == expected,
            Error::EscrowBalanceMismatch,
        )?;
        events::Funded {
            depositor: escrow.depositor.clone(),
            amount: escrow.amount,
            reference: escrow.reference.clone(),
        }
        .publish(&env);
        Ok(())
    }

    /// Pays the beneficiary. Callable by the depositor or the arbiter.
    ///
    /// The beneficiary cannot release to themselves -- that is the whole point
    /// of the escrow.
    pub fn release(env: Env, caller: Address) -> Result<(), Error> {
        let mut escrow = storage::load(&env)?;
        Self::require_funded(&escrow)?;
        auth::require_auth_either_opt(&caller, &escrow.depositor, &escrow.arbiter)?;

        escrow.state = State::Released;
        storage::save(&env, &escrow);

        Self::pay(&env, &escrow, &escrow.beneficiary, escrow.amount);
        events::Released {
            beneficiary: escrow.beneficiary.clone(),
            amount: escrow.amount,
            released_by: caller,
        }
        .publish(&env);
        Ok(())
    }

    /// Returns the funds to the depositor.
    ///
    /// The depositor may do this only once the deadline has passed. The
    /// arbiter may do it at any time. If a dispute is unresolved after the
    /// dispute grace period expires (7 days past the original deadline), anyone
    /// may refund to recover funds from an unresponsive arbiter.
    ///
    /// # Dispute recovery
    ///
    /// Once the deadline passes, an unresolved dispute becomes a safety
    /// mechanism: if the arbiter is unresponsive, either party (or anyone) may
    /// call `refund` after the grace period expires to recover the deposited
    /// funds to the original depositor.
    pub fn refund(env: Env, caller: Address) -> Result<(), Error> {
        let mut escrow = storage::load(&env)?;
        let now = env.ledger().timestamp();
        const DISPUTE_GRACE_PERIOD_SECONDS: u64 = 7 * 24 * 60 * 60; // 7 days

        // If disputed and past the grace period, allow recovery by anyone.
        if escrow.state == State::Disputed {
            let dispute_timeout = escrow.deadline.saturating_add(DISPUTE_GRACE_PERIOD_SECONDS);
            if now >= dispute_timeout {
                escrow.state = State::Refunded;
                storage::save(&env, &escrow);
                Self::pay(&env, &escrow, &escrow.depositor, escrow.amount);
                events::Refunded {
                    depositor: escrow.depositor.clone(),
                    amount: escrow.amount,
                    refunded_by: caller,
                }
                .publish(&env);
                return Ok(());
            }
            // If disputed but within the grace period, require arbiter permission.
            auth::require_auth_either_opt(&caller, &escrow.depositor, &escrow.arbiter)?;
            return Err(Error::DeadlineNotReached);
        }

        // For Funded state, use the original refund logic.
        Self::require_funded(&escrow)?;
        auth::require_auth_either_opt(&caller, &escrow.depositor, &escrow.arbiter)?;

        let is_arbiter = escrow.arbiter.as_ref().is_some_and(|a| *a == caller);
        if !is_arbiter && now < escrow.deadline {
            return Err(Error::DeadlineNotReached);
        }

        escrow.state = State::Refunded;
        storage::save(&env, &escrow);

        Self::pay(&env, &escrow, &escrow.depositor, escrow.amount);
        events::Refunded {
            depositor: escrow.depositor.clone(),
            amount: escrow.amount,
            refunded_by: caller,
        }
        .publish(&env);
        Ok(())
    }

    /// Freezes the escrow pending an arbiter decision.
    ///
    /// Callable by either party. Requires that an arbiter was configured --
    /// without one there would be nobody able to resolve the dispute, so the
    /// funds would be stuck.
    pub fn dispute(env: Env, caller: Address) -> Result<(), Error> {
        let mut escrow = storage::load(&env)?;
        auth::require(escrow.arbiter.is_some(), Error::EscrowNoArbiter)?;
        if escrow.state == State::Disputed {
            return Err(Error::EscrowAlreadyDisputed);
        }
        Self::require_funded(&escrow)?;
        auth::require_auth_either(&caller, &escrow.depositor, &escrow.beneficiary)?;

        escrow.state = State::Disputed;
        escrow.disputed_at = Some(env.ledger().timestamp());
        storage::save(&env, &escrow);
        events::Disputed { raised_by: caller }.publish(&env);
        Ok(())
    }

    /// Refunds the depositor once an unresolved dispute has outlived the
    /// `dispute_timeout` set in `init`. Callable by either party.
    ///
    /// The arbiter keeps the power to `resolve` until this runs. Both paths
    /// require `Disputed`, and each moves the escrow to a terminal state, so
    /// whichever transaction lands first decides the outcome. An arbiter who
    /// resolves at `disputed_at + dispute_timeout - 1` therefore wins, and a
    /// timeout call at the same moment fails with `EscrowNotDisputed`.
    ///
    /// Fails with [`Error::EscrowNoDisputeTimeout`] when `init` configured no
    /// timeout, and with [`Error::EscrowDisputeTimeoutNotReached`] before the
    /// timeout has elapsed. The boundary is inclusive: the call succeeds at
    /// exactly `disputed_at + dispute_timeout`.
    pub fn resolve_timeout(env: Env, caller: Address) -> Result<(), Error> {
        let mut escrow = storage::load(&env)?;
        auth::require(escrow.state == State::Disputed, Error::EscrowNotDisputed)?;
        let timeout = escrow
            .dispute_timeout
            .ok_or(Error::EscrowNoDisputeTimeout)?;
        // `dispute` always sets `disputed_at`, so this is defense-in-depth,
        // in the same spirit as the `arbiter` check in `resolve`.
        let disputed_at = escrow.disputed_at.ok_or(Error::EscrowNotDisputed)?;
        auth::require_auth_either(&caller, &escrow.depositor, &escrow.beneficiary)?;

        let unlocks_at = disputed_at.saturating_add(timeout);
        auth::require(
            env.ledger().timestamp() >= unlocks_at,
            Error::EscrowDisputeTimeoutNotReached,
        )?;

        escrow.state = State::Refunded;
        storage::save(&env, &escrow);

        Self::pay(&env, &escrow, &escrow.depositor, escrow.amount);
        events::Refunded {
            depositor: escrow.depositor.clone(),
            amount: escrow.amount,
            refunded_by: caller,
        }
        .publish(&env);
        Ok(())
    }

    /// Moves the deadline of a funded escrow later. Requires agreement from
    /// both the depositor and the beneficiary.
    ///
    /// Allowed only while `Funded` and strictly before the current deadline.
    /// At or after the deadline, the depositor already holds the refund right,
    /// so the call fails with [`Error::DeadlinePassed`]. A `new_deadline` that
    /// is not strictly later than the current one fails with
    /// [`Error::InvalidTimeRange`]. Emits `DeadlineExtended` with the old and
    /// new values.
    pub fn extend_deadline(env: Env, new_deadline: u64) -> Result<(), Error> {
        let mut escrow = storage::load(&env)?;
        Self::require_funded(&escrow)?;
        auth::require(
            env.ledger().timestamp() < escrow.deadline,
            Error::DeadlinePassed,
        )?;
        auth::require(new_deadline > escrow.deadline, Error::InvalidTimeRange)?;
        // Both parties are fixed, so no membership check is needed. Each
        // signature is requested in turn, depositor first.
        escrow.depositor.require_auth();
        escrow.beneficiary.require_auth();

        let old_deadline = escrow.deadline;
        escrow.deadline = new_deadline;
        storage::save(&env, &escrow);

        events::DeadlineExtended {
            depositor: escrow.depositor.clone(),
            beneficiary: escrow.beneficiary.clone(),
            old_deadline,
            new_deadline,
        }
        .publish(&env);
        Ok(())
    }

    /// Seconds remaining until `deadline`, by the ledger clock. Returns `0`
    /// once the deadline has been reached. Read-only, so it does not extend
    /// the TTL.
    pub fn time_to_deadline(env: Env) -> Result<u64, Error> {
        let escrow = storage::load_view(&env)?;
        Ok(escrow.deadline.saturating_sub(env.ledger().timestamp()))
    }

    /// Splits the funds between the parties. Arbiter only.
    ///
    /// `split_bps` is the share paid to the **beneficiary**, in basis points;
    /// the remainder goes to the depositor. The remainder is computed by
    /// subtraction, so the two payments always sum to exactly the escrowed
    /// amount with no rounding leakage.
    ///
    /// # Invariants
    ///
    /// `resolve` is only callable from `State::Disputed`, which can only be
    /// reached via `dispute`, which requires `escrow.arbiter.is_some()`.
    /// Therefore an arbiter is guaranteed to exist here — no need to handle
    /// the `None` case (#105).
    pub fn resolve(env: Env, split_bps: u32) -> Result<(), Error> {
        let mut escrow = storage::load(&env)?;
        auth::require(escrow.state == State::Disputed, Error::EscrowNotDisputed)?;
        // `dispute` requires `arbiter.is_some()` (#105), so this arm is
        // unreachable through the public API. The explicit check is defense-in-depth:
        // if the invariant is ever violated by a storage migration or test bypass,
        // the contract returns a typed error rather than aborting with a panic.
        let arbiter = escrow.arbiter.clone().ok_or(Error::EscrowNoArbiter)?;
        arbiter.require_auth();

        let fee = math::mul_bps(escrow.amount, escrow.arbiter_fee_bps)?;
        let after_fee = escrow.amount - fee;
        let (to_beneficiary, to_depositor) = math::split_bps(after_fee, split_bps)?;

        escrow.state = State::Resolved;
        storage::save(&env, &escrow);

        if fee > 0 {
            Self::pay(&env, &escrow, &arbiter, fee);
        }
        if to_beneficiary > 0 {
            Self::pay(&env, &escrow, &escrow.beneficiary, to_beneficiary);
        }
        if to_depositor > 0 {
            Self::pay(&env, &escrow, &escrow.depositor, to_depositor);
        }
        events::Resolved {
            arbiter,
            fee,
            split_bps,
            to_beneficiary,
            to_depositor,
        }
        .publish(&env);
        Ok(())
    }

    /// The current lifecycle position.
    pub fn state(env: Env) -> Result<State, Error> {
        Ok(storage::load_view(&env)?.state)
    }

    /// Whether the escrow can currently be funded.
    pub fn is_fundable(env: Env) -> Result<bool, Error> {
        let escrow = storage::load_view(&env)?;
        Ok(Self::is_fundable(&escrow, env.ledger().timestamp()))
    }

    /// Extends the instance TTL for a long-lived escrow without changing it.
    pub fn bump(env: Env) -> Result<(), Error> {
        storage::load_view(&env)?;
        sororail_common::storage::extend_instance(&env);
        Ok(())
    }

    /// The full agreement.
    pub fn get(env: Env) -> Result<Escrow, Error> {
        storage::load_view(&env)
    }

    /// The arbiter, if one was configured.
    pub fn arbiter(env: Env) -> Result<Option<Address>, Error> {
        Ok(storage::load_view(&env)?.arbiter)
    }

    /// The depositor and beneficiary as (depositor, beneficiary).
    pub fn parties(env: Env) -> Result<(Address, Address), Error> {
        let escrow = storage::load_view(&env)?;
        Ok((escrow.depositor, escrow.beneficiary))
    }
}

// Helpers. Deliberately outside `#[contractimpl]` so they do not become
// contract entry points.
impl EscrowContract {
    fn is_fundable(escrow: &Escrow, now: u64) -> bool {
        escrow.state == State::Created && now < escrow.deadline
    }
    /// Errors unless the escrow is holding funds and still open.
    fn require_funded(escrow: &Escrow) -> Result<(), Error> {
        if escrow.state.is_terminal() {
            return Err(Error::EscrowClosed);
        }
        auth::require(escrow.state == State::Funded, Error::EscrowNotFunded)
    }

    /// Sends `amount` of the escrowed token from this contract to `to`.
    ///
    /// Always called after the state transition has been persisted, so a
    /// reentrant token cannot observe a stale state.
    fn pay(env: &Env, escrow: &Escrow, to: &Address, amount: i128) {
        debug_assert!(amount > 0, "pay requires a positive amount");
        token::TokenClient::new(env, &escrow.token).transfer(
            &env.current_contract_address(),
            to,
            &amount,
        );
    }
}
