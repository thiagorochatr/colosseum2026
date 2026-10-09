//! Field primitives shared by the circuit, the on-chain program and the host
//! tooling: canonical BN254 scalars, Poseidon hashing with domain separation by
//! arity, the incremental Merkle accumulator, and the note model.
//!
//! This crate is linked into the on-chain program, so it carries no arkworks.
//! Field elements cross every boundary as canonical 32-byte big-endian arrays.
#![forbid(unsafe_code)]

mod field;
mod hash;
mod merkle;
mod note;

pub use field::{Field, MODULUS_BE};
pub use hash::{
    action_binding, commitment, field_from_digest, hash_node, nullifier, poseidon1, poseidon2,
    poseidon3, ACTION_DOMAIN,
};
pub use merkle::{
    baked_ladder, zero_ladder, Frontier, MerkleProof, MerkleTree, TREE_DEPTH, ZERO_LADDER,
};
pub use note::Note;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MirrorError {
    #[error("field element is not canonical: at or above the BN254 scalar modulus")]
    NonCanonicalField,
    #[error("field element must be exactly 32 bytes")]
    BadFieldLength,
    #[error("poseidon hashing failed")]
    Poseidon,
    #[error("the accumulator is full")]
    TreeFull,
    // There is no `BadPathLength`. A `MerkleProof` carries
    // `siblings: [Field; TREE_DEPTH]`, a fixed-size array, so a path of the
    // wrong length is not constructible and the check has nothing to reject.
    // The property is enforced by the type rather than by a runtime branch, and
    // carrying an error variant for it would advertise a check that does not
    // exist.
    #[error("leaf index is outside the tree")]
    LeafIndexOutOfRange,
}
