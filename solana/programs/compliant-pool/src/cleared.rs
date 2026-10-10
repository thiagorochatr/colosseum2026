//! SubmitClearedSpend: mirror-pool's SubmitSpend, with the clearance in place
//! of the proof. Every other check is mirror-pool's, in mirror-pool's order,
//! and the spend record it writes is byte-for-byte the one SubmitSpend writes,
//! so mirror-pool's own SettleEpoch pays it out unchanged.

use crate::{CompliantPoolError, COMPLIANCE_PROGRAM_ID, REQUIRED_CURATOR};
use compliance_interface::Clearance;
use mirror_core::{action_binding, Field};
use mirror_pool_program::{
    error::MirrorProgramError,
    pda::{spend_address, SPEND_SEED},
    processor::{SELECTOR_INVOKE, SELECTOR_INVOKE_SIGNED, SELECTOR_TRANSFER},
    spend::{spend_len, Spend, MAX_PAYLOAD},
    state::Pool,
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    program::{invoke, invoke_signed},
    pubkey::Pubkey,
    rent::Rent,
    sysvar::Sysvar,
};
use solana_system_interface::{instruction as system_instruction, program as system_program};

/// Data after the tag: nullifier (32), selector (8), target_program (32),
/// beneficiary (32), relay_fee (8), action_accounts (1), payload_len (2), payload.
pub const BASE_LEN: usize = 32 + 8 + 32 + 32 + 8 + 1 + 2;

pub struct ClearedSpend {
    pub nullifier: [u8; 32],
    pub selector: u64,
    pub target_program: [u8; 32],
    pub beneficiary: [u8; 32],
    pub relay_fee: u64,
    pub action_accounts: u8,
    pub payload: Vec<u8>,
}

impl ClearedSpend {
    pub fn decode(data: &[u8]) -> Result<Self, MirrorProgramError> {
        if data.len() < BASE_LEN {
            return Err(MirrorProgramError::MalformedInstruction);
        }
        let a32 = |at: usize| -> [u8; 32] { data[at..at + 32].try_into().expect("32 bytes") };
        let u64_at = |at: usize| u64::from_le_bytes(data[at..at + 8].try_into().expect("8 bytes"));
        let payload_len = u16::from_le_bytes([data[113], data[114]]) as usize;
        // The declared length accounts for every remaining byte, as in mirror-pool.
        if data.len() != BASE_LEN + payload_len {
            return Err(MirrorProgramError::MalformedInstruction);
        }
        Ok(ClearedSpend {
            nullifier: a32(0),
            selector: u64_at(32),
            target_program: a32(40),
            beneficiary: a32(72),
            relay_fee: u64_at(104),
            action_accounts: data[112],
            payload: data[BASE_LEN..].to_vec(),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(BASE_LEN + self.payload.len());
        out.extend_from_slice(&self.nullifier);
        out.extend_from_slice(&self.selector.to_le_bytes());
        out.extend_from_slice(&self.target_program);
        out.extend_from_slice(&self.beneficiary);
        out.extend_from_slice(&self.relay_fee.to_le_bytes());
        out.push(self.action_accounts);
        out.extend_from_slice(&(self.payload.len() as u16).to_le_bytes());
        out.extend_from_slice(&self.payload);
        out
    }
}

/// mirror-pool's squat-resistant PDA creation.
fn create_pda_account<'a>(
    payer: &AccountInfo<'a>,
    account: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
    space: usize,
    owner: &Pubkey,
    seeds: &[&[u8]],
) -> ProgramResult {
    let required = Rent::get()?.minimum_balance(space);
    let held = account.lamports();
    if held < required {
        invoke(
            &system_instruction::transfer(payer.key, account.key, required - held),
            &[payer.clone(), account.clone(), system.clone()],
        )?;
    }
    invoke_signed(
        &system_instruction::allocate(account.key, space as u64),
        &[account.clone(), system.clone()],
        &[seeds],
    )?;
    invoke_signed(
        &system_instruction::assign(account.key, owner),
        &[account.clone(), system.clone()],
        &[seeds],
    )
}

/// Checks the clearance against this spend, in the order the cheapest
/// mismatch is found first.
fn check_clearance(
    clearance: &AccountInfo,
    pool: &Pubkey,
    nullifier: &[u8; 32],
    binding: &Field,
) -> Result<(), CompliantPoolError> {
    if *clearance.owner != COMPLIANCE_PROGRAM_ID {
        return Err(CompliantPoolError::InvalidClearance);
    }
    let data = clearance
        .try_borrow_data()
        .map_err(|_| CompliantPoolError::InvalidClearance)?;
    let c = Clearance::read(&data).ok_or(CompliantPoolError::InvalidClearance)?;
    if c.pool != *pool {
        return Err(CompliantPoolError::ClearanceForAnotherPool);
    }
    if c.nullifier != *nullifier {
        return Err(CompliantPoolError::ClearanceForAnotherNullifier);
    }
    if c.curator_authority != REQUIRED_CURATOR {
        return Err(CompliantPoolError::WrongCurator);
    }
    let slot = Clock::get()
        .map_err(|_| CompliantPoolError::StaleClearance)?
        .slot;
    if c.slot != slot {
        return Err(CompliantPoolError::StaleClearance);
    }
    if c.binding != binding.to_bytes() {
        return Err(CompliantPoolError::BindingMismatch);
    }
    Ok(())
}

/// Accounts: relay (signer, pays), pool, spend PDA (writable), system program, clearance.
pub fn submit_cleared_spend(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let req = ClearedSpend::decode(data)?;
    let iter = &mut accounts.iter();
    let relay = next_account_info(iter)?;
    let pool_account = next_account_info(iter)?;
    let spend_account = next_account_info(iter)?;
    let system = next_account_info(iter)?;
    let clearance = next_account_info(iter)?;

    // From here to the binding, mirror-pool's submit_spend, minus the root.
    if !relay.is_signer {
        return Err(MirrorProgramError::MissingSignature.into());
    }
    if !system_program::check_id(system.key) {
        return Err(MirrorProgramError::InvalidOwner.into());
    }
    if pool_account.owner != program_id {
        return Err(MirrorProgramError::InvalidOwner.into());
    }
    Field::from_bytes(req.nullifier).map_err(MirrorProgramError::from)?;

    let (denomination, k_floor, deposit_count) = {
        let mut data = pool_account.try_borrow_mut_data()?;
        let pool = Pool::load(&mut data)?;
        (pool.denomination(), pool.k_floor(), pool.deposit_count())
    };
    if req.relay_fee >= denomination {
        return Err(MirrorProgramError::RelayFeeTooLarge.into());
    }
    if deposit_count < k_floor as u64 {
        return Err(MirrorProgramError::BelowAnonymityFloor.into());
    }

    let (expected_spend, spend_bump) = spend_address(program_id, pool_account.key, &req.nullifier);
    if *spend_account.key != expected_spend {
        return Err(MirrorProgramError::InvalidPda.into());
    }
    if !spend_account.data_is_empty() {
        return Err(MirrorProgramError::NullifierAlreadySpent.into());
    }
    if req.payload.len() > MAX_PAYLOAD {
        return Err(MirrorProgramError::PayloadTooLarge.into());
    }
    match req.selector {
        SELECTOR_TRANSFER if req.action_accounts != 0 => {
            return Err(MirrorProgramError::MalformedInstruction.into());
        }
        SELECTOR_TRANSFER | SELECTOR_INVOKE | SELECTOR_INVOKE_SIGNED => {}
        _ => return Err(MirrorProgramError::UnknownSelector.into()),
    }
    let binding = action_binding(
        req.selector,
        &req.target_program,
        &req.beneficiary,
        &relay.key.to_bytes(),
        req.relay_fee,
        req.action_accounts,
        &req.payload,
    );

    // In place of the pairing: the clearance the compliance program wrote.
    check_clearance(clearance, pool_account.key, &req.nullifier, &binding)?;

    // From here on, mirror-pool's submit_spend again: the same record.
    create_pda_account(
        relay,
        spend_account,
        system,
        spend_len(req.payload.len()),
        program_id,
        &[
            SPEND_SEED,
            pool_account.key.as_ref(),
            &req.nullifier,
            &[spend_bump],
        ],
    )?;
    let now = Clock::get()?.unix_timestamp;
    let mut spend_data = spend_account.try_borrow_mut_data()?;
    let mut record = Spend::load_uninitialised(&mut spend_data)?;
    record.initialise(
        spend_bump,
        req.selector,
        req.relay_fee,
        now,
        &req.beneficiary,
        &relay.key.to_bytes(),
        &pool_account.key.to_bytes(),
        &req.target_program,
        req.action_accounts,
        &req.payload,
    )?;
    Ok(())
}
