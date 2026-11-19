use super::*;

// ---------------------------------------------------------------- property

mod accrued_at_properties {
    use proptest::prelude::*;

    use super::*;

    fn arb_stream(env: &Env) -> Stream {
        Stream {
            sender: Address::generate(env),
            recipient: Address::generate(env),
            token: Address::generate(env),
            rate_per_second: 0,
            start: 0,
            stop: 1,
            cancellable: false,
            deposited: 0,
            withdrawn: 0,
            refunded: 0,
            cancelled_at: None,
        }
    }

    proptest! {
        /// `accrued_at` can never exceed `deposited`, no matter how far past
        /// `stop` -- or how large a rate -- it is asked about. The stream is
        /// only ever funded for `rate_per_second * (stop - start)`, so
        /// accrual cannot pay out more than was ever put in.
        #[test]
        fn accrued_at_never_exceeds_deposit(
            rate_per_second in 0i128..=1_000_000_000,
            start in 0u64..=1_000_000_000,
            span in 1u64..=10_000_000,
            at_offset in 0u64..=20_000_000,
        ) {
            let env = Env::default();
            let stop = start + span;
            let deposited = rate_per_second * i128::from(span);

            let stream = Stream {
                rate_per_second,
                start,
                stop,
                deposited,
                ..arb_stream(&env)
            };

            let at = start.saturating_add(at_offset);
            let accrued = stream
                .accrued_at(at)
                .expect("accrued_at must not error for these bounded inputs");

            prop_assert!(
                accrued <= deposited,
                "accrued_at({at}) = {accrued} exceeds deposited = {deposited} (rate={rate_per_second}, start={start}, stop={stop})"
            );
            prop_assert!(accrued >= 0, "accrued_at({at}) = {accrued} is negative");
        }

        /// Cancellation freezes accrual at the cancellation timestamp, same
        /// as `stop` would -- and the bound against `deposited` still holds.
        #[test]
        fn accrued_at_respects_cancellation_and_the_deposit_bound(
            rate_per_second in 0i128..=1_000_000_000,
            start in 0u64..=1_000_000_000,
            span in 1u64..=10_000_000,
            cancel_offset in 0u64..=10_000_000,
            at_offset in 0u64..=20_000_000,
        ) {
            let env = Env::default();
            let stop = start + span;
            let deposited = rate_per_second * i128::from(span);
            let cancelled_at = start.saturating_add(cancel_offset).min(stop);

            let stream = Stream {
                rate_per_second,
                start,
                stop,
                deposited,
                cancelled_at: Some(cancelled_at),
                ..arb_stream(&env)
            };

            let at = start.saturating_add(at_offset);
            let accrued = stream
                .accrued_at(at)
                .expect("accrued_at must not error for these bounded inputs");

            prop_assert!(accrued <= deposited);

            let at_cancellation = stream
                .accrued_at(cancelled_at)
                .expect("accrued_at(cancelled_at) must not error");
            let long_after = stream
                .accrued_at(cancelled_at.saturating_add(at_offset))
                .expect("accrued_at(long after cancellation) must not error");
            prop_assert_eq!(at_cancellation, long_after);
        }
    }
}

mod conservation_properties {
    use proptest::prelude::*;

    use super::*;

    /// One step of a random valid-or-invalid call sequence. Invalid calls are
    /// included on purpose: a rejected call must roll back and leave the
    /// conservation invariant untouched just as a valid one does.
    #[derive(Clone, Copy, Debug)]
    enum Op {
        Advance(u64),
        WithdrawAll,
        WithdrawSome(i128),
        TopUp(i128),
        Extend(u64),
        Cancel,
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        prop_oneof![
            (1u64..=2_000).prop_map(Op::Advance),
            Just(Op::WithdrawAll),
            (-5i128..=500).prop_map(Op::WithdrawSome),
            (0i128..=500).prop_map(Op::TopUp),
            (0u64..=2_000).prop_map(Op::Extend),
            Just(Op::Cancel),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// The stream's conservation invariant
        /// `withdrawn + refunded + remaining == deposited` holds after every
        /// call of an arbitrary sequence of withdraw / cancel / top_up /
        /// extend calls, and the contract's real token balance always equals
        /// `remaining`.
        #[test]
        fn conservation_holds_after_any_call_sequence(
            rate in 1i128..=1_000,
            span in 1u64..=1_000,
            ops in proptest::collection::vec(op_strategy(), 1..=24),
        ) {
            let start = 1_000_000u64;
            let stop = start + span;
            let te = TestEnv::at(start);
            let (token, sender) = te.make_token(1_000_000_000_000_000);
            let token_address = token.address.clone();
            let recipient = te.make_address();
            let env = te.env().clone();

            let client = StreamContractClient::new(&env, &env.register(StreamContract, ()));
            client.create(
                &sender,
                &recipient,
                &token_address,
                &rate,
                &start,
                &stop,
                &true,
            );

            for op in ops {
                match op {
                    Op::Advance(delta) => {
                        te.advance_to(te.timestamp().saturating_add(delta));
                    }
                    Op::WithdrawAll => {
                        let _ = client.try_withdraw(&None);
                    }
                    Op::WithdrawSome(amount) => {
                        let _ = client.try_withdraw(&Some(amount));
                    }
                    Op::TopUp(periods) => {
                        let _ = client.try_top_up(&periods.saturating_mul(rate));
                    }
                    Op::Extend(extra) => {
                        let current_stop = client.get().stop;
                        let _ = client.try_extend(&current_stop.saturating_add(extra));
                    }
                    Op::Cancel => {
                        let _ = client.try_cancel();
                    }
                }

                let s = client.get();
                let remaining = client.remaining();
                prop_assert!(
                    s.withdrawn >= 0 && s.refunded >= 0 && remaining >= 0,
                    "negative component after {:?}: {:?}", op, s
                );
                prop_assert_eq!(
                    s.withdrawn + s.refunded + remaining,
                    s.deposited,
                    "conservation violated after {:?}: {:?}", op, s
                );
                prop_assert_eq!(
                    token.balance(&client.address),
                    remaining,
                    "held balance diverged after {:?}: {:?}", op, s
                );
            }
        }
    }
}
