//! The note model.
//!
//! A note is the unit of membership. Depositing escrows exactly the pool's
//! denomination and inserts `commitment(k, r, denom_tag)` as a leaf. Spending
//! proves knowledge of `(k, r)` for some leaf and publishes `nullifier(k)`,
//! which the program records permanently.
//!
//! The denomination is a pool constant rather than a field inside the note, so
//! the escrowed lamports and the hidden commitment cannot disagree. Carrying the
//! amount inside the note is the obvious alternative and it is a trap: unless
//! the amount is bound into the commitment *and* checked against the escrow, a
//! depositor of one lamport withdraws the whole pool holding an entirely valid
//! proof. Here that state is not representable rather than merely rejected.

use crate::{commitment, nullifier, Field, MirrorError};

/// A note's secret material. `k` is the nullifier preimage; `r` blinds the
/// commitment so that two notes with the same `k` are still distinct leaves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Note {
    pub k: Field,
    pub r: Field,
    pub denom_tag: Field,
}

impl Note {
    pub fn new(k: Field, r: Field, denom_tag: Field) -> Self {
        Note { k, r, denom_tag }
    }

    /// The Merkle leaf for this note.
    pub fn commitment(&self) -> Result<Field, MirrorError> {
        commitment(self.k, self.r, self.denom_tag)
    }

    /// The nullifier published when this note is spent.
    pub fn nullifier(&self) -> Result<Field, MirrorError> {
        nullifier(self.k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(v: u64) -> Field {
        Field::from_u64(v)
    }

    #[test]
    fn blinding_makes_two_notes_with_one_nullifier_preimage_distinct_leaves() {
        let a = Note::new(f(1), f(10), f(100));
        let b = Note::new(f(1), f(11), f(100));
        assert_ne!(a.commitment().unwrap(), b.commitment().unwrap());
        // ...but they share a nullifier, so only one of them is ever spendable.
        // That is the depositor's own mistake to make, and it costs them a note
        // rather than costing the pool anything.
        assert_eq!(a.nullifier().unwrap(), b.nullifier().unwrap());
    }

    #[test]
    fn the_nullifier_does_not_reveal_the_commitment() {
        let n = Note::new(f(7), f(8), f(9));
        assert_ne!(n.nullifier().unwrap(), n.commitment().unwrap());
    }

    #[test]
    fn a_note_is_bound_to_one_denomination() {
        let a = Note::new(f(1), f(2), f(1));
        let b = Note::new(f(1), f(2), f(2));
        assert_ne!(
            a.commitment().unwrap(),
            b.commitment().unwrap(),
            "a note minted in the 0.1 SOL pool must not be a valid leaf in the 10 SOL pool"
        );
    }
}
