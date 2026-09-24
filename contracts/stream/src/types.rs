use soroban_sdk::{contracttype, Address};
use sororail_common::{math, Error};

/// A continuous per-second transfer. One per deployed contract instance.
///
/// # The funding invariant
///
/// `deposited` is always exactly `rate_per_second * (stop - start)`. Every
/// operation that changes `stop` moves `deposited` with it, and vice versa, so
/// a stream is always fully funded for its whole declared span. This is what
/// makes the conservation property checkable:
///
/// ```text
/// withdrawn + refunded + remaining == deposited
/// ```
///
/// where `remaining` is what the contract still holds for this stream.
///
/// # Accrual is never written
///
/// There is no per-second state. Accrual is computed from the ledger
/// timestamp at read time by [`Stream::accrued_at`], which is a pure function
/// of the struct and a timestamp, and so is testable in isolation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stream {
    /// Funds the stream and receives the remainder if it is cancelled.
    pub sender: Address,
    /// Accrues the funds and may withdraw them.
    pub recipient: Address,
    /// The token being streamed.
    pub token: Address,
    /// Tokens accrued per second, in the token's smallest unit.
    pub rate_per_second: i128,
    /// Ledger timestamp at which accrual begins.
    pub start: u64,
    /// Ledger timestamp at which accrual ends.
    pub stop: u64,
    /// Whether the sender may cancel before `stop`.
    pub cancellable: bool,
    /// Total ever pulled from the sender. Always `rate * (stop - start)`.
    pub deposited: i128,
    /// Total ever paid out to the recipient.
    pub withdrawn: i128,
    /// Total ever returned to the sender on cancellation.
    pub refunded: i128,
    /// When the stream was cancelled, if it was. Accrual stops here.
    pub cancelled_at: Option<u64>,
}

impl Stream {
    /// Total accrued to the recipient as of `at`, whether withdrawn or not.
    ///
    /// Pure: no storage, no environment. Clamped at both ends -- nothing
    /// accrues before `start`, and accrual halts at `stop` or at the
    /// cancellation timestamp, whichever comes first. The result can never
    /// exceed `deposited`.
    pub fn accrued_at(&self, at: u64) -> Result<i128, Error> {
        let mut effective = at;
        if let Some(cancelled) = self.cancelled_at {
            effective = effective.min(cancelled);
        }
        effective = effective.min(self.stop);

        if effective <= self.start {
            return Ok(0);
        }
        let elapsed = math::sub(effective as i128, self.start as i128)?;
        let accrued = math::mul(self.rate_per_second, elapsed)?;
        Ok(math::min(accrued, self.deposited))
    }

    /// Accrued but not yet withdrawn, as of `at`. What the recipient can take.
    pub fn available_at(&self, at: u64) -> Result<i128, Error> {
        math::sub(self.accrued_at(at)?, self.withdrawn)
    }

    /// What the contract still holds for this stream.
    pub fn remaining(&self) -> Result<i128, Error> {
        math::sub(math::sub(self.deposited, self.withdrawn)?, self.refunded)
    }

    /// Whether the stream has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled_at.is_some()
    }

    /// The declared span in seconds.
    pub fn duration(&self) -> u64 {
        self.stop.saturating_sub(self.start)
    }
}
