//! mirror-pool: a behavioral anonymity set. A member's protocol action is
//! executed by the pool PDA, gated by a Groth16 membership proof verified
//! on-chain, so an observer sees that an action happened but cannot attribute it
//! to a member.
#![forbid(unsafe_code)]

pub mod error;
pub mod instruction;
pub mod pda;
pub mod processor;
pub mod spend;
pub mod state;
pub mod vk;

pub use error::MirrorProgramError;
pub use instruction::Instruction;
pub use state::{Pool, POOL_LEN, POOL_VERSION, ROOT_HISTORY};

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(entry);

#[cfg(not(feature = "no-entrypoint"))]
fn entry(
    program_id: &solana_program::pubkey::Pubkey,
    accounts: &[solana_program::account_info::AccountInfo],
    data: &[u8],
) -> solana_program::entrypoint::ProgramResult {
    processor::process(program_id, accounts, data)
}
