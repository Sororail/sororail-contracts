use soroban_sdk::{contract, contractimpl, token, Address, Env};
use sororail_common::{auth, math, Error};

use crate::{events, storage, types::Authorization};

fn charge_one_period(
    env: &Env,
    authorization: &mut Authorization,
    now: u64,
    reset_schedule_from_now: bool,
) -> Result<i128, Error> {
    let amount = authorization.amount_per_period;
    authorization.periods_charged = authorization
        .periods_charged
        .checked_add(1)
        .ok_or(Error::Overflow)?;
    let next_chargeable_at = if reset_schedule_from_now {
        now.checked_add(authorization.period_seconds)
    } else {
        authorization
            .next_chargeable_at
            .checked_add(authorization.period_seconds)
    }
    .ok_or(Error::InvalidTimeRange)?;
    authorization.next_chargeable_at = next_chargeable_at;
    // The period is consumed and persisted here, *before* `transfer_from`
    // moves any funds. This ordering is intentional: Soroban aborts the
    // whole invocation and rolls back every state change when a host call
    // (the token transfer) traps, so a failed transfer cannot leave the
    // period consumed -- the `periods_charged` bump and the advanced
    // `next_chargeable_at` are reverted together with the failed call.
    // That rollback, not the save-first ordering itself, is what makes
    // this safe; it also means a reentrant token sees the already-advanced
    // `next_chargeable_at` and cannot charge the same period twice.
    // Compare `escrow::pay`, where persisting state before the token call
    // serves reentrancy protection specifically.
    authorization.total_charged = math::add(authorization.total_charged, amount)?;
    storage::save(env, authorization);

    token::TokenClient::new(env, &authorization.token).transfer_from(
        &env.current_contract_address(),
        &authorization.payer,
        &authorization.payee,
        &amount,
    );

    // Apply pending amount if one was scheduled.
    if let Some(pending) = authorization.pending_amount_per_period.take() {
        authorization.amount_per_period = pending;
        storage::save(env, &authorization);
    }

    events::Charged {
        payee: authorization.payee.clone(),
        payer: authorization.payer.clone(),
        amount,
        period_index: authorization.periods_charged,
        periods_charged: authorization.periods_charged,
        next_chargeable_at: authorization.next_chargeable_at,
        remaining: authorization.remaining_periods(),
    }
    .publish(env);
    Ok(amount)
}

#[contract]
pub struct RecurringContract;

#[contractimpl]
impl RecurringContract {
    /// Records a recurring pull authorization.
    ///
    /// Requires the payer's authorization -- they are the one consenting to be
    /// charged. The first charge becomes available one full period after this
    /// call, so a subscription never bills at the moment of signup without the
    /// payer seeing a period pass.
    ///
    /// `payer` and `payee` must differ; a self-authorization is rejected.
    ///
    /// **This call does not by itself let the payee take anything.** The payer
    /// must also `approve` this contract as a spender on the token for at
    /// least the amount they intend to allow. That allowance is the real cap:
    /// revoking it on the token stops charges even without cancelling here.
    ///
    /// To fail fast, `authorize` checks that the payer's current allowance for
    /// this contract covers `amount_per_period * max_periods` when capped, or
    /// one `amount_per_period` when uncapped, and returns
    /// `RecurringInsufficientAllowance` otherwise. So the payer must `approve`
    /// **before** calling `authorize`. This is a point-in-time check only: the
    /// allowance can still be lowered, expire or be revoked later, and a
    /// charge then fails with the token's own error.
    pub fn authorize(
        env: Env,
        payer: Address,
        payee: Address,
        token: Address,
        amount_per_period: i128,
        period_seconds: u64,
        max_periods: Option<u32>,
        valid_until: Option<u64>,
    ) -> Result<(), Error> {
        if storage::is_initialized(&env) {
            return Err(Error::AlreadyInitialized);
        }
        if payer == payee {
            return Err(Error::IdenticalParties);
        }
        math::require_positive(amount_per_period)?;
        if period_seconds == 0 {
            return Err(Error::InvalidDuration);
        }
        if max_periods == Some(0) {
            return Err(Error::InvalidAmount);
        }

        payer.require_auth();

        let required_allowance = match max_periods {
            Some(cap) => math::mul(amount_per_period, i128::from(cap))?,
            None => amount_per_period,
        };
        let allowance = token::TokenClient::new(&env, &token)
            .allowance(&payer, &env.current_contract_address());
        if allowance < required_allowance {
            return Err(Error::RecurringInsufficientAllowance);
        }

        let first_chargeable_at = env
            .ledger()
            .timestamp()
            .checked_add(period_seconds)
            .ok_or(Error::InvalidTimeRange)?;

        let authorization = Authorization {
            payer: payer.clone(),
            payee: payee.clone(),
            token: token.clone(),
            amount_per_period,
            period_seconds,
            max_periods,
            periods_charged: 0,
            next_chargeable_at: first_chargeable_at,
            cancelled: false,
            valid_until,
            max_amount: None,
            pending_amount_per_period: None,
            total_charged: 0,
            paused_at: None,
        };
        storage::save(&env, &authorization);

        events::Authorized {
            payer,
            payee,
            token,
            amount_per_period,
            period_seconds,
            max_periods,
            first_chargeable_at,
            valid_until,
        }
        .publish(&env);
        Ok(())
    }

    /// Pulls one period's payment from the payer to the payee.
    ///
    /// Callable by the payee. Moves funds with `transfer_from`, spending the
    /// allowance the payer granted this contract on the token.
    ///
    /// The next charge is scheduled at `now + period_seconds`, **not** one
    /// period after the previous due date. Skipped periods are forfeited
    /// rather than banked -- see [`Authorization`] for why.
    pub fn charge(env: Env) -> Result<i128, Error> {
        let mut authorization = storage::load(&env)?;
        // Require auth before state checks so unauthenticated callers cannot
        // probe contract state via typed errors (CONTRIBUTING.md convention).
        authorization.payee.require_auth();
        if authorization.cancelled {
            return Err(Error::RecurringCancelled);
        }
        if authorization.is_paused() {
            return Err(Error::RecurringPaused);
        }
        if authorization.is_exhausted() {
            return Err(Error::RecurringExhausted);
        }

        let now = env.ledger().timestamp();
        if now < authorization.next_chargeable_at {
            return Err(Error::RecurringPeriodNotElapsed);
        }
        if let Some(until) = authorization.valid_until {
            if now >= until {
                return Err(Error::RecurringExhausted);
            }
        }

        charge_one_period(&env, &mut authorization, now, true)
    }

    /// Pulls up to `max_periods` elapsed payments in one call.
    ///
    /// Callable by the payee. Unlike `charge`, this advances the schedule one
    /// period at a time from its existing due date, so missed periods can be
    /// collected only after their scheduled timestamps have elapsed. The
    /// first scheduled timestamp is still one full period after authorization.
    /// The authorization's `max_periods` cap applies across all calls.
    ///
    /// Returns the total amount transferred. A value of zero for
    /// `max_periods` is invalid. If no period is due, this returns
    /// `RecurringPeriodNotElapsed`.
    pub fn charge_up_to(env: Env, max_periods: u32) -> Result<i128, Error> {
        let mut authorization = storage::load(&env)?;
        authorization.payee.require_auth();
        if max_periods == 0 {
            return Err(Error::InvalidAmount);
        }
        if authorization.cancelled {
            return Err(Error::RecurringCancelled);
        }
        if authorization.is_paused() {
            return Err(Error::RecurringPaused);
        }
        if authorization.is_exhausted() {
            return Err(Error::RecurringExhausted);
        }

        let now = env.ledger().timestamp();
        if now < authorization.next_chargeable_at {
            return Err(Error::RecurringPeriodNotElapsed);
        }
        if let Some(until) = authorization.valid_until {
            if now >= until {
                return Err(Error::RecurringExhausted);
            }
        }

        let mut total = 0i128;
        let mut charged = 0u32;
        while charged < max_periods
            && now >= authorization.next_chargeable_at
            && !authorization.is_exhausted()
            && authorization
                .valid_until
                .map_or(true, |until| authorization.next_chargeable_at < until)
        {
            let amount = charge_one_period(&env, &mut authorization, now, false)?;
            total = math::add(total, amount)?;
            charged = charged.checked_add(1).ok_or(Error::Overflow)?;
        }
        Ok(total)
    }

    /// Pauses charging. Callable only by the payer.
    ///
    /// While paused, `charge` and `charge_up_to` fail with `RecurringPaused`.
    /// The authorization and the token allowance are untouched. See
    /// [`Authorization`] ("Pausing") for how paused time affects the schedule.
    pub fn pause(env: Env) -> Result<(), Error> {
        let mut authorization = storage::load(&env)?;
        authorization.payer.require_auth();
        if authorization.cancelled {
            return Err(Error::RecurringCancelled);
        }
        if authorization.is_paused() {
            return Err(Error::RecurringPaused);
        }

        let now = env.ledger().timestamp();
        authorization.paused_at = Some(now);
        storage::save(&env, &authorization);

        events::Paused {
            payer: authorization.payer.clone(),
            paused_at: now,
        }
        .publish(&env);
        Ok(())
    }

    /// Resumes a paused authorization. Callable only by the payer.
    ///
    /// Paused time **shifts the schedule**: `next_chargeable_at` moves later by
    /// the length of the pause, so a period already partly elapsed when the
    /// payer paused resumes with the same time left, and the payer is never
    /// charged the instant they resume.
    pub fn resume(env: Env) -> Result<(), Error> {
        let mut authorization = storage::load(&env)?;
        authorization.payer.require_auth();
        if authorization.cancelled {
            return Err(Error::RecurringCancelled);
        }
        let paused_at = authorization.paused_at.ok_or(Error::RecurringNotPaused)?;

        let now = env.ledger().timestamp();
        let paused_seconds = now.saturating_sub(paused_at);
        authorization.next_chargeable_at = authorization
            .next_chargeable_at
            .checked_add(paused_seconds)
            .ok_or(Error::InvalidTimeRange)?;
        authorization.paused_at = None;
        storage::save(&env, &authorization);

        events::Resumed {
            payer: authorization.payer.clone(),
            resumed_at: now,
            paused_seconds,
            next_chargeable_at: authorization.next_chargeable_at,
        }
        .publish(&env);
        Ok(())
    }

    /// Updates the amount per period, effective from the next period.
    ///
    /// Requires payer authorization. The in-flight period (the one ready to
    /// charge but not yet charged) uses the old amount; the update applies
    /// starting from the charge after that.
    ///
    /// If `max_amount` was set, the new amount cannot exceed it.
    pub fn update_amount(env: Env, new_amount: i128) -> Result<(), Error> {
        let mut authorization = storage::load(&env)?;
        authorization.payer.require_auth();

        math::require_positive(new_amount)?;
        if let Some(max) = authorization.max_amount {
            if new_amount > max {
                return Err(Error::InvalidAmount);
            }
        }

        let old_amount = authorization.amount_per_period;
        authorization.pending_amount_per_period = Some(new_amount);
        storage::save(&env, &authorization);

        events::AmountUpdated {
            old_amount,
            new_amount,
            effective_from: authorization.next_chargeable_at + authorization.period_seconds,
        }
        .publish(&env);
        Ok(())
    }

    /// Cancels the authorization, effective immediately.
    ///
    /// Callable by either party. The payer can always stop a subscription
    /// without needing the payee's cooperation.
    pub fn cancel(env: Env, caller: Address) -> Result<(), Error> {
        let mut authorization = storage::load(&env)?;
        if authorization.cancelled {
            return Err(Error::RecurringCancelled);
        }
        auth::require_auth_either(&caller, &authorization.payer, &authorization.payee)?;

        authorization.cancelled = true;
        storage::save(&env, &authorization);

        events::Cancelled {
            cancelled_by: caller,
            periods_charged: authorization.periods_charged,
            cancelled_at: env.ledger().timestamp(),
        }
        .publish(&env);
        Ok(())
    }

    /// The earliest timestamp at which the next charge may be taken.
    ///
    /// Reports the stored schedule regardless of whether the authorization is
    /// cancelled or exhausted; check `is_chargeable` for whether a
    /// charge would actually succeed.
    pub fn next_chargeable_at(env: Env) -> Result<u64, Error> {
        Ok(storage::load_view(&env)?.next_chargeable_at)
    }

    /// Whether a charge would succeed right now.
    pub fn is_chargeable(env: Env) -> Result<bool, Error> {
        let authorization = storage::load_view(&env)?;
        Ok(authorization.is_chargeable_at(env.ledger().timestamp()))
    }

    /// Charges still permitted under the cap, if there is one.
    pub fn remaining_periods(env: Env) -> Result<Option<u32>, Error> {
        Ok(storage::load_view(&env)?.remaining_periods())
    }

    /// Charges still permitted under the cap, if there is one.
    ///
    /// Same value as `remaining_periods`; paired with `charges_made` so that
    /// `charges_made + charges_remaining == max_periods` for capped
    /// authorizations.
    pub fn charges_remaining(env: Env) -> Result<Option<u32>, Error> {
        Ok(storage::load_view(&env)?.remaining_periods())
    }

    /// How many charges have been taken so far.
    pub fn charges_made(env: Env) -> Result<u32, Error> {
        Ok(storage::load_view(&env)?.periods_charged)
    }

    /// Cumulative amount charged so far, in the token's smallest unit.
    ///
    /// Equals the sum of every per-charge amount, including charges made after
    /// an `update_amount`.
    pub fn total_charged(env: Env) -> Result<i128, Error> {
        Ok(storage::load_view(&env)?.total_charged)
    }

    /// Extends the instance TTL for a long-lived authorization without changing it.
    pub fn bump(env: Env) -> Result<(), Error> {
        storage::load_view(&env)?;
        sororail_common::storage::extend_instance(&env);
        Ok(())
    }

    /// The full authorization record.
    pub fn get(env: Env) -> Result<Authorization, Error> {
        storage::load_view(&env)
    }
}
