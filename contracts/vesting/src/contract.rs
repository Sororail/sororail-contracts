use soroban_sdk::{contract, contractimpl, token, Address, Env, Vec};
use sororail_common::{auth, math, Error};

use crate::{
    events, storage,
    types::{Checkpoint, Grant, MAX_CHECKPOINTS},
};

#[contract]
pub struct VestingContract;

#[contractimpl]
impl VestingContract {
    /// Creates and fully funds a vesting grant.
    ///
    /// `cliff` and `duration` are spans in seconds from `start`, not absolute
    /// timestamps. `cliff == duration` is a single all-or-nothing unlock;
    /// `cliff == 0` is linear vesting with no cliff. A cliff after the end
    /// would make the schedule unreachable, so it is rejected.
    ///
    /// `start + duration` is rejected if it would overflow `u64`, rather than
    /// letting [`Grant::end_at`](crate::types::Grant::end_at)'s
    /// `saturating_add` silently cap it. This is what [`Grant::vested_amount`]
    /// depends on to compute `effective - start` without underflowing: once
    /// `create` has validated that the schedule's end is representable,
    /// `effective` (which is always clamped into `[cliff_at, end]`) can never
    /// fall below `start`.
    ///
    /// The full total is pulled from the grantor immediately -- a grant is
    /// never partially funded.
    ///
    /// `grantor` and `beneficiary` must differ; a self-grant is rejected.
    ///
    /// # Why positional arguments (not a `CreateGrantParams` struct)
    ///
    /// Soroban does support a single `#[contracttype]` struct parameter, and
    /// that would silence this lint. Positional arguments were kept anyway so
    /// the on-chain ABI stays a flat argument list — the same shape the
    /// TypeScript SDK's uniform build → simulate → sign → send → confirm
    /// surface (SPEC.md) will bind without introducing a nested params object
    /// per entry point. Same decision as `stream::create`; revisit only if the
    /// SDK deliberately adopts struct params across every contract.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        env: Env,
        grantor: Address,
        beneficiary: Address,
        token: Address,
        total: i128,
        start: u64,
        cliff: u64,
        duration: u64,
        revocable: bool,
    ) -> Result<(), Error> {
        if storage::is_initialized(&env) {
            return Err(Error::AlreadyInitialized);
        }
        auth::require_distinct(&grantor, &beneficiary)?;
        math::require_positive(total)?;
        if duration == 0 {
            return Err(Error::InvalidDuration);
        }
        if cliff > duration {
            return Err(Error::VestingCliffAfterEnd);
        }
        if start.checked_add(duration).is_none() {
            return Err(Error::VestingScheduleOverflow);
        }

        grantor.require_auth();

        let grant = Grant {
            grantor: grantor.clone(),
            beneficiary: beneficiary.clone(),
            token: token.clone(),
            total,
            start,
            cliff,
            duration,
            revocable,
            claimed: 0,
            returned: 0,
            revoked_at: None,
        };
        if grant.checkpoint_count() > MAX_CHECKPOINTS {
            return Err(Error::VestingTooManyCheckpoints);
        }
        storage::save(&env, &grant);

        token::TokenClient::new(&env, &grant.token).transfer(
            &grantor,
            env.current_contract_address(),
            &total,
        );
        sororail_common::events::publish(
            &env,
            &events::Created {
                grantor,
                beneficiary,
                token,
                total,
                start,
                cliff,
                duration,
                revocable,
            },
        );
        Ok(())
    }

    /// Transfers vested-but-unclaimed tokens to the beneficiary.
    ///
    /// Returns the amount claimed. Remains available after revocation: a
    /// revoked grant keeps its vested portion claimable, which is the whole
    /// point of vesting.
    pub fn claim(env: Env) -> Result<i128, Error> {
        Self::claim_internal(&env, None, None)
    }

    /// Transfers a specific amount of vested-but-unclaimed tokens to the beneficiary.
    ///
    /// Returns the amount claimed. Rejects zero, negative amounts, or amounts
    /// exceeding what is claimable.
    pub fn claim_amount(env: Env, amount: i128) -> Result<i128, Error> {
        if amount <= 0 {
            return Err(Error::VestingNothingToClaim);
        }
        Self::claim_internal(&env, Some(amount), None)
    }

        grant.claimed = math::add(grant.claimed, claimable)?;
        storage::save(&env, &grant);

        Self::pay(&env, &grant, &grant.beneficiary, claimable);
        sororail_common::events::publish(
            &env,
            &events::Claimed {
                beneficiary: grant.beneficiary.clone(),
                amount: claimable,
                total_claimed: grant.claimed,
            },
        );
        Ok(claimable)
    }

    /// Reclaims the unvested portion for the grantor.
    ///
    /// Vesting freezes at this moment. Whatever had vested stays in the
    /// contract and remains claimable by the beneficiary; only the unvested
    /// remainder is returned.
    pub fn revoke(env: Env) -> Result<i128, Error> {
        let mut grant = storage::load(&env)?;
        if grant.is_revoked() {
            return Err(Error::VestingRevoked);
        }
        auth::require(grant.revocable, Error::VestingNotRevocable)?;
        grant.grantor.require_auth();

        let now = env.ledger().timestamp();
        let vested = grant.vested_amount(now)?;
        let unvested = math::sub(grant.total, vested)?;

        grant.revoked_at = Some(now);
        grant.returned = unvested;
        storage::save(&env, &grant);

        if unvested > 0 {
            Self::pay(&env, &grant, &grant.grantor, unvested);
        }
        sororail_common::events::publish(
            &env,
            &events::Revoked {
                grantor: grant.grantor.clone(),
                returned_to_grantor: unvested,
                // `claimed` can never exceed `vested` here: `claim` adds
                // exactly the positive `claimable_at = vested - claimed`, so
                // it never overshoots the vested amount, and `vested_amount` is
                // monotonic in time, so the `vested` computed at revocation is
                // at least the `vested` that capped every earlier claim. The
                // subtraction is therefore total -- `Underflow` is unreachable,
                // which `revoke_succeeds_after_claiming_every_vested_token`
                // pins down.
                still_claimable: math::sub(vested, grant.claimed)?,
                revoked_at: now,
            },
        );
        Ok(unvested)
    }

    /// Hands the grant to a new beneficiary, for key rotation or a wallet
    /// migration. Only the current beneficiary may do this.
    ///
    /// The schedule, `claimed` and `remaining` are untouched, so vesting
    /// continues as before. Every later `claim` pays `new_beneficiary`.
    /// Tokens already claimed stay with the previous beneficiary.
    ///
    /// `new_beneficiary` must differ from both the current beneficiary and the
    /// grantor, which keeps the `grantor != beneficiary` rule from `create`.
    /// A revoked grant cannot be transferred.
    pub fn transfer_beneficiary(env: Env, new_beneficiary: Address) -> Result<(), Error> {
        let mut grant = storage::load(&env)?;
        if grant.is_revoked() {
            return Err(Error::VestingRevoked);
        }
        grant.beneficiary.require_auth();
        auth::require_distinct(&new_beneficiary, &grant.beneficiary)?;
        auth::require_distinct(&new_beneficiary, &grant.grantor)?;

        let from = grant.beneficiary.clone();
        grant.beneficiary = new_beneficiary.clone();
        storage::save(&env, &grant);

        events::BeneficiaryTransferred {
            from,
            to: new_beneficiary,
        }
        .publish(&env);
        Ok(())
    }

    /// Total vested as of `at`, whether claimed or not. Pure view.
    pub fn vested_amount(env: Env, at: u64) -> Result<i128, Error> {
        storage::load_view(&env)?.vested_amount(at)
    }

    /// Vested but not yet claimed, as of now.
    pub fn claimable(env: Env) -> Result<i128, Error> {
        let grant = storage::load_view(&env)?;
        grant.claimable_at(env.ledger().timestamp())
    }

    /// Total not yet vested as of `at`, whether claimed or not. Pure view.
    pub fn unvested(env: Env, at: u64) -> Result<i128, Error> {
        storage::load_view(&env)?.unvested(at)
    }

    /// Percentage of total vested as of `at`, in basis points (0-10000). Pure view.
    pub fn percent_vested_bps(env: Env, at: u64) -> Result<i128, Error> {
        storage::load_view(&env)?.percent_vested_bps(at)
    }

    /// Next unlock timestamp after `at`, or `None` if fully vested. Pure view.
    pub fn next_unlock_at(env: Env, at: u64) -> Result<Option<u64>, Error> {
        Ok(storage::load_view(&env)?.next_unlock_at(at))
    }

    /// The schedule as a list of checkpoints, in ascending time order.
    ///
    /// Lets a client draw the schedule without recomputing it. Between two
    /// consecutive checkpoints vesting is linear, so the list is the exact
    /// curve. Reports the schedule as created; use `vested_amount` for the
    /// revocation-aware figure.
    pub fn schedule(env: Env) -> Result<Vec<Checkpoint>, Error> {
        storage::load_view(&env)?.checkpoints(&env)
    }

    /// What the contract still holds for this grant.
    pub fn remaining(env: Env) -> Result<i128, Error> {
        storage::load_view(&env)?.remaining()
    }

    /// Extends the instance TTL for a long-lived grant without changing it.
    pub fn bump(env: Env) -> Result<(), Error> {
        storage::load_view(&env)?;
        sororail_common::storage::extend_instance(&env);
        Ok(())
    }

    /// The full grant record.
    pub fn get(env: Env) -> Result<Grant, Error> {
        storage::load_view(&env)
    }
}

// Helpers. Outside `#[contractimpl]` so they are not contract entry points.
impl VestingContract {
    /// Internal claim helper used by `claim`, `claim_amount`, and `claim_to`.
    ///
    /// `amount_opt`: if Some, claim exactly that amount; if None, claim all claimable.
    /// `destination_opt`: if Some, send to this address; if None, send to beneficiary.
    fn claim_internal(
        env: &Env,
        amount_opt: Option<i128>,
        destination_opt: Option<Address>,
    ) -> Result<i128, Error> {
        let mut grant = storage::load(env)?;
        grant.beneficiary.require_auth();

        let now = env.ledger().timestamp();
        let effective = match grant.revoked_at {
            Some(revoked) => now.min(revoked),
            None => now,
        };
        if effective < grant.cliff_at() {
            return Err(Error::VestingCliffNotReached);
        }

        let claimable = grant.claimable_at(now)?;
        if claimable <= 0 {
            return Err(Error::VestingNothingToClaim);
        }

        let amount = match amount_opt {
            Some(amt) => {
                if amt > claimable {
                    return Err(Error::VestingNothingToClaim);
                }
                amt
            }
            None => claimable,
        };

        grant.claimed = math::add(grant.claimed, amount)?;
        storage::save(env, &grant);

        let recipient = destination_opt.unwrap_or_else(|| grant.beneficiary.clone());
        Self::pay(env, &grant, &recipient, amount);
        events::Claimed {
            beneficiary: grant.beneficiary.clone(),
            amount,
            total_claimed: grant.claimed,
        }
        .publish(env);
        Ok(amount)
    }

    /// Sends `amount` of the granted token from this contract to `to`.
    ///
    /// Always called after the state change has been persisted.
    fn pay(env: &Env, grant: &Grant, to: &Address, amount: i128) {
        debug_assert!(amount > 0, "pay requires a positive amount");
        token::TokenClient::new(env, &grant.token).transfer(
            &env.current_contract_address(),
            to,
            &amount,
        );
    }
}
