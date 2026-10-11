# Hackathon Colosseum World's Fair 2026

[![CI](https://github.com/thiagorochatr/colosseum2026/actions/workflows/ci.yml/badge.svg)](https://github.com/thiagorochatr/colosseum2026/actions/workflows/ci.yml) [![On-chain](https://github.com/thiagorochatr/colosseum2026/actions/workflows/onchain.yml/badge.svg)](https://github.com/thiagorochatr/colosseum2026/actions/workflows/onchain.yml)

A compliance layer for private payments, built for the Colosseum World's Fair hackathon.

> Tempo has compliance without privacy. Solana has privacy without compliance.
> One zero-knowledge proof closes both gaps.

A user proves that their deposit belongs to a **clean set** without revealing which deposit is theirs. The same circuit and the same verifying key run on both chains:

- **Tempo:** a private stablecoin pool whose clean set is derived on-chain from the token issuer's own TIP-403 policy. A depositor who is blocklisted after depositing falls out of the set and cannot withdraw.
- **Solana:** a compliance program that verifies the proof and records a clearance any program can check. mirror-pool is the reference pool.

**Try it:** [colosseum2026.vercel.app](https://colosseum2026.vercel.app). It runs on Tempo's Moderato testnet and Solana devnet, with a throwaway test wallet in the browser. Deposit, prove in the browser and withdraw to a fresh address, or play the issuer or the curator and block a depositor.

Work in progress.

## Layout

| Path | What it is |
|---|---|
| `mirror-pool/` | Prior work, imported unchanged. See below. |
| `docs/SPEC.md` | The protocol: notes, trees, circuit, bindings, and each chain's contracts. |
| `docs/THREAT_MODEL.md` | Who can do what, and the known limits. |
| `circuits/` | The withdraw circuit, frozen, with its tests. |
| `tools/vectors/` | Test vectors for the circuit, computed with `mirror-core`. |
| `solana/` | The compliance program, the compliant pool around mirror-pool, the client and curator CLI. Live on devnet. |
| `sdk/` | Trees, circuit inputs, proving, and each chain's binding and proof layout. |
| `web/` | The app: one tab per chain. |
| `api/` | The demo service as Vercel functions: faucet, relayer, issuer and curator, holding the test-network keys. |
| `tempo/` | The private stablecoin pool, its tests, the relayer and the end-to-end flow. Live on Moderato. |
| `.github/workflows/` | CI on every push, and a daily check that both networks run this source. See [SECURITY.md](SECURITY.md). |

## Prior work

`mirror-pool/` is my own earlier project. I, Thiago ([@thiagorochatr](https://github.com/thiagorochatr)), designed and built [mirror-pool](https://github.com/solanabr/mirror-pool) for Superteam Brasil's **Privacy-Through-Noise** bounty, and it was one of the three winning submissions:

- **Winners on Superteam Earn:** the [Noise bounty listing](https://superteam.fun/earn/listing/noise/) names me among the three winners, each awarded 1,300 USDG. My Earn profile is [superteam.fun/earn/t/thiago](https://superteam.fun/earn/t/thiago/).
- **The winning PR:** [solanabr/mirror-pool#5](https://github.com/solanabr/mirror-pool/pull/5), merged into Superteam Brasil's repository on 2026-08-05.
- **The judge's verdict**, in the [review on that PR](https://github.com/solanabr/mirror-pool/pull/5#issuecomment-5186212116): *"Your PR was the highest-scoring submission in the cohort."*
- **The paper:** [A Behavioural Anonymity Set for Solana](mirror-pool/docs/a-behavioural-anonymity-set-for-solana.pdf), 39 pages, written alongside the code.

It was imported here at commit `3600c97`, which adds the judge's follow-up recommendations R1 and R2 on top of the merged version. Those are open upstream as [PR #6](https://github.com/solanabr/mirror-pool/pull/6). The first commit of this repository is that import, byte for byte. Every commit after it is work done during the hackathon window, and the diff against that first commit is exactly what was built here.

## License

MIT. `mirror-pool/` keeps its own MIT license.
