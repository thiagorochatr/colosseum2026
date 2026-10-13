# Go-to-market

Alibi lets a payment be private and still follow the rules of whoever is accountable for the money: a stablecoin issuer on Tempo, a pool or its compliance provider on Solana. A withdrawal proves it comes from a deposit in the clean set, without saying which one.

## The problem, by who pays for it

- **Stablecoin issuers and payment companies on Tempo.** Every TIP-20 transfer is public, which exposes payroll, supplier payments and treasury moves. Tempo's own material calls this visibility a dealbreaker for enterprises. Its private standard is listed as "coming soon". The one thing shipped, Tempo Zones, is a ledger run by an operator who sees everything. An issuer cannot offer privacy today without giving up the control its TIP-403 policy gives it.
- **Privacy pools on Solana.** Privacy Cash, Cloak, Umbra and Vanish screen a deposit through an off-chain API once, when it arrives. Funds flagged after that still withdraw, and nothing on chain shows a partner that a withdrawal came from clean funds. That is what stands between these pools and exchanges, offramps and institutional users.
- **Offramps and exchanges that receive the withdrawals.** Today they get an anonymous transfer from a pool. With Alibi they can check, on chain, that it passed a named policy.

## What we sell

| Customer | Product | How they pay |
|---|---|---|
| Stablecoin issuer or payment company on Tempo | A private pool for its token that follows its existing TIP-403 policy. No new trusted party: the issuer's block also blocks the withdrawal. | A fee per private payment, in basis points, plus the relayer fee. |
| Privacy pool on Solana | A clearance check: the pool accepts one more account, and gets withdrawal-time compliance it can show partners. | Integration is free and open source. The pool pays for hosted curation and relaying. |
| Compliance provider (Range, Elliptic, TRM, Chainalysis) | A way to publish its screening as a clean set that users prove against. | Revenue share on curation. It brings the policy, we bring the proofs. |

Verifying a clearance stays free and permissionless. We earn on operating the parts that need uptime: curators, keepers and relayers, and on enterprise support for issuers (deployment, audits and SLAs).

## Distribution

1. **Tempo, issuer first.** Pilot with one TIP-20 issuer or payment company on Moderato, then mainnet. Propose a TIP for privacy-preserving TIP-403 compliance, so the pattern becomes a Tempo standard rather than a contract only we deploy. Tempo's protocol team already uses Groth16 on BN254, the stack Alibi uses.
2. **Solana, through the pools that exist.** Send integration PRs to Cloak and Privacy Cash: the clearance check is one account and one same-slot rule, already exercised by the compliant pool around mirror-pool. Superteam Brasil is the first channel: it already runs a privacy week with Cloak.
3. **Compliance providers as curators.** Range sponsored Solana's privacy hackathon asking for exactly this: compliance proofs and selective disclosure. One provider publishing clean sets gives every integrated pool a policy on day one.
4. **Brazil as the first offramp market.** Stablecoin payments here end at regulated offramps, and we are here. One offramp accepting cleared withdrawals is the proof point for the others.

## Why now

- **Solana:** pairing syscalls, Poseidon and 4 KB transactions are live on mainnet, which makes a Groth16 clearance cost 126k compute units in one transaction.
- **Tempo:** mainnet since March 2026, with payment partners already on it, and privacy publicly promised but not shipped.
- **Ethereum:** Privacy Pools showed the model works: 0xbow raised a $3.5M seed to take association sets to other chains, and has not reached Solana or Tempo.

## Competition

| Approach | Where | What it misses |
|---|---|---|
| Screening at deposit | Solana pools | Funds flagged later still exit. Nothing a partner can check. |
| Tempo Zones | Tempo | An operator sees every transaction. Trust in a TEE. |
| Token-2022 confidential transfers | Solana | Hides amounts, not who pays whom. No issuer has turned it on. |
| Privacy Pools (0xbow) | Ethereum | Not on Solana or Tempo, and not tied to an issuer's own policy. |

Alibi is the only one where the compliance rule is the issuer's own on-chain policy, and the same proof and key run on two chains.

## Risks, and what we do about them

- **"Compliance" read as surveillance.** Nobody learns which deposit a withdrawal came from. We published what each kind of policy costs in anonymity, on real mainnet depositors: [ANONYMITY_COST.md](ANONYMITY_COST.md).
- **Trust in the Solana curator.** The policy and the clean set are public in every update. Next comes several curators, with the user choosing among them, and exclusion derived from the token's freeze state, as on Tempo.
- **Not production-ready.** It runs on testnets and has not been audited. A multi-party ceremony and an audit come before any mainnet pool.

## Evidence of demand

We have not run customer interviews yet; they are the first item below. What we have is what the buyers say in public:

- **Tempo** calls the visibility of payments a [dealbreaker for enterprises](https://tempo.xyz/learn/blockchain-payments/) and lists a native private token standard as "coming soon" in its [README](https://github.com/tempoxyz/tempo).
- **Range**, a compliance provider, [sponsored Solana's privacy hackathon](https://www.range.org/blog/sponsoring-the-solana-privacy-hack-building-compliant-privacy-infrastructure) asking for compliance proofs from private history and selective disclosure.
- **The Solana Foundation** sells [privacy with compliance](https://solana.com/privacy) to institutions.
- **0xbow** raised a $3.5M seed to bring Privacy Pools, the same idea on Ethereum, [to other chains](https://blockeden.xyz/blog/2026/04/12/0xbow-privacy-compliance-defi-fatf-travel-rule/).

## Next 90 days

1. Customer interviews: Tempo issuers and payment companies, Cloak and Privacy Cash, a compliance provider, and Brazilian offramps.
2. A production trusted setup with independent contributors, and an audit.
3. A pilot issuer on Tempo, and the TIP proposal.
4. Integration PRs to Cloak and Privacy Cash, and one compliance provider publishing clean sets.
5. On Solana: ragequit for excluded depositors, and SPL and Token-2022 pools.

## Team

- **Thiago Rocha**: protocol, circuit, contracts and app. Cryptography R&D at Banco Inter; researches programmable cryptography; built mirror-pool, winner of Superteam Brasil's Build Privacy-Through-Noise bounty.
- **Stefany Lopes** ([@stelopesz](https://x.com/stelopesz)): product and go-to-market. Product and project manager in Web3 since 2021, with a background in product design; 2× hackathon winner (Radar, ORE track; Crecimiento Argentina).
