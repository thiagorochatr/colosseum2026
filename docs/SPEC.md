# Specification

Status: **frozen for implementation**, Phase 1. Items marked *open* are confirmed during Phase 3 and do not change the protocol below.

## 1. What this is

A compliance layer for private pools. A withdrawer proves, in zero knowledge, that the note they spend sits in the pool **and** in a **clean set**, without revealing which note it is. The clean set is a mask over the pool: the same leaves at the same indices, with every non-clean deposit replaced by zero.

| | Tempo | Solana |
|---|---|---|
| Pool | new: a fixed-denomination TIP-20 pool | `mirror-pool`, adapted |
| Who decides what is clean | the token issuer's TIP-403 policy, read on-chain | a curator, who publishes the clean root |
| Who updates the clean set | anyone, checked on-chain against the policy | the curator |
| Verification | Solidity verifier inside the pool | a separate compliance program that writes a clearance |

One circuit, one proving key, one verifying key on both chains.

## 2. Encodings

- **Field:** the BN254 scalar field, `p = 21888242871839275222246405745257275088548364400416034343698204186575808495617`.
- **Bytes:** a field element on Solana is 32 bytes, big-endian, canonical (`< p`). On Tempo it is a `uint256` `< p`.
- **Poseidon:** circomlib's Poseidon over BN254 with `x^5`, which is exactly Solana's `sol_poseidon` with `Bn254X5` and big-endian input. `H1`, `H2` and `H3` take 1, 2 and 3 inputs. These are three different instances, so the arity is part of the domain.
- **Digest to field:** `F(d)` takes a 32-byte keccak digest and sets its first byte to zero. The result is always `< p`. This is `field_from_digest` in `mirror-core`.

Reference vector: `H2(1, 2) = 0x115cc0f5e7d690413df64c6b9662e9cf2a3617f2743245519e19607a4417189a`.

## 3. Notes

A note is `(k, r, denom_tag)`, with `k` and `r` uniformly random field elements.

- **Leaf:** `C = H3(k, r, denom_tag)`.
- **Nullifier:** `N = H1(k)`.
- **denom_tag:** the pool's denomination in base units, as a field element. On Solana that is lamports. On Tempo it is the token's 6-decimal units.

Both formulas are `mirror-core`'s, unchanged, so existing `mirror-pool` notes remain spendable.

`N` is not separated by pool or chain. **A client must draw a fresh `k` for every note.** Reusing `k` across pools or chains links the notes.

## 4. Trees

- **Shape:** depth 20, node `H2(left, right)`, empty leaf `0`.
- **Zero ladder:** `Z[0] = 0`, `Z[i+1] = H2(Z[i], Z[i])`, identical to `mirror-core`'s `ZERO_LADDER`.
- **Pool tree:** append-only. Leaf `i` is the commitment of deposit `i`.
- **Clean tree:** same indices. `clean[i] = pool[i]` if deposit `i` is clean, else `0`.

A clean leaf is never `0` for a real note, because that would need a Poseidon preimage of zero. So membership of `C` in the clean tree at index `i` means deposit `i` is clean.

### Why the proof needs both roots

On Solana the curator supplies the clean root. Without the pool root, a dishonest curator could put leaves in the clean tree that were never deposited, and withdraw against them. Requiring the same leaf at the same index in the pool tree limits the curator to **removing** deposits. On Tempo the contract builds both trees, so the pool root is redundant there but costs nothing.

## 5. Circuit

`circuits/withdraw.circom`, circom 2.2.3, circomlib 2.0.5.

**Public inputs, in this order:**

1. `poolRoot`
2. `cleanRoot`
3. `nullifierHash`
4. `binding`

The circuit has no outputs. circom orders public inputs by their declaration order inside the template, not by the order of the `public [...]` list. So the template declares the four signals in exactly this order, and `main` lists them in the same order: `component main {public [poolRoot, cleanRoot, nullifierHash, binding]}`. A test checks the order in snarkjs's `public.json` against this list.

**Private inputs:** `k`, `r`, `denomTag`, `pathIndices[20]`, `poolSiblings[20]`, `cleanSiblings[20]`.

**Constraints:**

1. `leaf = H3(k, r, denomTag)`.
2. `nullifierHash === H1(k)`.
3. Every `pathIndices[j]` is a bit.
4. Folding `leaf` up `poolSiblings` with `pathIndices` gives `poolRoot`.
5. Folding `leaf` up `cleanSiblings` with the **same** `pathIndices` gives `cleanRoot`.
6. `bindingSquare <== binding * binding`, so `binding` takes part in a constraint and cannot be dropped by the compiler.

The circuit treats `binding` as opaque. Each chain computes it from the action it executes, so one verifying key serves both chains.

Size estimate: 42 Poseidon permutations, which is 40 `H2`, one `H3` and one `H1`. The compiled circuit, built with `--O2`, has 10,175 constraints, all non-linear. That fits a `2^14` powers-of-tau file.

**Setup:**
- **Phase 1:** a published perpetual powers-of-tau file.
- **Phase 2:** a multi-party snarkjs ceremony, closed with a public beacon (see `CEREMONY.md`).
- **Development:** a single-contribution development key is used only until the ceremony closes, and is never deployed as final.

## 6. Action binding

The binding commits the proof to what happens with the money. A relayer that changes any field produces a different `binding`, and the pairing fails.

**Solana:** `mirror-pool`'s existing binding, unchanged:

```
binding = F(keccak(
  "mirror-pool:action:v2" || selector_le64 || target_program || beneficiary ||
  relay || relay_fee_le64 || action_accounts_u8 || payload))
```

**Tempo:**

```
binding = F(keccak256(abi.encodePacked(
  "compliance-pool:tempo:withdraw:v1",
  uint256(block.chainid), address(pool), address(recipient),
  address(relayer), uint256(fee))))
```

The roots and the domain strings differ between chains, so a proof made for one chain cannot be replayed on the other.

## 7. Tempo pool

**Constructor parameters:** `token` (a TIP-20), `denomination`, `verifier`. All are immutable.

**State:**
- **Pool tree:** an append-only frontier with a ring of the last 32 roots.
- **Clean tree:** every node is stored, so any leaf can be rewritten.
- **Clean roots:** a ring of the last 32 roots, each tagged with a `cleanEpoch`.
- **Per deposit:** `depositor[i]` and `commitment[i]`.
- **Spent nullifiers:** `spent[nullifierHash]`.

**Functions:**

- **`deposit(commitment)`**
  1. Pulls `denomination` from `msg.sender` with `transferFrom`. The token's own policy already refuses an unauthorized sender.
  2. Inserts `commitment` at index `i` in both trees and records `depositor[i] = msg.sender`.
  3. Pushes both new roots.
- **`exclude(i)`**, callable by anyone.
  1. Requires that `i` exists, that `clean[i] != 0`, and that `isAuthorizedSender(token.transferPolicyId(), depositor[i])` is `false` in the TIP-403 registry. That is the same check the token applies to a sender.
  2. Sets `clean[i] = 0` and increments `cleanEpoch`, which invalidates every earlier clean root. It then pushes the new root.
- **`reinclude(i)`**, callable by anyone.
  1. Requires that `clean[i] == 0` and that the depositor is authorized again.
  2. Restores `clean[i] = commitment[i]` and pushes the new root. Adding a leaf cannot help an excluded note, so the epoch does not move.
- **`withdraw(proof, poolRoot, cleanRoot, nullifierHash, recipient, relayer, fee)`**
  1. Checks the request:
     - `poolRoot` is in the pool ring;
     - `cleanRoot` is in the clean ring and tagged with the current epoch;
     - `nullifierHash` is unspent;
     - `fee <= denomination`.
  2. Recomputes `binding` and verifies the proof.
  3. Marks the nullifier spent.
  4. Transfers `denomination - fee` to `recipient` and `fee` to `relayer`. The token's policy refuses an unauthorized recipient.

**Events:** `Deposit(index, commitment, depositor, poolRoot, cleanRoot)`, `Excluded(index, depositor, cleanRoot, cleanEpoch)`, `Reincluded(index, cleanRoot)`, `Withdrawal(nullifierHash, recipient, relayer, fee)`.

**Policy semantics:**
- **Supported policy types:** `WHITELIST`, `BLACKLIST` and `COMPOUND`. A deposit is clean while its depositor is an authorized **sender** under the token's **current** policy. For a compound policy that is its sender part, which `isAuthorizedSender` answers.
- **Whitelist tokens:** the issuer must authorize the pool's own address, or the pool can neither receive nor pay.

**Closing the race:** the issuer blocks an address and calls `exclude` for each of its deposits in **one Tempo batch transaction**, so no withdrawal fits in between. For issuers that do not batch, a keeper applies the policy: it watches the registry's `BlacklistUpdated` and `WhitelistUpdated` events and sends the `exclude` and `reinclude` calls they require, in one batch (`tempo/keeper/keeper.mjs`, built on the SDK's `readSweep`). The demo service's relayer runs the same sweep before every withdrawal it submits. A blocked depositor who sees the block coming can still withdraw before the keeper's transaction lands, as a TIP-20 holder can move funds before a freeze.

## 8. Solana

### 8.1 Compliance program

A native program, in the same style as `mirror-pool`.

**Accounts:**
- **Curator**, at PDA `["curator", authority]`. It holds the `authority`, a `policy_hash`, an `epoch` and a ring of the last 32 clean roots, each tagged with its epoch.
  - `policy_hash` is the sha256 of the published policy document.
- **Clearance**, at PDA `["clearance", pool, nullifierHash]`. It holds a version, `pool`, `curator`, `nullifierHash`, `binding`, `cleanRoot` and the slot it was written in.

**Instructions:**
- **`InitCurator { policy_hash }`**, signed by the authority.
- **`PublishRoot { root }`**, signed by the authority. It pushes `root` tagged with the current epoch.
- **`Revoke {}`**, signed by the authority. It increments the epoch, invalidating every earlier root.
- **`UpdatePolicy { policy_hash }`**, signed by the authority. It replaces the policy hash when the policy changes, for example when a depositor is blocked. Roots and epoch are untouched.
- **`VerifyClearance { proof, poolRoot, cleanRoot, nullifierHash, binding }`**:
  1. Checks the pool account:
     - it has mirror-pool's pool layout: its length, version and tree depth;
     - `poolRoot` is in its 128-root history, at offset 696.

     It does not check the account's owner. Section 8.2 explains why a fake pool account gains nothing.
  2. Checks that `cleanRoot` is in the curator's ring, tagged with the current epoch.
  3. Verifies the Groth16 proof with `groth16-solana` over the four public inputs.
  4. Creates the clearance account. Because its address is keyed by pool and nullifier, it can exist only once.

A clearance is public evidence. Anyone, such as an offramp, a DEX or a transfer hook, can check that pool `P` spent nullifier `N`, bound to `binding`, under curator `C`'s clean root `R`.

### 8.2 The compliant pool

`mirror-pool/` stays exactly as imported. The compliant pool is a separate program, `solana/programs/compliant-pool`, that links mirror-pool as a library:

- **Forwarded unchanged:** `InitPool`, `Deposit` and `SettleEpoch` go straight to mirror-pool's processor. Pools, vaults, spend records and settlement are mirror-pool's own.
- **Refused:** mirror-pool's plain `SubmitSpend`, error `100`.
- **Added:** `SubmitClearedSpend`, tag `4`.
  1. It runs every check of mirror-pool's `SubmitSpend`, in the same order, except the root and the pairing.
  2. In their place it requires the clearance from the compliance program, and checks that it:
     - names this pool and this nullifier;
     - was issued under the required curator;
     - was issued **in this slot**;
     - carries the binding this spend recomputes, relay key included.
  3. It then writes the same spend record, so mirror-pool's `SettleEpoch` pays it out.
- **One transaction:** a relay sends `VerifyClearance` and `SubmitClearedSpend` together. The same-slot rule means a clearance can never wait out a curator's revocation.
- **The curator is fixed per deployment.** The compliance program id and the required curator authority are compile-time constants, `MIRROR_COMPLIANCE_PROGRAM_ID` and `MIRROR_REQUIRED_CURATOR`. mirror-pool creates pools permissionlessly and names them by denomination alone, so a curator chosen per pool would let anyone take a denomination with a curator of their own.
- **The compliance program does not check the pool's owner.** A clearance names the pool account it was verified against, and a pool accepts only clearances naming itself. A fake pool account yields a clearance for a different address, which no pool consumes. A test checks exactly this.

### 8.3 Curator, off-chain

**The policy** is a canonical JSON document: `{"version": 1, "rule": "exclude every deposit whose depositor is on the blocklist", "blocklist": [...]}`, with the blocklist sorted and deduplicated. Its sha256 is the `policy_hash` the curator account holds. The genesis policy is `solana/deployments/devnet-policy.json`.

**Each publication** rebuilds the pool from its transaction history, takes each deposit's depositor (the signer of `Deposit`), applies the policy, and builds the clean tree as a mask of the pool tree. One transaction then carries:

1. `Revoke`, only when the policy removes someone, so no earlier root survives;
2. `UpdatePolicy` with the policy's hash;
3. `PublishRoot` with the clean root;
4. a Memo instruction with the clean set and the policy document:

```json
{ "v": 2, "pool": "<base58>", "n": 10, "x": [1], "r": "<clean root, hex>",
  "policy": { "version": 1, "rule": "...", "blocklist": ["<base58>"] } }
```

**A client accepts a memo** only if its root is held in the curator's current epoch and the sha256 of its policy document equals the hash in the curator account. It then rebuilds the clean tree from the pool tree and `x`, and must get `r`. Nothing about the clean set lives off-chain.

## 9. Clients

- **Tempo:** the client rebuilds both trees from `Deposit`, `Excluded` and `Reincluded` events.
- **Solana:** the client rebuilds the pool tree from history and the clean tree from the curator's latest accepted memo.
- **Proving:** snarkjs with the circuit's `wasm` and the final `zkey`, in the browser or in Node.
- **Verifying key:** both verifiers are generated from one `vk.json`, and its sha256 is printed by both deploy scripts. Matching digests are the check that the two chains verify the same statement.

## 10. Confirmed in Phase 3

1. **Policy getter.** A TIP-20 token reports its policy with `transferPolicyId()`. A new token defaults to policy `1`, always-allow. Tokens are created with `TIP20_FACTORY.createToken(name, symbol, currency, quoteToken, admin, salt)` at `0x20Fc000000000000000000000000000000000000`. The interfaces come from `tempo-std` v0.1.6.
2. **Policy checks.** `isAuthorizedSender` flips when an address is blacklisted, when it leaves a whitelist, and through the sender part of a compound policy. The token itself refuses a blocked sender and a blocked recipient. All of this is tested against Tempo's precompiles with `forge test --network tempo`.
3. **Poseidon in Solidity.** `poseidon-solidity` 0.0.5, MIT, the library Semaphore uses through zk-kit. It matches the circomlib vector and the zero ladder, and rebuilds the vector pool and clean roots from each note's secrets. Its functions are public library functions, so the libraries are deployed and linked. One path of 20 `H2` hashes costs about 603k gas.
4. **Groth16 on Tempo.**
   - The snarkjs verifier costs about 216k gas to verify with four public inputs, and about 280k gas as a whole transaction on a local Tempo node.
   - Deploying it costs about 2.43M gas on a local Tempo node, and an estimated 2.45M on Moderato. The difference from Ethereum comes from Tempo's 1,000 gas per deployed byte.
5. **Groth16 on Solana.** `VerifyProof` with four public inputs costs 96,940 compute units in the worst case, matching mirror-pool's model.
