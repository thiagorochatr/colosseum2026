//! A compliant mirror-pool. See docs/SPEC.md section 8.2.
//!
//! mirror-pool itself is untouched: this program links it as a library and
//! hands it every instruction except one. Its plain SubmitSpend, which pays
//! against a membership proof alone, is refused. In its place,
//! SubmitClearedSpend records exactly the same spend, but only against a
//! clearance from the compliance program that:
//!
//! - names this pool and this nullifier,
//! - was issued by the curator this deployment is bound to,
//! - was issued in this same slot, so it cannot outlive a curator's revocation,
//! - carries the action binding this spend recomputes from its own fields and
//!   its relay's key, exactly as mirror-pool computes it.
//!
//! The curator and the compliance program are fixed when the program is built,
//! from MIRROR_COMPLIANCE_PROGRAM_ID and MIRROR_REQUIRED_CURATOR. mirror-pool
//! creates pools permissionlessly and names them by denomination alone, so a
//! curator chosen per pool would let anyone take a denomination with a curator
//! of their own. Binding it to the deployment closes that.

#![forbid(unsafe_code)]

pub mod cleared;
pub mod error;

pub use error::CompliantPoolError;

use solana_program::pubkey::Pubkey;

/// The compliance program whose clearances this pool accepts.
pub const COMPLIANCE_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const(env!("MIRROR_COMPLIANCE_PROGRAM_ID"));
/// The curator authority whose clean roots this pool accepts.
pub const REQUIRED_CURATOR: Pubkey = Pubkey::from_str_const(env!("MIRROR_REQUIRED_CURATOR"));

/// mirror-pool's SubmitSpend tag, refused here.
pub const TAG_SUBMIT_SPEND: u8 = 2;
/// The one instruction this program adds. Tags 0, 1 and 3 are mirror-pool's.
pub const TAG_SUBMIT_CLEARED_SPEND: u8 = 4;

pub fn process(
    program_id: &Pubkey,
    accounts: &[solana_program::account_info::AccountInfo],
    data: &[u8],
) -> solana_program::entrypoint::ProgramResult {
    match data.first() {
        Some(&TAG_SUBMIT_SPEND) => Err(CompliantPoolError::ClearanceRequired.into()),
        Some(&TAG_SUBMIT_CLEARED_SPEND) => {
            cleared::submit_cleared_spend(program_id, accounts, &data[1..])
        }
        _ => mirror_pool_program::processor::process(program_id, accounts, data),
    }
}

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(entry);

#[cfg(not(feature = "no-entrypoint"))]
fn entry(
    program_id: &Pubkey,
    accounts: &[solana_program::account_info::AccountInfo],
    data: &[u8],
) -> solana_program::entrypoint::ProgramResult {
    process(program_id, accounts, data)
}
