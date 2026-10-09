# Using the pool

Every command below was run against devnet to produce the output shown. Nothing
here is illustrative.

The tool needs no server, no indexer and no account with anybody. It reads what
it needs from the chain each time, which is why a member can act from a machine
that has never seen this pool before, carrying nothing but their note file.

```
cargo build --release        # the binary is target/release/mirror
```

Every command defaults to `--url https://api.devnet.solana.com` and to the Solana
CLI's own keypair at `~/.config/solana/id.json`, so the common case needs neither
flag.

## The shape of it

```
note-new  →  deposit  →  ( wait for a crowd )  →  spend  →  settle
```

A note is a secret you generate locally. Depositing it puts its *commitment* into
the pool's tree and escrows the denomination. Later you prove you own some note
in that tree — without saying which — and the pool acts. Settlement executes
everybody's action together.

## 1. A pool

One pool per denomination, globally, and anybody can create one. Splitting
deposits of the same size across two pools splits the anonymity set, and a split
set is worse for everyone in it.

```
$ mirror init-pool --program 8H3cYoiAA9LM36cyPr4UEv38dhHasSu2XPSdiBfyrLEa \
                   --denomination 31000003 --k-floor 2
pool    4754WzZxtuVat5RZLKkh6JNpGx5pGjz2F1gLjQGLkmnU
vault   2VgGhVgfLuYqxdKMFcRtKwsrXiSjtpBn2y5bGFdFixkq
timeout 3600s   (the program's default: this pool set none)
signature 3UxVSDBCtJPyjdjispAKfJjbUD5272P4Yc6D3SdTrNDomLregNnpkGMJxWowyEH2o4K3MYj7MeEdh65o7UiAgCsV
```

`--k-floor` is the number of notes the pool must hold before it will act at all.
It bounds *program-visible* membership, which is all a program can check — see
`THREAT_MODEL.md` for what it does not bound.

`--settle-timeout` is how long a spend waits before it may settle below that
floor, in seconds, between 60 and 604800. It defaults to 0, meaning the
program's own hour. Like the floor, it is fixed at creation and cannot be
changed afterwards, so a pool that wants something other than an hour has to say
so here.

## 2. A note

```
$ mirror note-new --denomination 31000003 --out m1.json
wrote m1.json
  denomination 31000003
  commitment   01400612e536431914d827909319e451e6539ab823d0496cd48f18655aab248e

This file is the note. Treat it exactly as you would a keypair:
anyone holding it can spend the deposit, and losing it loses the
deposit with no way back.

Next: mirror deposit --note m1.json
```

The file is plain JSON and unencrypted, deliberately. Encrypting it would put a
password between you and your funds and imply a security property this tool
cannot deliver by itself. Back it up the way you back up a keypair.

It will not overwrite an existing note file, and there is no `--force`: an
overwrite here destroys a deposit.

## 3. Deposit

```
$ mirror deposit --program 8H3cYoiAA9LM36cyPr4UEv38dhHasSu2XPSdiBfyrLEa --note m1.json
deposited 31000003 lamports into 4754WzZxtuVat5RZLKkh6JNpGx5pGjz2F1gLjQGLkmnU
leaf      0
signature 4Mg3MhgrNdf583LtxGupULbuC66V1SizFx9qxc6npU4WU2AhnuB5FHQP5XaiBt5Utr3CVYvPoMrs9W3PpcUX7EAz

Keep m1.json safe. It is the only way to spend this note,
and nobody — including this pool's authors — can reissue it.
```

The deposit is public and it is signed by you. That is fine and unavoidable —
what the pool hides is not that you joined, but which member later acted.

The escrowed amount is read from the pool, never from your instruction, so a
deposit cannot claim a size the pool did not set.

## 4. Check the tree — optional, and worth doing once

```
$ mirror tree --program 8H3cYoiAA9LM36cyPr4UEv38dhHasSu2XPSdiBfyrLEa --denomination 31000003
pool          4754WzZxtuVat5RZLKkh6JNpGx5pGjz2F1gLjQGLkmnU
vault         2VgGhVgfLuYqxdKMFcRtKwsrXiSjtpBn2y5bGFdFixkq
denomination  31000003
k floor       2
timeout       3600s
notes         2 deposited, 0 settled, 2 outstanding

  That k is program-visible membership only, and the effective set is smaller.
  Measured against a live pool of comparable shape — Privacy Cash,
  9fhQBbumKEFuXtMBDw8AaQyAjCorLGJQiS3skWZdQyQD — knowing a member's funding
  class left ρ = 0.0955, inside an unresolved bracket of 0.0350 .. 0.1136. Roughly
  an order of magnitude of the nominal figure.
  Method, and what it does not cover: docs/PROVENANCE_METHOD.md.

rebuilding the accumulator from chain history:
  3 transactions touched this pool

  leaves recovered  2
  pool reports      2
  rebuilt root      210e6d7fb10346b463f62d5e351442b6e09e9958c0699310c07ca0835580cea1
  on-chain root     210e6d7fb10346b463f62d5e351442b6e09e9958c0699310c07ca0835580cea1

  the rebuilt tree matches the chain — proofs built from it will verify
```

This is the piece the rest stands on. The program stores only the accumulator's
*frontier* — one node per level — which is enough to append a leaf and produce a
root, and not enough to prove any particular leaf is in the tree. The leaves are
not lost: every commitment was an argument to a `Deposit` instruction, so the
whole set is in the transaction history in insertion order.

Rebuilding it and finding the same root the program holds proves the recovered
set is complete and correctly ordered, which is exactly the precondition a
membership proof needs. `spend` does this rebuild itself; this command just lets
you watch it.

## 5. Spend

**The relay signs, never you.** A member who pays their own fee signs with their
own wallet, and `accountKeys[0]` is then the member — which publishes the link
the pool exists to break. Use a key that is not your wallet and has no history
with it.

The tool refuses outright if the relay you name has also deposited into this
pool, because that is the same mistake wearing a different key: the deposit and
the spend would both carry it, an observer joins them by reading two public
transactions, and the proof stays sound while protecting nothing.

Funding the relay matters too, and no tool can check it for you. A relay topped
up from the wallet that deposited leads back to it in one hop.

```
$ mirror spend --program 8H3cYoiAA9LM36cyPr4UEv38dhHasSu2XPSdiBfyrLEa \
               --note m1.json --to BQ6piLqJD4CWn4V3VyDBh3RpU1FtsPh63P9y4wtwBxJg \
               --relay relay.json --relay-fee 100000
rebuilding the accumulator so this note can be proved a member:
  3 transactions touched this pool
  this note is leaf 0 of 2
deriving the proving key from the published seed (this takes a moment)
proving membership

submitted. The note is spent and the action is recorded.
  nullifier 04db90736f4bf9005b79e9000c510804713ad9fe7f2688c22beed948d18d5883
  record    9gPytZBVVcWDT5MecfkgQGNgo8TLrEeyLd9q44eTW4Vn
  signature 3xmHiy1fLvCDJGRejgLvvKduhdNbajXLYBRjJvpaAfQHB7zz3SpccbK3wH4v63Pyv8136vHZd1A3a4BTx8JMRcQ2

Nothing has been paid out yet — settlement executes the batch, which is
what gives every member's action one timestamp. Run `mirror settle`, or
wait for anyone else to.
```

The relay fee comes *out of* the denomination, never in addition to it, and a fee
at or above the denomination is refused on-chain.

**Pay what everyone else pays.** You receive `denomination − relay_fee`, and that
number is public. A batch whose members paid different fees settles into visibly
different amounts, so an observer partitions it by value without breaking
anything — which is why the program refuses a mixed batch outright and `settle`
groups pending spends by fee. A fee nobody else pays is a crowd of one, and no
rule in the program can fix that for you.

### Doing something other than paying

The interesting case is not moving lamports. `--invoke` calls any program with
any payload:

```
mirror spend ... --invoke <program> --payload <hex> --accounts <n>
```

`--accounts` is how many accounts the call takes, and it is bound into the proof
so a settler cannot add or drop one. Which accounts fill those slots is chosen at
settlement — a real limit, stated in `THREAT_MODEL.md`.

Add `--pool-signs` to hand the pool's vault to the callee as a **signer**, so the
pool acts as your authority rather than only as your funder. That is what a stake
delegation or a governance vote needs and a payment does not; `PROOF.md` has a
real stake delegation done this way on devnet.

A word on what the proof does and does not promise here. It binds the selector,
the target program, the beneficiary, the fee, the payload and the **number** of
accounts — not which accounts fill the slots. For `DelegateStake` the validator
lives in an account slot rather than in the payload, so *the settler chooses your
validator*, and nothing on chain records which one you asked for. Check the
result: read the stake account back and see who it backs. `CROWD.md` is a devnet
run of six members delegating to six different validators that does exactly
that, for every member.

## 6. Settle

Permissionless. Anyone can settle, so no operator's absence can strand you, and
you can always settle your own batch.

Below the crowd floor it tells you what it is waiting for rather than failing
with an error code:

```
$ mirror settle --program 8H3cYoiAA9LM36cyPr4UEv38dhHasSu2XPSdiBfyrLEa --denomination 31000003
looking for spends waiting to settle:
  4 transactions touched this pool

  this batch carries 1 spend(s) and the pool's floor is 2.
  A batch below the floor may settle once every spend in it has waited 3600s;
  the youngest has waited 14s, so 3586s remain.

  This is a liveness guarantee rather than a restriction: nobody can
  hold a member's funds waiting for a crowd that never arrives.
```

Once the crowd is there:

```
$ mirror settle --program 8H3cYoiAA9LM36cyPr4UEv38dhHasSu2XPSdiBfyrLEa --denomination 31000003
looking for spends waiting to settle:
  5 transactions touched this pool

settling 2 spends in one legacy transaction, 437 bytes

settled 2 spends in one transaction
  signature jes9YgrHzYvccsogn1ZGrtDWXtgHuTd2SgWPUucGVTGXMVbAuAnBeV4TGg1BYA1uCuLs1cp9YbgyonZwYqxRsA2

Every payout in that batch shares one timestamp and one ordering, which
is what stops arrival time from telling the members apart.

The 2 members above are that batch's nominal set.
  That k is program-visible membership only, and the effective set is smaller.
  Measured against a live pool of comparable shape — Privacy Cash,
  9fhQBbumKEFuXtMBDw8AaQyAjCorLGJQiS3skWZdQyQD — knowing a member's funding
  class left ρ = 0.0955, inside an unresolved bracket of 0.0350 .. 0.1136. Roughly
  an order of magnitude of the nominal figure.
  Method, and what it does not cover: docs/PROVENANCE_METHOD.md.
```

Both beneficiaries received `31000003 − 100000 = 30900003` lamports, in one
transaction, at one timestamp.

That last paragraph is printed after **every** settlement, deliberately. The
batch size is a nominal anonymity set and it is the number a member walks away
with; the measured discount travels with it rather than living in a document
nobody has open.

### Settling below the floor

Once the timeout has run, a batch smaller than the floor *may* settle — and the
command will not do it on your say-so alone. It prints what the members of that
batch would actually get and stops, and settling it takes `--allow-below-floor`.

That is not bureaucracy. The timeout is a solvency guarantee and it is not an
anonymity one: a batch of one gives its member an anonymity set of one, and the
action becomes attributable to whoever submitted it. The program refuses an
under-floor batch nobody asked for (error `26`), and the settlement that does
land carries a log line naming the count it carried and the floor it missed — so
the members can tell afterwards what crowd they were actually in.

If your funds would otherwise stay escrowed, that is exactly what the flag is
for. If you can wait for company, wait.

`settle` executes plain transfers on its own. A pending CPI action is left alone
and reported, because the record binds *how many* accounts the call takes and
never *which*, so no tool can infer the account list its callee expects.

## 7. Proving it was you — to one person, later, if you want to

Anonymity that cannot be given up on purpose is a liability. At some point a
member may need to show an exchange, an accountant or a counterparty that a
particular action was theirs — and the usual answer to that is a viewing key or
an auditor, which means a standing capability somebody else holds.

There is none here. A disclosure is a **file you hand to one person you chose**:

```
mirror disclose --program <program-id> --note m1.json --out disclosure.json
```

Give them the file. They check it against the chain, and nothing in it is taken
on trust:

```
$ mirror disclose-verify --file disclosure.json
rebuilding the pool from chain history:
  14 transactions touched this pool

disclosure for leaf 0 of Cqg4gj4zwHZGfp1P2v6j6pB4dLbWgsL1vJB7YkjsFWAK
  nullifier 25609bc4fe513e50fcc8874e1dbe8d7a556ff8b95724c606baa429aa6fdcaee2
  record    4beEKJNuU4pHM3rUqLAm8ocVkf7kzRdLP2XMECCMwy77

  the secrets recompute to the commitment    PASS
      H3(k, r, denom_tag) = 13833afd2062da2c5a0217dd53b14c8118a10ded98d84f8af71853383509f2fd
  the pool matches the denomination          PASS
      Cqg4gj4zwHZGfp1P2v6j6pB4dLbWgsL1vJB7YkjsFWAK is the pool for 43000007 lamports
  the commitment is the stated leaf          PASS
      leaf 0 of 6 is 13833afd2062da2c5a0217dd53b14c8118a10ded98d84f8af71853383509f2fd
  the rebuilt root matches the chain         PASS
      6 leaves rebuild to 039bbb3012b6ff6554141437df6da9c12db6a5333c3c38e00146c85df87bfe08
  the secrets recompute to the nullifier     PASS
      H1(k) = 25609bc4fe513e50fcc8874e1dbe8d7a556ff8b95724c606baa429aa6fdcaee2
  the record address is the nullifier's PDA  PASS
      4beEKJNuU4pHM3rUqLAm8ocVkf7kzRdLP2XMECCMwy77 is the record for H1(k)
  the spend record exists on chain           PASS
  the spend record is settled                PASS
      settled
  the spend record belongs to this pool      PASS
  the record matches the claimed action      PASS
      selector 2 paid 5P9AHY2tGoQ9xedLeyvC5WsU4FXxxH8LgtC2kzRUMZtT, fee 200000, 4 payload byte(s)

VERIFIED. Every value in this file was recomputed from the secrets or read off the
cluster. The holder of this note is whoever asked the pool for the action above.
```

That one is a real disclosure of a **stake delegation** — selector 2, the
pool-signed action from `CROWD.md` — proved by its member after the fact.

Every check is reported separately and a check that cannot be completed is a
failure, never a silence. A file whose stated nullifier disagrees with what your
secrets produce fails on that check while the rest still pass, which tells the
verifier exactly what was tampered with rather than merely that something was.

**It only works after settlement, and the tool refuses before it.** The disclosure
carries the note's secrets, and before the note is spent those secrets *are* the
money — anyone holding them can prove membership and send the payout wherever
they like. After the nullifier is burnt they authorise nothing, and all that is
left in them is the ability to demonstrate the link. That is the whole difference
between a disclosure and handing over a deposit.

**Disclosing costs the other members, and the tool says so before you do it.**
Proving one action was yours removes you as a candidate for every other action in
the pool. If that would leave the remaining set below the pool's floor, the
command refuses and tells you what it would cost:

```
Error: REFUSING: this pool has settled 6 spend(s) and its floor is 6. Naming one
of them as yours leaves 5 unattributed, which is below the floor.

The people that costs are not you. Every settled action is a candidate for every
member; removing one narrows the guess for all the rest, and they did not agree
to it and will not be told.

If you have weighed that and still want to, pass --i-accept-the-cost-to-others.
Nothing here can stop you disclosing out-of-band anyway — the secrets are yours.
This exists so the cost is visible at the moment it is paid.
```

The gate is advisory by construction, and the message says so rather than
pretending otherwise.

## How many members settle together

It depends on what the members are doing:

| the batch | members per settlement |
|---|---|
| plain payments | 10 |
| stake delegations, everyone to the same validator | 7 |
| stake delegations, a different validator each | 6 |

Those are the numbers for a **legacy** transaction, which names every account by
its full 32 bytes. That is what you get with no setup at all, and the 1232-byte
packet is what stops it.

**Bigger batches settle through a lookup table, and `settle` does it for you.**
When a batch will not fit legacy, the command publishes a table naming the
accounts, settles a v0 transaction that refers to them by one byte each, and
takes the table back down:

```
$ mirror settle --program 8H3cYoiAA9LM36cyPr4UEv38dhHasSu2XPSdiBfyrLEa --denomination 5000011
looking for spends waiting to settle:
  49 transactions touched this pool

  24 spend(s) pending; settling 20 of them, which is what fits under the
  64-account lock limit. Run this again for the rest — the cost is a
  second timestamp, which is a real cost to the anonymity of both halves.

  20 spends do not fit a legacy transaction: 2219 bytes, 987 over the 1232-byte packet.
  Settling through a lookup table instead.
  lookup table A5FCt5jXPDpUsbfTwdkLvC9sMUPX1XYdFENPoAngsc3c
  62 addresses published
  settlement is 333 bytes of 1232, one signature
  table deactivated. Close it after ~513 slots to reclaim the rent and remove
  the published address list:
    mirror close-table --table A5FCt5jXPDpUsbfTwdkLvC9sMUPX1XYdFENPoAngsc3c

settled 20 spends in one transaction
  signature 4rKtNcZyAqxf5EAXNCtXpde9Zt3qqEEou4cG8GFV1vVA1kGGxFRBaBvZWNfsxVY9d9FgwTyYXYzz8dahaXenGM5C

Every payout in that batch shares one timestamp and one ordering, which
is what stops arrival time from telling the members apart.

The 20 members above are that batch's nominal set.
  That k is program-visible membership only, and the effective set is smaller.
  Measured against a live pool of comparable shape — Privacy Cash,
  9fhQBbumKEFuXtMBDw8AaQyAjCorLGJQiS3skWZdQyQD — knowing a member's funding
  class left ρ = 0.0955, inside an unresolved bracket of 0.0350 .. 0.1136. Roughly
  an order of magnitude of the nominal figure.
  Method, and what it does not cover: docs/PROVENANCE_METHOD.md.
```

**Twenty is the ceiling because each member locks three distinct accounts** — its
record, its beneficiary and its relay — and the transaction may lock 64 including
the pool's own three. A member whose beneficiary *is* their relay costs two
instead of three, and a batch of those settles 22 in 1713 bytes. That is a real
measurement of a shape nobody uses: paying the relay is paying the one party who
already knows which spend was yours.

**The packet stops mattering entirely** — twenty members weigh 333 of 1232
bytes. What binds instead is the number of accounts one transaction may lock:
three per member plus three for the pool, so twenty members sit at exactly 64.
`settle` counts them and defers the rest rather than building a transaction the
cluster refuses.

**Close the table afterwards.** It costs rent, but the reason to close it is what
it is while it exists: a public, durable account listing every address the
settlement touched, published before the settlement landed. Leaving one behind
per batch builds a permanent on-chain index of who settled together.

```
$ mirror close-table --table A5FCt5jXPDpUsbfTwdkLvC9sMUPX1XYdFENPoAngsc3c
closed A5FCt5jXPDpUsbfTwdkLvC9sMUPX1XYdFENPoAngsc3c
  signature 5vZBtQmkphZv8RFu2b2LRWLQ864iFt5qFarhiD7UkCvJPGeYz8BpbAESU6jUyEhVHWTHE6A9WpasKJkcKWD4Ni5k
  reclaimed 15084280 lamports, and the published address list is gone
```

**Do not set a crowd floor above what one settlement can carry.** `init-pool`
refuses it now, because a pool whose floor exceeds the per-transaction ceiling
can never meet it by crowd and can only settle through the timeout — and the
floor is fixed at creation, so the fix is a different pool.

For a legacy batch the limit is the packet in every row. Each spend brings
accounts nobody else shares — its record, its beneficiary, its relay — so a
payment costs about 99 bytes per member, and a call costs more because it also
names its callee and the callee's accounts.

**What you ask for changes how many people you can hide among.** A member who
delegates to their own choice of validator adds a vote account nobody else in
the batch names, and the batch loses a member. That is worth knowing before you
choose: a crowd that converges on one validator is both larger and less
distinguishable than a crowd that does not.

For payments compute is nowhere near binding — ten settle in 19,545 of the
200,000 compute units a single instruction gets, and twenty through a lookup
table in 35,895. For delegations it is much closer: on devnet the six-member
divergent batch used 142,856. In a *legacy* transaction there is no way out of
that, because the two limits shut together — asking for a larger compute budget
costs a second instruction worth 40 bytes and the batch has 38 to spare. Through
a table the bytes are there; how many delegations then fit is unmeasured, and
`CROWD.md` says so rather than assuming.

These are ceilings per *transaction*, not per epoch. Settlement is
permissionless, so a pool with thirty pending spends settles in three batches;
the cost is three timestamps rather than one, which is a real cost to the
anonymity and the reason the numbers are worth knowing.

`ten_spends_fit_in_one_settlement_and_the_packet_is_what_stops_the_eleventh` and
`every_account_an_action_names_costs_the_batch_a_member` pin them, and
`CROWD.md` is a live devnet settlement of the six-member divergent case.

## Escaping without a relay

There is no `self-spend` command because none is needed. Relay for yourself with
`--relay-fee 0` and settle your own batch once the timeout passes. The cost is
the expected one — your own wallet signs, so you give up the anonymity the relay
path provides. It is an escape hatch, not a mode of operation, and it exists so
that nothing in the protocol can hold your escrow.

## What can go wrong

| | |
|---|---|
| `no pool exists at …` | Nobody has created that denomination yet. `init-pool` does, permissionlessly. |
| `this note is not a leaf of the pool` | The note was never deposited, or it belongs to a different denomination. |
| `this note file is inconsistent` | The file was edited or corrupted. Spending it would burn a nullifier against a leaf that is not in the tree, so it is refused first. |
| `the rebuilt root does not match the pool's` | The endpoint's history is incomplete — usually pruning. `mirror check-endpoint` tests for exactly that. |
| `this pool holds N notes and its floor is M` | The pool refuses to act below its floor. Wait for more members, or use a pool with a floor you can meet. |
| `the relay … has also deposited into this pool` | The worst mistake available, refused rather than warned about. The relay signs the spend, so a key that also signed a deposit links the two and this action's anonymity set collapses to one. Use a key with no history with the pool. |
