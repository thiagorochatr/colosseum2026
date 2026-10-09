//! Rebuilding a pool's state from the chain, with no index and no local file.
//!
//! The program stores the accumulator's *frontier* — one node per level — which
//! is all it needs to append a leaf and produce a root, and is not enough to
//! prove that any particular leaf is in the tree. A member needs the whole leaf
//! set for that.
//!
//! The leaves are not lost, though: every commitment was an argument to a
//! `Deposit` instruction, so the complete set is sitting in the transaction
//! history in the order it was inserted. This module reads it back.
//!
//! That matters beyond convenience. A client that depended on a local note
//! ledger would strand a member who lost the file, and one that depended on an
//! indexer would put a server between a member and their own money. Neither is
//! acceptable for a tool whose entire premise is that you should not have to
//! trust the operator — including us.

use crate::chain::Chain;
use anyhow::{anyhow, Result};
use mirror_core::{Field, MerkleTree};
use mirror_pool_program::{instruction::Instruction as MirrorIx, pda::spend_address};
use solana_program::pubkey::Pubkey;

/// Where the pool account sits in the account list of the two instructions this
/// module reads. Both put the signer first and the pool second.
const POOL_ACCOUNT_INDEX: usize = 1;

/// A spend that was submitted, and where its record lives.
pub struct SubmittedSpend {
    pub nullifier: [u8; 32],
    pub spend: Pubkey,
}

/// Everything a client needs that the pool account does not hold.
pub struct History {
    /// Note commitments in insertion order. Index `i` is leaf `i`.
    pub commitments: Vec<[u8; 32]>,
    pub spends: Vec<SubmittedSpend>,
    /// Every key that has signed a deposit into this pool.
    ///
    /// Collected so the tool can catch the one mistake that undoes everything:
    /// relaying a spend with a key that also deposited. Depositors are public by
    /// necessity — a deposit is signed — so reading them costs nothing beyond
    /// the scan already being done.
    pub depositors: Vec<Pubkey>,
}

impl History {
    /// Rebuilds the host-side Merkle tree the prover needs.
    pub fn tree(&self) -> Result<MerkleTree> {
        let mut tree = MerkleTree::new().map_err(|e| anyhow!("{e:?}"))?;
        for (i, c) in self.commitments.iter().enumerate() {
            let leaf = Field::from_bytes(*c)
                .map_err(|e| anyhow!("commitment {i} is not a canonical field element: {e:?}"))?;
            tree.insert(leaf).map_err(|e| anyhow!("{e:?}"))?;
        }
        Ok(tree)
    }

    /// The index of `commitment`, if this pool holds it.
    pub fn index_of(&self, commitment: &[u8; 32]) -> Option<u64> {
        self.commitments
            .iter()
            .position(|c| c == commitment)
            .map(|i| i as u64)
    }
}

/// Reads every deposit and spend this pool has seen, oldest first.
///
/// Scans the *pool* account rather than the program. A program can host one pool
/// per denomination, so scanning the program would walk every pool's history to
/// build one pool's tree, and would then have to discard most of it. The pool
/// account appears in exactly the transactions that concern it.
pub fn scan(chain: &Chain, program_id: &Pubkey, pool: &Pubkey, verbose: bool) -> Result<History> {
    let signatures = chain.signatures_for_address(pool)?;
    if verbose {
        println!("  {} transactions touched this pool", signatures.len());
    }

    let mut commitments = Vec::new();
    let mut spends = Vec::new();
    let mut depositors: Vec<Pubkey> = Vec::new();

    for (n, signature) in signatures.iter().enumerate() {
        if verbose && n > 0 && n % 25 == 0 {
            println!("  {n}/{} …", signatures.len());
        }
        // Paced deliberately. `getTransaction` is the expensive RPC method and
        // the one public endpoints throttle first, and a rebuild asks for it
        // once per signature — so a pool with any history sends a burst that
        // gets the client throttled partway through and fails a member who was
        // only trying to spend their own note. A short pause costs seconds and
        // removes the failure entirely.
        if n > 0 {
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        let Some(tx) = chain.transaction(signature)? else {
            continue;
        };
        let keys = &tx.message.account_keys;

        for ix in &tx.message.instructions {
            let Some(program) = keys.get(ix.program_id_index as usize) else {
                continue;
            };
            if program != program_id {
                continue;
            }
            // An instruction of ours that names a different pool belongs to a
            // different denomination and a different tree.
            let names_this_pool = ix
                .accounts
                .get(POOL_ACCOUNT_INDEX)
                .and_then(|i| keys.get(*i as usize))
                .map(|k| k == pool)
                .unwrap_or(false);
            if !names_this_pool {
                continue;
            }
            // Anything that does not decode is not ours to interpret. The pool
            // account can appear in a transaction for reasons this module has no
            // opinion about.
            let Ok(decoded) = MirrorIx::unpack(&ix.data) else {
                continue;
            };
            match decoded {
                MirrorIx::Deposit { commitment } => {
                    commitments.push(commitment);
                    if let Some(who) = ix.accounts.first().and_then(|i| keys.get(*i as usize)) {
                        if !depositors.contains(who) {
                            depositors.push(*who);
                        }
                    }
                }
                MirrorIx::SubmitSpend { nullifier, .. } => {
                    let (spend, _) = spend_address(program_id, pool, &nullifier);
                    spends.push(SubmittedSpend { nullifier, spend });
                }
                _ => {}
            }
        }
    }

    Ok(History {
        commitments,
        spends,
        depositors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirror_core::Frontier;

    fn commitments(n: u64) -> Vec<[u8; 32]> {
        (1..=n)
            .map(|i| Field::from_u64(i * 1_000 + 7).to_bytes())
            .collect()
    }

    /// The rebuilt tree must agree with the accumulator the program keeps.
    ///
    /// This is the property the whole module exists for, and it is a property of
    /// the *order*: the same leaves inserted in a different order give a
    /// different root, so a scan that returned them newest-first would produce a
    /// tree that looks healthy and against which no proof verifies.
    #[test]
    fn a_rebuilt_tree_matches_the_frontier_the_program_keeps() {
        let history = History {
            commitments: commitments(9),
            spends: Vec::new(),
            depositors: Vec::new(),
        };
        let mut frontier = Frontier::new().unwrap();
        for c in &history.commitments {
            frontier.insert(Field::from_bytes(*c).unwrap()).unwrap();
        }
        assert_eq!(history.tree().unwrap().root().unwrap(), frontier.root());
    }

    #[test]
    fn the_wrong_order_produces_a_different_root() {
        let forward = History {
            commitments: commitments(5),
            spends: Vec::new(),
            depositors: Vec::new(),
        };
        let mut backward = forward.commitments.clone();
        backward.reverse();
        let reversed = History {
            commitments: backward,
            spends: Vec::new(),
            depositors: Vec::new(),
        };
        assert_ne!(
            forward.tree().unwrap().root().unwrap(),
            reversed.tree().unwrap().root().unwrap(),
            "insertion order does not affect the root, which would mean the scan \
             order does not matter — it does"
        );
    }

    #[test]
    fn a_commitment_is_found_at_the_index_it_was_inserted() {
        let history = History {
            commitments: commitments(6),
            spends: Vec::new(),
            depositors: Vec::new(),
        };
        for (i, c) in history.commitments.clone().iter().enumerate() {
            assert_eq!(history.index_of(c), Some(i as u64));
        }
        assert_eq!(history.index_of(&[0xAB; 32]), None);
    }

    /// A leaf the accumulator would reject must be refused while it is still a
    /// bad byte string, not deep inside a Poseidon call.
    #[test]
    fn a_non_canonical_commitment_fails_the_rebuild_by_name() {
        let history = History {
            commitments: vec![[0xff; 32]],
            spends: Vec::new(),
            depositors: Vec::new(),
        };
        let err = history.tree().unwrap_err().to_string();
        assert!(err.contains("canonical"), "{err}");
    }
}
