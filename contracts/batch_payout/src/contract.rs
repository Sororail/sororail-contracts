use soroban_sdk::{contract, contractimpl, token, xdr::ToXdr, Address, BytesN, Env, Vec};
use sororail_common::{auth, math, Error};

use crate::{
    events,
    types::{EqualSplitReceipt, MultiTokenPayment, Payment, Receipt, MAX_RECIPIENTS},
};

#[contract]
pub struct BatchPayoutContract;

#[contractimpl]
impl BatchPayoutContract {
    /// Pays each recipient their own amount, in one transaction.
    ///
    /// All-or-nothing: if any transfer fails, the host reverts the entire
    /// invocation including the transfers already made. Nobody is paid unless
    /// everybody is.
    ///
    /// The whole batch is validated and totalled **before** any money moves,
    /// so an invalid line is rejected without a partial payout ever being
    /// attempted.
    ///
    /// Paying the funder's own address is intentionally permitted. Such a
    /// line counts toward the receipt total but transfers no net funds;
    /// only transfers to this contract's own address are rejected.
    ///
    /// When `only_eoa` is `true`, every recipient must be a Stellar account
    /// address (not a smart contract). `BatchRecipientIsContract` is returned
    /// for the first recipient that fails that check.
    ///
    /// `memo` is an opaque per-batch reference (for example a pay-run id) that
    /// is echoed in the [`Receipt`] and the `Executed` event. `None` behaves
    /// exactly as before the memo existed.
    pub fn execute(
        env: Env,
        funder: Address,
        token: Address,
        recipients: Vec<Payment>,
        only_eoa: bool,
        memo: Option<BytesN<32>>,
    ) -> Result<Receipt, Error> {
        let count = Self::check_size(&recipients)?;
        let contract_address = env.current_contract_address();

        let mut total: i128 = 0;
        for payment in recipients.iter() {
            auth::require_distinct(&payment.to, &contract_address)?;
            if only_eoa && is_contract_addr(&env, &payment.to) {
                return Err(Error::BatchRecipientIsContract);
            }
            math::require_positive(payment.amount)?;
            total = math::add(total, payment.amount)?;
        }

        funder.require_auth();

        let client = token::TokenClient::new(&env, &token);
        for payment in recipients.iter() {
            client.transfer(&funder, &payment.to, &payment.amount);
        }

        Ok(Self::publish_executed(
            &env, funder, token, count, total, memo,
        ))
    }

    /// Pays every recipient the same amount, in one transaction.
    ///
    /// The common payroll shape, and cheaper to submit than `execute`
    /// because the amount crosses the wire once rather than once per line.
    /// The funder may appear among recipients, including more than once;
    /// those self-payments count in the receipt but have no net balance effect.
    ///
    /// # Dust
    ///
    /// There is none. `execute_equal` never divides: every recipient receives
    /// exactly `amount_each` and the total moved is exactly
    /// `amount_each * count`. To split a fixed *total* evenly instead, use
    /// [`Self::execute_equal_split`], whose remainder goes to the last
    /// recipient (see [`EqualSplitReceipt`]).
    ///
    /// When `only_eoa` is `true`, every recipient must be a Stellar account
    /// address (not a smart contract). `BatchRecipientIsContract` is returned
    /// for the first recipient that fails that check.
    ///
    /// `memo` is echoed in the [`Receipt`] and the `Executed` event; see
    /// [`Self::execute`].
    pub fn execute_equal(
        env: Env,
        funder: Address,
        token: Address,
        recipients: Vec<Address>,
        amount_each: i128,
        only_eoa: bool,
        memo: Option<BytesN<32>>,
    ) -> Result<Receipt, Error> {
        math::require_positive(amount_each)?;

        let count = Self::check_size_of(recipients.len())?;
        Self::check_recipients(&env, &recipients, only_eoa)?;

        let total = math::mul(amount_each, i128::from(count))?;

        funder.require_auth();

        let client = token::TokenClient::new(&env, &token);
        for to in recipients.iter() {
            client.transfer(&funder, &to, &amount_each);
        }

        Ok(Self::publish_executed(
            &env, funder, token, count, total, memo,
        ))
    }

    /// Splits `total` evenly across `recipients`, in one transaction.
    ///
    /// # The dust rule
    ///
    /// Each recipient receives `total / count` (truncating); the remainder
    /// `total % count` -- the *dust*, always smaller than `count` -- is paid
    /// on top to the **last** recipient in the list. The payouts therefore
    /// always sum to exactly `total`. When `total < count`, every recipient
    /// but the last receives `0` (no transfer is made for a zero line) and the
    /// last receives all of `total`. The returned [`EqualSplitReceipt`]
    /// reports `amount_each` and `dust` so the split is auditable.
    ///
    /// Validation is the same as `execute_equal` (`total` must be positive),
    /// and is shared with [`Self::preview_equal`], so a successful preview
    /// returns exactly the receipt this call will.
    ///
    /// All-or-nothing, like every batch: a failed transfer reverts them all.
    pub fn execute_equal_split(
        env: Env,
        funder: Address,
        token: Address,
        recipients: Vec<Address>,
        total: i128,
        only_eoa: bool,
        memo: Option<BytesN<32>>,
    ) -> Result<EqualSplitReceipt, Error> {
        let split = Self::equal_split(&env, &recipients, total, only_eoa)?;

        funder.require_auth();

        let client = token::TokenClient::new(&env, &token);
        let last = split.count.saturating_sub(1);
        for (i, to) in (0u32..).zip(recipients.iter()) {
            let amount = if i == last {
                math::add(split.amount_each, split.dust)?
            } else {
                split.amount_each
            };
            if amount > 0 {
                client.transfer(&funder, &to, &amount);
            }
        }

        Self::publish_executed(&env, funder, token, split.count, split.total, memo.clone());
        Ok(EqualSplitReceipt { memo, ..split })
    }

    /// Computes an equal split of `total` without moving anything.
    ///
    /// The dry-run counterpart of [`Self::execute_equal_split`]: same
    /// validation, same calculation (one shared helper), same
    /// [`EqualSplitReceipt`] shape including `amount_each` and `dust`. `memo`
    /// is always `None` because a preview takes none. No event is emitted.
    pub fn preview_equal(
        env: Env,
        recipients: Vec<Address>,
        total: i128,
        only_eoa: bool,
    ) -> Result<EqualSplitReceipt, Error> {
        Self::equal_split(&env, &recipients, total, only_eoa)
    }

    /// Splits `total` across recipients by basis-point `shares`, in one
    /// transaction. The revenue-split shape: percentages rather than amounts.
    ///
    /// `shares` pairs each recipient with its share in basis points
    /// (`10_000` = 100%). The shares must sum to exactly `10_000`, otherwise
    /// the call fails with `InvalidBasisPoints` before anything moves. A zero
    /// share is permitted; it pays nothing.
    ///
    /// # The dust rule
    ///
    /// Every recipient except the last receives `math::mul_bps(total, bps)`,
    /// which truncates toward zero. The **last** recipient in `shares`
    /// receives `total` minus everything paid to the others, so it absorbs
    /// all truncation dust (at most `count - 1` units) and the payouts always
    /// sum to exactly `total`. List the recipient who should absorb rounding
    /// last. A line whose payout is `0` makes no transfer but still counts.
    ///
    /// Validation order: `total` positive (`InvalidAmount`), batch size
    /// (`BatchEmpty` / `BatchTooLarge`), each recipient (`IdenticalParties`,
    /// then `BatchRecipientIsContract` when `only_eoa`), then the share sum
    /// (`InvalidBasisPoints`).
    ///
    /// `memo` is echoed in the [`Receipt`] and the `Executed` event; see
    /// [`Self::execute`].
    pub fn execute_weighted(
        env: Env,
        funder: Address,
        token: Address,
        total: i128,
        shares: Vec<(Address, u32)>,
        only_eoa: bool,
        memo: Option<BytesN<32>>,
    ) -> Result<Receipt, Error> {
        math::require_positive(total)?;
        let count = Self::check_size_of(shares.len())?;
        let contract_address = env.current_contract_address();

        let mut bps_sum: u32 = 0;
        for (to, bps) in shares.iter() {
            auth::require_distinct(&to, &contract_address)?;
            if only_eoa && is_contract_addr(&env, &to) {
                return Err(Error::BatchRecipientIsContract);
            }
            bps_sum = bps_sum.checked_add(bps).ok_or(Error::InvalidBasisPoints)?;
        }
        if bps_sum != math::MAX_BPS {
            return Err(Error::InvalidBasisPoints);
        }

        // Compute every payout before any money moves.
        let last = count.saturating_sub(1);
        let mut payouts: Vec<i128> = Vec::new(&env);
        let mut paid: i128 = 0;
        for (i, (_, bps)) in (0u32..).zip(shares.iter()) {
            let amount = if i == last {
                math::sub(total, paid)?
            } else {
                math::mul_bps(total, bps)?
            };
            paid = math::add(paid, amount)?;
            payouts.push_back(amount);
        }

        funder.require_auth();

        let client = token::TokenClient::new(&env, &token);
        for ((to, _), amount) in shares.iter().zip(payouts.iter()) {
            if amount > 0 {
                client.transfer(&funder, &to, &amount);
            }
        }

        Ok(Self::publish_executed(
            &env, funder, token, count, total, memo,
        ))
    }

    /// Pays each recipient with its own token and amount, in one transaction.
    ///
    /// A multi-token batch: each line specifies which token to draw from,
    /// the recipient, and the amount. The funder must hold sufficient balance
    /// of every token used. All transfers are all-or-nothing: a failure on any
    /// line reverts the entire invocation.
    ///
    /// All validation happens before any money moves, so an invalid line is
    /// rejected without a partial payout ever being attempted.
    ///
    /// Because the payments span different tokens, the receipt `total` is the
    /// arithmetic sum of all raw amounts regardless of token denominations; it
    /// is provided for reference only and should not be compared across tokens.
    ///
    /// When `only_eoa` is `true`, every recipient must be a Stellar account
    /// address. `BatchRecipientIsContract` is returned for the first recipient
    /// that fails that check.
    pub fn execute_many_tokens(
        env: Env,
        funder: Address,
        payments: Vec<MultiTokenPayment>,
        only_eoa: bool,
    ) -> Result<Receipt, Error> {
        let count = Self::check_size_multi(&payments)?;
        let contract_address = env.current_contract_address();

        let mut total: i128 = 0;
        for payment in payments.iter() {
            auth::require_distinct(&payment.to, &contract_address)?;
            if only_eoa && is_contract_addr(&env, &payment.to) {
                return Err(Error::BatchRecipientIsContract);
            }
            math::require_positive(payment.amount)?;
            total = math::add(total, payment.amount)?;
        }

        funder.require_auth();

        for payment in payments.iter() {
            token::TokenClient::new(&env, &payment.token).transfer(
                &funder,
                &payment.to,
                &payment.amount,
            );
        }

        let receipt = Receipt {
            count,
            total,
            memo: None,
        };
        events::ExecutedMany { funder, count }.publish(&env);
        Ok(receipt)
    }

    /// The maximum recipients permitted in one batch.
    ///
    /// Exposed so a client can split an oversized payroll before submitting
    /// rather than discovering the cap by having a transaction rejected.
    pub fn max_recipients(_env: Env) -> u32 {
        MAX_RECIPIENTS
    }

    /// Totals a batch without moving anything, for a pre-signature preview.
    ///
    /// Runs exactly the validation `execute` does, so a preview that
    /// succeeds means the batch itself will not be rejected for size, an
    /// invalid amount, an overflowing total, or (when `only_eoa` is `true`)
    /// a contract-address recipient.
    pub fn preview(env: Env, recipients: Vec<Payment>, only_eoa: bool) -> Result<Receipt, Error> {
        let count = Self::check_size(&recipients)?;
        let contract_address = env.current_contract_address();
        let mut total: i128 = 0;
        for payment in recipients.iter() {
            auth::require_distinct(&payment.to, &contract_address)?;
            if only_eoa && is_contract_addr(&env, &payment.to) {
                return Err(Error::BatchRecipientIsContract);
            }
            math::require_positive(payment.amount)?;
            total = math::add(total, payment.amount)?;
        }
        Ok(Receipt {
            count,
            total,
            memo: None,
        })
    }
}

// Helpers. Outside `#[contractimpl]` so they are not contract entry points.
impl BatchPayoutContract {
    fn check_size(recipients: &Vec<Payment>) -> Result<u32, Error> {
        Self::check_size_of(recipients.len())
    }

    fn check_size_multi(payments: &Vec<MultiTokenPayment>) -> Result<u32, Error> {
        Self::check_size_of(payments.len())
    }

    /// Rejects this contract's own address and, when `only_eoa`, any contract
    /// address. Shared by `execute_equal`, `execute_equal_split` and
    /// `preview_equal`.
    fn check_recipients(env: &Env, recipients: &Vec<Address>, only_eoa: bool) -> Result<(), Error> {
        let contract_address = env.current_contract_address();
        for to in recipients.iter() {
            auth::require_distinct(&to, &contract_address)?;
            if only_eoa && is_contract_addr(env, &to) {
                return Err(Error::BatchRecipientIsContract);
            }
        }
        Ok(())
    }

    /// The one equal-split calculation behind `execute_equal_split` and
    /// `preview_equal`. See [`EqualSplitReceipt`] for the dust rule.
    fn equal_split(
        env: &Env,
        recipients: &Vec<Address>,
        total: i128,
        only_eoa: bool,
    ) -> Result<EqualSplitReceipt, Error> {
        math::require_positive(total)?;
        let count = Self::check_size_of(recipients.len())?;
        Self::check_recipients(env, recipients, only_eoa)?;

        let n = i128::from(count);
        let amount_each = math::div(total, n)?;
        let dust = math::sub(total, math::mul(amount_each, n)?)?;
        Ok(EqualSplitReceipt {
            count,
            total,
            amount_each,
            dust,
            memo: None,
        })
    }

    /// Emits `Executed` and builds the matching [`Receipt`].
    fn publish_executed(
        env: &Env,
        funder: Address,
        token: Address,
        count: u32,
        total: i128,
        memo: Option<BytesN<32>>,
    ) -> Receipt {
        events::Executed {
            funder,
            token,
            count,
            total,
            memo: memo.clone(),
        }
        .publish(env);
        Receipt { count, total, memo }
    }

    fn check_size_of(len: u32) -> Result<u32, Error> {
        if len == 0 {
            return Err(Error::BatchEmpty);
        }
        if len > MAX_RECIPIENTS {
            return Err(Error::BatchTooLarge);
        }
        Ok(len)
    }
}

/// Returns `true` when `addr` is a smart-contract address.
///
/// `ScAddress` is XDR-encoded as a 4-byte big-endian discriminant followed by
/// the address payload. Discriminant 0 = Stellar account, 1 = contract.
fn is_contract_addr(env: &Env, addr: &Address) -> bool {
    let xdr = addr.clone().to_xdr(env);
    xdr.get(3).map_or(false, |b| b == 1)
}
