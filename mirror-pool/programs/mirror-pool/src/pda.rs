//! Program-derived addresses.
//!
//! One pool per denomination, globally. That is a privacy decision rather than a
//! convenience one: fragmenting deposits of the same size across several pools
//! splits the anonymity set, and a split set is strictly worse for every member
//! in it. Making the denomination the only seed means two users choosing "0.1
//! SOL" cannot accidentally end up in different crowds.

use solana_program::pubkey::Pubkey;

pub const POOL_SEED: &[u8] = b"pool";
pub const VAULT_SEED: &[u8] = b"vault";
pub const SPEND_SEED: &[u8] = b"spend";

/// The pool account for a denomination.
pub fn pool_address(program_id: &Pubkey, denomination: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[POOL_SEED, &denomination.to_le_bytes()], program_id)
}

/// The vault that escrows a pool's notes.
///
/// Kept separate from the pool account so that the accounting invariant reads
/// against a balance holding nothing but escrow and its own rent. Entry fees
/// accrue on the pool account instead, so reward lamports can never be mistaken
/// for lamports backing an unspent note.
pub fn vault_address(program_id: &Pubkey, pool: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[VAULT_SEED, pool.as_ref()], program_id)
}

/// The spend record for a nullifier.
///
/// Seeded by the nullifier itself, so the account's existence *is* the replay
/// guard: a second spend of the same note fails at account creation, before the
/// verifier is even reached.
pub fn spend_address(program_id: &Pubkey, pool: &Pubkey, nullifier: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[SPEND_SEED, pool.as_ref(), nullifier], program_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_deterministic() {
        let program = Pubkey::new_unique();
        let (a, bump_a) = pool_address(&program, 100_000);
        let (b, bump_b) = pool_address(&program, 100_000);
        assert_eq!(a, b);
        assert_eq!(bump_a, bump_b);
    }

    #[test]
    fn each_denomination_gets_its_own_pool() {
        let program = Pubkey::new_unique();
        assert_ne!(
            pool_address(&program, 100_000).0,
            pool_address(&program, 200_000).0
        );
    }

    #[test]
    fn the_vault_is_derived_from_the_pool_not_the_denomination() {
        let program = Pubkey::new_unique();
        let (pool, _) = pool_address(&program, 100_000);
        let (vault, _) = vault_address(&program, &pool);
        assert_ne!(vault, pool);
        assert_eq!(vault, vault_address(&program, &pool).0);
    }

    #[test]
    fn each_nullifier_gets_its_own_spend_record() {
        let program = Pubkey::new_unique();
        let (pool, _) = pool_address(&program, 1);
        let a = spend_address(&program, &pool, &[1u8; 32]).0;
        let b = spend_address(&program, &pool, &[2u8; 32]).0;
        assert_ne!(a, b);
        assert_eq!(a, spend_address(&program, &pool, &[1u8; 32]).0);
    }

    #[test]
    fn one_nullifier_in_two_pools_is_two_records() {
        // Pool-scoped, so a nullifier burned in one denomination does not block
        // an unrelated note in another.
        let program = Pubkey::new_unique();
        let (pool_a, _) = pool_address(&program, 1);
        let (pool_b, _) = pool_address(&program, 2);
        assert_ne!(
            spend_address(&program, &pool_a, &[7u8; 32]).0,
            spend_address(&program, &pool_b, &[7u8; 32]).0
        );
    }

    #[test]
    fn a_different_program_yields_different_addresses() {
        let (a, _) = pool_address(&Pubkey::new_unique(), 1);
        let (b, _) = pool_address(&Pubkey::new_unique(), 1);
        assert_ne!(a, b);
    }
}
