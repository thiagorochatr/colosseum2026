//! The native Stake program, as much of it as a pool needs.
//!
//! Delegation is the case the pool's signature exists for. A member cannot be a
//! stake account's staker authority without appearing on chain and undoing the
//! point, so the pool is that authority and the pool signs — which is a
//! capability a payment never needs.
//!
//! Everything here is a byte layout belonging to somebody else's program, and
//! getting one wrong is not a compile error and rarely a clear runtime one: an
//! account list in the wrong order makes the stake program read the config
//! account as the authority and report a missing signature, which sends the
//! reader looking in the wrong place entirely. So the layouts live in one place
//! with the derivation written out, rather than in each caller that needs them.

use anyhow::{anyhow, Result};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

pub const PROGRAM: &str = "Stake11111111111111111111111111111111111111";
/// Still in `DelegateStake`'s account list, deprecated but not removed.
pub const CONFIG: &str = "StakeConfig11111111111111111111111111111111";
pub const SYSVAR_CLOCK: &str = "SysvarC1ock11111111111111111111111111111111";
pub const SYSVAR_STAKE_HISTORY: &str = "SysvarStakeHistory1111111111111111111111111";
pub const SYSVAR_RENT: &str = "SysvarRent111111111111111111111111111111111";

/// `StakeInstruction::DelegateStake`, a bare u32 discriminant.
pub const DELEGATE_STAKE: [u8; 4] = [2, 0, 0, 0];

/// How many accounts `DelegateStake` takes. Bound into the member's proof, so a
/// settler can neither add one nor drop one.
pub const DELEGATE_ACCOUNTS: u8 = 6;

/// `StakeStateV2` is 200 bytes whatever variant it holds.
pub const ACCOUNT_LEN: usize = 200;

/// The variant a stake account only reaches by being delegated.
///
/// An initialised but undelegated account is variant 1, so the discriminant
/// alone separates "the instruction landed" from "the delegation took" — which
/// is the difference between a signature and a result.
const STATE_STAKE: u32 = 2;

/// Where `Delegation.voter_pubkey` sits inside a serialized `StakeStateV2`.
///
/// Bincode, no padding: a u32 discriminant, then `Meta` — 8 bytes of rent
/// reserve, two 32-byte authorities, and a 48-byte `Lockup` of an i64, a u64 and
/// a 32-byte custodian — and then `Stake`, whose first field is `Delegation`,
/// whose first field is the vote account. 4 + 8 + 64 + 48 = 124.
const VOTER_OFFSET: usize = 124;

pub fn program_id() -> Result<Pubkey> {
    parse(PROGRAM)
}

fn parse(s: &str) -> Result<Pubkey> {
    s.parse().map_err(|e| anyhow!("{s}: {e}"))
}

/// Creates a stake account and initialises its two authorities.
///
/// The pair belongs together: an account created into the stake program and left
/// uninitialised is rent paid for nothing, and both halves fit in one
/// transaction, so they are returned as one unit rather than as two steps a
/// caller could interleave.
///
/// The **staker** and the **withdrawer** are separate arguments and are meant to
/// be different keys. The pool's signature is available to every member, so an
/// authority the vault holds is an authority every member holds: delegation is
/// safe on those terms — the worst a member can do is re-delegate to another
/// validator — and withdrawal is not.
pub fn create(
    payer: &Pubkey,
    stake: &Pubkey,
    staker: &Pubkey,
    withdrawer: &Pubkey,
    lamports: u64,
) -> Result<[Instruction; 2]> {
    let program = program_id()?;
    let create = solana_system_interface::instruction::create_account(
        payer,
        stake,
        lamports,
        ACCOUNT_LEN as u64,
        &program,
    );

    // `StakeInstruction::Initialize { Authorized, Lockup }`: a u32 discriminant,
    // the two authorities, then a zero lockup.
    let mut data = 0u32.to_le_bytes().to_vec();
    data.extend_from_slice(&staker.to_bytes());
    data.extend_from_slice(&withdrawer.to_bytes());
    data.extend_from_slice(&0i64.to_le_bytes()); // lockup.unix_timestamp
    data.extend_from_slice(&0u64.to_le_bytes()); // lockup.epoch
    data.extend_from_slice(&[0u8; 32]); // lockup.custodian
    let initialise = Instruction::new_with_bytes(
        program,
        &data,
        vec![
            AccountMeta::new(*stake, false),
            AccountMeta::new_readonly(parse(SYSVAR_RENT)?, false),
        ],
    );
    Ok([create, initialise])
}

/// The account list `DelegateStake` expects, in the callee's own order.
///
/// Taken from what the `solana` CLI builds rather than from memory. The
/// authority sits in the last slot and the deprecated config account is still in
/// the list; a list that omits it puts the authority one slot early and the
/// stake program reports a missing signature for an account that did sign.
pub fn delegate_accounts(
    stake: &Pubkey,
    vote: &Pubkey,
    authority: &Pubkey,
) -> Result<Vec<AccountMeta>> {
    Ok(vec![
        AccountMeta::new(*stake, false),
        AccountMeta::new_readonly(*vote, false),
        AccountMeta::new_readonly(parse(SYSVAR_CLOCK)?, false),
        AccountMeta::new_readonly(parse(SYSVAR_STAKE_HISTORY)?, false),
        AccountMeta::new_readonly(parse(CONFIG)?, false),
        AccountMeta::new_readonly(*authority, false),
    ])
}

/// Which validator a stake account currently backs.
///
/// Reads the answer out of the account rather than assuming it from what was
/// requested, so a delegation that landed against a different validator than the
/// member asked for is a caught error rather than an unnoticed one.
pub fn delegated_voter(data: &[u8]) -> Result<Pubkey> {
    if data.len() < VOTER_OFFSET + 32 {
        return Err(anyhow!(
            "the stake account is {} bytes, too short to be a stake state",
            data.len()
        ));
    }
    let discriminant = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if discriminant != STATE_STAKE {
        return Err(anyhow!(
            "the stake account is in state {discriminant}, not Stake({STATE_STAKE}): \
             the delegation did not take"
        ));
    }
    Ok(Pubkey::new_from_array(
        data[VOTER_OFFSET..VOTER_OFFSET + 32]
            .try_into()
            .map_err(|_| anyhow!("reading the vote account"))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(discriminant: u32, voter: &Pubkey) -> Vec<u8> {
        let mut data = vec![0u8; ACCOUNT_LEN];
        data[..4].copy_from_slice(&discriminant.to_le_bytes());
        data[VOTER_OFFSET..VOTER_OFFSET + 32].copy_from_slice(&voter.to_bytes());
        data
    }

    #[test]
    fn a_delegated_account_reports_the_validator_it_backs() {
        let voter = Pubkey::new_unique();
        assert_eq!(delegated_voter(&state(STATE_STAKE, &voter)).unwrap(), voter);
    }

    /// The check that separates a signature from a result. An initialised but
    /// undelegated account carries a plausible-looking key at the voter offset —
    /// zeroes, here — and reading it without checking the variant would report a
    /// delegation that never happened.
    #[test]
    fn an_initialised_but_undelegated_account_is_not_read_as_delegated() {
        let err = delegated_voter(&state(1, &Pubkey::new_unique()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("did not take"), "{err}");
    }

    #[test]
    fn an_account_too_short_to_hold_a_delegation_is_refused() {
        assert!(delegated_voter(&[0u8; 64]).is_err());
    }

    /// The order is the whole point of the function, and it is the one thing a
    /// reader cannot check by looking at the callee from here.
    #[test]
    fn the_authority_is_the_last_account_and_the_vote_account_the_second() {
        let stake = Pubkey::new_unique();
        let vote = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let metas = delegate_accounts(&stake, &vote, &authority).unwrap();
        assert_eq!(metas.len(), DELEGATE_ACCOUNTS as usize);
        assert_eq!(metas[0].pubkey, stake);
        assert!(metas[0].is_writable, "the stake account is written");
        assert_eq!(metas[1].pubkey, vote);
        assert_eq!(metas[5].pubkey, authority);
        assert!(
            metas[1..].iter().all(|m| !m.is_writable),
            "only the stake account is written"
        );
    }

    /// The authority the pool holds must not be the authority that can take the
    /// money out. Asserted on the instruction this crate builds, because the
    /// argument order is the only thing standing between the two.
    #[test]
    fn the_staker_and_the_withdrawer_are_written_to_different_slots() {
        let payer = Pubkey::new_unique();
        let stake = Pubkey::new_unique();
        let staker = Pubkey::new_unique();
        let withdrawer = Pubkey::new_unique();
        let [_, initialise] = create(&payer, &stake, &staker, &withdrawer, 1_000_000).unwrap();
        assert_eq!(&initialise.data[4..36], staker.to_bytes().as_slice());
        assert_eq!(&initialise.data[36..68], withdrawer.to_bytes().as_slice());
    }
}
