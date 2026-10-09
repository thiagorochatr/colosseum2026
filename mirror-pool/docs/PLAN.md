# The design as decided, and where the protocol departs from it

This is the design record: what was decided before the protocol was built, and —
in the last section — the six things that did not ship and why each was a
decision rather than an overrun. It is kept because the departures are the
interesting part; `ARCHITECTURE.md` describes what exists today.

## The thesis

`mirror-pool` is meant to be a crowd-sourced privacy set: users pool their
activity so that an observer sees an action happened but cannot say who
initiated it. The hard part is not the proof system. It is that **an anonymity
set on a public ledger can be partitioned by where each member's capital came
from**, and once you sort a set of `k` members into funding-provenance classes,
the anonymity that remains is the size of the actor's class, not `k`.

It is the problem a privacy pool is most often built around rather than through:
the proof system gets the attention, the funding graph is conceded in a sentence,
and nobody measures what the concession costs. So it is the problem this
contribution takes as its subject.

We do not claim to hide provenance. Value moves one way on a ledger, and no
deposit pool controls where its users' money came from. What we claim is
narrower and defensible:

1. **The action side is closed.** Actions are executed by the pool's vault PDA, so the
   on-chain funding trace of an action leads to the pool and is identical for
   every member.
2. **The membership side is measured, not asserted.** Effective anonymity is
   computed from real mainnet chain data at several label resolutions, and the
   method and its limits are published with the number.
3. **The measurement is load-bearing.** A pool that cannot demonstrate an
   effective-`k` above a floor refuses to settle. The metric is a protocol
   parameter, not a marketing line.
   *(Withdrawn during implementation — see "What actually got cut". A program
   cannot check provenance, so this claim was not deliverable.)*

If we cannot support a claim with a measurement whose method we publish, we do
not make the claim.

## What we will not claim

- Not "unlinkable", not "untraceable", not "anonymous" without a named adversary
  and a stated population.
- Not that the funding-provenance channel is closed. It is reduced on the action
  side and measured on the membership side.
- Not that a single-contributor trusted setup is secure. It is reproducible,
  which is a different and lesser property, and we say which one we have.
- No number in any document may come from a synthetic fixture presented as a
  measurement. Every reported figure names its data source.

## Architecture

### Pool model — fixed denomination, note-based

One pool instance serves exactly one denomination `D`. A deposit escrows exactly
`D` plus an entry fee and inserts a note commitment into an incremental Poseidon
Merkle accumulator. A spend proves membership of some note, publishes its
nullifier, and directs the pool to execute one action disbursing exactly
`D − relay_fee`.

```
note        = (k, r)                 secret nullifier preimage, blinding
commitment  = Poseidon(k, r)         the Merkle leaf
                                     (shipped as Poseidon(k, r, denom_tag) —
                                      binding the denomination in is cheap
                                      defence in depth)
nullifier   = Poseidon(k)            revealed on spend, spent once ever
```

The denomination is a pool constant, so it never needs to be bound into the
commitment and can never disagree with the escrowed amount.

**Accounting invariant**, asserted in tests:
`vault.lamports >= D × (deposits − spends) + rent_exempt_minimum`.

This is deliberate, and it targets two drains that a shielded pool reaches by
building in the obvious direction. Carry the amount as a note field without
binding it to the hidden commitment, and a depositor of one lamport withdraws
the pool holding a valid proof. Scope the nullifier to an epoch — natural once
epochs are how batches form — and a single deposit pays out once per epoch,
forever. A fixed denomination with spend-once nullifiers makes both
unrepresentable rather than merely untested.

### Circuit — 3 public inputs

The statement: *I know `(k, r)` such that `Poseidon(k, r)` is a leaf of the tree
with root `R`, my nullifier is `Poseidon(k)`, and this proof is bound to
`action`.*

| Public input | Why it must be public |
|---|---|
| `root` | the program checks it against its own root history |
| `nullifier` | the program records it to prevent replay |
| `action_binding` | the program recomputes it, so a relay cannot redirect or re-price |

Measured cost is `74,179 + 5,661 × N` CU, so three inputs land near 91k CU. That
is the going rate for on-chain Groth16 on Solana, and the third input buys the
relay binding rather than being spent on a value that could have been folded in.
Any further value we need to commit to gets folded into `action_binding` rather
than added as a fourth input. See `GROTH16_INTEGRATION.md` for the verified
conversion path and its pitfalls.

The action binding covers the action selector, its parameters, the beneficiary,
the relay and the relay fee, so every economically meaningful field is inside
the proof.

### Two-phase epochs — synchronization without a CU explosion

The premise is synchronized crowds: many identical actions landing
together so that timing and ordering carry no signal. Verifying `N` proofs in one
transaction does not fit — three proofs exhaust the transaction size limit long
before the compute limit.

So the protocol splits it:

- **Phase 1, any time during the epoch.** A member submits their proof in its own
  relay-signed transaction. The program verifies it, burns the nullifier and
  writes a ticket PDA recording the bound action. Nothing executes yet.
- **Phase 2, at epoch close.** The pool executes every ticket's action in one
  settlement, sharing a single timestamp and ordering.

The two phases also close a structural leak that a single-phase design walks
into: if settlement is one transaction that every participant must sign, then
that transaction publishes the entire membership set by pubkey, and the
anonymity set is disclosed by the very step meant to protect it. Here no member
key ever appears on chain.

### Relays, and never holding users hostage

A member who pays their own fee signs with their own wallet and destroys their
own anonymity, so spends are relay-signed and the relay is paid out of `D`.

Two rules follow, and both are easy to lose by accident:

- **Relaying is permissionless.** Any key may relay. There is no single immutable
  authority whose loss freezes the pool. Baking a relay authority into the PDA
  seeds is the tempting shortcut, and it is a trap: seeds cannot be changed, so
  without a rotation instruction the authority is permanent and its loss is
  terminal.

  > **Shipped narrower, and deliberately:** any key may relay, but a given proof
  > names the relay it was made for, because the relay is part of the action
  > binding. The property this rule was protecting — no authority whose absence
  > freezes the pool — is untouched: there is still no allowlist and no
  > registration. What changed is that the member chooses their relay instead of
  > the choice going to whoever lands the transaction first. `THREAT_MODEL.md`
  > has the attack that forced it.
- **There is always an exit.** A member may always self-spend, paying their own
  fee and accepting the privacy loss, so funds are never hostage to a relay's
  liveness.

### Incentives

Entry fees accumulate in a reward pool paid pro-rata to *dwell* — how long a note
stayed unspent — which is the behaviour that makes everyone else's set larger.
Payouts are capped per epoch and gated on the `k` floor, so a lone participant
cannot recycle their own fee back to themselves — a reward scheme that pays out
without a crowd gate is not an incentive, it is a refund with extra steps.

> **Shipped instead:** none of this. The dwell mechanism was cut, and once it
> was, the fee funding it had no recipient — so the fee was cut as well, by
> refusing any nonzero value at pool creation. See *What actually got cut*.
>
> What did ship is four incentives the program enforces rather than pays for, and
> the reason the monetary one is harder than it looks: a reward has to name an
> address, and naming a member's address is the one thing this design spends
> everything else avoiding. `INCENTIVES.md` has all four, the gap, and the
> second-nullifier design that would close it.

### Provenance measurement

A separate crate collects real mainnet funding data and reports effective
anonymity at several label resolutions, with the raw sample committed so results
reproduce without RPC access. The methodology is `docs/PROVENANCE_METHOD.md`, and
it is built around three failure modes that make a provenance number read better
than it is: selection that drops whatever is expensive to trace, which inflates
the untraceable bucket in the flattering direction; a sample spanning seconds of
chain time presented as a population; and a class key at raw-address resolution,
which splits two members funded by the same exchange into different classes and
so reports a larger anonymity set than exists.

## What was cut, and why

This document is the design as it was decided, so the places where the shipped
protocol departs from it are the interesting part. Six things here did not ship,
and each was a decision rather than an overrun.

- **`open_epoch` and explicit epoch state.** Folded into `submit_spend` and
  `settle_epoch`, which need neither: a spend records its own timestamp and
  settlement reads the clock. Four instructions instead of seven, and one fewer
  account whose lifecycle could disagree with the pool's.
- **`claim_reward`, dwell rewards, and the entry fee with them.** The rewards
  went first. The fee that funded them went second, and for a sharper reason: a
  fee with no payout is not a simplification, it is a fund trap. Fees would
  accrue on the pool account, no instruction would pay them out, and none could
  be added without an authority this program deliberately does not have.
  `Pool::initialise` now refuses any nonzero entry fee, so the trap is
  unrepresentable rather than documented. The field stays in the layout for a
  version that ships a real payout path.
- **`self_spend`.** Not built because it is not needed: a member relays for
  themselves at zero fee and settles their own batch once the timeout passes.
  The same exit, one fewer instruction, less surface. Pinned by
  `a_member_can_always_exit_without_any_relay`.
- **The `audit` command.** Shipped as `analyze`, alongside `compare` and
  `selection`, which the design did not anticipate needing.
- **"The measurement is load-bearing."** Withdrawn rather than descoped. The
  design above claimed a pool would refuse to settle below a measured effective
  anonymity. A program cannot check funding provenance — the data is off-chain
  and the classification is a judgement — so the on-chain `k_floor` bounds
  program-visible membership only, and the measurement lives beside the protocol
  instead of inside it. Descoping the claim while keeping the language would have
  been the marketing line this document opens by disavowing.
- **Mainnet.** Deliberately not deployed. The trusted setup is reproducible
  rather than secure, so a live pool would be inviting deposits it cannot
  protect. `README.md` states the reasoning where a reader will meet it.

`EFFECTIVE_K` shipped as `docs/PROVENANCE_METHOD.md`.

Two things shipped that the design did not contain at all: a selector that lets
the pool sign a call as a member's **authority**, which is what a stake
delegation needs and a transfer does not, and the measurement programme that
produced the eight runs in `docs/MEASUREMENT_LOG.md`. Both are described where
they live rather than here.
