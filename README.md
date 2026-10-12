<img src="brand/alibi.svg" width="72" alt="Alibi">

# Alibi

[![CI](https://github.com/thiagorochatr/colosseum2026/actions/workflows/ci.yml/badge.svg)](https://github.com/thiagorochatr/colosseum2026/actions/workflows/ci.yml) [![On-chain](https://github.com/thiagorochatr/colosseum2026/actions/workflows/onchain.yml/badge.svg)](https://github.com/thiagorochatr/colosseum2026/actions/workflows/onchain.yml)

A compliance layer for private payments, built for the Colosseum World's Fair hackathon.

> Tempo has compliance without privacy. Solana has privacy without compliance.
> One zero-knowledge proof closes both gaps.

A user proves that their deposit belongs to a **clean set** without revealing which deposit is theirs. The same circuit and the same verifying key run on both chains:

- **Tempo:** a private stablecoin pool whose clean set is derived on-chain from the token issuer's own TIP-403 policy. A depositor who is blocklisted after depositing falls out of the set and cannot withdraw.
- **Solana:** a compliance program that verifies the proof and records a clearance any program can check. mirror-pool is the reference pool.

**Try it:** [colosseum2026.vercel.app](https://colosseum2026.vercel.app). It runs on Tempo's Moderato testnet and Solana devnet, with a throwaway test wallet in the browser. Deposit, prove in the browser and withdraw to a fresh address, or play the issuer or the curator and block a depositor.

Testnet only, not audited.

## How it works

1. **Deposit.** A fixed amount, with a commitment `Poseidon(k, r, denomination)` made from two secrets that stay in the user's browser. The pool appends the commitment to a Merkle tree.
2. **The clean set.** A second tree holds the same deposits, minus those whose depositor the policy blocks. On Tempo the pool derives it from the token issuer's TIP-403 policy, on chain, with no curator. On Solana a curator publishes its root, with the policy and the clean set in a memo anyone can audit.
3. **Prove.** In the browser, the user proves that their commitment is in both trees, reveals its nullifier so it cannot be spent twice, and binds the proof to one recipient, relayer and fee. Nothing else is revealed: not which deposit, not who deposited it.
4. **Pay.** On Tempo the pool checks the proof and pays at once, to a fresh address, through a relayer that pays the gas. On Solana the compliance program checks the proof and writes a public clearance; in the same transaction the compliant pool records the spend against that clearance, and mirror-pool pays recorded spends in batches.

A deposit whose depositor is blocked after depositing falls out of the clean set, so no valid proof exists for it, and it cannot be withdrawn until the block is lifted.

The circuit is Groth16 over BN254, 10,175 constraints, with four public inputs: the pool root, the clean root, the nullifier hash and the binding. Its key comes from a [public ceremony](docs/CEREMONY.md) closed with a beacon announced in advance, and both chains verify under that one key.

## Tempo: privacy that keeps the issuer's rules

Tempo stablecoins already carry their issuer's policy on chain. This pool keeps it, privately:

- **No new trust.** The clean set is the issuer's own TIP-403 policy. `exclude` and `reinclude` are permissionless, and the pool checks each against the registry, so anyone can apply the policy and no one can bend it.
- **No race.** The issuer can block an address and exclude its deposits in one Tempo batch transaction. A keeper applies blocks made elsewhere, and the relayer applies the policy again before every withdrawal it submits.
- **No owner.** The pool cannot be upgraded or paused by anyone.

Details in [`tempo/`](tempo/README.md).

## Solana: compliance as a layer, not another pool

Solana's privacy pools screen deposits through off-chain APIs, and nothing on chain says a withdrawal came from clean funds. This adds that, as a layer any pool can require:

- **A public clearance.** The compliance program verifies the proof against the pool's root and the curator's clean root, and writes a clearance account that any program can check.
- **mirror-pool, unchanged.** The compliant pool links mirror-pool byte for byte and adds one instruction: a spend it records only against a clearance written in the same slot, which mirror-pool then pays like any other.
- **An auditable curator.** The curator is a pluggable role: in production a risk-data provider such as Range, which already screens for Solana's privacy pools, or a token's issuer. Its policy's hash is on chain, every clean set is in a memo, and it can only remove deposits, never add one.

Details in [`solana/`](solana/README.md).

## Why a layer

Screening at deposit misses funds flagged after they were deposited, and leaves no evidence a partner can check. Checking at withdrawal, against the rule in force, catches both and turns "we screened this" into a public proof. Pools such as Cloak, Privacy Cash, Umbra and Vanish could require the same clearance rather than build zero-knowledge compliance themselves. More in [docs/LIMITS_AND_FAQ.md](docs/LIMITS_AND_FAQ.md).

## Measured

| | Cost |
|---|---|
| Proving, in the browser | under a second, after a one-time download of the 6 MB proving key |
| Tempo, a withdrawal | about 0.8M to 1.1M gas, of which the Groth16 check is about 216k |
| Tempo, block and exclude in one batch | about 1.6M gas |
| Solana, `VerifyProof` | 96,954 compute units |
| Solana, clear and spend in one transaction | about 126k compute units |

Every deployment and transaction behind these numbers is listed, with explorer links, in [docs/PROOF.md](docs/PROOF.md).

What a clean-set policy costs in anonymity is measured on real Privacy Cash depositors in [docs/ANONYMITY_COST.md](docs/ANONYMITY_COST.md), with the method of mirror-pool's funding-provenance study. A blocklist unrelated to funding costs about its own share: blocking 10% of depositors keeps 0.94 (0.79 to 1.06) of effective anonymity. A policy keyed on funding source can cost far more or far less, depending on how the pool's depositors are funded, so it has to be measured pool by pool.

## Running it

```sh
npm ci && npm test                                    # the SDK
cd circuits && npm ci && npm run build && npm test   # the circuit
cd tempo && git submodule update --init && npm ci && forge test --network tempo
cd solana && make verify                              # both programs, and mirror-pool's real program in the flow tests
npm run dev                                           # the app and the demo service, at localhost:5173
```

The demo service needs the test-network keys it acts with (`TEMPO_ISSUER_KEY`, `TEMPO_RELAYER_KEY`, `SOLANA_RELAY_KEY`, `SOLANA_CURATOR_KEY`) in `.env.local`. The Tempo and Solana tests prove with the real key: build the circuit first, and put `withdraw_final.zkey` from the [ceremony-v1 release](https://github.com/thiagorochatr/colosseum2026/releases/tag/ceremony-v1) in `circuits/ceremony/`.

## Limits and roadmap

What this does not do yet is written down in [docs/LIMITS_AND_FAQ.md](docs/LIMITS_AND_FAQ.md) and [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md). The main ones: one human contributor in the ceremony so far, the Solana curator as new trust, native SOL only, and on Solana a block does not stop a payout already recorded.

Next:

- a production ceremony with several independent contributors, and a new announced beacon;
- several curators, with the user choosing whose clean set a withdrawal proves against;
- on Solana, ragequit for an excluded depositor, SPL and Token-2022 pools, and exclusion derived from a token's freeze state, as Tempo does with TIP-403;
- re-checking the curator's epoch when a spend is recorded and when it is paid;
- variable amounts with join-split, selective disclosure to an auditor with viewing keys, and adapters for existing pools such as Cloak and Privacy Cash.

## Layout

| Path | What it is |
|---|---|
| `mirror-pool/` | Prior work, imported unchanged. See below. |
| `docs/SPEC.md` | The protocol: notes, trees, circuit, bindings, and each chain's contracts. |
| `docs/THREAT_MODEL.md` | Who can do what, and the known limits. |
| `docs/LIMITS_AND_FAQ.md` | What this does not do, and answers to the questions it is most often asked. |
| `docs/CEREMONY.md` | The trusted setup: the starting key, the contribution, the beacon and how to verify them. |
| `docs/PROOF.md` | Every deployment and transaction on both test networks, with explorer links, and how to check them. |
| `docs/ANONYMITY_COST.md` | How much anonymity each kind of clean-set policy costs, measured offline on real mainnet depositors. |
| `circuits/` | The withdraw circuit, frozen, with its tests. |
| `tools/vectors/` | Test vectors for the circuit, computed with `mirror-core`. |
| `tools/curation-cost/` | The measurement behind `docs/ANONYMITY_COST.md`, one offline command. |
| `solana/` | The compliance program, the compliant pool around mirror-pool, the client and curator CLI. Live on devnet. |
| `sdk/` | Trees, circuit inputs, proving, and each chain's binding and proof layout. |
| `web/` | The app, one page per chain: [/tempo](https://colosseum2026.vercel.app/tempo) and [/solana](https://colosseum2026.vercel.app/solana). |
| `api/` | The demo service as Vercel functions: faucet, relayer, issuer and curator, holding the test-network keys. |
| `tempo/` | The private stablecoin pool, its tests, the relayer and the end-to-end flow. Live on Moderato. |
| `brand/` | The Alibi mark, and PNGs of it for profiles and link previews. |
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
