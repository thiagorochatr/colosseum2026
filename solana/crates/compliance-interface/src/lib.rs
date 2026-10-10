//! What the compliance program and a consuming pool must agree on: the layout
//! of a clearance, the layout of a curator, their seeds, and the instruction
//! tags. See docs/SPEC.md section 8.
//!
//! Kept free of logic so a pool can depend on it without pulling the verifier.

#![forbid(unsafe_code)]
#![no_std]

use solana_program::pubkey::Pubkey;

pub const CURATOR_SEED: &[u8] = b"curator";
pub const CLEARANCE_SEED: &[u8] = b"clearance";

/// Number of clean roots a curator keeps.
pub const CURATOR_ROOTS: usize = 32;

/// Instruction tags of the compliance program.
pub mod tag {
    /// Verifies a withdraw proof and nothing else.
    pub const VERIFY_PROOF: u8 = 0;
    /// Creates a curator. Data: policy_hash (32).
    pub const INIT_CURATOR: u8 = 1;
    /// Pushes a clean root in the current epoch. Data: root (32).
    pub const PUBLISH_ROOT: u8 = 2;
    /// Starts a new epoch, invalidating every root published so far.
    pub const REVOKE: u8 = 3;
    /// Verifies a proof against a pool and a curator, and writes a clearance.
    /// Data: a (64), b (128), c (64), poolRoot, cleanRoot, nullifierHash, binding.
    pub const VERIFY_CLEARANCE: u8 = 4;
    /// Replaces the curator's policy hash, when the policy it applies changes.
    /// Data: policy_hash (32).
    pub const UPDATE_POLICY: u8 = 5;
}

/// A curator: who publishes clean roots for which policy.
pub mod curator {
    pub const VERSION: u8 = 1;
    pub mod at {
        pub const VERSION: usize = 0;
        pub const BUMP: usize = 1;
        pub const EPOCH: usize = 2;
        pub const POS: usize = 10;
        pub const AUTHORITY: usize = 14;
        pub const POLICY_HASH: usize = 46;
        pub const ROOTS: usize = 78;
        pub const ROOT_EPOCHS: usize = ROOTS + 32 * super::super::CURATOR_ROOTS;
        pub const END: usize = ROOT_EPOCHS + 8 * super::super::CURATOR_ROOTS;
    }
    pub const LEN: usize = at::END;
}

/// A clearance: proof that pool `pool` spent `nullifier`, bound to `binding`,
/// under a clean root of curator `curator_authority`, verified in `slot`.
pub mod clearance {
    pub const VERSION: u8 = 1;
    pub mod at {
        pub const VERSION: usize = 0;
        pub const BUMP: usize = 1;
        pub const SLOT: usize = 2;
        pub const EPOCH: usize = 10;
        pub const POOL: usize = 18;
        pub const CURATOR_AUTHORITY: usize = 50;
        pub const NULLIFIER: usize = 82;
        pub const BINDING: usize = 114;
        pub const CLEAN_ROOT: usize = 146;
        pub const END: usize = 178;
    }
    pub const LEN: usize = at::END;
}

/// The mirror-pool account layout the compliance program reads a pool root
/// from. A test in the compliance program checks it against mirror-pool.
pub mod mirror_pool {
    pub const LEN: usize = 4792;
    pub const VERSION_AT: usize = 0;
    pub const VERSION: u8 = 1;
    pub const DEPTH_AT: usize = 3;
    pub const DEPTH: u8 = 20;
    pub const ROOTS_AT: usize = 696;
    pub const ROOT_HISTORY: usize = 128;
}

pub fn curator_address(program_id: &Pubkey, authority: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[CURATOR_SEED, authority.as_ref()], program_id)
}

pub fn clearance_address(program_id: &Pubkey, pool: &Pubkey, nullifier: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[CLEARANCE_SEED, pool.as_ref(), nullifier], program_id)
}

fn read32(data: &[u8], at: usize) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&data[at..at + 32]);
    out
}

fn read_u64(data: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&data[at..at + 8]);
    u64::from_le_bytes(b)
}

/// A clearance read back from its account data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clearance {
    pub slot: u64,
    pub epoch: u64,
    pub pool: Pubkey,
    pub curator_authority: Pubkey,
    pub nullifier: [u8; 32],
    pub binding: [u8; 32],
    pub clean_root: [u8; 32],
}

impl Clearance {
    /// None unless the data is a version-1 clearance of exactly the right size.
    pub fn read(data: &[u8]) -> Option<Self> {
        if data.len() != clearance::LEN || data[clearance::at::VERSION] != clearance::VERSION {
            return None;
        }
        use clearance::at;
        Some(Clearance {
            slot: read_u64(data, at::SLOT),
            epoch: read_u64(data, at::EPOCH),
            pool: Pubkey::new_from_array(read32(data, at::POOL)),
            curator_authority: Pubkey::new_from_array(read32(data, at::CURATOR_AUTHORITY)),
            nullifier: read32(data, at::NULLIFIER),
            binding: read32(data, at::BINDING),
            clean_root: read32(data, at::CLEAN_ROOT),
        })
    }
}
