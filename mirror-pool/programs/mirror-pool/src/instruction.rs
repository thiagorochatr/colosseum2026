//! Instruction encoding.
//!
//! Every variant parses at an exact length. A trailing byte, a missing byte, or
//! an unknown tag is rejected rather than tolerated — there is no "read what you
//! need and ignore the rest", because that is how a caller ends up believing it
//! sent a field the program never read.
//!
//! Encoding is hand-rolled rather than borsh: the instruction data is part of
//! the on-chain ABI, the shapes are small and fixed, and a hand-rolled decoder
//! is one place to audit for length handling.

use crate::error::MirrorProgramError;

/// Instruction tags. Stable across versions; new instructions take new tags
/// rather than reusing a retired one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Tag {
    InitPool = 0,
    Deposit = 1,
    SubmitSpend = 2,
    SettleEpoch = 3,
}

impl Tag {
    fn from_u8(v: u8) -> Result<Self, MirrorProgramError> {
        match v {
            0 => Ok(Tag::InitPool),
            1 => Ok(Tag::Deposit),
            2 => Ok(Tag::SubmitSpend),
            3 => Ok(Tag::SettleEpoch),
            _ => Err(MirrorProgramError::MalformedInstruction),
        }
    }
}

/// `SubmitSpend` carries a 256-byte proof, so it is far larger than the other
/// variants. Boxing it — clippy's usual remedy — would move it to the heap, and
/// a heap allocation inside an on-chain program costs compute units to avoid a
/// stack cost we can comfortably afford: 368 bytes against a 4 KB frame, decoded
/// once per transaction. The size difference is deliberate.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instruction {
    /// Creates the pool and its vault, and seeds the accumulator to an empty
    /// tree.
    InitPool {
        denomination: u64,
        entry_fee: u64,
        k_floor: u32,
        /// How long a spend waits before it may settle below the floor. Zero
        /// means the program's default, which is what every pool created before
        /// this field existed holds.
        settle_timeout_seconds: u32,
    },
    /// Escrows exactly `denomination + entry_fee` and appends `commitment` to
    /// the accumulator.
    ///
    /// The amount is not a parameter. It is read from the pool, so a deposit
    /// cannot claim a size the pool did not set.
    Deposit { commitment: [u8; 32] },
    /// Proves membership, burns the nullifier, and records the authorised
    /// action. Nothing is paid out here — settlement executes the batch.
    ///
    /// The action binding is **not** transmitted. It is recomputed on-chain from
    /// `selector`, `beneficiary` and `relay_fee` and used as the third public
    /// input, so a relay that alters any of them produces a different binding
    /// and the pairing simply fails. There is no separate field to forget to
    /// check.
    SubmitSpend {
        proof_a: [u8; 64],
        proof_b: [u8; 128],
        proof_c: [u8; 64],
        root: [u8; 32],
        nullifier: [u8; 32],
        selector: u64,
        /// The program the pool will invoke on the member's behalf.
        target_program: [u8; 32],
        beneficiary: [u8; 32],
        relay_fee: u64,
        /// How many accounts that invocation expects.
        action_accounts: u8,
        /// Instruction data for the invocation, stored in the spend record so a
        /// settle transaction carries only account references and several
        /// actions still fit in one transaction.
        payload: Vec<u8>,
    },
    /// Executes a batch of pending spends in one transaction, so every payout in
    /// an epoch shares a timestamp and an ordering.
    ///
    /// `count` is the number of spend records that follow in the account list.
    /// Permissionless: anyone may settle, so no operator's absence can strand a
    /// member's funds.
    SettleEpoch {
        count: u8,
        /// Consent to settling a batch smaller than the pool's floor.
        ///
        /// The flag does not skip the timeout — an under-floor batch still has
        /// to wait it out. What it does is stop that settlement from happening
        /// by accident: a batch below the floor costs its members the anonymity
        /// set they deposited for, and the program should hear somebody say so
        /// rather than infer it from a count.
        allow_below_floor: bool,
    },
}

/// `InitPool`: tag + u64 + u64 + u32 + u32.
pub const INIT_POOL_LEN: usize = 1 + 8 + 8 + 4 + 4;
/// `Deposit`: tag + one field element.
pub const DEPOSIT_LEN: usize = 1 + 32;
/// `SubmitSpend` without its payload. The encoding is variable length.
pub const SUBMIT_SPEND_BASE_LEN: usize = 1 + 64 + 128 + 64 + 32 + 32 + 8 + 32 + 32 + 8 + 1 + 2;
/// `SettleEpoch`: tag + count + the below-floor flag.
pub const SETTLE_EPOCH_LEN: usize = 1 + 1 + 1;

fn read_u64(data: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&data[at..at + 8]);
    u64::from_le_bytes(b)
}

fn read_u32(data: &[u8], at: usize) -> u32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&data[at..at + 4]);
    u32::from_le_bytes(b)
}

impl Instruction {
    pub fn unpack(data: &[u8]) -> Result<Self, MirrorProgramError> {
        let tag = Tag::from_u8(
            *data
                .first()
                .ok_or(MirrorProgramError::MalformedInstruction)?,
        )?;

        match tag {
            Tag::InitPool => {
                if data.len() != INIT_POOL_LEN {
                    return Err(MirrorProgramError::MalformedInstruction);
                }
                Ok(Instruction::InitPool {
                    denomination: read_u64(data, 1),
                    entry_fee: read_u64(data, 9),
                    k_floor: read_u32(data, 17),
                    settle_timeout_seconds: read_u32(data, 21),
                })
            }
            Tag::Deposit => {
                if data.len() != DEPOSIT_LEN {
                    return Err(MirrorProgramError::MalformedInstruction);
                }
                let mut commitment = [0u8; 32];
                commitment.copy_from_slice(&data[1..33]);
                Ok(Instruction::Deposit { commitment })
            }
            Tag::SettleEpoch => {
                if data.len() != SETTLE_EPOCH_LEN {
                    return Err(MirrorProgramError::MalformedInstruction);
                }
                // Exactly zero or one. A bool is one bit of meaning and the
                // wire gives it eight, so seven of them have no defined value —
                // and "anything nonzero is true" would let a caller send 0x02
                // believing they had asked for something else. Refused for the
                // same reason a trailing byte is.
                let allow_below_floor = match data[2] {
                    0 => false,
                    1 => true,
                    _ => return Err(MirrorProgramError::MalformedInstruction),
                };
                Ok(Instruction::SettleEpoch {
                    count: data[1],
                    allow_below_floor,
                })
            }
            Tag::SubmitSpend => {
                if data.len() < SUBMIT_SPEND_BASE_LEN {
                    return Err(MirrorProgramError::MalformedInstruction);
                }
                let mut proof_a = [0u8; 64];
                let mut proof_b = [0u8; 128];
                let mut proof_c = [0u8; 64];
                let mut root = [0u8; 32];
                let mut nullifier = [0u8; 32];
                let mut target_program = [0u8; 32];
                let mut beneficiary = [0u8; 32];
                proof_a.copy_from_slice(&data[1..65]);
                proof_b.copy_from_slice(&data[65..193]);
                proof_c.copy_from_slice(&data[193..257]);
                root.copy_from_slice(&data[257..289]);
                nullifier.copy_from_slice(&data[289..321]);
                let selector = read_u64(data, 321);
                target_program.copy_from_slice(&data[329..361]);
                beneficiary.copy_from_slice(&data[361..393]);
                let relay_fee = read_u64(data, 393);
                let action_accounts = data[401];
                let mut len_bytes = [0u8; 2];
                len_bytes.copy_from_slice(&data[402..404]);
                let payload_len = u16::from_le_bytes(len_bytes) as usize;
                // The declared length must account for every remaining byte, so
                // trailing data cannot ride along unread.
                if data.len() != SUBMIT_SPEND_BASE_LEN + payload_len {
                    return Err(MirrorProgramError::MalformedInstruction);
                }
                Ok(Instruction::SubmitSpend {
                    proof_a,
                    proof_b,
                    proof_c,
                    root,
                    nullifier,
                    selector,
                    target_program,
                    beneficiary,
                    relay_fee,
                    action_accounts,
                    payload: data[SUBMIT_SPEND_BASE_LEN..].to_vec(),
                })
            }
        }
    }

    /// Serialises for a client. Kept beside the decoder so the two cannot drift.
    pub fn pack(&self) -> Vec<u8> {
        match self {
            Instruction::InitPool {
                denomination,
                entry_fee,
                k_floor,
                settle_timeout_seconds,
            } => {
                let mut out = Vec::with_capacity(INIT_POOL_LEN);
                out.push(Tag::InitPool as u8);
                out.extend_from_slice(&denomination.to_le_bytes());
                out.extend_from_slice(&entry_fee.to_le_bytes());
                out.extend_from_slice(&k_floor.to_le_bytes());
                out.extend_from_slice(&settle_timeout_seconds.to_le_bytes());
                out
            }
            Instruction::Deposit { commitment } => {
                let mut out = Vec::with_capacity(DEPOSIT_LEN);
                out.push(Tag::Deposit as u8);
                out.extend_from_slice(commitment);
                out
            }
            Instruction::SubmitSpend {
                proof_a,
                proof_b,
                proof_c,
                root,
                nullifier,
                selector,
                target_program,
                beneficiary,
                relay_fee,
                action_accounts,
                payload,
            } => {
                let mut out = Vec::with_capacity(SUBMIT_SPEND_BASE_LEN + payload.len());
                out.push(Tag::SubmitSpend as u8);
                out.extend_from_slice(proof_a);
                out.extend_from_slice(proof_b);
                out.extend_from_slice(proof_c);
                out.extend_from_slice(root);
                out.extend_from_slice(nullifier);
                out.extend_from_slice(&selector.to_le_bytes());
                out.extend_from_slice(target_program);
                out.extend_from_slice(beneficiary);
                out.extend_from_slice(&relay_fee.to_le_bytes());
                out.push(*action_accounts);
                out.extend_from_slice(&(payload.len() as u16).to_le_bytes());
                out.extend_from_slice(payload);
                out
            }
            Instruction::SettleEpoch {
                count,
                allow_below_floor,
            } => {
                vec![Tag::SettleEpoch as u8, *count, *allow_below_floor as u8]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init() -> Instruction {
        Instruction::InitPool {
            denomination: 100_000_000,
            entry_fee: 5_000,
            k_floor: 8,
            settle_timeout_seconds: 900,
        }
    }

    fn deposit() -> Instruction {
        Instruction::Deposit {
            commitment: [3u8; 32],
        }
    }

    fn settle() -> Instruction {
        Instruction::SettleEpoch {
            count: 7,
            allow_below_floor: true,
        }
    }

    fn submit_spend() -> Instruction {
        // Distinct byte patterns per field so a swapped offset cannot pass.
        Instruction::SubmitSpend {
            proof_a: [0xA1; 64],
            proof_b: [0xB2; 128],
            proof_c: [0xC3; 64],
            root: [0xD4; 32],
            nullifier: [0xE5; 32],
            selector: 0x0102_0304_0506_0708,
            target_program: [0x7A; 32],
            beneficiary: [0xF6; 32],
            relay_fee: 0x1112_1314_1516_1718,
            action_accounts: 3,
            payload: vec![1, 2, 3, 4, 5],
        }
    }

    #[test]
    fn packing_then_unpacking_is_the_identity() {
        for ix in [init(), deposit(), submit_spend(), settle()] {
            assert_eq!(Instruction::unpack(&ix.pack()).unwrap(), ix);
        }
    }

    #[test]
    fn the_encoded_lengths_are_pinned() {
        // These are the on-chain ABI. A change here breaks every deployed client.
        assert_eq!(init().pack().len(), INIT_POOL_LEN);
        assert_eq!(deposit().pack().len(), DEPOSIT_LEN);
        assert_eq!(submit_spend().pack().len(), SUBMIT_SPEND_BASE_LEN + 5);
        assert_eq!(INIT_POOL_LEN, 25);
        assert_eq!(DEPOSIT_LEN, 33);
        assert_eq!(SUBMIT_SPEND_BASE_LEN, 404);
        assert_eq!(settle().pack().len(), SETTLE_EPOCH_LEN);
        assert_eq!(SETTLE_EPOCH_LEN, 3);
    }

    #[test]
    fn empty_data_is_refused() {
        assert!(matches!(
            Instruction::unpack(&[]),
            Err(MirrorProgramError::MalformedInstruction)
        ));
    }

    #[test]
    fn an_unknown_tag_is_refused() {
        for tag in [4u8, 5, 99, 255] {
            assert!(
                matches!(
                    Instruction::unpack(&[tag]),
                    Err(MirrorProgramError::MalformedInstruction)
                ),
                "tag {tag} was accepted"
            );
        }
    }

    /// Fail-closed on shape, checked rather than asserted in a README. A short
    /// buffer must not be zero-extended and a long one must not be truncated.
    #[test]
    fn every_wrong_length_is_refused() {
        for ix in [init(), deposit(), submit_spend(), settle()] {
            let good = ix.pack();
            for len in 0..good.len() {
                assert!(
                    Instruction::unpack(&good[..len]).is_err(),
                    "a {len}-byte prefix was accepted for {ix:?}"
                );
            }
            let mut long = good.clone();
            long.push(0);
            assert!(
                Instruction::unpack(&long).is_err(),
                "a trailing byte was tolerated for {ix:?}"
            );
        }
    }

    #[test]
    fn fields_decode_at_the_right_offsets() {
        // Distinct values so a swapped offset cannot pass by coincidence.
        let ix = Instruction::InitPool {
            denomination: 0x1122_3344_5566_7788,
            entry_fee: 0x99aa_bbcc_ddee_ff00,
            k_floor: 0xdead_beef,
            settle_timeout_seconds: 0xcafe_f00d,
        };
        assert_eq!(Instruction::unpack(&ix.pack()).unwrap(), ix);
    }

    /// The flag is one bit of meaning in eight bits of wire, and the seven
    /// spare ones have no defined value.
    ///
    /// "Anything nonzero is true" is the tolerant reading, and it is how a
    /// caller who sent `2` meaning something of their own comes to settle a
    /// batch below the floor believing they asked for no such thing.
    #[test]
    fn a_below_floor_flag_that_is_neither_zero_nor_one_is_refused() {
        for byte in [2u8, 3, 0x80, 0xff] {
            assert!(
                matches!(
                    Instruction::unpack(&[Tag::SettleEpoch as u8, 4, byte]),
                    Err(MirrorProgramError::MalformedInstruction)
                ),
                "flag byte {byte:#x} was accepted"
            );
        }
        // And the two that do mean something still decode.
        for (byte, expected) in [(0u8, false), (1u8, true)] {
            assert_eq!(
                Instruction::unpack(&[Tag::SettleEpoch as u8, 4, byte]).unwrap(),
                Instruction::SettleEpoch {
                    count: 4,
                    allow_below_floor: expected,
                }
            );
        }
    }

    #[test]
    fn a_deposit_commitment_survives_verbatim() {
        let mut commitment = [0u8; 32];
        for (i, b) in commitment.iter_mut().enumerate() {
            *b = i as u8;
        }
        let ix = Instruction::Deposit { commitment };
        match Instruction::unpack(&ix.pack()).unwrap() {
            Instruction::Deposit { commitment: got } => assert_eq!(got, commitment),
            other => panic!("decoded as {other:?}"),
        }
    }
}
