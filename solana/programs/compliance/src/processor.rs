//! The compliance program's instructions. See docs/SPEC.md section 8.1.

use crate::{
    verify::{is_canonical, verify, ProofRequest},
    ComplianceError,
};
use compliance_interface::{
    clearance, clearance_address, curator, curator_address, mirror_pool, tag, Clearance,
    CLEARANCE_SEED, CURATOR_ROOTS, CURATOR_SEED,
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

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let (&t, rest) = data
        .split_first()
        .ok_or(ComplianceError::UnknownInstruction)?;
    match t {
        tag::VERIFY_PROOF => Ok(verify(&ProofRequest::decode(rest)?)?),
        tag::INIT_CURATOR => init_curator(program_id, accounts, rest),
        tag::PUBLISH_ROOT => publish_root(program_id, accounts, rest),
        tag::REVOKE => revoke(program_id, accounts, rest),
        tag::VERIFY_CLEARANCE => verify_clearance(program_id, accounts, rest),
        tag::UPDATE_POLICY => update_policy(program_id, accounts, rest),
        _ => Err(ComplianceError::UnknownInstruction.into()),
    }
}

fn array32(data: &[u8]) -> Result<[u8; 32], ComplianceError> {
    data.try_into()
        .map_err(|_| ComplianceError::MalformedInstruction)
}

fn u64_at(data: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(data[at..at + 8].try_into().expect("8 bytes"))
}

/// Creates a program-owned account at a PDA, tolerating lamports already sent
/// to it. Every address here is derived from public data, so a plain
/// create_account could be blocked by anyone sending the rent first. This is
/// mirror-pool's create_pda_account.
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

/// A curator account this program owns, with the expected size and version.
fn check_curator(program_id: &Pubkey, account: &AccountInfo) -> Result<(), ComplianceError> {
    if account.owner != program_id {
        return Err(ComplianceError::InvalidAccount);
    }
    let data = account
        .try_borrow_data()
        .map_err(|_| ComplianceError::InvalidAccount)?;
    if data.len() != curator::LEN || data[curator::at::VERSION] != curator::VERSION {
        return Err(ComplianceError::InvalidAccount);
    }
    Ok(())
}

/// The signer must be the authority stored in the curator.
fn check_authority(account: &AccountInfo, authority: &AccountInfo) -> Result<(), ComplianceError> {
    if !authority.is_signer {
        return Err(ComplianceError::MissingSignature);
    }
    let data = account
        .try_borrow_data()
        .map_err(|_| ComplianceError::InvalidAccount)?;
    let at = curator::at::AUTHORITY;
    if data[at..at + 32] != authority.key.to_bytes() {
        return Err(ComplianceError::WrongAuthority);
    }
    Ok(())
}

// ------------------------------------------------------------------- curator

/// Accounts: authority (signer, pays), curator PDA (writable), system program.
/// Data: policy_hash (32), the sha256 of the published policy document.
fn init_curator(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let iter = &mut accounts.iter();
    let authority = next_account_info(iter)?;
    let account = next_account_info(iter)?;
    let system = next_account_info(iter)?;
    let policy_hash = array32(data)?;

    if !authority.is_signer {
        return Err(ComplianceError::MissingSignature.into());
    }
    if !system_program::check_id(system.key) {
        return Err(ComplianceError::InvalidAccount.into());
    }
    let (expected, bump) = curator_address(program_id, authority.key);
    if *account.key != expected {
        return Err(ComplianceError::InvalidAccount.into());
    }
    if !account.data_is_empty() {
        return Err(ComplianceError::CuratorExists.into());
    }
    create_pda_account(
        authority,
        account,
        system,
        curator::LEN,
        program_id,
        &[CURATOR_SEED, authority.key.as_ref(), &[bump]],
    )?;
    let mut d = account.try_borrow_mut_data()?;
    d[curator::at::VERSION] = curator::VERSION;
    d[curator::at::BUMP] = bump;
    d[curator::at::AUTHORITY..curator::at::AUTHORITY + 32].copy_from_slice(authority.key.as_ref());
    d[curator::at::POLICY_HASH..curator::at::POLICY_HASH + 32].copy_from_slice(&policy_hash);
    Ok(())
}

/// Accounts: authority (signer), curator (writable). Data: root (32).
fn publish_root(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let iter = &mut accounts.iter();
    let authority = next_account_info(iter)?;
    let account = next_account_info(iter)?;
    let root = array32(data)?;
    if root == [0u8; 32] || !is_canonical(&root) {
        return Err(ComplianceError::NonCanonicalInput.into());
    }
    check_curator(program_id, account)?;
    check_authority(account, authority)?;

    let mut d = account.try_borrow_mut_data()?;
    let epoch = u64_at(&d, curator::at::EPOCH);
    let pos = u32::from_le_bytes(
        d[curator::at::POS..curator::at::POS + 4]
            .try_into()
            .expect("4 bytes"),
    );
    let next = (pos as usize + 1) % CURATOR_ROOTS;
    let r = curator::at::ROOTS + 32 * next;
    let e = curator::at::ROOT_EPOCHS + 8 * next;
    d[r..r + 32].copy_from_slice(&root);
    d[e..e + 8].copy_from_slice(&epoch.to_le_bytes());
    d[curator::at::POS..curator::at::POS + 4].copy_from_slice(&(next as u32).to_le_bytes());
    Ok(())
}

/// Accounts: authority (signer), curator (writable). Data: policy_hash (32).
///
/// The policy a curator applies can change, for example when a depositor is
/// blocked. The hash on chain must keep matching the policy document the
/// curator publishes, so it changes with it, signed by the same authority.
fn update_policy(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let iter = &mut accounts.iter();
    let authority = next_account_info(iter)?;
    let account = next_account_info(iter)?;
    let policy_hash = array32(data)?;
    check_curator(program_id, account)?;
    check_authority(account, authority)?;
    let mut d = account.try_borrow_mut_data()?;
    d[curator::at::POLICY_HASH..curator::at::POLICY_HASH + 32].copy_from_slice(&policy_hash);
    Ok(())
}

/// Accounts: authority (signer), curator (writable). Starts a new epoch, so no
/// root published before it is accepted any more.
fn revoke(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    if !data.is_empty() {
        return Err(ComplianceError::MalformedInstruction.into());
    }
    let iter = &mut accounts.iter();
    let authority = next_account_info(iter)?;
    let account = next_account_info(iter)?;
    check_curator(program_id, account)?;
    check_authority(account, authority)?;
    let mut d = account.try_borrow_mut_data()?;
    let epoch = u64_at(&d, curator::at::EPOCH)
        .checked_add(1)
        .ok_or(ComplianceError::ArithmeticOverflow)?;
    d[curator::at::EPOCH..curator::at::EPOCH + 8].copy_from_slice(&epoch.to_le_bytes());
    Ok(())
}

// ----------------------------------------------------------------- clearance

/// True if the curator holds `root`, published in its current epoch.
fn curator_holds(data: &[u8], root: &[u8; 32]) -> bool {
    if *root == [0u8; 32] {
        return false;
    }
    let epoch = u64_at(data, curator::at::EPOCH);
    (0..CURATOR_ROOTS).any(|i| {
        let r = curator::at::ROOTS + 32 * i;
        data[r..r + 32] == root[..] && u64_at(data, curator::at::ROOT_EPOCHS + 8 * i) == epoch
    })
}

/// True if `data` is a mirror-pool account whose root history contains `root`.
///
/// The pool's owner is not checked, and it does not need to be: the clearance
/// names this account, and a pool accepts only clearances naming itself. A fake
/// account yields a clearance for a different address, which no pool consumes.
fn pool_knows(data: &[u8], root: &[u8; 32]) -> bool {
    if data.len() != mirror_pool::LEN
        || data[mirror_pool::VERSION_AT] != mirror_pool::VERSION
        || data[mirror_pool::DEPTH_AT] != mirror_pool::DEPTH
        || *root == [0u8; 32]
    {
        return false;
    }
    (0..mirror_pool::ROOT_HISTORY).any(|i| {
        let at = mirror_pool::ROOTS_AT + 32 * i;
        data[at..at + 32] == root[..]
    })
}

/// Accounts: payer (signer), curator, pool, clearance PDA (writable), system program.
/// Data: the proof and its four public inputs, as for VerifyProof.
fn verify_clearance(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let iter = &mut accounts.iter();
    let payer = next_account_info(iter)?;
    let curator_account = next_account_info(iter)?;
    let pool = next_account_info(iter)?;
    let clearance_account = next_account_info(iter)?;
    let system = next_account_info(iter)?;

    let req = ProofRequest::decode(data)?;
    let [pool_root, clean_root, nullifier, binding] = req.inputs;

    if !payer.is_signer {
        return Err(ComplianceError::MissingSignature.into());
    }
    if !system_program::check_id(system.key) {
        return Err(ComplianceError::InvalidAccount.into());
    }
    check_curator(program_id, curator_account)?;

    // Cheap checks first: a stale root or a replay costs an account read, not a pairing.
    let (curator_authority, epoch) = {
        let d = curator_account.try_borrow_data()?;
        if !curator_holds(&d, &clean_root) {
            return Err(ComplianceError::StaleCleanRoot.into());
        }
        let at = curator::at::AUTHORITY;
        (
            Pubkey::new_from_array(d[at..at + 32].try_into().expect("32 bytes")),
            u64_at(&d, curator::at::EPOCH),
        )
    };
    if !pool_knows(&pool.try_borrow_data()?, &pool_root) {
        return Err(ComplianceError::UnknownPoolRoot.into());
    }
    let (expected, bump) = clearance_address(program_id, pool.key, &nullifier);
    if *clearance_account.key != expected {
        return Err(ComplianceError::InvalidAccount.into());
    }
    // A clearance is good for the slot it was written in, since a spend must
    // consume it in that same slot. One left over from an earlier slot, by a
    // spend that never followed or by someone who sent VerifyClearance alone
    // with a proof they saw, is overwritten: refusing it would freeze the note
    // for good. A spent note gains nothing from this, as its spend record
    // already exists and the pool refuses a second spend.
    let slot = Clock::get()?.slot;
    let replace = !clearance_account.data_is_empty();
    if replace {
        if clearance_account.owner != program_id {
            return Err(ComplianceError::InvalidAccount.into());
        }
        let earlier = Clearance::read(&clearance_account.try_borrow_data()?)
            .ok_or(ComplianceError::InvalidAccount)?;
        if earlier.slot == slot {
            return Err(ComplianceError::ClearanceExists.into());
        }
    }

    verify(&req)?;

    if !replace {
        create_pda_account(
            payer,
            clearance_account,
            system,
            clearance::LEN,
            program_id,
            &[CLEARANCE_SEED, pool.key.as_ref(), &nullifier, &[bump]],
        )?;
    }
    let mut d = clearance_account.try_borrow_mut_data()?;
    use clearance::at;
    d[at::VERSION] = clearance::VERSION;
    d[at::BUMP] = bump;
    d[at::SLOT..at::SLOT + 8].copy_from_slice(&slot.to_le_bytes());
    d[at::EPOCH..at::EPOCH + 8].copy_from_slice(&epoch.to_le_bytes());
    d[at::POOL..at::POOL + 32].copy_from_slice(pool.key.as_ref());
    d[at::CURATOR_AUTHORITY..at::CURATOR_AUTHORITY + 32]
        .copy_from_slice(curator_authority.as_ref());
    d[at::NULLIFIER..at::NULLIFIER + 32].copy_from_slice(&nullifier);
    d[at::BINDING..at::BINDING + 32].copy_from_slice(&binding);
    d[at::CLEAN_ROOT..at::CLEAN_ROOT + 32].copy_from_slice(&clean_root);
    Ok(())
}
