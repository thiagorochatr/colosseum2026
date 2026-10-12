# Solana

The compliance program, the compliant pool, and the bridge from snarkjs to `groth16-solana`. See [`docs/SPEC.md`](../docs/SPEC.md) section 8.

| Path | What it is |
|---|---|
| `crates/compliance-interface` | The curator and clearance layouts, seeds and instruction tags both programs agree on. |
| `crates/snarkjs-bridge` | Reads snarkjs keys and proofs, checks them with arkworks, converts them for `groth16-solana`. |
| `programs/compliance` | Curators publish clean roots. `VerifyClearance` checks a withdraw proof and records a clearance. |
| `programs/compliant-pool` | mirror-pool, unchanged and linked as a library, behind an entrypoint that pays out only against a clearance. |
| `client/` | Instruction builders, the pool rebuilt from history, the curator CLI, and the end-to-end flow. |
| `tests/keys/` | Public test keys the test build is bound to. |

## Build and test

```sh
make verify       # lint, the test build of both programs for SBF, and every test
make build-test   # the SBF build the tests load, bound to the public keys in tests/keys/
make build-sbf    # a deployment build: refuses unless its ids are set, see DEPLOY.md
```

45 tests:

| Suite | Tests | What it covers |
|---|---|---|
| `verify_proof.rs` | 7 | The Groth16 layer on its own. |
| `clearance.rs` | 17 | Curators and clearances against a stand-in pool. |
| `flow.rs` | 16 | The compliant pool with the real mirror-pool program and real proofs: clear, spend and settle, plus every way to misuse a clearance. |
| `bridge.rs` | 5 | The snarkjs reader. |

| | Compute units |
|---|---|
| `VerifyProof` | 96,954 |
| `VerifyClearance`, on devnet | 111,461 |
| `VerifyClearance` + `SubmitClearedSpend`, one transaction, on devnet | 126,407 |

The two devnet figures are from alice's spend in the run below, and vary by a few thousand from one note to the next: finding the clearance's and the spend record's addresses costs more for some nullifiers.

## Live on devnet

Deployed on 2026-10-10. See [`DEPLOY.md`](DEPLOY.md) for how.

| | Address |
|---|---|
| Compliance program | [`HHapM3yA1JkhFJ1msTc7n3CEQ7PMZvxHMvQz7G7orQ2j`](https://explorer.solana.com/address/HHapM3yA1JkhFJ1msTc7n3CEQ7PMZvxHMvQz7G7orQ2j?cluster=devnet) |
| Compliant pool program | [`Fg1FxuHcjj54ERfdQQL9tzZq9wDmNfjGmFSkAXxVYdzB`](https://explorer.solana.com/address/Fg1FxuHcjj54ERfdQQL9tzZq9wDmNfjGmFSkAXxVYdzB?cluster=devnet) |
| Pool, 0.1 SOL | [`4RwTZq6ucSEDWEQZ6RSfa1ruhqEayS4QsF6LVPA7yvGP`](https://explorer.solana.com/address/4RwTZq6ucSEDWEQZ6RSfa1ruhqEayS4QsF6LVPA7yvGP?cluster=devnet) |
| Required curator | [`AdUihuLoEVQyyirSHLrrLMRvctXeFihqTQyGyV1sv5fA`](https://explorer.solana.com/address/AdUihuLoEVQyyirSHLrrLMRvctXeFihqTQyGyV1sv5fA?cluster=devnet) |

The pool and the curator were created on 2026-10-10, before the three upgrades in `deployments/devnet.json`:

| Step | Transaction |
|---|---|
| init pool | [4cQzPc3QUH…](https://explorer.solana.com/tx/4cQzPc3QUH5pYBi2Za6isssK49oEfBbshxmcNNYdP9M9qX2v6sBiLhwmUDbF4gddii2p5EBtTZaYko2Tqm4TJJoP?cluster=devnet) |
| curator created | [3HhKyduZnH…](https://explorer.solana.com/tx/3HhKyduZnHBTPtnitDLfb4t13zy9sSmCT2fUD8s1CwHoZDPWjgZcPXjzna3Tnv8JYLuvVQHshkU34YSzRRStMyKj?cluster=devnet) |

`client/e2e.mjs` then ran the whole flow again on 2026-10-12, against the programs as upgraded:

| Step | Transaction |
|---|---|
| deposit by alice | [37QrZSbG7Y…](https://explorer.solana.com/tx/37QrZSbG7YEFGmi4GZ1GQLhb8oz6xebHRuX9F4pxS7bDnRQikHUU2M94rUNYU3Rdvg1q5uDuW76gn2jVS7ED5Qdf?cluster=devnet) |
| deposit by bob | [39NvJAE2Bv…](https://explorer.solana.com/tx/39NvJAE2BvwhWhB6nKxZ3NrTu6hSGL3PFGjhPEtpBk1bdWKJeJ8YWdUc8eKnToR1TB3o6MzhacWbhWaCx8WXFQnb?cluster=devnet) |
| deposit by carol | [N43Z2YUZtP…](https://explorer.solana.com/tx/N43Z2YUZtPVVeTfA4YsB13iEsj81QGNjirq2K3takatzkYfnFQp44M5gj1rf1moSFwNSLXrrfT6jLJa1aa2zJyT?cluster=devnet) |
| curator publishes the clean root, excluding bob's deposits [1, 22] | [5tY1jfMNkW…](https://explorer.solana.com/tx/5tY1jfMNkWCbfFpK4HRd7H6c4bwZMoFBb7CUNL5HQwpHJUz36CQexFyn6Pm8wEZWHyTKV9WCBrzp8f4teupQ98eX?cluster=devnet) |
| alice's note cleared and spent, one transaction | [3JTkP82dMB…](https://explorer.solana.com/tx/3JTkP82dMB8PBbVb6MB3TcamePA2FGrrASTx7cv9KDVvLJ7dLtjftRbyEGKTB1csAAnN3TZXDCHUynMn5RyQWcmh?cluster=devnet) |
| carol's note cleared and spent, one transaction | [5G9UyDsLen…](https://explorer.solana.com/tx/5G9UyDsLenjovrXC5KAv3pYjMtuSjYNVHuPBswQpg3BWweCcv2SGBTVJFtwFTMmvtoJvd3cL2fAfZKo5hZhp64Sy?cluster=devnet) |
| mirror-pool settles both spends in one batch | [3fjFeczHDC…](https://explorer.solana.com/tx/3fjFeczHDCsmaw2kmAH2fAf6Jmt1K4wrfp87xT4zTot34U3MTmRWudkqJrbvMco4j25UAxXhX7RxfEn1a5GxuxDe?cluster=devnet) |

Read back from devnet afterwards:

- **The pool:** owned by the compliant pool program, holding these 3 deposits among the demo's, 24 in all.
- **Each fresh beneficiary:** holds 0.099 SOL, the denomination minus the relay's fee.
- **The curator:** its account is owned by the compliance program, at epoch 3, each block in the demo having moved it forward.
- **Bob's note:** cannot be proved against the published clean set: the circuit fails at the clean-root check.

The full record is `deployments/devnet-e2e.json`. The curator's clean set in force is the memo of its latest publication, on chain, and its genesis policy is `deployments/devnet-policy.json`.
