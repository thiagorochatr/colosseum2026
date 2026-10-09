# Architecture

## What the protocol is

An anonymity set for *actions*. Members deposit a fixed denomination; later, a
member proves in zero knowledge that they own some note in the set and directs
the pool to act. The pool executes. An observer sees that an action happened and
cannot say which member asked for it.

Four crates and one program:

```
mirror-core        field, Poseidon, Merkle accumulator, notes   (linked on-chain)
mirror-circuit     R1CS gadget, membership circuit, prover      (host only)
mirror-pool        the on-chain program
mirror-provenance  funding-provenance measurement               (host only)
mirror-cli         members:  init-pool, note-new, deposit, tree, spend, settle,
                             disclose, disclose-verify
                   operators: setup, verify-setup, soak, crowd, close-table
                   measuring: check-endpoint, seeds, collect, analyze,
                              compare, selection
```

`mirror-core` is shared by the program and the host deliberately: a commitment,
a nullifier and an action binding each have exactly one implementation, so the
two sides cannot compute different answers and discover it in production.

## The note model

```
note        = (k, r)
commitment  = H3(k, r, denom_tag)     the Merkle leaf
nullifier   = H1(k)                   revealed on spend, spent once ever
```

**The denomination is a pool constant, not a field in the note.** One pool serves
one denomination, so the escrowed lamports and the hidden commitment cannot
disagree. This is not a stylistic choice — it makes a class of drain
unrepresentable rather than merely untested, and the class is worth stating
because a shielded pool built the obvious way falls into it. Carry the amount as
a note field and fail to bind it to the commitment, and a depositor of one
lamport withdraws the entire pool holding a valid proof. Scope the nullifier to
an epoch — the natural move once epochs are how batches form — and a single
deposit pays out once per epoch, forever.

Nullifiers here are spend-once, never epoch-scoped.

### Domain separation by arity

Poseidon instances of different width are different permutations, so arity is
itself a domain separator and a free one:

| value | arity |
|---|---|
| nullifier | 1 |
| Merkle node | 2 |
| note commitment | 3 |

The action binding is not in this table. Its payload is variable length and
Poseidon is a fixed-arity compression, so the binding is a keccak digest under
the domain tag `mirror-pool:action:v2`, masked into the field.

An integer tag was the first design and it was wrong: with a small tag constant,
a Merkle node whose left child equals the tag collides with a nullifier.
Reaching that state needs a Poseidon preimage, so it was not exploitable — but
arity separation removes the question rather than bounding it.

## The accounting invariant

```
vault.lamports  >=  denomination × (deposits − settled_spends) + rent
```

The amount owed is a function of two counters and the pool's constant
denomination. Nothing a prover supplies can influence it. It is re-read from the
vault *after* lamports move rather than inferred from the arithmetic that moved
them, and the spend counter refuses to exceed the deposit counter outright.

Escrow lives in its own vault PDA holding no data, so the invariant reads against
a balance containing nothing but escrow and its own rent. Nothing else is ever
credited to it, and pool creation refuses a nonzero entry fee, so there is no
second category of lamports anywhere that could be mistaken for backing for an
unspent note.

`docs/PROOF.md` carries the devnet numbers: five notes settled, 100,000,115
lamports owed and 100,000,115 paid, and a vault that came to rest on its
rent-exempt floor with a remainder of zero. The soak asserts that rather than
printing it, so a run that disagreed would fail instead of publishing.

## The circuit

*I know `(k, r, denom_tag)` such that `H3(k, r, denom_tag)` is a leaf of the tree
with root `R`, my nullifier is `H1(k)`, and this proof is bound to `action`.*

Three public inputs, and that is a cost decision. On-chain verification measures
as `74,179 + 5,661 × N` compute units, so each input costs about 5.7k CU. See
`GROTH16_INTEGRATION.md`; the figure this repository reproduces directly is the
whole `submit_spend` instruction at about 101,000 CU.

| public input | why it cannot be a witness |
|---|---|
| `root` | the program checks it against its own root history — the last `ROOT_HISTORY = 128` roots, so a proof stays valid for 128 deposits after the one it was built against, and no longer |
| `nullifier` | the program records it to prevent replay |
| `action_binding` | the program recomputes it from the action it executes |

`denom_tag` stays a witness because Merkle membership already constrains it: a
pool's tree only ever contains leaves committed at that pool's denomination.

The action binding is squared under constraint. A Groth16 public input
participates in verification only through the R1CS columns that reference it; an
input used in no constraint has an all-zero column, its `gamma_abc` term is the
identity, and *any* value satisfies the equation. Without that one constraint a
relay could swap the action after proving and the proof would still verify.
`crates/mirror-circuit/tests/onchain_layout.rs` tampers with that exact input and asserts the real
verifier rejects it, so the property is checked rather than reasoned about.

### Three-way parity

The gadget, the host and the syscall must compute one function. `solana-poseidon`
is the only Poseidon entry point, and it is cfg-gated upstream to the syscall
on-chain and to light-poseidon off-chain, so host and program agree by
construction. The R1CS gadget then reads light-poseidon's published round
constants rather than re-deriving them.

The gadget and the host are each checked against circomlib's published
`poseidon([1,2])` vector rather than against each other, and the syscall is then
checked against the host on-chain — the end-to-end suite asserts the root the
deployed program builds equals the root the host built. A gadget whose native and
in-circuit hashes are different functions is the classic failure here: nothing
catches it until proving time, and the symptom — proofs that verify nowhere —
points at everything except the hash. This is the test that catches it.

A pure-Rust Poseidon on SBF overflows the 4 KB stack frame and costs roughly
1,500× the syscall even where codegen lets it complete, so no arkworks code is
linked into the program.

## Instructions

**`init_pool`** — permissionless. One pool per denomination, globally: splitting
deposits of the same size across pools splits the anonymity set, and a split set
is worse for every member in it. There is no privileged authority, so no key
whose loss freezes the escrow.

**`deposit`** — escrows exactly the pool's denomination, read from the pool and
never from the instruction, and appends the commitment to the accumulator. The
tree is `TREE_DEPTH = 20`, so a pool holds up to 1,048,576 notes; the program
keeps only the frontier — one node per level — which is enough to append a leaf
and produce a root, and not enough to prove any particular leaf is in the tree.
Recovering the leaves is the client's job, and `mirror tree` does it from the
transaction history.

**`submit_spend`** — verifies the Groth16 proof on-chain, burns the nullifier,
records the authorised action. Pays out nothing.

The action binding is never transmitted. It is recomputed on-chain from the
selector, the target program, the beneficiary, the relay, the relay fee, the
declared account count and the payload, and used as the third public input,
so a relay that alters any of them produces a different binding and the pairing
fails. There is no separate field that could be checked incorrectly.

The relay is taken from the account that signed, never from anything the caller
states. A proof is therefore spendable only by the relay the member made it for,
which is what stops a bystander from lifting it out of an unlanded transaction
and landing it first under their own key.

The relay signs, never the member. A member paying their own fee would sign with
their own wallet and destroy their own anonymity, so no member key appears on
chain on this path.

One limit follows and `docs/THREAT_MODEL.md` states it: the binding fixes *how
many* accounts an action takes but not *which* ones.

### Funding or signing, and why the member picks

| selector | | |
|---|---|---|
| 0 | transfer | pay the beneficiary, no CPI |
| 1 | invoke | fund the beneficiary, **then** call |
| 2 | invoke signed | call with the vault as **signer**, then pay |

Selector two is what lets the pool act as a delegated authority rather than only
as a funder — the thing a stake delegation or a governance vote needs and a
transfer does not.

The split is forced by the runtime rather than chosen. This program moves the
vault's lamports by direct mutation; an account mutated that way and then handed
across a CPI boundary makes the runtime reject the whole instruction as
`UnbalancedInstruction`. So paying first and signing are mutually exclusive, and
which one an action needs is a property of the action. The selector is inside
the action binding, so the choice belongs to the member and settlement cannot
revise it.

That constraint was measured, not reasoned about: the earlier design refused the
vault outright and documented the refusal as a property of the runtime. It is a
property of the *ordering*.

It is also a property of the **batch**, which only a live cluster showed. The
runtime objects to any lamport this program moved anywhere in the same
instruction, so a signed call settled behind other members' payouts fails where
the same call alone succeeds. Settlement runs every signed call first and pays
afterwards — which it must, because a signed action that could only settle alone
would have to wait out the timeout instead of joining a crowd.

`docs/PROOF.md` carries the case the selector exists for: a real stake
delegation on devnet, `DelegateStake` signed by the vault as staker authority,
settled in the same transaction as three plain transfers and a memo.

**`settle_epoch`** — executes a batch in one transaction so every payout shares a
timestamp and an ordering.

The crowd rule is conditional: a batch needs `k_floor` spends, **or** every spend
in it must have waited out the pool's timeout — set at creation, an hour by
default. Requiring the crowd unconditionally is a liveness hazard — a quiet pool
could hold a member's funds until a crowd that never comes. Dropping it makes
"synchronised" a word rather than a property.

The timeout half is opt-in and marked. A settler who wants an under-floor batch
passes `allow_below_floor`, and without it the program refuses rather than
settling; the settlement that does land logs the count it carried against the
floor it missed. Neither closes the hole — `THREAT_MODEL.md` argues why it stays
open — but a batch that costs its members their crowd is now something somebody
asked for and something the chain records.

### The permissionless exit

There is no `self_spend` instruction because none is needed. A member acts as
their own relay with a zero fee, and settlement is already permissionless, so
they settle their own batch once the timeout passes. Nothing in the protocol can
hold their escrow.

The cost is the expected one: their own wallet signs, giving up the anonymity the
relay path provides. It is an escape hatch, not a mode of operation.

## What the k floor does and does not do

`k_floor` bounds **program-visible membership**: how many notes the tree holds.
That is all a program can check, because the thing that actually shrinks an
anonymity set is not visible on chain.

An observer can partition members by where their capital came from. Learning a
member's funding class leaves only that class to guess within, so the anonymity
that survives is the size of the class rather than `k`. No deposit pool controls
where its users' money came from.

So the protocol does two things about it, and claims exactly those two:

1. **The action side is closed.** Actions are executed by the pool's vault PDA, so the
   on-chain funding trace of an action leads to the pool and is identical for
   every member.
2. **The membership side is measured.** `mirror-provenance` computes it from real
   chain data, and the method and its limits are published with the number.

## The measurement

Two passes, and the split is the point.

**Pass one** walks each member's funding chain by the birth edge — the oldest
value credit, the event that created the account — and writes everything it
observed to a sample file. It is the only networked step.

**Pass two** classifies over the complete sample, offline. Two of the five
terminal rules are properties of the *set* rather than of an address, so
classifying during traversal would make a member's class depend on visit order.
Given the same sample, pass two always produces the same partition, which is what
lets someone else check a published number without RPC access.

Design choices that exist to avoid specific published defects:

- Edges come from **balance deltas**, not instruction parsing, which is blind to
  every program that moves lamports by direct account mutation.
- The **birth edge** is the oldest credit, so the walk reaches the *start* of a
  wallet's history. Scanning the most recent transactions instead is the wrong
  end of the record for any wallet with more than a handful of them.
- The **hub threshold is decoupled from the paging cap**. Making them the same
  number is an easy collapse, since both answer "how many signatures do we look
  at" — and it turns "reaches an attributable origin" into a synonym for "hit
  the RPC page cap".
- **RPC failures are never evidence.** They are counted separately and excluded
  from the distribution, and above a 1% failure rate the run refuses to print a
  headline rather than printing a warning above one.
- The endpoint is **checked before the run**. A truncated endpoint returns `null`
  rather than an error for pruned history, so a collection against one would look
  healthy and report every old funding event as absent.
- Seeds that are not wallets are excluded on a **definitional** criterion and the
  count is published. Excluding addresses for *looking hard to trace* is a
  different thing entirely — it drops exactly the members that would have
  widened the class distribution, and inflates the result in the flattering
  direction.

The headline is the loss factor `ρ = 2^−H(C)` rather than effective-k, because it
is independent of `k` and therefore comparable across pools. Effective-k measured
at small `k` systematically understates the steady-state loss and cannot be
extrapolated upward.

### Three uncertainties, kept apart

They answer different questions, and collapsing any two of them into one number
is how a provenance figure comes to mean less than it appears to.

| | question | mechanism |
|---|---|---|
| unresolved bracket | what if the unresolved had landed differently? | exact bound over both extremes |
| sampling spread | how much of `ρ` is *which* members we drew? | bootstrap over members, seeded and published |
| selection | are the resolved members a fair draw of the classes? | one frame at two budgets, cheap vs expensive to trace (see `README.md` for which published pair is a genuine budget margin) |

The third is the one usually left as an assumption. `mirror selection` tests it,
and **separation is the bad outcome**: it would mean the unresolved are not
missing at random, that the resolved subset is biased toward whatever is cheap to
trace, and that no extra budget repairs it. Measured on both populations here, it
does not separate.

`mirror compare` then bootstraps the *difference* between two populations rather
than subtracting point estimates, and refuses to rank them when the interval
contains zero or when either side resolves under half its members. It has refused
on both grounds — on the first for one pool measured at two budgets, which ought
not to separate, and on the second for the cross-population comparison this
project most wanted.

One bias is not fixed and runs against us: plug-in entropy is biased low at small
`n`, so `ρ` is biased **high** and the pools plausibly leak less than reported.
It falls the same way on every population measured the same way, which is what
keeps a difference meaningful where an absolute number is shaky.

`2^H(C)` — entropy over the class-size distribution — is widely quoted as the
effective anonymity set and is **inverted**: it is maximised when every member
stands alone, which is total deanonymisation. It is the leakage. The anonymity is
`2^H(X|C)`.
