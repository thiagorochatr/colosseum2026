# Threat model

## What this protects

An observer sees that an action happened and cannot say which member asked for
it. That is the whole claim, and everything below is either how it is achieved or
where it stops.

## The adversary

Passive, global, retrospective, chain-only. They read every transaction ever
made, run any analysis they like over it, and never need to compromise a key or
a machine. They may also label addresses using off-chain knowledge — exchange
deposit addresses, published attributions, their own records.

They cannot break BN254 discrete log, invert Poseidon or find keccak collisions.

## What holds

**Deposit and action are unlinkable through the proof.** A spend proves
membership in the accumulator without naming a leaf. Against an adversary who
sees only the chain, the posterior over which member acted is uniform on the
pool's notes.

**No member key appears on chain.** Spends are relay-signed and settlements are
settler-signed. A member's wallet never touches the protocol after depositing.

**A relay cannot alter what was authorised.** The action binding covers the
selector, the target program, the beneficiary, the relay, the relay fee, the
declared account count and the payload, under a domain tag, and is recomputed
on-chain rather than transmitted.

**A proof is spendable only by the relay it was made for.** The relay in that
binding is read from the account that signed, so a proof cannot be carried by a
key the member did not name.

**Actions carry one caller.** Every action is invoked by the pool program on a
member's behalf and funded out of the pool's vault, so the on-chain trace of a
stake made through the pool is identical whoever asked for it. Under
`SELECTOR_INVOKE_SIGNED` the vault also signs the call, so the pool can be the
*authority* for an action and not only its funder — with a consequence stated
below.

**Payouts share a timestamp.** Settlement batches, so arrival time does not
separate members within a batch.

**Escrow cannot be drained.** The denomination is a pool constant and nullifiers
are spend-once, so `vault ≥ denomination × outstanding` is a function of two
counters that nothing a prover supplies can influence.

**Nothing can hold a member's funds.** A member spends as their own relay and
settles their own batch after the timeout. No key's absence freezes anything.

## What does not hold

### Funding provenance — the open channel

An adversary can partition members by where their capital came from. Learning a
member's class leaves only that class to guess within, so the anonymity that
survives is the size of the class rather than `k`.

**This is not closed and cannot be by a better circuit.** No deposit pool
controls where its users' money came from. What we do instead:

- the *action* side is closed, because actions execute from the pool's vault PDA;
- the *membership* side is measured from real chain data, and the method, the
  failure census and the sampling frame are published beside the number.

`k_floor` bounds program-visible membership only. That is all a program can check.

### The trusted setup is not secure

It is *reproducible*, which is a different and lesser property. The seed is
public, so the toxic waste is public, so **proofs are forgeable by anyone who
runs the setup**.

`mirror verify-setup` is what reproducibility buys. It re-derives the key from the
public seed and this circuit and compares it **element by element** — alpha, beta,
gamma, delta and every IC point — against the key compiled into the program. The
digest it should print is

    b0165d5eac6fe8273b6564c78e8ba548c97e6050ae785e9142de63c81aa905b7

Checking the whole key matters, and it is a place where a ceremony verifier is
easy to get subtly wrong: comparing only `delta` — the element a contribution
actually changes — leaves `alpha`, `beta`, `gamma` and the `IC` vector
unchecked, so the verifier would happily certify a key belonging to an entirely
different circuit. This one compares every element. Reproducibility is worth
having and it is not security.

Production needs a multi-party ceremony. The scaffolding for one is not in this
repository and we do not claim it is.

### The program is upgradeable

Whoever holds the upgrade authority can replace the code, including with code
that steals escrow. This is a named trust assumption, not a property.
`solana program set-upgrade-authority --final` removes it and correspondingly
removes the ability to fix anything.

### The pool's signature is available to every member

`SELECTOR_INVOKE_SIGNED` hands the vault to the callee as a signer, so the pool
can act as a delegated authority — a stake authority, a governance voter —
rather than only as a source of funds. That is the capability the behavioural
case needs, and it comes with a property worth stating plainly:

> **Any member can make the pool sign anything, at any target program.**

That is safe here only because the vault owns nothing but its own lamports, and
those can be debited by this program alone. The signature therefore grants
authority over nothing. It stops being safe the moment the pool is made an
authority over shared state: make the vault the withdraw authority of a stake
account, and any member can withdraw it. Anyone integrating this pool as an
authority is inheriting that, and no on-chain check here can prevent it, because
the payload is opaque by design.

The obvious attack is tested rather than argued.
`the_pools_signature_cannot_be_turned_against_its_own_vault` points the pool's
own signature at the System Program with a well-formed transfer draining the
vault to an address the member picked. Every ingredient is legitimate — genuine
proof, owned note, offered selector — and this program never inspects the
payload, so nothing here refuses it. The runtime does:
`ExternalAccountLamportSpend`, *instruction spent from the balance of an account
it does not own*. The attack reaches the System Program, which is the proof that
the signature really was granted, and dies on the ownership rule. That rule is
what the safety rests on, which is worth knowing precisely, because it stops
holding the moment the vault acquires a second owner.

The ordering constraint behind the two selectors is real and was measured rather
than reasoned about. `SELECTOR_INVOKE` funds the beneficiary *before* invoking,
so the target sees the value; the runtime then rejects the vault crossing the CPI
boundary with `UnbalancedInstruction`, because this program mutated its lamports
directly beforehand. `SELECTOR_INVOKE_SIGNED` pays *after*, which leaves the
balance untouched at the moment of the call. Both orderings cannot hold at once,
so the member picks, and the selector is inside the action binding — a settler
cannot obtain the pool's signature for a proof that did not ask for it, and is
refused by name if it tries.

The constraint is a property of the **batch** and not of the spend, which a live
cluster established and the tests had missed. The runtime objects to any lamport
this program moved anywhere in the same instruction, so a signed call settled
behind three transfers fails even though the same call settled alone succeeds.
Settlement therefore runs every signed call before any payout in the batch.
`a_signed_action_settles_inside_a_batch_of_plain_transfers` is the regression
test, and the shape matters on its own: a signed action that could only settle
alone would have to wait out `SETTLE_TIMEOUT_SECONDS` instead of joining a crowd,
surrendering the shared timestamp that makes the crowd worth standing in.

`the_pool_signs_an_action_as_its_own_authority` asserts the capability against
the real SPL Memo program, which refuses any account handed to it that has not
signed and names its signers in its logs. The test reads that log for the
vault's own pubkey, so the claim rests on a third-party program's behaviour
rather than on ours. `docs/PROOF.md` carries the same evidence from devnet, read
back off the cluster by the soak rather than asserted by it.

The devnet run also applies this section rather than restating it. It delegates
a real stake account with the vault as **staker** authority and the operator as
**withdraw** authority, because delegation survives being available to every
member — the worst any of them can do is re-delegate to another validator — and
withdrawal does not. An integrator who gives the vault a withdraw authority has
given it to the whole pool.

### The action's account list is chosen by the settler

The proof binds the selector, the target program, the beneficiary, the relay
fee, the payload and **how many** accounts the action takes. It does not bind **which** accounts fill
those slots — settlement is permissionless, so whoever settles picks them.

For a target whose destination is instruction data this changes nothing. For a
target whose destination is an *account* — an SPL token transfer, for instance —
a settler could point the action at accounts the member did not choose. The
member's own escrow is not at risk, because only this program can debit the
vault, but anything the action itself would move is.

Binding an account-list commitment into the proof would close this. It is not
implemented, and the claim elsewhere that "a relay cannot redirect an action" is
about the selector, target, fee and payload, not about the account list.

### The submission phase is public, and it is the weaker half

Settlement is one transaction, one signer and one timestamp, and that is the
property the design is built around. It is also only half of what an observer
sees, and the other half deserves stating plainly rather than being left implicit
in a claim carefully scoped to the settled transaction.

`submit_spend` is **one transaction per member**, at a moment of the relay's
choosing, and it publishes in cleartext the beneficiary, the selector, the target
program, the payload and the fee — everything about the action except who asked
for it. Three things follow, and all three are real:

- **Timing.** A relay that submits immediately on request leaks when the member
  asked. This is relay policy, not a protocol guarantee.
- **Ordering.** Settlement executes the batch in the order the settler passes the
  records, and nothing shuffles them. A settler who preserves submission order
  makes position in the settled batch a restatement of submission order, and
  arrival order is public.
- **The signer.** The relay signs, so `accountKeys[0]` of a `submit_spend` is the
  relay. If a member relays for themselves — which the protocol permits, and
  which `USAGE.md` documents as the escape hatch — that transaction names them
  beside the action they are about to take, and the settlement's anonymity is
  worth nothing to them.

The last one is the sharpest, because it is not a subtle statistical channel: it
is one public transaction that ends the question. **The tool refuses the version
of this mistake it can detect** — a relay key that has also deposited into the
pool — and cannot detect the rest, because nothing on chain distinguishes a
member's own fresh key from a genuine third-party relay. Funding is where it
usually goes wrong: a relay topped up from the depositing wallet leads back in
one hop.

So the honest statement of what the pool provides is narrower than "one signer,
therefore anonymous". It is: **given that the member never signs and never funds
their own relay, the settled action cannot be attributed to them.** The first
clause is a discipline the member keeps, not a property the program enforces, and
a threat model that omits it is describing a smaller adversary than the one that
exists.

### The relay market is attackable even though the relay's fee is not

Everything in a `submit_spend` travels in clear text, which the section above
treats as a disclosure problem. It is also a transferability problem, and that
half is worth stating on its own because the fix is recent and the reasoning
behind it is easy to get backwards.

A bystander watching an unlanded `submit_spend` can copy the proof and every
field around it. Until the relay joined the action binding, they could name
themselves as the relay and land it first: the recomputed binding was unchanged,
the pairing succeeded, and the fee settled to them. The member's payout was never
reachable — the beneficiary and the amount have always been bound — so this was
theft from the relay, not from the member.

**It was never a profitable theft, and that is the part worth being precise
about.** Whoever lands the transaction funds the spend record's rent, and that
record is never closed, because it *is* the replay guard. At the fee levels this
repository has actually used, an attacker sank roughly twenty times what they
collected. Anyone presenting this as a drain would be overstating it.

The reason to close it anyway is that **an attacker does not need to be paid to
be effective**. Front-running every spend costs them rent each time and denies
every relay their fee. A relay whose fee can be sniped at will is a relay nobody
runs, and a pool with no relays is a pool where members submit from their own
funded wallets — which is precisely the disclosure the section above calls the
sharpest one available. The attack never touched the cryptography. It made the
honest path uneconomic and let the members deanonymise themselves.

So the relay is now read from the account that signed and folded into the
binding, and a proof is spendable only by the relay it was made for. What remains
open is unchanged and stated where it belongs: a member who relays for themselves
still names themselves, and no binding can prevent that.

### A large settlement publishes its participant list early

A batch that outgrows the 1232-byte packet settles through an address lookup
table, and that table is an account: created by the settler, holding every
address the settlement is about to touch, and **on chain before the settlement
lands**. It discloses nothing the settlement does not disclose a slot later — the
same records, beneficiaries and relays, in the same order — but it discloses it
*earlier*, and while it exists it is a durable, queryable index of who settled
together, tied to the key that created it.

Two consequences worth stating rather than discovering:

- **Timing.** An observer watching the lookup table program sees the batch
  assembling before it executes. That is a warning, not a linkage: the addresses
  in the table are the same public addresses the settlement names.
- **Persistence.** A table left behind outlives the transaction that needed it,
  and one per batch accumulates into a permanent record of every cohort a pool
  ever settled.

So settlement deactivates the table immediately and `mirror close-table` removes
it once the runtime's cooldown has passed, which returns the rent and — the
reason that matters here — deletes the list. The cooldown means there is a window
in which the table exists and cannot yet be closed; nothing shortens it.

Legacy settlement publishes no table at all, which is why it stays the default
for any batch that fits without one.

### The relay fee is a payout amount, and a mixed batch is a partitioned one

The denomination is uniform by construction, but a member receives
`denomination - relay_fee`, and that figure lands in a public account balance. So
the fee is not a private arrangement between a member and their relay: it is a
number an observer reads off the settlement.

A batch whose members paid different fees settles into visibly different payouts,
and an observer partitions it by value. No proof is broken and no secret is
learned — the balances are simply different, and the crowd rule, the shared
timestamp and the single settling signature are all defeated by arithmetic.

**The program refuses such a batch** (`FeeNotUniform`, code 25). The check is in
settlement rather than submission because it is a property of the batch and not
of any record: a member may agree any fee with their relay, and settles with the
members who agreed the same one. `mirror settle` groups pending spends by fee and
settles the largest group.

What this does *not* fix: a fee that is unusual is still a small crowd. A member
who negotiates a fee nobody else pays settles alone, or with the few who match —
and a batch of one is not an anonymity set whatever the program allows. The
uniform-fee rule turns a silent partition into a visible one; choosing a common
fee is still the member's job, and a pool whose members all pay the tool's
default is better off than one where they do not.

### Amounts are public

Fixed denominations mean the amount is a pool constant rather than a secret. A
member who needs an unusual amount is identifiable by the pool they chose. There
is no confidential-value layer here.

### The crowd rule is threshold-or-timeout, and the timeout side has no floor

A batch settles if it carries `k_floor` spends **or** if every spend in it has
waited out the pool's settlement timeout — a value fixed at creation, defaulting
to an hour. The second clause has no minimum size. **A batch of one settles, and
executes.**

This is the standard trade in mix design, and the standard analysis of it is
Serjantov, Dingledine and Syverson, *From a Trickle to a Flood: Active Attacks on
Several Mix Types* (Information Hiding 2002), which examines
threshold, timed, and threshold-or-timed batching and finds the disjunction
inherits the weakness of its weaker half. The argument for our case does not need
the paper, though — it follows from the code:

- **Settlement is permissionless**, so an adversary may be the settler. They
  choose the moment and the composition of every batch they send.
- A spend submitted at `t` becomes settleable **alone** at `t + timeout`,
  regardless of what else is pending.
- So for any member whose spend outlives the timeout without company, an
  adversary can settle it by itself, and that member's anonymity set is one.

They do not even have to be adversarial. On a quiet pool this is simply what
happens, and nothing in the program prevents it: `k_floor` bounds a batch that
settles *by crowd* and bounds nothing about a batch that settles by clock.

**Why it is still the right trade.** The alternative is an unconditional floor,
and an unconditional floor means a member's escrow is held hostage to the arrival
of strangers. A pool that never reaches `k_floor` again would freeze every note
in it, permanently, with no authority able to release them — the program has no
such authority by design. Given the choice between "your action may be
attributable" and "your money may be unrecoverable", this design takes the first
and says so, rather than advertising a floor it would have to break to keep
anyone solvent.

**What a member can do about it.** The protection is traffic, not the program.
Submitting into a pool that already has spends pending is what buys a crowd;
submitting into an empty one and waiting is what does not. The tool reports the
pending count before it settles and says plainly when a batch is below the floor,
because a member who is about to settle alone should know that is what they are
doing. What the tool cannot do is manufacture other members.

**What the program does about it**, short of refusing the trade. Two things, and
neither of them changes what is permitted:

- **The settlement has to be asked for.** `SettleEpoch` carries an
  `allow_below_floor` flag, and without it an under-floor batch is refused with
  its own error code (`BelowFloorNotPermitted`, 26) rather than settled. The flag
  is consent and not a bypass: the timeout still has to have run, and a batch
  that has not waited it out is still refused with `CrowdTooSmall` (21). The two
  codes are distinct so a rejection says which gate closed. `mirror settle`
  requires `--allow-below-floor` for the same reason, and stops with an
  explanation rather than warning and proceeding.
- **The settlement leaves a mark.** A batch that lands below the floor logs the
  count it carried, the floor it missed, and the timeout it settled on. Before
  this, a solo timeout settlement and a full crowd were the same transaction
  shape read two ways — an observer could count accounts, but the floor it fell
  short of was not in the transaction at all. Now the members of that batch can
  tell afterwards what crowd they actually got.

Neither closes the hole. An adversarial settler will happily pass the flag, and
the mark tells you about your anonymity after the fact rather than protecting it.
What they buy is that the trade is now visible in the ledger and deliberate in
the tooling, instead of a default that happens quietly to whoever waited longest.

**The timeout is per pool.** It is set at creation alongside `k_floor`, immutable
afterwards for the same reason, and bounded to between a minute and seven days —
a timeout that never elapses would escrow deposits with no crowd and no clock
able to release them, which is the griefing vector `MAX_K_FLOOR` already guards
against from the other direction. Zero means the program's default of an hour,
which is what every pool created before the parameter existed holds.

**What would fix it properly**, and is not built: a batch that fails to reach the
floor could *refund* the member rather than execute — the escape hatch would then
cost the member their action instead of their anonymity. That is a different
protocol, with a different nullifier lifecycle, and it is named here because it
is the honest answer to this section rather than left for a reader to think of.

### Small crowds

`k_floor` is enforced against notes in the tree, and a pool whose deposits are
mostly Sybils of one actor has a large nominal `k` and a small real one.

Nothing in this program prevents that, and the entry fee that was meant to price
it does not exist: it would have accrued on the pool account with no instruction
able to pay it out, so pool creation refuses any nonzero value. What remains as a
cost to a Sybil is the denomination itself, which is recoverable, plus rent and
fees, which are not — a weak deterrent, stated as one.

The provenance measurement is the honest reading of what a set is actually worth,
and it is the reason this limitation is measurable rather than merely admitted.

### Not audited

No external review. The end-to-end suite runs against the compiled program on a
real SVM, and every negative case but two carries this program's own error code.
The two exceptions are caught by the runtime before this program can rule on
them, and they are marked as such rather than counted as our checks: a truncated
account list, and the attempt to turn the pool's signature against its own vault,
which dies on the System Program's ownership rule. That is evidence of behaviour
and not a substitute for an audit.

## Deliberate non-goals

**Hiding funds** — with a precision this deserves, because the protocol plainly
touches value and a flat "not a mixer" would be too convenient.

What is here: a fixed-denomination escrow whose purpose is to make members
interchangeable and to fund the actions they authorise. Selector one invokes an
arbitrary program, which is the point of the design; selector zero is a plain
transfer, kept because expressing "pay this account" should not require a target
program. That selector does mean a member can deposit a denomination and have it
paid to an address of their choosing, unlinked to the deposit. Calling that
anything other than what it is would be dishonest.

What is *not* here, and is what the word "mixer" usually means: confidential
amounts, a value-shielding layer, or any attempt to obscure how much moved.
Denominations are pool constants and public. Nothing hides quantity.

And what the design is *for* is the behavioural case. The measurement, the
crowd rule, the shared settlement timestamp and the CPI dispatch all exist to
make a stake or a vote unattributable. A design that wanted a value mixer
would not need any of them.

**Defeating an adversary with off-chain data.** Someone who knows a member
deposited — because they watched them do it, or because the member told them —
is not in scope. The proof hides which member acted, not that a given person is
a member.

**Compute-optimal proving.** The circuit is three public inputs at about 91k CU
because that is what binding everything that matters costs. It has not been
tuned further.

## Claim language

Words this project does not use about itself: *anonymous*, *untraceable*,
*unlinkable* without a named adversary and a stated population. Every quantitative
claim names the data it came from, and `docs/MEASUREMENT_LOG.md` records the runs
that produced nothing alongside the ones that did.
