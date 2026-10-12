# Proof that it runs

Everything below is on a public test network and can be checked without trusting this repository: each address and transaction links to a block explorer. Two scheduled checks keep it honest after the fact, and the last section says how to repeat them.

## One key, two chains

Both chains verify the same circuit under the same key, from the [ceremony](CEREMONY.md):

| | sha256 |
|---|---|
| Proving key, `withdraw_final.zkey` | `a3217e83f95632a27e27d7593167cdfe5bb9ecaa0539e926cdce38248ce180f5` |
| Verifying key, `circuits/ceremony/vk.json` | `e5d67dab6cb8819d8b6e11f1f36d460d68f542392049f86fc5c0b0b5bec07cf0` |
| The same key in Solana's on-chain layout | `e453d98a0fea776c6cbc7a307532e44947a0f13aafeae3bebf8793897b455c4a` |

The keys are in the [ceremony-v1 release](https://github.com/thiagorochatr/colosseum2026/releases/tag/ceremony-v1). The Tempo verifier and the Solana verifying-key module are both generated from that proving key, and CI regenerates them on every push and fails if either differs from the committed file.

## Tempo, Moderato testnet

| | Address |
|---|---|
| Pool | [`0x7228B24c607151E74DBCb4Ad818B87cd993a64E9`](https://explore.testnet.tempo.xyz/address/0x7228B24c607151E74DBCb4Ad818B87cd993a64E9) |
| Verifier | [`0x11A0D22C805E50ED833343cA2802173bd8CB795a`](https://explore.testnet.tempo.xyz/address/0x11A0D22C805E50ED833343cA2802173bd8CB795a) |
| Demo USD, a TIP-20 token | [`0x20C0000000000000000000002Bc6afccdA61E264`](https://explore.testnet.tempo.xyz/address/0x20C0000000000000000000002Bc6afccdA61E264) |
| The token's TIP-403 blacklist policy | `1253146` |
| Demo relayer | [`0x1625f9D780A284D50581fb68B7f8C9bF7e10AF86`](https://explore.testnet.tempo.xyz/address/0x1625f9D780A284D50581fb68B7f8C9bF7e10AF86) |

The pool has no owner and cannot be upgraded. `tempo/e2e/run.mjs` ran the whole flow on 2026-10-10, recorded at commit `8d744d8`:

| Step | Transaction | Gas |
|---|---|---|
| Alice deposits 100 dUSD | [0xfd5ea9c1…](https://explore.testnet.tempo.xyz/tx/0xfd5ea9c182bec1c7101d9a463176a6f56b977dd998fc58545e1ba18ab551fb4b) | 13,200,071 |
| Bob deposits | [0x7f1b5391…](https://explore.testnet.tempo.xyz/tx/0x7f1b539195116d71a4288a03543a664679e74408fa7f04efa4804f618f26f082) | 2,808,151 |
| Carol deposits | [0xc6a0f072…](https://explore.testnet.tempo.xyz/tx/0xc6a0f07220c9196198468242db77551418f09a69a1c6992800d7c451104b115e) | 3,055,403 |
| The issuer blocks Bob and excludes his deposit, in one batch transaction | [0xf0dc3cf0…](https://explore.testnet.tempo.xyz/tx/0xf0dc3cf03ba9da933fef3e697fef12746c114acc850fba22e1a1f0313c76fbb5) | 1,580,840 |
| Alice withdraws to a fresh address, through the relayer | [0xfd2355fb…](https://explore.testnet.tempo.xyz/tx/0xfd2355fb92eebd100aa3a1a34ff9aa4c67fd149de3b682322e18cb9d3ee2509e) | 1,064,745 |
| Carol withdraws to a fresh address, through the relayer | [0xd60c4be5…](https://explore.testnet.tempo.xyz/tx/0xd60c4be5003273ae16c20294a3dab0e13e3316d8f1cee53efacda062719ce0d6) | 812,481 |

- **Bob:** his note cannot be proved: the circuit fails at the clean-root check.
- **Each fresh recipient:** received 99 dUSD, the deposit less the relayer's 1 dUSD fee, and has never sent a transaction.

The record is [`tempo/deployments/42431-e2e.json`](../tempo/deployments/42431-e2e.json).

## Solana, devnet

| | Address |
|---|---|
| Compliance program | [`HHapM3yA1JkhFJ1msTc7n3CEQ7PMZvxHMvQz7G7orQ2j`](https://explorer.solana.com/address/HHapM3yA1JkhFJ1msTc7n3CEQ7PMZvxHMvQz7G7orQ2j?cluster=devnet) |
| Compliant pool program, wrapping mirror-pool unchanged | [`Fg1FxuHcjj54ERfdQQL9tzZq9wDmNfjGmFSkAXxVYdzB`](https://explorer.solana.com/address/Fg1FxuHcjj54ERfdQQL9tzZq9wDmNfjGmFSkAXxVYdzB?cluster=devnet) |
| Pool, 0.1 SOL | [`4RwTZq6ucSEDWEQZ6RSfa1ruhqEayS4QsF6LVPA7yvGP`](https://explorer.solana.com/address/4RwTZq6ucSEDWEQZ6RSfa1ruhqEayS4QsF6LVPA7yvGP?cluster=devnet) |
| Curator authority | [`AdUihuLoEVQyyirSHLrrLMRvctXeFihqTQyGyV1sv5fA`](https://explorer.solana.com/address/AdUihuLoEVQyyirSHLrrLMRvctXeFihqTQyGyV1sv5fA?cluster=devnet) |

Both programs were deployed on 2026-10-10 and upgraded three times since; [`solana/deployments/devnet.json`](../solana/deployments/devnet.json) lists each upgrade with its transaction and what it changed. `solana/client/e2e.mjs` ran the whole flow against the upgraded programs on 2026-10-12, recorded at commit `2b784da`:

| Step | Transaction |
|---|---|
| Alice deposits 0.1 SOL | [37QrZSbG7Y…](https://explorer.solana.com/tx/37QrZSbG7YEFGmi4GZ1GQLhb8oz6xebHRuX9F4pxS7bDnRQikHUU2M94rUNYU3Rdvg1q5uDuW76gn2jVS7ED5Qdf?cluster=devnet) |
| Bob deposits | [39NvJAE2Bv…](https://explorer.solana.com/tx/39NvJAE2BvwhWhB6nKxZ3NrTu6hSGL3PFGjhPEtpBk1bdWKJeJ8YWdUc8eKnToR1TB3o6MzhacWbhWaCx8WXFQnb?cluster=devnet) |
| Carol deposits | [N43Z2YUZtP…](https://explorer.solana.com/tx/N43Z2YUZtPVVeTfA4YsB13iEsj81QGNjirq2K3takatzkYfnFQp44M5gj1rf1moSFwNSLXrrfT6jLJa1aa2zJyT?cluster=devnet) |
| The curator publishes a clean root excluding Bob's deposits, with the clean set and its policy in a memo | [5tY1jfMNkW…](https://explorer.solana.com/tx/5tY1jfMNkWCbfFpK4HRd7H6c4bwZMoFBb7CUNL5HQwpHJUz36CQexFyn6Pm8wEZWHyTKV9WCBrzp8f4teupQ98eX?cluster=devnet) |
| Alice's note: VerifyClearance and SubmitClearedSpend, one transaction | [3JTkP82dMB…](https://explorer.solana.com/tx/3JTkP82dMB8PBbVb6MB3TcamePA2FGrrASTx7cv9KDVvLJ7dLtjftRbyEGKTB1csAAnN3TZXDCHUynMn5RyQWcmh?cluster=devnet) |
| Carol's note, the same | [5G9UyDsLen…](https://explorer.solana.com/tx/5G9UyDsLenjovrXC5KAv3pYjMtuSjYNVHuPBswQpg3BWweCcv2SGBTVJFtwFTMmvtoJvd3cL2fAfZKo5hZhp64Sy?cluster=devnet) |
| mirror-pool settles both spends in one batch | [3fjFeczHDC…](https://explorer.solana.com/tx/3fjFeczHDCsmaw2kmAH2fAf6Jmt1K4wrfp87xT4zTot34U3MTmRWudkqJrbvMco4j25UAxXhX7RxfEn1a5GxuxDe?cluster=devnet) |

- **Bob:** his note cannot be proved against the published clean set.
- **Each fresh beneficiary:** holds 0.099 SOL, the deposit less the relay's fee.
- **Cost:** the clear-and-spend transaction used 126,407 compute units: 111,461 for `VerifyClearance` and 14,946 for the spend.

The record is [`solana/deployments/devnet-e2e.json`](../solana/deployments/devnet-e2e.json).

## The networks run this source

The [On-chain workflow](https://github.com/thiagorochatr/colosseum2026/actions/workflows/onchain.yml) runs on every push and every day:

- **Moderato runs this source:** the pool, its verifier and the linked Poseidon library on Moderato match this repository's build byte for byte, and the pool's immutables match the deployment record.
- **Devnet runs this source:** both Solana programs are rebuilt from source with the devnet ids and compared byte for byte with what devnet holds. An upgrade that swapped either program would turn this red within a day.
- **The ceremony beacon:** the beacon applied to the key is the blockhash of the Solana mainnet slot announced in advance.

## Checking it yourself

```sh
# The Solana programs on devnet against this source (macOS on Apple silicon reproduces the build)
cd solana && sh scripts/check-deployed.sh

# The Tempo pool and verifier on Moderato against this source
cd tempo && npm ci && forge build && node script/check-deployed.mjs

# The beacon against Solana mainnet
node circuits/ceremony/solana-beacon.mjs 455095000 | diff - circuits/ceremony/beacon.json

# Both flows again, against the live deployments (needs funded demo keys)
cd tempo && NETWORK=moderato node e2e/run.mjs
cd solana/client && NETWORK=devnet node e2e.mjs
```

The live app at [alibi.thiagorochatr.com](https://alibi.thiagorochatr.com) runs the same flows against the same deployments, and its log links every transaction it sends.
