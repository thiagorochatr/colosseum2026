# Live-cluster evidence

Every line here is a transaction that landed. Nothing is simulated: the proof is produced by the host prover and verified by the deployed program's own Groth16 syscall.

- cluster: `devnet`
- program: `8H3cYoiAA9LM36cyPr4UEv38dhHasSu2XPSdiBfyrLEa`
- pool: `4hHZHNL4fdgbqhso1nh5qCzrbt67GXRQ8h5C62fGEkvt`
- vault: `21Az6ixXzVDD7pncK3RRMYoTphQE7MsmvamhhwmAJQPK`

## Flows

| step | signature | slot | note |
|---|---|---|---|
| init_pool | [`2Jd2TR87LaKaNr8Pa34PnbnnpgFZ7QJtPPhZYSMTimCvo58t4zbHD9Td1bTQtrxBkuenUv5BZcjRELtJYCe4nzLd`](https://explorer.solana.com/tx/2Jd2TR87LaKaNr8Pa34PnbnnpgFZ7QJtPPhZYSMTimCvo58t4zbHD9Td1bTQtrxBkuenUv5BZcjRELtJYCe4nzLd?cluster=devnet) | 481486119 | denomination 20000029, k_floor 4 |
| deposit | [`57vE8o7qvfVuVnozMq1bSeNrJuigYYpkfuFjo1Nspi4tX3Pa9bitPvZU2ebsQLA4uKMZrRXtnUofMMPuDDz87GqC`](https://explorer.solana.com/tx/57vE8o7qvfVuVnozMq1bSeNrJuigYYpkfuFjo1Nspi4tX3Pa9bitPvZU2ebsQLA4uKMZrRXtnUofMMPuDDz87GqC?cluster=devnet) | 481486125 | note 1 |
| deposit | [`4nDwWr1QonzzV94EJyxXy4QbMDB5EmY3y8N8eZa5iRDnaL9ukgseYkWireERqRDSJCVRWjd24qzPnyxWZS7aaKnW`](https://explorer.solana.com/tx/4nDwWr1QonzzV94EJyxXy4QbMDB5EmY3y8N8eZa5iRDnaL9ukgseYkWireERqRDSJCVRWjd24qzPnyxWZS7aaKnW?cluster=devnet) | 481486130 | note 2 |
| deposit | [`3ySCPPmNytc8cPhmeqY8QHuhbW4XWk5bXTTHR3xTPuLeLCYSS6hjQ748zThLK3ysxoDXoDu8RztoXFAe2wxzNvQe`](https://explorer.solana.com/tx/3ySCPPmNytc8cPhmeqY8QHuhbW4XWk5bXTTHR3xTPuLeLCYSS6hjQ748zThLK3ysxoDXoDu8RztoXFAe2wxzNvQe?cluster=devnet) | 481486136 | note 3 |
| deposit | [`5SKjtdFgkxY8qNUD1MFSDccMVaPdJTnebFFwD7wVHTbGSY3hdXGGKNbgRhBT9v2yWGoN3JGmHKHwe1mYR3ERNSYL`](https://explorer.solana.com/tx/5SKjtdFgkxY8qNUD1MFSDccMVaPdJTnebFFwD7wVHTbGSY3hdXGGKNbgRhBT9v2yWGoN3JGmHKHwe1mYR3ERNSYL?cluster=devnet) | 481486141 | note 4 |
| deposit | [`3ihbqsBgKYiCmtGVyN4fNgDPifyfwFKJ2tSoteSRvRqj1rq7edFLixqxA9i7qVpHEHSTXQ4MV69aEvfHqz7SPeDc`](https://explorer.solana.com/tx/3ihbqsBgKYiCmtGVyN4fNgDPifyfwFKJ2tSoteSRvRqj1rq7edFLixqxA9i7qVpHEHSTXQ4MV69aEvfHqz7SPeDc?cluster=devnet) | 481486147 | note 5 |
| create stake account | [`5Jcgr2aMUCyzk8YJwdZCAwtHuRfHXUMTsbgPKWHHsCkZhtqvx9pEZpFUjjhRSZE1hsf43Po38kiHweKRSC5rMdcP`](https://explorer.solana.com/tx/5Jcgr2aMUCyzk8YJwdZCAwtHuRfHXUMTsbgPKWHHsCkZhtqvx9pEZpFUjjhRSZE1hsf43Po38kiHweKRSC5rMdcP?cluster=devnet) | 481486153 | 1100000000 lamports, staker = the pool's vault, withdrawer = the operator |
| submit_spend | [`WtXw9KtimxeXkyB6ztxN43k4DfMDHJ3yhqdPs5zTzoMK7FGojnkWHKTDVD6bm26dTw9vbwTC8JZ3JjSYSrygTns`](https://explorer.solana.com/tx/WtXw9KtimxeXkyB6ztxN43k4DfMDHJ3yhqdPs5zTzoMK7FGojnkWHKTDVD6bm26dTw9vbwTC8JZ3JjSYSrygTns?cluster=devnet) | 481486180 | note 0, relay-signed |
| submit_spend | [`6BEoAQFrGMCqcJeaaekiS2S8na9MW5X6tpc9YERVkuZvVFBSQbKjA1Bk4YGGcxS2kkRRApKPiY4yjR1U76WBoA1`](https://explorer.solana.com/tx/6BEoAQFrGMCqcJeaaekiS2S8na9MW5X6tpc9YERVkuZvVFBSQbKjA1Bk4YGGcxS2kkRRApKPiY4yjR1U76WBoA1?cluster=devnet) | 481486207 | note 1, relay-signed |
| submit_spend | [`5joi2J71sCz9bPVLZBeVoUEDchJE8oD6FF2aH4o2bokjWUx9oW1aWdicUWifbqAstyTtwEJUfR5a6gG1mxQGUk46`](https://explorer.solana.com/tx/5joi2J71sCz9bPVLZBeVoUEDchJE8oD6FF2aH4o2bokjWUx9oW1aWdicUWifbqAstyTtwEJUfR5a6gG1mxQGUk46?cluster=devnet) | 481486235 | note 2, relay-signed |
| submit_spend | [`WqcPensA2X37HGSC98jai5ceEKbxAT4AKMANBcDVwcaYWzxhnjkf9SyxDTee35qMERnYzcE8Kw6eLD99Q9cHUbT`](https://explorer.solana.com/tx/WqcPensA2X37HGSC98jai5ceEKbxAT4AKMANBcDVwcaYWzxhnjkf9SyxDTee35qMERnYzcE8Kw6eLD99Q9cHUbT?cluster=devnet) | 481486261 | note 3, relay-signed, action: pool-signed CPI to SPL Memo |
| submit_spend | [`3mwBWN29M3uRGAmtiAEMUCERhJdjoRfhvo8RFWSwGmFL3XvzoZuJabLdCbdoYVhZC88JCNwkhUbEXEriFLkQqMRt`](https://explorer.solana.com/tx/3mwBWN29M3uRGAmtiAEMUCERhJdjoRfhvo8RFWSwGmFL3XvzoZuJabLdCbdoYVhZC88JCNwkhUbEXEriFLkQqMRt?cluster=devnet) | 481486289 | note 4, relay-signed, action: pool-signed stake delegation |
| settle_epoch | [`5cw6HK2Xb6KrkR2qv5ECofSZN59hDmxdeKkgTiV4yVVWgucqtwAujMXKeCmxWBgNG4JHKQUG94GGfWqoHu9Anvqc`](https://explorer.solana.com/tx/5cw6HK2Xb6KrkR2qv5ECofSZN59hDmxdeKkgTiV4yVVWgucqtwAujMXKeCmxWBgNG4JHKQUG94GGfWqoHu9Anvqc?cluster=devnet) | 481486295 | 5 spends in one transaction, 2 of them a CPI the pool signed |
| deposit | [`3UH1uZao97QjT6vz69XShcC7GkMG5AddqRz1wtp3vzNYTujHuPkNDft7STECkJZgqnmZTUCA9g3mJFyw4VrAQT5q`](https://explorer.solana.com/tx/3UH1uZao97QjT6vz69XShcC7GkMG5AddqRz1wtp3vzNYTujHuPkNDft7STECkJZgqnmZTUCA9g3mJFyw4VrAQT5q?cluster=devnet) | 481486302 | note 6 |

The slots are there because **devnet history is pruned**. Every signature above resolved through `getTransaction` when this file was written, and a reader coming to it later may find that call returning null for a transaction that did land. That is the cluster forgetting, not the evidence being wrong, and the way to tell the difference is `getSignatureStatuses` with `--search-transaction-history`, which still answers for a pruned transaction — checked against the slot in this table.

## Vault accounting

The accounting invariant is a statement about the vault's lamports, so here are the lamports. The vault holds escrow and carries no data, which is what makes its floor the rent-exempt minimum for a zero-byte account — read from the cluster during the run, not assumed.

| quantity | lamports |
|---|---|
| denomination | 20000029 |
| notes settled | 5 |
| relay fee (taken out of the denomination, not added) | 200000 |
| vault before settlement | 100891025 |
| vault after settlement | 890880 |
| rent-exempt minimum, 0 bytes | 890880 |
| **paid out** | **100000145** |
| **owed** (denomination × notes) | **100000145** |

Paid out equals owed, and the vault came to rest on its floor with a remainder of 0 lamports. The soak asserts both and fails the run otherwise, so this table cannot record a discrepancy and still exit successfully.

## The pool signed an action, and the callee said so

The settlement above carried 5 spends, and 2 of them were not a transfer: the pool invoked SPL Memo as one member's **authority**, in the same transaction as the other 4. That is the capability a stake delegation or a governance vote needs and a payment does not.

A signature only proves the transaction landed. It says nothing about who signed the instruction the pool made *inside* it, so the evidence has to come from the callee. SPL Memo refuses any account handed to it that has not signed, and names the ones that did:

```
Program log: Signed by 21Az6ixXzVDD7pncK3RRMYoTphQE7MsmvamhhwmAJQPK
```

That is the pool's vault, `21Az6ixXzVDD7pncK3RRMYoTphQE7MsmvamhhwmAJQPK`, which has no private key — it signed through seeds only the program holds. The soak reads this line back from the cluster and fails the run if it is absent, so this section cannot appear without the callee having said it.

## The pool delegated stake, as a member's authority

The same settlement carried a second signed action, and this one is the case the design exists for: a **real stake delegation**. Stake account [`BwxCHw288XQNPiapnUjTJfbYVKMZSsmi49X84jY3rFdG`](https://explorer.solana.com/address/BwxCHw288XQNPiapnUjTJfbYVKMZSsmi49X84jY3rFdG?cluster=devnet) is now delegated to validator [`2f9C9AU8nFRKUub8NHToNiZzcwmYiNeipVuP8akKgRVv`](https://explorer.solana.com/address/2f9C9AU8nFRKUub8NHToNiZzcwmYiNeipVuP8akKgRVv?cluster=devnet).

`DelegateStake` requires the **staker authority** to sign. No member can be that authority without appearing on chain and undoing the point, so the pool is, and the pool signed. The account state is read back after settlement: a stake account only reaches the `Stake` variant by being delegated — an initialised but undelegated one is a different variant — so the check distinguishes "the instruction landed" from "the delegation took", and the validator's key is read out of the account rather than assumed from what was requested.

**The withdraw authority is deliberately not the pool.** The pool's signature is available to every member, so an authority the vault holds is an authority every member holds. Delegation is safe on those terms — the worst a member can do is re-delegate to another validator. Withdrawal is not, and it stays with the operator. That is `docs/THREAT_MODEL.md` applied rather than repeated.

What is hidden here is precisely one thing: **which member asked**. The stake account, the validator, the amount and the timing are all public, and the anonymity set is the 5 members who settled together. What an observer cannot recover is which of them authorised this delegation rather than one of the plain transfers beside it.

## Rejections

A negative case is only evidence if the program's own error code is what rejected it. A bare runtime failure proves nothing about this program.

| case | error code | what the rejection establishes |
|---|---|---|
| replayed nullifier | `0xf` | a nullifier is spent once, ever -- not once per epoch |
| redirected beneficiary | `0x12` | a relay cannot send a member's payout somewhere the member did not choose |
| inflated relay fee | `0x12` | a relay cannot re-price the work after the member authorised it |
| front-run relay | `0x12` | a proof is spendable only by the relay the member made it for |

The last three attack a note that is still live, deposited after settlement precisely so that they would have to. Against an already-spent note the replay guard fires first and the rejection would say nothing about the check under test — which is how a negative case comes to pass for the wrong reason.

## The key those proofs were checked against

Every proof above was verified on chain against `programs/mirror-pool/src/vk.rs`, and that file is generated. A wrong byte in it is not a compile error, not a failure anywhere else in the suite, and not visible in a diff anyone reads carefully — so the binding between the committed circuit and the deployed key is asserted by a test rather than left to inspection.

`programs/mirror-pool/tests/vk_drift.rs` regenerates the key from the committed seed under the committed `Cargo.lock` and compares it to the program's own constants **element by element**, then separately checks that the digest `README.md` publishes is the digest of that key. Two tests rather than one, because if both fail the key moved and if only the second fails the documentation is stale — and knowing which without reading any code is the point.

The same check by hand, against the deployed program:

```
mirror verify-setup --expect <the digest in README.md>
```

What this establishes is *reproducibility*, not security. The seed is public, so the toxic waste is public, so proofs against this key are forgeable — which is why the program is on devnet and stays there. A real multi-party ceremony is the prerequisite for anything value-bearing, and `docs/THREAT_MODEL.md` says so rather than leaving it to be discovered.

## Scope

Devnet is devnet. This is a live-cluster functional proof, not a claim of mainnet operation or of an anonymity crowd: the run is scripted by one operator with a handful of notes. What it establishes is that the circuit, the prover, the byte layout and the deployed program agree on a real validator.
