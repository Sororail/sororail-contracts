use soroban_sdk::{
    testutils::{storage::Instance as _, Address as _, Events as _, Ledger as _},
    token::{StellarAssetClient, TokenClient},
    Address, Env, Event,
};
use sororail_common::{storage::INSTANCE_BUMP, testutils::TestEnv, Error};

use crate::{
    contract::{EscrowContract, EscrowContractClient},
    events::{Cancelled, Created, Disputed, Funded, Refunded, Released, Resolved},
    types::State,
};

mod budget;
mod cancel;
mod dispute;
mod fee_on_transfer;
mod fund;
mod init;
mod refund;
mod release;
mod resolve;

const AMOUNT: i128 = 1_000_000;
const START_TS: u64 = 1_000_000;
const DEADLINE: u64 = START_TS + 10_000;

struct Fixture<'a> {
    env: Env,
    client: EscrowContractClient<'a>,
    token: TokenClient<'a>,
    depositor: Address,
    beneficiary: Address,
    arbiter: Address,
    outsider: Address,
}

impl Fixture<'_> {
    /// Builds an initialized escrow. `with_arbiter` controls whether the
    /// dispute path is available.
    fn new(with_arbiter: bool) -> Self {
        Self::with_amount(with_arbiter, AMOUNT)
    }

    fn with_amount(with_arbiter: bool, amount: i128) -> Self {
        let te = TestEnv::at(START_TS);
        let (token, depositor) = te.make_token(
            amount
                .checked_mul(10)
                .expect("test token mint amount remains in range"),
        );
        let token_address = token.address.clone();
        let beneficiary = te.make_address();
        let arbiter = te.make_address();
        let outsider = te.make_address();
        let env = te.env().clone();

        let contract_id = env.register(EscrowContract, ());
        let client = EscrowContractClient::new(&env, &contract_id);

        client.init(
            &depositor,
            &beneficiary,
            &if with_arbiter {
                Some(arbiter.clone())
            } else {
                None
            },
            &token_address,
            &amount,
            &DEADLINE,
        );

        Fixture {
            token: TokenClient::new(&env, &token_address),
            env,
            client,
            depositor,
            beneficiary,
            arbiter,
            outsider,
        }
    }

    fn funded(with_arbiter: bool) -> Self {
        let f = Fixture::new(with_arbiter);
        f.client.fund();
        f
    }

    fn funded_amount(amount: i128) -> Self {
        let f = Fixture::with_amount(true, amount);
        f.client.fund();
        f
    }

    fn escrow_balance(&self) -> i128 {
        self.token.balance(&self.client.address)
    }

    fn advance_past_deadline(&self) {
        self.env.ledger().with_mut(|l| l.timestamp = DEADLINE + 1);
    }
}
