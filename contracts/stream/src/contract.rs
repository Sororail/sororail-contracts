use soroban_sdk::{contract, contractimpl, token, Address, Env};
use sororail_common::{auth, math, Error};

use crate::{events, storage, types::Stream};

#[contract]
pub struct StreamContract;

#[contractimpl]
impl StreamContract {
    /// Creates and fully funds a stream.
    ///
    /// The whole span is funded up front: `rate_per_second * (stop - start)`
    /// is pulled from the sender immediately. A stream is never partially
    /// funded, so the recipient can rely on what it promises.
    ///
    /// `start` may be in the past, which backdates accrual -- the sender is
    /// choosing to make funds immediately withdrawable.
    ///
    /// `sender` and `recipient` must differ; a self-stream is rejected.
    ///
    /// # Why positional arguments (not a `CreateStreamParams` struct)
    ///
    /// Soroban does support a single `#[contracttype]` struct parameter, and
    /// that would silence this lint. Positional arguments were kept anyway so
    /// the on-chain ABI stays a flat argument list — the same shape the
    /// TypeScript SDK's uniform build → simulate → sign → send → confirm
    /// surface (SPEC.md) will bind without introducing a nested params object
    /// per entry point. Revisit only if the SDK deliberately adopts struct
    /// params across every contract.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        env: Env,
        sender: Address,
        recipient: Address,
        token: Address,
        rate_per_second: i128,
        start: u64,
        stop: u64,
        cancellable: bool,
    ) -> Result<(), Error> {
        if storage::is_initialized(&env) {
            return Err(Error::AlreadyInitialized);
        }
        auth::require_distinct(&sender, &recipient)?;
        math::require_positive(rate_per_second)?;
        if stop <= start {
            return Err(Error::InvalidTimeRange);
        }
        let deposited = math::mul(
            rate_per_second,
            math::sub(i128::from(stop), i128::from(start))?,
        )?;

        sender.require_auth();

        let stream = Stream {
            sender: sender.clone(),
            recipient: recipient.clone(),
            token: token.clone(),
            rate_per_second,
            start,
            stop,
            cancellable,
            deposited,
            withdrawn: 0,
            refunded: 0,
            cancelled_at: None,
        };
        storage::save(&env, &stream);

        Self::pull(&env, &stream, deposited);
        events::Created {
            sender,
            recipient,
            token,
            rate_per_second,
            start,
            stop,
            cancellable,
            deposited,
        }
        .publish(&env);
        Ok(())
    }

    /// Withdraws accrued funds to the recipient.
    ///
    /// `None` withdraws everything currently available. Returns the amount
    /// actually withdrawn.
    pub fn withdraw(env: Env, amount: Option<i128>) -> Result<i128, Error> {
        let stream = storage::load(&env)?;
        let recipient = stream.recipient.clone();
        Self::withdraw_into(&env, stream, &recipient, amount)
    }

    /// Withdraws accrued funds like `withdraw`, but pays them to
    /// `destination` (for example a treasury or an exchange deposit address)
    /// in the same transaction.
    ///
    /// Requires the recipient's authorization, exactly like `withdraw`: the
    /// recipient owns the accrued funds and chooses where they go. Accrual,
    /// the `withdrawn` total and every check are shared with `withdraw`, so
    /// the stream's accounting is identical whichever entry point is used.
    /// `destination` may be the recipient itself. It may not be this contract
    /// ([`Error::IdenticalParties`]): paying the contract would count funds
    /// as withdrawn while they stayed locked in the stream. `None` withdraws
    /// everything currently available. Returns the amount withdrawn.
    pub fn withdraw_to(
        env: Env,
        destination: Address,
        amount: Option<i128>,
    ) -> Result<i128, Error> {
        auth::require_distinct(&destination, &env.current_contract_address())?;
        let stream = storage::load(&env)?;
        Self::withdraw_into(&env, stream, &destination, amount)
    }

    /// Cancels the stream: settles what has accrued to the recipient and
    /// returns the rest to the sender. Sender only, and only if the stream was
    /// created `cancellable`.
    ///
    /// Settlement is immediate and complete, so nothing is left for the
    /// recipient to withdraw afterwards.
    pub fn cancel(env: Env) -> Result<(), Error> {
        let mut stream = storage::load(&env)?;
        if stream.is_cancelled() {
            return Err(Error::StreamCancelled);
        }
        auth::require(stream.cancellable, Error::StreamNotCancellable)?;
        stream.sender.require_auth();

        let now = env.ledger().timestamp();
        let settled = stream.available_at(now)?;
        stream.withdrawn = math::add(stream.withdrawn, settled)?;
        let refund = math::sub(stream.deposited, stream.withdrawn)?;
        stream.refunded = refund;
        stream.cancelled_at = Some(now);
        storage::save(&env, &stream);

        if settled > 0 {
            Self::pay(&env, &stream, &stream.recipient, settled);
        }
        if refund > 0 {
            Self::pay(&env, &stream, &stream.sender, refund);
        }
        events::Cancelled {
            sender: stream.sender.clone(),
            settled_to_recipient: settled,
            refunded_to_sender: refund,
            cancelled_at: now,
        }
        .publish(&env);
        Ok(())
    }

    /// Adds funds, extending `stop` by the span they buy.
    ///
    /// `amount` must be an exact multiple of `rate_per_second`. A partial
    /// second cannot be represented without breaking the funding invariant
    /// (`deposited == rate * (stop - start)`), and silently keeping the dust
    /// would strand it, so an inexact amount is rejected instead.
    pub fn top_up(env: Env, amount: i128) -> Result<(), Error> {
        let mut stream = storage::load(&env)?;
        if stream.is_cancelled() {
            return Err(Error::StreamCancelled);
        }
        stream.sender.require_auth();
        Self::extend_by_funding(&mut stream, amount)?;
        storage::save(&env, &stream);

        Self::pull(&env, &stream, amount);
        events::ToppedUp {
            sender: stream.sender.clone(),
            amount,
            new_stop: stream.stop,
            deposited: stream.deposited,
        }
        .publish(&env);
        Ok(())
    }

    /// Adds funds on behalf of a third party, who authorizes the transfer.
    ///
    /// Same accounting as [`StreamContract::top_up`]: `amount` must be an exact
    /// multiple of `rate_per_second`, `stop` moves later by the seconds it buys,
    /// and `deposited` grows by `amount`.
    ///
    /// Any address may fund a stream, so `funder` has no membership to check.
    /// It only has to authorize the transfer of its own tokens.
    ///
    /// Funding confers no rights. Cancellation still refunds the sender, and the
    /// funder cannot withdraw, cancel or reclaim anything afterwards. See the
    /// "Third-party top-ups" section of SECURITY.md.
    pub fn top_up_from(env: Env, funder: Address, amount: i128) -> Result<(), Error> {
        let mut stream = storage::load(&env)?;
        if stream.is_cancelled() {
            return Err(Error::StreamCancelled);
        }
        funder.require_auth();
        Self::extend_by_funding(&mut stream, amount)?;
        storage::save(&env, &stream);

        Self::pull_from(&env, &stream, &funder, amount);
        events::ToppedUpFrom {
            funder,
            sender: stream.sender.clone(),
            amount,
            new_stop: stream.stop,
            deposited: stream.deposited,
        }
        .publish(&env);
        Ok(())
    }

    /// Extends `stop`, pulling the additional funds the longer span requires.
    pub fn extend(env: Env, new_stop: u64) -> Result<(), Error> {
        let mut stream = storage::load(&env)?;
        if stream.is_cancelled() {
            return Err(Error::StreamCancelled);
        }
        stream.sender.require_auth();
        if new_stop <= stream.stop {
            return Err(Error::StreamNotExtendable);
        }

        let old_stop = stream.stop;
        let added = math::mul(
            stream.rate_per_second,
            math::sub(i128::from(new_stop), i128::from(old_stop))?,
        )?;
        stream.stop = new_stop;
        stream.deposited = math::add(stream.deposited, added)?;
        storage::save(&env, &stream);

        Self::pull(&env, &stream, added);
        events::Extended {
            sender: stream.sender.clone(),
            added,
            old_stop,
            new_stop,
            deposited: stream.deposited,
        }
        .publish(&env);
        Ok(())
    }

    /// Accrued but unwithdrawn for the recipient; refundable remainder for the
    /// sender; zero for anyone else.
    pub fn balance_of(env: Env, who: Address) -> Result<i128, Error> {
        let stream = storage::load_view(&env)?;
        let now = env.ledger().timestamp();

        if who == stream.recipient {
            return stream.available_at(now);
        }
        if who == stream.sender {
            let unaccrued = math::sub(stream.deposited, stream.accrued_at(now)?)?;
            return math::sub(unaccrued, stream.refunded);
        }
        Ok(0)
    }

    /// What the contract still holds for this stream.
    pub fn remaining(env: Env) -> Result<i128, Error> {
        storage::load_view(&env)?.remaining()
    }

    /// Whether accrual has reached `stop` as of the ledger time. Cancellation is
    /// not considered here; use `get` and check `cancelled_at` for that.
    pub fn has_ended(env: Env) -> Result<bool, Error> {
        let stream = storage::load_view(&env)?;
        Ok(stream.has_ended(env.ledger().timestamp()))
    }

    /// Extends the instance TTL for a long-lived stream without changing it.
    pub fn bump(env: Env) -> Result<(), Error> {
        storage::load_view(&env)?;
        sororail_common::storage::extend_instance(&env);
        Ok(())
    }

    /// The full stream record.
    pub fn get(env: Env) -> Result<Stream, Error> {
        storage::load_view(&env)
    }

    /// Transfers the stream to a new recipient, allowing the stream to be handed
    /// to a different address. Only the current recipient may do this.
    ///
    /// The new recipient must be different from the current recipient, but can
    /// be the same as the sender. This allows the stream's funds to revert to
    /// the sender by explicit handoff (not just by cancellation).
    ///
    /// Any accrued but unwithdrawn amount remains the property of the current
    /// (original) recipient and must be withdrawn before the transfer.
    pub fn transfer_recipient(env: Env, new_recipient: Address) -> Result<(), Error> {
        let mut stream = storage::load(&env)?;
        if stream.is_cancelled() {
            return Err(Error::StreamCancelled);
        }
        stream.recipient.require_auth();

        auth::require_distinct(&new_recipient, &stream.recipient)?;

        let old_recipient = stream.recipient.clone();
        stream.recipient = new_recipient.clone();
        storage::save(&env, &stream);

        events::RecipientTransferred {
            from: old_recipient,
            to: new_recipient,
        }
        .publish(&env);
        Ok(())
    }
}

// Helpers. Outside `#[contractimpl]` so they are not contract entry points.
impl StreamContract {
    /// The withdrawal shared by `withdraw` and `withdraw_to`: requires the
    /// recipient's authorization, validates `amount` against what has
    /// accrued, records it in `withdrawn`, pays `destination`, and emits
    /// `Withdrawn` naming both the recipient and the destination.
    fn withdraw_into(
        env: &Env,
        mut stream: Stream,
        destination: &Address,
        amount: Option<i128>,
    ) -> Result<i128, Error> {
        stream.recipient.require_auth();

        let available = stream.available_at(env.ledger().timestamp())?;
        let requested = amount.unwrap_or(available);
        math::require_positive(requested)?;
        if requested > available {
            return Err(Error::StreamInsufficientAccrued);
        }

        stream.withdrawn = math::add(stream.withdrawn, requested)?;
        storage::save(env, &stream);

        Self::pay(env, &stream, destination, requested);
        events::Withdrawn {
            recipient: stream.recipient.clone(),
            amount: requested,
            total_withdrawn: stream.withdrawn,
            destination: destination.clone(),
        }
        .publish(env);
        Ok(requested)
    }

    /// The funding arithmetic shared by `top_up` and `top_up_from`.
    ///
    /// Checks the amount, then moves `stop` later by the seconds it buys and
    /// adds it to `deposited`. Operates on the in-memory record only, so an
    /// error leaves storage untouched.
    fn extend_by_funding(stream: &mut Stream, amount: i128) -> Result<(), Error> {
        math::require_positive(amount)?;
        if amount.checked_rem(stream.rate_per_second) != Some(0) {
            return Err(Error::InvalidAmount);
        }

        let seconds = u64::try_from(math::div(amount, stream.rate_per_second)?)
            .map_err(|_| Error::InvalidTimeRange)?;
        stream.stop = stream
            .stop
            .checked_add(seconds)
            .ok_or(Error::InvalidTimeRange)?;
        stream.deposited = math::add(stream.deposited, amount)?;
        Ok(())
    }

    /// Pulls `amount` of the streamed token from the sender into this contract.
    fn pull(env: &Env, stream: &Stream, amount: i128) {
        Self::pull_from(env, stream, &stream.sender, amount);
    }

    /// Pulls `amount` of the streamed token from `from` into this contract.
    fn pull_from(env: &Env, stream: &Stream, from: &Address, amount: i128) {
        debug_assert!(amount > 0, "pull requires a positive amount");
        token::TokenClient::new(env, &stream.token).transfer(
            from,
            env.current_contract_address(),
            &amount,
        );
    }

    /// Sends `amount` of the streamed token from this contract to `to`.
    ///
    /// Always called after the state change has been persisted.
    fn pay(env: &Env, stream: &Stream, to: &Address, amount: i128) {
        debug_assert!(amount > 0, "pay requires a positive amount");
        token::TokenClient::new(env, &stream.token).transfer(
            &env.current_contract_address(),
            to,
            &amount,
        );
    }
}
