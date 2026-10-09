//! The spend record.
//!
//! One account per nullifier, seeded by it. Its *existence* is the replay
//! guard — `submit_spend` refuses outright if it is already there — and its
//! contents are the action the proof authorised, held until settlement.
//!
//! Folding the nullifier marker and the pending action into one account is
//! deliberate: two accounts keyed by the same value can disagree, and an account
//! that exists to be checked but never read is easy to forget to check.

use crate::error::MirrorProgramError;

/// Not yet executed.
pub const STATUS_PENDING: u8 = 1;
/// Executed at settlement.
pub const STATUS_SETTLED: u8 = 2;

mod offset {
    pub const VERSION: usize = 0;
    pub const STATUS: usize = 1;
    pub const BUMP: usize = 2;
    pub const ACTION_ACCOUNTS: usize = 3;
    pub const PAYLOAD_LEN: usize = 4;
    pub const SELECTOR: usize = 8;
    pub const RELAY_FEE: usize = 16;
    pub const SUBMITTED_AT: usize = 24;
    pub const BENEFICIARY: usize = 32;
    pub const RELAY: usize = 64;
    pub const POOL: usize = 96;
    pub const TARGET_PROGRAM: usize = 128;
    pub const PAYLOAD: usize = 160;
}

/// Fixed part of the record. The payload follows it.
pub const SPEND_BASE_LEN: usize = offset::PAYLOAD;

/// Largest action payload a spend may carry.
///
/// Bounded because the payload lives in the record, which is what lets
/// settlement batch several actions into one transaction: the instruction data
/// is already on chain, so a settle transaction carries only account references.
/// An unbounded payload would trade the synchronised crowd for expressiveness.
pub const MAX_PAYLOAD: usize = 256;

pub const SPEND_VERSION: u8 = 1;

const _: () = assert!(SPEND_BASE_LEN == 160);

/// Account size for a record carrying `payload_len` bytes.
pub fn spend_len(payload_len: usize) -> usize {
    SPEND_BASE_LEN + payload_len
}

pub struct Spend<'a> {
    data: &'a mut [u8],
}

impl<'a> Spend<'a> {
    pub fn load(data: &'a mut [u8]) -> Result<Self, MirrorProgramError> {
        // The upper bound is what stops this from being a type confusion.
        //
        // `POOL_VERSION` and `SPEND_VERSION` are both 1 and both live at offset
        // zero, so the version byte distinguishes *layout revisions* and not
        // account *kinds*: a pool account passed where a spend record is
        // expected clears the version check. Settlement's other checks —
        // `record.pool()` against the pool key — would then have to be read out
        // of Merkle frontier bytes and are what actually refuse it, which is a
        // coincidence to depend on rather than a rule. No spend record is ever
        // larger than this, and every other account this program owns is, so
        // one comparison makes the confusion unrepresentable.
        if data.len() < SPEND_BASE_LEN
            || data.len() > spend_len(MAX_PAYLOAD)
            || data[offset::VERSION] != SPEND_VERSION
        {
            return Err(MirrorProgramError::InvalidSpendAccount);
        }
        let spend = Spend { data };
        // The declared payload length must match the account, or a reader could
        // be pointed past the end of the data it was given.
        if spend.data.len() != spend_len(spend.payload_len()) {
            return Err(MirrorProgramError::InvalidSpendAccount);
        }
        Ok(spend)
    }

    pub fn load_uninitialised(data: &'a mut [u8]) -> Result<Self, MirrorProgramError> {
        if data.len() < SPEND_BASE_LEN || data.len() > spend_len(MAX_PAYLOAD) {
            return Err(MirrorProgramError::InvalidSpendAccount);
        }
        if data[offset::VERSION] != 0 {
            return Err(MirrorProgramError::NullifierAlreadySpent);
        }
        Ok(Spend { data })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn initialise(
        &mut self,
        bump: u8,
        selector: u64,
        relay_fee: u64,
        submitted_at: i64,
        beneficiary: &[u8; 32],
        relay: &[u8; 32],
        pool: &[u8; 32],
        target_program: &[u8; 32],
        action_accounts: u8,
        payload: &[u8],
    ) -> Result<(), MirrorProgramError> {
        if payload.len() > MAX_PAYLOAD || self.data.len() != spend_len(payload.len()) {
            return Err(MirrorProgramError::InvalidSpendAccount);
        }
        self.data[offset::VERSION] = SPEND_VERSION;
        self.data[offset::STATUS] = STATUS_PENDING;
        self.data[offset::BUMP] = bump;
        self.data[offset::ACTION_ACCOUNTS] = action_accounts;
        self.data[offset::PAYLOAD_LEN..offset::PAYLOAD_LEN + 2]
            .copy_from_slice(&(payload.len() as u16).to_le_bytes());
        self.data[offset::TARGET_PROGRAM..offset::TARGET_PROGRAM + 32]
            .copy_from_slice(target_program);
        self.data[offset::PAYLOAD..offset::PAYLOAD + payload.len()].copy_from_slice(payload);
        self.data[offset::SELECTOR..offset::SELECTOR + 8].copy_from_slice(&selector.to_le_bytes());
        self.data[offset::RELAY_FEE..offset::RELAY_FEE + 8]
            .copy_from_slice(&relay_fee.to_le_bytes());
        self.data[offset::SUBMITTED_AT..offset::SUBMITTED_AT + 8]
            .copy_from_slice(&submitted_at.to_le_bytes());
        self.data[offset::BENEFICIARY..offset::BENEFICIARY + 32].copy_from_slice(beneficiary);
        self.data[offset::RELAY..offset::RELAY + 32].copy_from_slice(relay);
        self.data[offset::POOL..offset::POOL + 32].copy_from_slice(pool);
        Ok(())
    }

    /// How many accounts the action's CPI expects.
    pub fn action_accounts(&self) -> u8 {
        self.data[offset::ACTION_ACCOUNTS]
    }

    pub fn payload_len(&self) -> usize {
        let mut b = [0u8; 2];
        b.copy_from_slice(&self.data[offset::PAYLOAD_LEN..offset::PAYLOAD_LEN + 2]);
        u16::from_le_bytes(b) as usize
    }

    /// The program the pool will invoke on this member's behalf.
    pub fn target_program(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        b.copy_from_slice(&self.data[offset::TARGET_PROGRAM..offset::TARGET_PROGRAM + 32]);
        b
    }

    /// The instruction data for that invocation, stored at submit time so a
    /// settle transaction carries only account references.
    pub fn payload(&self) -> &[u8] {
        &self.data[offset::PAYLOAD..offset::PAYLOAD + self.payload_len()]
    }

    pub fn status(&self) -> u8 {
        self.data[offset::STATUS]
    }
    pub fn bump(&self) -> u8 {
        self.data[offset::BUMP]
    }
    pub fn selector(&self) -> u64 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.data[offset::SELECTOR..offset::SELECTOR + 8]);
        u64::from_le_bytes(b)
    }
    pub fn relay_fee(&self) -> u64 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.data[offset::RELAY_FEE..offset::RELAY_FEE + 8]);
        u64::from_le_bytes(b)
    }
    /// Unix time the spend was accepted. Settlement uses it to decide whether a
    /// batch may go out below the crowd size.
    pub fn submitted_at(&self) -> i64 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.data[offset::SUBMITTED_AT..offset::SUBMITTED_AT + 8]);
        i64::from_le_bytes(b)
    }
    pub fn beneficiary(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        b.copy_from_slice(&self.data[offset::BENEFICIARY..offset::BENEFICIARY + 32]);
        b
    }
    pub fn relay(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        b.copy_from_slice(&self.data[offset::RELAY..offset::RELAY + 32]);
        b
    }
    /// The pool this spend belongs to.
    ///
    /// Settlement checks it, so a record from one denomination cannot be
    /// presented to another pool's vault.
    pub fn pool(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        b.copy_from_slice(&self.data[offset::POOL..offset::POOL + 32]);
        b
    }

    /// Marks the spend executed, refusing to do so twice.
    pub fn mark_settled(&mut self) -> Result<(), MirrorProgramError> {
        if self.status() != STATUS_PENDING {
            return Err(MirrorProgramError::AlreadySettled);
        }
        self.data[offset::STATUS] = STATUS_SETTLED;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYLOAD: &[u8] = b"delegate stake";

    fn fresh() -> Vec<u8> {
        vec![0u8; spend_len(PAYLOAD.len())]
    }

    fn init(data: &mut [u8]) {
        let mut s = Spend::load_uninitialised(data).unwrap();
        s.initialise(
            251,
            7,
            12_345,
            1_700_000_000,
            &[9u8; 32],
            &[4u8; 32],
            &[5u8; 32],
            &[6u8; 32],
            4,
            PAYLOAD,
        )
        .unwrap();
    }

    #[test]
    fn a_spend_reads_back_what_was_written() {
        let mut data = fresh();
        init(&mut data);
        let s = Spend::load(&mut data).unwrap();
        assert_eq!(s.status(), STATUS_PENDING);
        assert_eq!(s.bump(), 251);
        assert_eq!(s.selector(), 7);
        assert_eq!(s.relay_fee(), 12_345);
        assert_eq!(s.submitted_at(), 1_700_000_000);
        assert_eq!(s.beneficiary(), [9u8; 32]);
        assert_eq!(s.relay(), [4u8; 32]);
        assert_eq!(s.pool(), [5u8; 32]);
        assert_eq!(s.target_program(), [6u8; 32]);
        assert_eq!(s.action_accounts(), 4);
        assert_eq!(s.payload(), PAYLOAD);
    }

    /// The action a member proved must survive settlement byte for byte: a
    /// truncated or padded payload would invoke the target with parameters
    /// nobody authorised.
    #[test]
    fn a_payload_round_trips_at_every_length() {
        for len in [0usize, 1, 31, 32, 33, MAX_PAYLOAD] {
            let payload: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let mut data = vec![0u8; spend_len(len)];
            {
                let mut s = Spend::load_uninitialised(&mut data).unwrap();
                s.initialise(
                    1, 0, 0, 0, &[0u8; 32], &[0u8; 32], &[0u8; 32], &[0u8; 32], 0, &payload,
                )
                .unwrap();
            }
            let s = Spend::load(&mut data).unwrap();
            assert_eq!(s.payload(), &payload[..], "length {len} did not round trip");
            assert_eq!(s.payload_len(), len);
        }
    }

    #[test]
    fn a_payload_beyond_the_maximum_is_refused() {
        let payload = vec![0u8; MAX_PAYLOAD + 1];
        let mut data = vec![0u8; spend_len(payload.len())];
        assert!(Spend::load_uninitialised(&mut data).is_err());
    }

    /// An account whose length disagrees with its declared payload length would
    /// let a reader run past the end of what it was given.
    #[test]
    fn a_record_whose_length_contradicts_its_header_is_refused() {
        let mut data = fresh();
        init(&mut data);
        data.push(0);
        assert!(matches!(
            Spend::load(&mut data),
            Err(MirrorProgramError::InvalidSpendAccount)
        ));
    }

    #[test]
    fn an_existing_record_refuses_reinitialisation() {
        let mut data = fresh();
        init(&mut data);
        assert!(matches!(
            Spend::load_uninitialised(&mut data),
            Err(MirrorProgramError::NullifierAlreadySpent)
        ));
    }

    #[test]
    fn a_spend_settles_exactly_once() {
        let mut data = fresh();
        init(&mut data);
        let mut s = Spend::load(&mut data).unwrap();
        s.mark_settled().unwrap();
        assert_eq!(s.status(), STATUS_SETTLED);
        assert!(matches!(
            s.mark_settled(),
            Err(MirrorProgramError::AlreadySettled)
        ));
    }

    #[test]
    fn a_wrong_length_account_is_refused() {
        let mut short = vec![0u8; SPEND_BASE_LEN - 1];
        assert!(Spend::load(&mut short).is_err());
        assert!(Spend::load_uninitialised(&mut short).is_err());
    }

    #[test]
    fn fields_do_not_overlap() {
        // Distinct values in every field; if two overlapped, one would clobber
        // the other and this would fail.
        let mut data = fresh();
        {
            let mut s = Spend::load_uninitialised(&mut data).unwrap();
            s.initialise(
                0xEE,
                0x1122_3344_5566_7788,
                0x99AA_BBCC_DDEE_FF00,
                -0x0102_0304_0506_0708,
                &[0xAB; 32],
                &[0xCD; 32],
                &[0xEF; 32],
                &[0x12; 32],
                7,
                PAYLOAD,
            )
            .unwrap();
        }
        let s = Spend::load(&mut data).unwrap();
        assert_eq!(s.bump(), 0xEE);
        assert_eq!(s.selector(), 0x1122_3344_5566_7788);
        assert_eq!(s.relay_fee(), 0x99AA_BBCC_DDEE_FF00);
        assert_eq!(s.submitted_at(), -0x0102_0304_0506_0708);
        assert_eq!(s.beneficiary(), [0xAB; 32]);
        assert_eq!(s.relay(), [0xCD; 32]);
        assert_eq!(s.pool(), [0xEF; 32]);
        assert_eq!(s.target_program(), [0x12; 32]);
        assert_eq!(s.action_accounts(), 7);
    }

    /// A pool account must never load as a spend record.
    ///
    /// Both carry version 1 at offset zero, so the version byte cannot tell them
    /// apart. This pins the size bound that does. The value is not
    /// `POOL_LEN` by import but by construction: any account bigger than the
    /// largest possible record is refused, whatever it happens to be.
    #[test]
    fn an_account_too_large_to_be_a_record_is_refused() {
        let mut pool_sized = vec![0u8; 4792];
        pool_sized[offset::VERSION] = SPEND_VERSION;
        // Declare exactly the payload length that makes the arithmetic close,
        // which is what a colliding account would have to do.
        let declared = (4792 - SPEND_BASE_LEN) as u16;
        pool_sized[offset::PAYLOAD_LEN..offset::PAYLOAD_LEN + 2]
            .copy_from_slice(&declared.to_le_bytes());
        assert!(
            Spend::load(&mut pool_sized).is_err(),
            "an account far larger than any record loaded as one"
        );
    }

    #[test]
    fn the_largest_legal_record_still_loads() {
        let mut data = vec![0u8; spend_len(MAX_PAYLOAD)];
        {
            let mut spend = Spend::load_uninitialised(&mut data).unwrap();
            spend
                .initialise(
                    1,
                    0,
                    0,
                    0,
                    &[0; 32],
                    &[0; 32],
                    &[0; 32],
                    &[0; 32],
                    0,
                    &[7u8; MAX_PAYLOAD],
                )
                .unwrap();
        }
        assert_eq!(Spend::load(&mut data).unwrap().payload().len(), MAX_PAYLOAD);
    }
}
