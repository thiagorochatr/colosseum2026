# Incentives

A pool that nobody stays in has an anonymity set of one, so the question of what
keeps members in it is a design question and not a marketing one. This document
answers it in three parts: what the protocol actually does to align a member's
interest with everyone else's, what it deliberately does *not* pay for, and what
the missing piece would have to look like to be worth shipping.

The short version: **the incentives here are structural, not monetary.** Nothing
in this protocol pays you. Several things in it make the cooperative move the
only one available, or the cheapest one, and those are enforced by the program
rather than recommended by a README.

## The four that are enforced

Each of these is a rule in the on-chain program, not a convention. The citation
is where to check it.

### You cannot act until a crowd exists

`processor.rs:342` refuses `submit_spend` when the pool holds fewer notes than
its `k_floor`:

```rust
if deposit_count < k_floor as u64 {
    return Err(MirrorProgramError::BelowAnonymityFloor.into());
}
```

This is the load-bearing one. In a pool with no floor, the first member to act
acts alone, gets nothing from the pool, and learns that arriving early is
punished — so nobody arrives early and the pool never starts. The floor inverts
that: your deposit is what lets *other* people act, and theirs is what lets you.
Waiting is not patience, it is the protocol.

What it bounds is program-visible membership — how many notes the tree holds. It
is not the effective anonymity set, which is smaller, and `THREAT_MODEL.md` says
so at length rather than letting the floor stand in for a guarantee.

### Waiting is never a hostage situation

A floor with no escape is coercion, not an incentive: it would mean a quiet pool
holds your escrow indefinitely and no authority can release it. So
`SETTLE_TIMEOUT_SECONDS` (`processor.rs:510`) is one hour, after which a batch
settles whether or not it reached the floor.

The two rules only work together. The floor is what makes waiting valuable; the
timeout is what makes waiting safe. Removing either one turns the other into a
reason to leave.

The timeout has a cost and `THREAT_MODEL.md` argues it rather than hiding it: a
batch that settles on the clock has no floor, so a batch of one is reachable by
anyone willing to wait the pool's timeout out. `mirror settle` refuses to publish
one unless told to with `--allow-below-floor`, and the program marks the
settlement that results.

### Somebody else is paid to sign for you

A member who signs their own spend publishes the link the pool exists to break.
So spends are relay-signed, and the relay is paid out of the denomination — the
member receives `denomination - relay_fee`.

That fee is the only value transfer in the protocol, and it exists to make the
anonymity-preserving path economically available rather than merely permitted.
Relaying is permissionless: no allowlist, no registration, no authority whose
absence freezes the pool.

This is also why the relay is inside the action binding. A fee that can be
sniped by whoever lands the transaction first is a fee no relay can count on, and
a pool with no relays is a pool where members submit from their own wallets and
deanonymise themselves. `THREAT_MODEL.md` has that attack in full. **Protecting
the incentive layer is the same work as protecting the privacy layer** — that is
not an analogy, it is the same code.

### Paying what everybody else pays is enforced, not advised

A member is paid `denomination - relay_fee`, and that figure lands in a public
account balance. A batch mixing fees therefore pays visibly different amounts and
an observer partitions it by value without breaking a proof.

`processor.rs:634` refuses such a batch outright with `FeeNotUniform`, and
`mirror settle` groups pending spends by fee and settles the largest group rather
than assembling a transaction the program will reject.

So the incentive to converge on a common fee is not a suggestion in a usage
guide. Deviating does not get you a worse anonymity set; it gets your batch
refused until you settle with members who paid what you did.

## The one that is not there

**Nothing pays a member for dwell** — for leaving a note unspent longer than they
needed to, which is the behaviour that makes everyone else's set larger.

`PLAN.md` designed exactly that: entry fees accumulating in a reward pool, paid
pro-rata to dwell, capped per epoch and gated on the floor. None of it shipped,
and `Pool::initialise` now refuses a nonzero entry fee outright
(`state.rs:208`), so the fee that would have funded it cannot even be collected.

The reason the fee went too is worth stating, because it is the smaller mistake
that would have looked like the safer one. A fee with no payout path is not a
half-built feature, it is a fund trap: lamports every depositor pays, that no
instruction can return, and that no authority exists to release — because this
program deliberately has no such authority. Shipping the fee and documenting the
gap would have left the trap armed behind a paragraph.

## Why the obvious fix is the wrong one

A reward has to be paid to somebody. Paying somebody on Solana means naming an
address in a transaction. And the entire claim of this protocol is that **no
member key appears on chain at any point after the deposit** — spends are
relay-signed, settlements are settler-signed, and actions execute from the pool's
vault.

So a dwell reward paid to a member's address does not cost a little anonymity. It
costs all of it, for that member, permanently, and it does so at exactly the
moment they are being rewarded for having protected everyone else's. The
mechanism would pay people to destroy the thing it exists to reward.

This is not hypothetical, and it is the shape the problem takes wherever it is
attempted: a reward path that pays an identity only functions on a path that
*has* identities — which is the path without anonymity. An anonymous path and a
per-identity reward counter are mutually exclusive by construction, not by
oversight.

## What would actually work, specified

The anonymity-preserving version is a second nullifier, and it is a real design
rather than a hand-wave:

- A note commits to a second secret alongside its spend secret, so a member holds
  two independent nullifiers for one note.
- `claim_reward` takes a Groth16 proof of the same membership statement the spend
  path uses, over the *reward* nullifier, and pays a fresh address the prover
  names in the action binding.
- The claim is gated on the same `k_floor`, for the same reason: a reward
  claimable by a crowd of one is a refund with extra steps, and it would let a
  lone participant recycle their own fee back to themselves.
- Dwell is measured from the note's leaf index and the claim's slot, both already
  on chain, so no per-identity counter is stored and none can be read.

That is a second value-bearing instruction, a second nullifier domain, a changed
note format, and a changed circuit — and every one of those is a place where the
audit trail in this repository found somebody else's bug. This repository's
standard for a value-bearing path is that it is exercised against a real SVM with
negative cases carrying the program's own error codes, and against a live cluster
with the signatures published. That standard is the reason to defer this rather
than the obstacle to it.

**So it is scoped, not vague, and absent rather than half-present.** The pool
ships with the incentives it can enforce, and without the one it cannot yet pay
safely.
