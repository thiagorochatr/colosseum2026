# Threat model

Status: **draft**, Phase 1. Finalized once the build is done.

## Actors

- **Depositor:** puts a note into a pool.
- **Withdrawer:** spends a note.
- **Relayer:** submits a withdrawal for a fee.
- **Issuer:** sets the token's TIP-403 policy, on Tempo.
- **Curator:** publishes clean roots, on Solana.
- **Keeper:** calls `exclude` when a policy changes, on Tempo.
- **Observer:** sees the chain.

## What stays private

- **Hidden:** which deposit a withdrawal spends. The anonymity set is the clean deposits of that pool that are still unspent.

**What stays public:**
- deposits and their depositors;
- the fixed denomination;
- withdrawal recipients and fees;
- the timing of every transaction;
- every exclusion, with the excluded depositor;
- every clearance, on Solana.

## What each party can do

| Party | Can | Cannot |
|---|---|---|
| Curator, Solana | censor: leave a deposit out of the clean set, or revoke every root | forge a note, spend someone else's note, learn which deposit a withdrawal spends |
| Issuer, Tempo | block an address, which removes its deposits from the clean set | the same three things |
| Anyone, Tempo | call `exclude` and `reinclude` | change the result, since both are checked against the issuer's policy |
| Relayer | refuse to submit, see the recipient | change the recipient, the fee or the relayer, because each is bound into the proof |

## Known limits

1. **Trusted setup.** Groth16 soundness rests on at least one honest contributor in the phase-2 ceremony and on the phase-1 file. A development key is forgeable and is never the final one.
2. **Race on Tempo.** Between an address being blocked and `exclude` being called, its notes can still withdraw. The issuer closes the window by batching both in one transaction. A keeper narrows it otherwise.
3. **Exclusion is visible.** It reveals that a given depositor was blocked, which the issuer's public policy already shows.
4. **Small anonymity sets.** A fixed denomination and a young pool give a small set. Timing and amount correlation shrink it further.
5. **Curator trust on Solana.** The curator decides what clean means. The policy document is published and hashed on-chain, but applying it is the curator's job.
6. **Note reuse.** The nullifier is not separated by pool or chain. A client that reuses `k` links its notes.
7. **Upgradeability and audits.** Programs and contracts are deployed upgradeable on test networks and are not audited.
