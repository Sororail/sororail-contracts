# Deployments

## Stellar testnet

Deployed 2026-09-07 from commit
[`0e55857`](https://github.com/Sororail/sororail-contracts/commit/0e558578f188c139e007a80914ec4707e265840f)
(`0e558578f188c139e007a80914ec4707e265840f`), built with soroban-sdk 27.0.6 and
the `stellar` CLI 27.1.0. That commit includes the `Payments` alias fix described
below; rebuild from this exact SHA to reproduce the wasm.

| Contract | Address |
|---|---|
| `escrow` | `CDXDUZIKHMAHEVINKAGR4RTCABOS3TBYW3BIQGMCE7MJEZEIUMV4TNW5` |
| `stream` | `CBEE4SRXRGCJDWXP6DDOSX6FR4S2PJ5KHUQCHI3ABY3SQTCHYSA7CGC7` |
| `vesting` | `CDUUUB5ECIBBLLYFT3P7PVFLUCXEUZPEEZEFXTPYA24W6YOBKS56T7CQ` |
| `recurring` | `CDYGZIYJCO4GTK26RGTVGIX2BR56A6LGJW7ZXLETZTLT2OTVMABKONZJ` |
| `batch_payout` | `CDKJ56S7K7QC4LG6SFF2OGDTG6N4QBCJOVRWHY7MKCWD5JPQ6MDAHRAM` |

```text
NETWORK_PASSPHRASE="Test SDF Network ; September 2015"
RPC_URL=https://soroban-testnet.stellar.org
DEPLOYER=GC4RWN3HH5H5GMD3NT4MOIYC3H3Q3NUF4BFWATGDI7NKRVLRURKHSIMC
```

The deployer is a **throwaway testnet key**, generated for this deployment and
funded by friendbot. Its secret is not in this repository and never will be.
Nothing about these contracts grants the deployer any privilege — none of them
has an admin or upgrade path — so the key matters only for redeploying.

> ### Unaudited. Testnet only.
>
> Do not deploy to mainnet or handle real value until an external audit is
> complete. No mainnet addresses will be published before then.

## These are single-use instances

Every contract except `batch_payout` holds **one position per deployed
instance** — one escrow, one stream, one grant, one subscription. The addresses
above are therefore *reference deployments for kicking the tyres*, not shared
infrastructure:

- The first caller to `init` / `create` / `authorize` on one of them claims it
  permanently. A second call returns `AlreadyInitialized` (error 1).
- Expect them to be consumed by whoever tries them first. That is fine; deploy
  your own instance per position.

`batch_payout` is the exception. It is stateless and holds no funds, so that
address is genuinely reusable by anyone.

Whether to keep this shape is [an open question in SPEC.md](SPEC.md#open-design-question)
— an id-keyed design would let one deployment hold many positions. Settle it
before the SDK is written.

## Verified after deploy

```console
$ stellar contract invoke --id CDKJ...HRAM --network testnet -- max_recipients
40

$ stellar contract invoke --id CDKJ...HRAM --network testnet -- preview \
    --recipients '[{"to":"GC4R...SIMC","amount":"250"}]'
{"count":1,"total":"250"}

$ stellar contract invoke --id CDXD...TNW5 --network testnet -- state
error: HostError: Error(Contract, #2)      # NotInitialized, as expected
```

### One bug this deployment caught

`batch_payout` was first deployed at
`CCOLI4YLKNKS6Z23OHVEA4EBZMJOBWVFIAL7KBNSKVL5IWU2OIH4Z6O3`, which is
**superseded and should not be used**.

`execute` and `preview` took a `Payments` type alias for `Vec<Payment>`.
`#[contractimpl]` reads argument types syntactically and cannot resolve an
alias, so the emitted contract spec referenced a user-defined type named
`Payments` that does not exist. The contract compiled, deployed, and passed all
161 Rust tests — but every client reading the spec broke, and
`stellar contract invoke` failed with `Missing Entry Payments` on *any*
function of the contract, including argument-less ones, because the interface
failed to load as a whole.

Nothing in the local test suite could see this: the Rust tests call the
generated client directly and never parse the spec. Only a real client did.
The fix was to spell the type out in the signature; the alias is gone and the
reason is recorded in `contracts/batch_payout/src/types.rs` so it is not
reintroduced.

## Still to do on testnet

- **Re-measure `MAX_RECIPIENTS`.** It is 40, measured locally, where the
  environment models the CPU/memory budget but not transaction size, and where
  `mock_all_auths` builds one authorization entry per transfer rather than the
  single signed tree a real submission uses. The on-chain figure may differ in
  either direction. Ramp `execute_equal` against testnet with a funded account
  and a real token to settle it.
- **Exercise the full lifecycles on-chain**, not just read-only simulation. The
  invocations above are simulations; nothing has actually moved a token on
  testnet yet.
- **Attach the contract specs** (`make specs` → `dist/specs/*.json`) to a
  GitHub release, so the SDK can generate against a pinned artifact.

## How to redeploy

```bash
make optimize
stellar contract deploy \
  --wasm target/wasm32v1-none/release/sororail_<contract>.optimized.wasm \
  --source <your-identity> --network testnet
```

Record the new address here, and mark what it supersedes rather than deleting
the old line — a stale address in someone's config is easier to diagnose when
this file still explains what happened to it.
