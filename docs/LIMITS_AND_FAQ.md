# Limits and FAQ

What this project does not do, and short answers to the questions it is most often asked. The parties and their powers are in [THREAT_MODEL.md](THREAT_MODEL.md), the security posture in [SECURITY.md](../SECURITY.md), and the trusted setup in [CEREMONY.md](CEREMONY.md).

## Limits

| Limit | Why | What answers it |
|---|---|---|
| **Small anonymity sets** | Test networks, about ten deposits per pool. | Privacy grows with use. Measuring what each clean-set policy costs in anonymity comes after the build. |
| **One fixed amount per pool** | A fixed amount is what stops matching a withdrawal to a deposit by value. | Variable amounts with join-split, on the roadmap. |
| **One human contributor in phase 2 of the ceremony** | Hackathon timeline. | The ceremony is open to new contributions. A production ceremony with several contributors is on the roadmap. |
| **The curator is new trust on Solana** | SOL has no issuer rule on-chain, so someone has to publish the clean set. | The policy is published and hashed on-chain, every clean set is in a memo anyone can audit, and the curator can only remove deposits, never add one. Several curators and ragequit are on the roadmap. |
| **No ragequit on Solana** | Hackathon timeline. | On the roadmap: an excluded depositor recovers the deposit, without privacy, to the depositing address. |
| **A window of seconds on Tempo** | A depositor who sees their own block coming can withdraw before the exclusion lands. | The same holds for the token itself: a holder can move funds before a freeze. The issuer closes it by blocking and excluding in one batch; otherwise the keeper narrows it to seconds. |
| **The keeper does not run around the clock** | Vercel's free cron runs once a day. | The demo relayer applies the policy before every withdrawal it submits. A permanent keeper is on the roadmap. |
| **The relayer sees an IP address and a recipient** | Every relayer does. | It never sees which deposit pays. Deposit and withdraw in separate sessions, use Tor or a VPN, use several relayers, or withdraw yourself with the command line. |
| **High gas on Tempo** | About 3.5M gas per deposit: Poseidon in Solidity, and new storage. | Fractions of a cent on Moderato. Optimizing is future work. |
| **Native SOL only on Solana** | mirror-pool moves lamports. | SPL and Token-2022, on the roadmap. |
| **Upgradeable Solana programs** | Test network. | CI rebuilds them from source daily and compares them byte for byte with devnet, so a swap is caught within a day. The Tempo pool has no owner and cannot be upgraded. |
| **Not audited** | Hackathon. | Tests, reproducible builds and [SECURITY.md](../SECURITY.md) say so plainly. |

**What this is not:** not KYC, not a viewing-key system, not a bridge. "Compliance" here means enforcing, with a proof, the rule of whoever already has authority over the money: the token's issuer on Tempo, a published curator on Solana.

## FAQ

### Positioning

**How is this different from Cloak, and from other Solana privacy protocols?**
Cloak is a pool; this is the layer any pool can use, Cloak included. Solana's privacy protocols screen addresses through off-chain risk APIs:

- **Cloak** screens at deposit and at relay-send through [Range's Risk API](https://range.org/blog/how-cloak-screens-for-sanctioned-addresses-before-its-privacy-layer-applies).
- **Privacy Cash** relayers screen the depositor through CipherOwl before a withdrawal, [per Solana Compass](https://solanacompass.com/projects/privacy-cash).
- **Umbra** [integrates Range](https://range.org/blog/umbra-integrates-range-to-power-real-time-risk-and-compliance-for-private-transfers-on-solana) for private transfers, and **Vanish** [screens deposits with Range](https://range.org/blog/vanish-trade-chooses-range-to-power-real-time-risk-screening-for-private-trading-on-solana).

Screening at deposit misses funds flagged after they went in. This layer checks at withdrawal, against the rule as it stands that moment, and leaves a public clearance on-chain that anyone can verify: the pool, a DEX, an offramp. A screening API's answer stays off-chain, so none of them can.

**Why would a pool like Cloak or Privacy Cash use it?**
For three reasons. It catches funds flagged after deposit. It turns "we screened this" into public on-chain evidence that partners, offramps and app stores can check for themselves, a defensible answer to "is this a mixer?". And it is ready-made: a circuit, a ceremony, an on-chain verifier and a clearance account, without building zero-knowledge compliance in-house. Range, which those pools already use, is a natural curator rather than a competitor.

**And 0xbow's Privacy Pools?**
The same family of idea, a provider of clean sets with ragequit, on Ethereum. This brings it to Solana, and on Tempo it removes the provider: the clean set comes straight from the issuer's TIP-403 policy, on-chain, with no one in between.

**And Helius Privacy Rings?**
Custom Rings are permissioned sub-pools with policies and authorities, so compliance lives inside one ring. This layer is for open pools, and its clearance is an account any program can read.

**Why not Tempo Zones?**
A Zone is a closed environment whose operator sees everything. This pool runs on the public chain with no operator, and the issuer does not see who withdrew what. The issuer only decides who may not withdraw.

**Why two chains?**
Their gaps are opposite. Tempo has compliance without privacy; Solana has privacy without compliance. One circuit and one verifying key close both, which shows the layer is portable.

### Trust

**Who is the curator on Solana, in real life?**
A pluggable role. In production it would be a risk-data provider such as Range, which already screens for Solana's privacy pools, or the issuer of a token. Its policy is public and each decision is on-chain in a memo. In the demo it is a key held by the demo service.

**Can the curator steal funds or add a fake deposit?**
No. It can only remove deposits from the clean set, never add one. A proof needs the same leaf at the same position in the pool tree, which only the pool program maintains. A circuit test checks exactly this, and the curator never sees a withdrawal key.

**What if the issuer or the curator abuses its power?**
On Tempo, the issuer can already freeze its own token, so the pool creates no new power. On Solana, the curator is new trust, which is why its policy is published, everything it does is auditable, and the roadmap has ragequit and several curators.

**Does "excluded" mean criminal?**
No. It means the rule of whoever has authority over the money does not authorize that depositor. The system applies the rule; it does not judge.

**If I am already blocked, can I use the pool to hide money?**
On Tempo, no: the token refuses a deposit from a blocked sender. If you are blocked after depositing, your deposit leaves the clean set and no proof exists for it. On Solana, the curator excludes you at withdrawal time: you can deposit, but you cannot withdraw privately.

**What happens to an excluded depositor's money?**
It stays in the pool. If the block is lifted, by the issuer on Tempo or the curator on Solana, the deposit is reincluded and can be withdrawn. Until then there is no way out on Solana; ragequit, on the roadmap, would return it to the depositing address without privacy.

### Security

**Can the relayer steal a withdrawal?**
No. The proof is bound to the recipient, the relayer and the fee, and the chain recomputes that binding itself. Change any of them and the proof fails. The relayer can only refuse to submit.

**Does the relayer know who I am?**
It sees the IP address that asked for the withdrawal and where the money went, but not which deposit paid for it: the proof hides that. The leak is in the network, not in the cryptography, and it affects every relayer. See the relayer row under Limits.

**What if the demo service is hacked?**
It holds test-network keys for the issuer, the curator and the relayers. With them an attacker can censor and freeze: block anyone, revoke every clean root, refuse to relay. They cannot steal from the pools or forge a proof, and they never see a withdrawal key, which stays in each user's browser.

**Why trust your ceremony?**
Almost all of it is verifiable without trusting anyone. Phase 1 has 80 independent contributors. The phase-2 chain is checked with one command, and CI does it on every push. The beacon was announced in a public commit before its block existed. What remains is trusting that the one phase-2 contributor destroyed their entropy, which is why this runs on test networks. The ceremony is open, and anyone can add a contribution. See [CEREMONY.md](CEREMONY.md).

**Why Groth16?**
It has the smallest proof, about 256 bytes, and the cheapest verification, about 97k compute units on Solana. Both chains support it natively. The price is a trusted setup, and this project ran one.

**How do I know the deployed code is this repository's?**
Every day, CI rebuilds everything from source and compares it byte for byte with what runs on Moderato and devnet. The badges at the top of the README show the result.

### The project

**What was built here, and what existed before?**
mirror-pool existed: it is the author's earlier project, built for Superteam Brasil's Privacy-Through-Noise bounty, where it was among the [three winners on Superteam Earn](https://superteam.fun/earn/listing/noise/). It is imported unchanged, and CI checks that byte for byte. Built here: the withdraw circuit and its ceremony, the Tempo pool, the compliance program, the compliant pool around mirror-pool, the SDK, the keeper, and the demo app.

**What is next?**
A production ceremony with several contributors, several curators and a user-chosen curator, ragequit on Solana, SPL tokens, and talking to issuers on Tempo and to Solana pools that need a compliant exit.
