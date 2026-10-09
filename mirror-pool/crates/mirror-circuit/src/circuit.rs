//! The membership circuit.
//!
//! The statement: *I know `(k, r, denom_tag)` such that `H3(k, r, denom_tag)`
//! is a leaf of the tree with root `R`, my nullifier is `H1(k)`, and this proof
//! is bound to `action`.*
//!
//! Public inputs are exactly three, and that is a cost decision: on-chain
//! verification measures at `74,179 + 5,661 x N` compute units, so every input
//! is ~5.7k CU. Anything else we need to commit to is folded into the action
//! binding rather than added as a fourth input.
//!
//! | public input | why it cannot be a witness |
//! |---|---|
//! | `root` | the program checks it against its own root history |
//! | `nullifier` | the program records it to prevent replay |
//! | `action_binding` | the program recomputes it from the action it executes |
//!
//! `denom_tag` stays a witness: the Merkle membership already constrains it,
//! because a pool's tree only ever contains leaves committed at that pool's
//! denomination.

use crate::poseidon_gadget::poseidon_var;
use ark_bn254::Fr;
use ark_r1cs_std::{
    alloc::AllocVar,
    boolean::Boolean,
    eq::EqGadget,
    fields::{fp::FpVar, FieldVar},
    select::CondSelectGadget,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use mirror_core::TREE_DEPTH;

#[derive(Clone)]
pub struct MembershipCircuit {
    // Public.
    pub root: Option<Fr>,
    pub nullifier: Option<Fr>,
    pub action_binding: Option<Fr>,
    // Witness.
    pub k: Option<Fr>,
    pub r: Option<Fr>,
    pub denom_tag: Option<Fr>,
    pub leaf_index: Option<u64>,
    pub siblings: Option<Vec<Fr>>,
}

impl MembershipCircuit {
    /// A circuit with no assignment, for key generation.
    pub fn blank() -> Self {
        MembershipCircuit {
            root: None,
            nullifier: None,
            action_binding: None,
            k: None,
            r: None,
            denom_tag: None,
            leaf_index: None,
            siblings: None,
        }
    }
}

impl ConstraintSynthesizer<Fr> for MembershipCircuit {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
        let missing = || SynthesisError::AssignmentMissing;

        // --- public inputs, in the order groth16-solana will receive them ---
        let root = FpVar::new_input(cs.clone(), || self.root.ok_or_else(missing))?;
        let nullifier = FpVar::new_input(cs.clone(), || self.nullifier.ok_or_else(missing))?;
        let action_binding =
            FpVar::new_input(cs.clone(), || self.action_binding.ok_or_else(missing))?;

        // --- witnesses ---
        let k = FpVar::new_witness(cs.clone(), || self.k.ok_or_else(missing))?;
        let r = FpVar::new_witness(cs.clone(), || self.r.ok_or_else(missing))?;
        let denom_tag = FpVar::new_witness(cs.clone(), || self.denom_tag.ok_or_else(missing))?;

        // Path orientation bits. `Boolean::new_witness` enforces booleanity, so
        // a prover cannot supply a non-binary direction and steer the fold.
        let mut path_bits = Vec::with_capacity(TREE_DEPTH);
        for level in 0..TREE_DEPTH {
            let bit = Boolean::new_witness(cs.clone(), || {
                let index = self.leaf_index.ok_or_else(missing)?;
                Ok((index >> level) & 1 == 1)
            })?;
            path_bits.push(bit);
        }

        let mut sibling_vars = Vec::with_capacity(TREE_DEPTH);
        for level in 0..TREE_DEPTH {
            let s = FpVar::new_witness(cs.clone(), || {
                let siblings = self.siblings.as_ref().ok_or_else(missing)?;
                siblings.get(level).copied().ok_or_else(missing)
            })?;
            sibling_vars.push(s);
        }

        // --- the note commitment is the leaf ---
        let leaf = poseidon_var(&[k.clone(), r, denom_tag])?;

        // --- fold the path to a root ---
        // bit = 0 means we are a left child, so the sibling goes on the right.
        let mut current = leaf;
        for (bit, sibling) in path_bits.iter().zip(sibling_vars.iter()) {
            let left = FpVar::conditionally_select(bit, sibling, &current)?;
            let right = FpVar::conditionally_select(bit, &current, sibling)?;
            current = poseidon_var(&[left, right])?;
        }
        current.enforce_equal(&root)?;

        // --- the nullifier is derived from the same k that opened the leaf ---
        let computed_nullifier = poseidon_var(&[k])?;
        computed_nullifier.enforce_equal(&nullifier)?;

        // --- bind the action ---
        // A Groth16 public input participates in verification only through the
        // R1CS columns that reference it. An input used in no constraint has an
        // all-zero column, so its `gamma_abc` term is the identity and *any*
        // value satisfies the verification equation — the input is present but
        // unbound. A relay could then swap the action after proving and the
        // proof would still verify.
        //
        // One multiplicative constraint is enough to give it a non-zero column.
        // The product itself is never used; emitting the constraint is the point.
        //
        // This is load-bearing rather than defensive, and it is checked rather
        // than assumed: `tests/onchain_layout.rs` tampers with this exact public
        // input and asserts the real on-chain verifier rejects the result. If
        // this line were removed, that test would fail.
        let _binding_is_referenced = action_binding.square()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::poseidon_gadget::fr_from_be;
    use ark_relations::r1cs::ConstraintSystem;
    use mirror_core::{Field, MerkleTree, Note};

    fn f(v: u64) -> Field {
        Field::from_u64(v)
    }

    /// Builds a tree, inserts notes, and returns a fully assigned circuit for
    /// the note at `target`.
    fn witness_for(target: usize, count: u64) -> (MembershipCircuit, Field) {
        let mut tree = MerkleTree::new().unwrap();
        let denom = f(1);
        let mut notes = Vec::new();
        for i in 1..=count {
            let note = Note::new(f(i * 10), f(i * 100), denom);
            tree.insert(note.commitment().unwrap()).unwrap();
            notes.push(note);
        }
        let note = notes[target];
        let proof = tree.proof(target as u64).unwrap();
        let root = tree.root().unwrap();
        let action_binding = f(0xabcdef);

        let circuit = MembershipCircuit {
            root: Some(fr_from_be(root.as_bytes())),
            nullifier: Some(fr_from_be(note.nullifier().unwrap().as_bytes())),
            action_binding: Some(fr_from_be(action_binding.as_bytes())),
            k: Some(fr_from_be(note.k.as_bytes())),
            r: Some(fr_from_be(note.r.as_bytes())),
            denom_tag: Some(fr_from_be(denom.as_bytes())),
            leaf_index: Some(target as u64),
            siblings: Some(
                proof
                    .siblings
                    .iter()
                    .map(|s| fr_from_be(s.as_bytes()))
                    .collect(),
            ),
        };
        (circuit, root)
    }

    fn satisfied(c: MembershipCircuit) -> bool {
        let cs = ConstraintSystem::<Fr>::new_ref();
        c.generate_constraints(cs.clone()).unwrap();
        cs.is_satisfied().unwrap()
    }

    #[test]
    fn an_honest_witness_satisfies_the_circuit() {
        let (circuit, _) = witness_for(3, 9);
        assert!(satisfied(circuit));
    }

    #[test]
    fn every_position_in_the_tree_works() {
        // Left children, right children, and the boundary where the frontier
        // pairs against an empty subtree.
        for target in 0..7 {
            let (circuit, _) = witness_for(target, 7);
            assert!(satisfied(circuit), "leaf {target} failed");
        }
    }

    #[test]
    fn a_wrong_root_is_rejected() {
        let (mut circuit, _) = witness_for(2, 5);
        circuit.root = Some(Fr::from(999u64));
        assert!(!satisfied(circuit));
    }

    #[test]
    fn a_wrong_nullifier_is_rejected() {
        let (mut circuit, _) = witness_for(2, 5);
        circuit.nullifier = Some(Fr::from(999u64));
        assert!(!satisfied(circuit));
    }

    #[test]
    fn a_forged_secret_is_rejected() {
        // Knowing the path is not enough; you must open the leaf.
        let (mut circuit, _) = witness_for(2, 5);
        circuit.k = Some(Fr::from(12345u64));
        assert!(!satisfied(circuit));
    }

    #[test]
    fn a_tampered_sibling_is_rejected() {
        let (mut circuit, _) = witness_for(2, 5);
        let mut siblings = circuit.siblings.clone().unwrap();
        siblings[0] = Fr::from(7u64);
        circuit.siblings = Some(siblings);
        assert!(!satisfied(circuit));
    }

    #[test]
    fn claiming_a_different_leaf_index_is_rejected() {
        let (mut circuit, _) = witness_for(2, 5);
        circuit.leaf_index = Some(3);
        assert!(!satisfied(circuit));
    }

    #[test]
    fn a_note_from_another_denomination_is_rejected() {
        let (mut circuit, _) = witness_for(2, 5);
        circuit.denom_tag = Some(Fr::from(2u64));
        assert!(!satisfied(circuit));
    }

    #[test]
    fn the_circuit_size_is_reasonable() {
        let cs = ConstraintSystem::<Fr>::new_ref();
        let (circuit, _) = witness_for(1, 4);
        circuit.generate_constraints(cs.clone()).unwrap();
        let n = cs.num_constraints();
        // depth 20 nodes + one commitment + one nullifier, ~243 each.
        assert!(
            (4_000..8_000).contains(&n),
            "circuit is {n} constraints, outside the expected band"
        );
        assert_eq!(
            cs.num_instance_variables(),
            4,
            "one plus three public inputs"
        );
    }
}
