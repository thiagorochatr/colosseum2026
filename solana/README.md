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
make verify   # lint, build both programs for SBF, and every test
```

41 tests:

| Suite | Tests | What it covers |
|---|---|---|
| `verify_proof.rs` | 7 | The Groth16 layer on its own. |
| `clearance.rs` | 16 | Curators and clearances against a stand-in pool. |
| `flow.rs` | 13 | The compliant pool with the real mirror-pool program and real proofs: clear, spend and settle, plus every way to misuse a clearance. |
| `bridge.rs` | 5 | The snarkjs reader. |

| | Compute units |
|---|---|
| `VerifyProof` | 96,954 |
| `VerifyClearance` | 109,552 |
| `VerifyClearance` + `SubmitClearedSpend`, one transaction | 121,441 |

## Live on devnet

Deployed on 2026-10-10. See [`DEPLOY.md`](DEPLOY.md) for how.

| | Address |
|---|---|
| Compliance program | [`HHapM3yA1JkhFJ1msTc7n3CEQ7PMZvxHMvQz7G7orQ2j`](https://explorer.solana.com/address/HHapM3yA1JkhFJ1msTc7n3CEQ7PMZvxHMvQz7G7orQ2j?cluster=devnet) |
| Compliant pool program | [`Fg1FxuHcjj54ERfdQQL9tzZq9wDmNfjGmFSkAXxVYdzB`](https://explorer.solana.com/address/Fg1FxuHcjj54ERfdQQL9tzZq9wDmNfjGmFSkAXxVYdzB?cluster=devnet) |
| Pool, 0.1 SOL | [`4RwTZq6ucSEDWEQZ6RSfa1ruhqEayS4QsF6LVPA7yvGP`](https://explorer.solana.com/address/4RwTZq6ucSEDWEQZ6RSfa1ruhqEayS4QsF6LVPA7yvGP?cluster=devnet) |
| Required curator | [`AdUihuLoEVQyyirSHLrrLMRvctXeFihqTQyGyV1sv5fA`](https://explorer.solana.com/address/AdUihuLoEVQyyirSHLrrLMRvctXeFihqTQyGyV1sv5fA?cluster=devnet) |

`client/e2e.mjs` ran the whole flow there:

| Step | Transaction |
|---|---|
| init pool | [4cQzPc3QUH…](https://explorer.solana.com/tx/4cQzPc3QUH5pYBi2Za6isssK49oEfBbshxmcNNYdP9M9qX2v6sBiLhwmUDbF4gddii2p5EBtTZaYko2Tqm4TJJoP?cluster=devnet) |
| curator created | [3HhKyduZnH…](https://explorer.solana.com/tx/3HhKyduZnHBTPtnitDLfb4t13zy9sSmCT2fUD8s1CwHoZDPWjgZcPXjzna3Tnv8JYLuvVQHshkU34YSzRRStMyKj?cluster=devnet) |
| deposit by alice | [3a7xSsjiBY…](https://explorer.solana.com/tx/3a7xSsjiBYe3cvN5615QZ5VM22MHKaKbjJhByQ2uxoKnZVQfkrxnoi4hgcLcqKoxSrc6Wf8oVcv5yFnF9UzYfy57?cluster=devnet) |
| deposit by bob | [4iDQNACZ1X…](https://explorer.solana.com/tx/4iDQNACZ1XuhdWrfp4Fu2nXCvtr4ZvuLMPsirGZbGBjEQCed4w875q274CPkF5awr6kC9gSbGwTQKDtmiYUMu3Ua?cluster=devnet) |
| deposit by carol | [3br7bKE2AH…](https://explorer.solana.com/tx/3br7bKE2AHVJCsPVvsa5iM5DRaDLiWTNVYU6niybuJeEEnhYmo3jVxUofcyGA78wEgB89VuPqdLk6bT8w1QXXXq3?cluster=devnet) |
| curator publishes the clean root, excluding [1] | [2gdX7JkypR…](https://explorer.solana.com/tx/2gdX7JkypRJmbWAk5REJj6NY6f8vY66h7sQ7Kina5wMfdzSqKLnE1KyZpYeAiUFudjMYhhfo6tjU6gjkx1FERWAW?cluster=devnet) |
| alice's note cleared and spent, one transaction | [2ZSfUaEubj…](https://explorer.solana.com/tx/2ZSfUaEubjwd5nTw5WGUFPU2qEjMR7FMHGCWSjdfGePh3zc2ioc7mL6JNs64Pbmua1iRY3wuXi7XAs7vfmEZiKTK?cluster=devnet) |
| carol's note cleared and spent, one transaction | [1JjBt2pEZy…](https://explorer.solana.com/tx/1JjBt2pEZyajJq2SP8aFCiYXSQJ8YrzauJTfuJn7dXv45PPZ4peQimTmZKipUWseX2fGspVZZs7Yaa914SNGgGV?cluster=devnet) |
| mirror-pool settles both spends in one batch | [2h8PTfPf15…](https://explorer.solana.com/tx/2h8PTfPf15447LPgELAayvDaggPt7Hbc6Lqmx2ouPsBEyBXHjRD61YpxDbpUPL7bRYjVJ1zVNiTEmDbHe6Zi7wvn?cluster=devnet) |

Read back from devnet afterwards:

- **The pool:** 3 deposits and 2 settled spends, owned by the compliant pool program.
- **Each fresh beneficiary:** holds 0.099 SOL, the denomination minus the relay's fee.
- **The curator:** its account is owned by the compliance program, at epoch 0 at that point. Each block in the demo has moved it forward since.
- **Bob's note:** cannot be proved against the published clean set.

The full record is `deployments/devnet-e2e.json`. The curator's clean set in force is the memo of its latest publication, on chain, and its genesis policy is `deployments/devnet-policy.json`.
