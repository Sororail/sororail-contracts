// Test fixtures do plain arithmetic on known-small constants; the checked-math
// rule is for contract code.
#![allow(clippy::arithmetic_side_effects)]

use soroban_sdk::{
    map,
    testutils::{
        storage::Instance as _, Address as _, Events as _, Ledger as _, MockAuth, MockAuthInvoke,
    },
    token::{StellarAssetClient, TokenClient},
    vec,
    xdr::{ScErrorCode, ScErrorType},
    Address, Env, Event, IntoVal, Symbol, Val,
};
use sororail_common::{storage::INSTANCE_BUMP, testutils::TestEnv, Error};

use crate::{
    contract::{StreamContract, StreamContractClient},
    events::{Cancelled, Created, Extended, ToppedUp, Withdrawn},
    types::Stream,
};

mod accrual;
mod cancel;
mod create;
mod events;
mod extend;
mod lifecycle;
mod properties;
mod top_up;
mod views;
mod withdraw;

const RATE: i128 = 100;
const START: u64 = 1_000_000;
const DURATION: u64 = 1_000;
const STOP: u64 = START + DURATION;
const DEPOSITED: i128 = RATE * DURATION as i128;
const MINT: i128 = DEPOSITED * 10;

struct Fixture<'a> {
    env: Env,
    client: StreamContractClient<'a>,
    token: TokenClient<'a>,
    sender: Address,
    recipient: Address,
    outsider: Address,
}

impl Fixture<'_> {
    fn new(cancellable: bool) -> Self {
        let te = TestEnv::at(START);
        let (token, sender) = te.make_token(MINT);
        let token_address = token.address.clone();
        let recipient = te.make_address();
        let outsider = te.make_address();
        let env = te.env().clone();

        let client = StreamContractClient::new(&env, &env.register(StreamContract, ()));
        client.create(
            &sender,
            &recipient,
            &token_address,
            &RATE,
            &START,
            &STOP,
            &cancellable,
        );

        Fixture {
            token: TokenClient::new(&env, &token_address),
            env,
            client,
            sender,
            recipient,
            outsider,
        }
    }

    fn at(&self, ts: u64) {
        self.env.ledger().with_mut(|l| l.timestamp = ts);
    }

    fn held(&self) -> i128 {
        self.token.balance(&self.client.address)
    }

    /// The property the whole contract exists to preserve.
    fn assert_conserved(&self) {
        let s = self.client.get();
        assert_eq!(
            s.withdrawn + s.refunded + self.client.remaining(),
            s.deposited,
            "conservation violated: {s:?}"
        );
        assert_eq!(
            self.held(),
            self.client.remaining(),
            "held balance diverged from remaining: {s:?}"
        );
    }
}

/// A bare struct for testing accrual in isolation, with no contract or ledger.
fn bare(env: &Env) -> Stream {
    Stream {
        sender: Address::generate(env),
        recipient: Address::generate(env),
        token: Address::generate(env),
        rate_per_second: RATE,
        start: START,
        stop: STOP,
        cancellable: true,
        deposited: DEPOSITED,
        withdrawn: 0,
        refunded: 0,
        cancelled_at: None,
    }
}
