//! The incremental Poseidon Merkle accumulator.
//!
//! Two views of one structure:
//!
//! * [`Frontier`] is what the on-chain program keeps — `O(depth)` state and an
//!   `O(depth)` append. It can produce the current root and nothing else.
//! * [`MerkleTree`] is the host-side full tree used to build the membership
//!   paths that go into a proof.
//!
//! They must agree, and the tests here assert that they do over real insertion
//! sequences. That assertion is the point of this module. It is easy to test
//! each half separately and never connect them — proving against an injected
//! fixture root rather than one the on-chain accumulator actually produced — and
//! a suite built that way stays green while the two implementations drift.

use crate::{hash_node, Field, MirrorError};

/// Tree depth. 2^20 leaves is roughly a million notes, which is far beyond any
/// anonymity set this protocol will realistically hold, and a depth-20 path
/// costs about 17k compute units to fold on-chain.
pub const TREE_DEPTH: usize = 20;

/// Precomputed empty-subtree roots: `ZERO_LADDER[i]` is the root of an empty
/// subtree of height `i`.
///
/// Baked rather than computed because the on-chain program needs the ladder on
/// every deposit, and recomputing it would cost about 17k compute units per
/// instruction. `the_baked_ladder_matches_the_computed_one` asserts the two
/// agree, so a change to the hash function cannot silently desynchronise them.
pub const ZERO_LADDER: [[u8; 32]; TREE_DEPTH + 1] = [
    // level 0
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0,
    ],
    // level 1
    [
        32, 152, 245, 251, 158, 35, 158, 171, 60, 234, 195, 242, 123, 129, 228, 129, 220, 49, 36,
        213, 95, 254, 213, 35, 168, 57, 238, 132, 70, 182, 72, 100,
    ],
    // level 2
    [
        16, 105, 103, 61, 205, 177, 34, 99, 223, 48, 26, 111, 245, 132, 167, 236, 38, 26, 68, 203,
        157, 198, 141, 240, 103, 164, 119, 68, 96, 177, 241, 225,
    ],
    // level 3
    [
        24, 244, 51, 49, 83, 126, 226, 175, 46, 61, 117, 141, 80, 247, 33, 6, 70, 124, 110, 234,
        80, 55, 29, 213, 40, 213, 126, 178, 184, 86, 210, 56,
    ],
    // level 4
    [
        7, 249, 216, 55, 203, 23, 176, 211, 99, 32, 255, 233, 59, 165, 35, 69, 241, 183, 40, 87,
        26, 86, 130, 101, 202, 172, 151, 85, 157, 188, 149, 42,
    ],
    // level 5
    [
        43, 148, 207, 94, 135, 70, 179, 245, 201, 99, 31, 76, 93, 243, 41, 7, 166, 153, 197, 140,
        148, 178, 173, 77, 123, 92, 236, 22, 57, 24, 63, 85,
    ],
    // level 6
    [
        45, 238, 147, 197, 166, 102, 69, 150, 70, 234, 125, 34, 204, 169, 225, 188, 254, 215, 30,
        105, 81, 185, 83, 97, 29, 17, 221, 163, 46, 160, 157, 120,
    ],
    // level 7
    [
        7, 130, 149, 229, 162, 43, 132, 233, 130, 207, 96, 30, 182, 57, 89, 123, 139, 5, 21, 168,
        140, 181, 172, 127, 168, 164, 170, 190, 60, 135, 52, 157,
    ],
    // level 8
    [
        47, 165, 229, 241, 143, 96, 39, 166, 80, 27, 236, 134, 69, 100, 71, 42, 97, 107, 46, 39,
        74, 65, 33, 26, 68, 76, 190, 58, 153, 243, 204, 97,
    ],
    // level 9
    [
        14, 136, 67, 118, 208, 216, 253, 33, 236, 183, 128, 56, 158, 148, 31, 102, 228, 94, 122,
        204, 227, 226, 40, 171, 62, 33, 86, 166, 20, 252, 215, 71,
    ],
    // level 10
    [
        27, 114, 1, 218, 114, 73, 79, 30, 40, 113, 122, 209, 165, 46, 180, 105, 249, 88, 146, 249,
        87, 113, 53, 51, 222, 97, 117, 229, 218, 25, 10, 242,
    ],
    // level 11
    [
        31, 141, 136, 34, 114, 94, 54, 56, 82, 0, 192, 178, 1, 36, 152, 25, 166, 230, 225, 228,
        101, 8, 8, 181, 190, 188, 107, 250, 206, 125, 118, 54,
    ],
    // level 12
    [
        44, 93, 130, 246, 108, 145, 75, 175, 185, 112, 21, 137, 186, 140, 252, 251, 97, 98, 176,
        161, 42, 207, 136, 168, 208, 135, 154, 4, 113, 181, 248, 90,
    ],
    // level 13
    [
        20, 197, 65, 72, 160, 148, 11, 184, 32, 149, 127, 90, 223, 63, 161, 19, 78, 245, 196, 170,
        161, 19, 244, 100, 100, 88, 242, 112, 224, 191, 191, 208,
    ],
    // level 14
    [
        25, 13, 51, 177, 47, 152, 111, 150, 30, 16, 192, 238, 68, 216, 185, 175, 17, 190, 37, 88,
        140, 173, 137, 212, 22, 17, 142, 75, 244, 235, 232, 12,
    ],
    // level 15
    [
        34, 249, 138, 169, 206, 112, 65, 82, 172, 23, 53, 73, 20, 173, 115, 237, 17, 103, 174, 101,
        150, 175, 81, 10, 165, 179, 100, 147, 37, 224, 108, 146,
    ],
    // level 16
    [
        42, 124, 124, 155, 108, 229, 136, 11, 159, 111, 34, 141, 114, 191, 106, 87, 90, 82, 111,
        41, 198, 110, 204, 238, 248, 183, 83, 211, 139, 186, 115, 35,
    ],
    // level 17
    [
        46, 129, 134, 229, 88, 105, 142, 193, 198, 122, 249, 193, 77, 70, 63, 252, 71, 0, 67, 201,
        194, 152, 139, 149, 77, 117, 221, 100, 63, 54, 185, 146,
    ],
    // level 18
    [
        15, 87, 197, 87, 30, 154, 78, 171, 73, 226, 200, 207, 5, 13, 174, 148, 138, 239, 110, 173,
        100, 115, 146, 39, 53, 70, 36, 157, 28, 31, 241, 15,
    ],
    // level 19
    [
        24, 48, 238, 103, 181, 251, 85, 74, 213, 246, 61, 67, 136, 128, 14, 28, 254, 120, 227, 16,
        105, 125, 70, 228, 60, 156, 227, 97, 52, 247, 44, 202,
    ],
    // level 20
    [
        33, 52, 231, 106, 197, 210, 26, 171, 24, 108, 43, 225, 221, 143, 132, 238, 136, 10, 30, 70,
        234, 247, 18, 249, 211, 113, 182, 223, 34, 25, 31, 62,
    ],
];

/// Derives the ladder from the hash function.
///
/// This is the reference implementation and the source `ZERO_LADDER` was
/// generated from. Nothing on the hot path calls it — the on-chain program and
/// both accumulator views read the baked constant instead — but it is what
/// `the_baked_ladder_matches_the_computed_one` checks that constant against, and
/// it is how the constant is regenerated if the hash ever changes.
pub fn zero_ladder() -> Result<[Field; TREE_DEPTH + 1], MirrorError> {
    let mut z = [Field::ZERO; TREE_DEPTH + 1];
    for i in 1..=TREE_DEPTH {
        z[i] = hash_node(z[i - 1], z[i - 1])?;
    }
    Ok(z)
}

/// The baked ladder as `Field`s, validating canonicality on the way through.
///
/// Costs no hashing, which is why the on-chain deposit path can afford it.
pub fn baked_ladder() -> Result<[Field; TREE_DEPTH + 1], MirrorError> {
    let mut z = [Field::ZERO; TREE_DEPTH + 1];
    for (slot, bytes) in z.iter_mut().zip(ZERO_LADDER.iter()) {
        *slot = Field::from_bytes(*bytes)?;
    }
    Ok(z)
}

/// Insert-only accumulator holding one node per level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frontier {
    filled: [Field; TREE_DEPTH],
    zeros: [Field; TREE_DEPTH + 1],
    next_index: u64,
    root: Field,
}

impl Frontier {
    pub fn new() -> Result<Self, MirrorError> {
        let zeros = baked_ladder()?;
        Ok(Frontier {
            filled: [Field::ZERO; TREE_DEPTH],
            zeros,
            next_index: 0,
            root: zeros[TREE_DEPTH],
        })
    }

    /// Number of leaves inserted so far.
    pub fn len(&self) -> u64 {
        self.next_index
    }

    pub fn is_empty(&self) -> bool {
        self.next_index == 0
    }

    pub fn root(&self) -> Field {
        self.root
    }

    /// Appends a leaf and returns its index.
    pub fn insert(&mut self, leaf: Field) -> Result<u64, MirrorError> {
        if self.next_index >= 1u64 << TREE_DEPTH {
            return Err(MirrorError::TreeFull);
        }
        let index = self.next_index;
        let mut current = leaf;
        let mut path = index;

        for level in 0..TREE_DEPTH {
            if path & 1 == 0 {
                // We are a left child: remember this node and pair with the
                // empty subtree to its right.
                self.filled[level] = current;
                current = hash_node(current, self.zeros[level])?;
            } else {
                // We are a right child: our left sibling is already known.
                current = hash_node(self.filled[level], current)?;
            }
            path >>= 1;
        }

        self.root = current;
        self.next_index += 1;
        Ok(index)
    }
}

/// A membership path: the sibling at each level, plus the leaf index whose bits
/// give the left/right orientation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MerkleProof {
    pub leaf_index: u64,
    pub siblings: [Field; TREE_DEPTH],
}

impl MerkleProof {
    /// Folds the path to a root. The circuit performs exactly this computation
    /// under constraints; this is the native reference it must agree with.
    pub fn fold(&self, leaf: Field) -> Result<Field, MirrorError> {
        let mut current = leaf;
        let mut path = self.leaf_index;
        for sibling in self.siblings.iter() {
            current = if path & 1 == 0 {
                hash_node(current, *sibling)?
            } else {
                hash_node(*sibling, current)?
            };
            path >>= 1;
        }
        Ok(current)
    }
}

/// Host-side full tree. Stores every inserted leaf so it can rebuild any path.
#[derive(Clone, Debug)]
pub struct MerkleTree {
    leaves: Vec<Field>,
    zeros: [Field; TREE_DEPTH + 1],
}

impl MerkleTree {
    pub fn new() -> Result<Self, MirrorError> {
        Ok(MerkleTree {
            leaves: Vec::new(),
            zeros: baked_ladder()?,
        })
    }

    pub fn len(&self) -> usize {
        self.leaves.len()
    }

    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    pub fn insert(&mut self, leaf: Field) -> Result<u64, MirrorError> {
        if self.leaves.len() as u64 >= 1u64 << TREE_DEPTH {
            return Err(MirrorError::TreeFull);
        }
        self.leaves.push(leaf);
        Ok(self.leaves.len() as u64 - 1)
    }

    /// The node at `(level, index)`, materialising empty subtrees from the
    /// zero ladder rather than storing them.
    fn node(&self, level: usize, index: usize) -> Result<Field, MirrorError> {
        if level == 0 {
            return Ok(self.leaves.get(index).copied().unwrap_or(self.zeros[0]));
        }
        let span = 1usize << level;
        if index * span >= self.leaves.len() {
            return Ok(self.zeros[level]);
        }
        let left = self.node(level - 1, index * 2)?;
        let right = self.node(level - 1, index * 2 + 1)?;
        hash_node(left, right)
    }

    pub fn root(&self) -> Result<Field, MirrorError> {
        self.node(TREE_DEPTH, 0)
    }

    /// Builds the membership path for a previously inserted leaf.
    pub fn proof(&self, leaf_index: u64) -> Result<MerkleProof, MirrorError> {
        if leaf_index as usize >= self.leaves.len() {
            return Err(MirrorError::LeafIndexOutOfRange);
        }
        let mut siblings = [Field::ZERO; TREE_DEPTH];
        let mut index = leaf_index as usize;
        for (level, slot) in siblings.iter_mut().enumerate() {
            *slot = self.node(level, index ^ 1)?;
            index >>= 1;
        }
        Ok(MerkleProof {
            leaf_index,
            siblings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(v: u64) -> Field {
        Field::from_u64(v)
    }

    #[test]
    fn an_empty_frontier_and_an_empty_tree_share_a_root() {
        let frontier = Frontier::new().unwrap();
        let tree = MerkleTree::new().unwrap();
        assert_eq!(frontier.root(), tree.root().unwrap());
        assert_eq!(frontier.root(), zero_ladder().unwrap()[TREE_DEPTH]);
    }

    /// The constant is only safe because of this test. If the hash function,
    /// its parameters, or the endianness ever change, the baked ladder becomes
    /// silently wrong — every root would be computed against empty subtrees that
    /// no longer exist, and deposits would still succeed while no proof could
    /// ever verify. This catches that at build time instead.
    #[test]
    fn the_baked_ladder_matches_the_computed_one() {
        let computed = zero_ladder().unwrap();
        let baked = baked_ladder().unwrap();
        for level in 0..=TREE_DEPTH {
            assert_eq!(
                computed[level], baked[level],
                "ZERO_LADDER is stale at level {level}; regenerate it"
            );
        }
    }

    #[test]
    fn the_baked_ladder_entries_are_canonical_field_elements() {
        // A non-canonical constant would be rejected by the Poseidon syscall at
        // runtime, on-chain, with no useful diagnostic.
        for (level, bytes) in ZERO_LADDER.iter().enumerate() {
            assert!(
                Field::from_bytes(*bytes).is_ok(),
                "ZERO_LADDER[{level}] is not a canonical field element"
            );
        }
    }

    #[test]
    fn the_zero_ladder_climbs() {
        let z = zero_ladder().unwrap();
        assert_eq!(z[0], Field::ZERO);
        for i in 1..=TREE_DEPTH {
            assert_eq!(z[i], hash_node(z[i - 1], z[i - 1]).unwrap());
            assert_ne!(z[i], z[i - 1], "level {i} collapsed");
        }
    }

    /// The assertion that connects the two halves: the on-chain accumulator and
    /// a host-generated proof must agree after a real sequence of deposits, not
    /// after a fixture root is injected. Without this test the two can drift
    /// apart indefinitely with every other test still passing.
    #[test]
    fn the_frontier_root_accepts_a_host_generated_proof() {
        let mut frontier = Frontier::new().unwrap();
        let mut tree = MerkleTree::new().unwrap();

        let leaves: Vec<Field> = (1..=9u64).map(f).collect();
        for leaf in &leaves {
            let a = frontier.insert(*leaf).unwrap();
            let b = tree.insert(*leaf).unwrap();
            assert_eq!(a, b, "the two views disagree on leaf index");
            assert_eq!(
                frontier.root(),
                tree.root().unwrap(),
                "roots diverged after {} insertions",
                a + 1
            );
        }

        // Every leaf's path must fold to the accumulator's current root.
        for (i, leaf) in leaves.iter().enumerate() {
            let proof = tree.proof(i as u64).unwrap();
            assert_eq!(
                proof.fold(*leaf).unwrap(),
                frontier.root(),
                "path for leaf {i} does not reach the on-chain root"
            );
        }
    }

    #[test]
    fn a_path_folded_against_the_wrong_leaf_misses_the_root() {
        let mut frontier = Frontier::new().unwrap();
        let mut tree = MerkleTree::new().unwrap();
        for v in 1..=4u64 {
            frontier.insert(f(v)).unwrap();
            tree.insert(f(v)).unwrap();
        }
        let proof = tree.proof(2).unwrap();
        assert_eq!(proof.fold(f(3)).unwrap(), frontier.root());
        assert_ne!(
            proof.fold(f(99)).unwrap(),
            frontier.root(),
            "a forged leaf must not reach the root"
        );
    }

    #[test]
    fn insertion_order_changes_the_root() {
        let mut a = Frontier::new().unwrap();
        let mut b = Frontier::new().unwrap();
        a.insert(f(1)).unwrap();
        a.insert(f(2)).unwrap();
        b.insert(f(2)).unwrap();
        b.insert(f(1)).unwrap();
        assert_ne!(a.root(), b.root());
    }

    #[test]
    fn a_proof_for_an_uninserted_leaf_is_refused() {
        let tree = MerkleTree::new().unwrap();
        assert!(matches!(
            tree.proof(0),
            Err(MirrorError::LeafIndexOutOfRange)
        ));
    }

    #[test]
    fn the_root_advances_on_every_insertion() {
        let mut frontier = Frontier::new().unwrap();
        let mut seen = vec![frontier.root()];
        for v in 1..=8u64 {
            frontier.insert(f(v)).unwrap();
            let root = frontier.root();
            assert!(!seen.contains(&root), "root repeated after inserting {v}");
            seen.push(root);
        }
        assert_eq!(frontier.len(), 8);
    }
}
