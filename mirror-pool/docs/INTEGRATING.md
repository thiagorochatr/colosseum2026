# Integrating a protocol

Nothing in the on-chain program knows what a stake delegation is. It knows how to
verify a proof, how to burn a nullifier, and how to invoke an instruction it was
handed. Adding a protocol is therefore a client-side exercise: **no program
change, no redeploy, no new circuit, and no governance.**

This document is the whole procedure, with a worked example that runs on devnet.

## The three shapes an action can take

A spend carries a selector, and there are only three.

| selector | what the pool does | what it is for |
|---|---|---|
| `0` transfer | pays the beneficiary from the vault | moving lamports; the degenerate case |
| `1` invoke | calls your program, funded by the vault | anything where the pool is the *payer* |
| `2` invoke-signed | calls your program with the vault as a **signer** | anything where the pool must be the *authority* |

The distinction between 1 and 2 is the one that matters, and it is not a
convenience. A stake delegation requires the staker authority's signature; a
governance vote requires the voter's. No member can supply that signature without
appearing on chain and undoing the point of being in the pool — so the pool
supplies it, from seeds only the program holds.

## The procedure

**1. Work out the instruction you want the pool to issue.** Its program id, its
instruction data, and how many accounts it takes. This is ordinary Solana work and
this repository has no opinion about it.

**2. Decide whether the pool must sign.** If the callee requires an authority
signature, you need `--pool-signs`. If it only needs lamports, you do not — and
you should not ask for it, because the pool's signature is available to every
member and `docs/THREAT_MODEL.md` is explicit that an authority the vault holds is
an authority every member holds.

**3. Spend, naming the call instead of a payee.**

```
mirror spend --program $P --note m1.json \
             --to <account the action centres on> \
             --relay relay.json --relay-fee 100000 \
             --invoke <target program id> \
             --payload <instruction data, hex> \
             --accounts <how many accounts it takes> \
             [--pool-signs]
```

**4. Settle.** Whoever settles supplies the accounts that fill the declared slots.

That is all of it. The proof is generated locally, the relay signs, and the
action executes from the vault beside everyone else's at one timestamp.

## A worked example: delegating stake

This is the example the repository actually runs, and
[`docs/PROOF.md`](PROOF.md) has it on devnet.

`StakeInstruction::DelegateStake` is a bare `u32` discriminant of `2` — four
bytes, `02000000` — and it takes six accounts: the stake account, the vote
account, the clock and stake-history sysvars, the stake config, and the staker
authority. The authority is the last of the six, and it must sign.

```
mirror spend --program $P --note m1.json \
             --to <stake account> \
             --relay relay.json --relay-fee 200000 \
             --invoke Stake11111111111111111111111111111111111111 \
             --payload 02000000 \
             --accounts 6 \
             --pool-signs
```

The stake account's **staker** is the pool's vault, which is what makes the
delegation possible. Its **withdrawer** is deliberately not the pool, for the
reason in the threat model: delegation is safe to hand every member, because the
worst a member can do is re-delegate. Withdrawal is not.

The same shape covers a governance vote, a liquid-staking deposit, or any
`CpiContext`-style call a program exposes — change the program id, the payload
and the account count.

## What the proof promises, and what it does not

**It binds the selector, the target program, the payload, the beneficiary, the
relay, the relay fee, and the number of accounts.** A relay handed your proof
cannot change the program being called, the data being sent, how many accounts it
takes, or who is paid. Any of those alterations produces a different action
binding and the pairing fails.

**It does not bind *which* accounts fill the declared slots.** Settlement is
permissionless, so whoever settles chooses them. For a target that takes its
destination in instruction data this costs nothing, because the payload is bound.
For a target that takes it in an account slot — `DelegateStake` names the
validator in slot 1, not in its four bytes — the settler chooses, and a member
cannot prove in advance which validator they will get.

This is a real limit and it is not papered over. What the repository does about
it is check rather than assert: [`docs/CROWD.md`](CROWD.md) settles six members
delegating to six *different* validators and then reads every stake account back
off the cluster to confirm each member got what the plan said. The read-back is
what turns the limit into a checked claim instead of a hope.

If your integration needs that promise cryptographically rather than by
after-the-fact check, the account list has to enter the action binding — which is
a circuit change and is not what this version does.

## Two constraints worth knowing before you design

**Account count is the currency.** Every account your action names costs the batch
space, and space is what the anonymity set is made of. A settlement holds 64
account locks; three go to the pool, three per member to their record, beneficiary
and relay, and the rest to whatever your action declares. An action naming six
accounts fits fewer members per batch than one naming zero — which is why
[`docs/CROWD.md`](CROWD.md) measures the ceiling per action shape rather than
quoting one number.

**Compute is not free either.** A cross-program invocation costs roughly an order
of magnitude more per member than a plain payment. A batch of delegations large
enough to be worth a lookup table needs a `SetComputeUnitLimit` instruction, and
that instruction brings its own program — which is another account, which is
another lock. The escape from one limit is paid out of the other.

Neither is a reason not to integrate. They are the reason to measure your shape
before promising a crowd size for it, and `mirror crowd` is the tool that does
that measuring.
