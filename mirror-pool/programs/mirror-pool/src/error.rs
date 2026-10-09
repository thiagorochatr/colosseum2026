//! Program errors.
//!
//! Every variant is a distinct code so that a failing transaction says which
//! invariant it violated. Devnet evidence records these codes alongside the
//! signature, which is what makes a negative test checkable by a third party
//! rather than a claim in a README.

use solana_program::program_error::ProgramError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MirrorProgramError {
    /// The instruction data was empty, truncated, or carried an unknown tag.
    MalformedInstruction = 1,
    /// An account was not the PDA the program derives for that role.
    InvalidPda = 2,
    /// A required signature was absent.
    MissingSignature = 3,
    /// An account was owned by the wrong program.
    InvalidOwner = 4,
    /// The pool account has the wrong length or an unrecognised version.
    InvalidPoolAccount = 5,
    /// The pool is already initialised.
    AlreadyInitialised = 6,
    /// A pool parameter was outside its permitted range.
    InvalidParameter = 7,
    /// A 32-byte value was not a canonical BN254 scalar.
    NonCanonicalField = 8,
    /// Poseidon hashing failed.
    PoseidonFailed = 9,
    /// The accumulator is full.
    TreeFull = 10,
    //
    // 11 is retired. It was `WrongDepositAmount`, and it was unreachable: a
    // deposit's size is read from the pool account, never from the instruction,
    // so there is no caller-supplied amount left to disagree with. Deleting the
    // variant rather than leaving it unconstructed keeps the enum an accurate
    // list of what this program can actually reject. The discriminant is left
    // unused rather than reassigned, so every code below stays stable for
    // anything already reading them.
    //
    /// Arithmetic overflowed or underflowed.
    ArithmeticOverflow = 12,
    /// The vault does not hold enough to cover every unspent note.
    ///
    /// This is the accounting invariant. It is checked rather than assumed
    /// because both standard drains in this design space surface exactly here,
    /// and an invariant that is only assumed is not an invariant.
    InsolventVault = 13,
    /// The spend account has the wrong length or an unrecognised version.
    InvalidSpendAccount = 14,
    /// This nullifier has already been recorded: the note is spent.
    NullifierAlreadySpent = 15,
    /// The spend has already been executed.
    AlreadySettled = 16,
    /// The proof references a root the pool does not retain.
    UnknownRoot = 17,
    /// The Groth16 proof did not verify.
    ProofVerificationFailed = 18,
    /// The pool holds fewer notes than its anonymity floor requires.
    BelowAnonymityFloor = 19,
    /// The relay fee is not less than the denomination.
    RelayFeeTooLarge = 20,
    /// The batch is below the crowd size and has not waited out the timeout.
    CrowdTooSmall = 21,
    /// The action selector is not one this program implements.
    UnknownSelector = 22,
    /// An action tried to invoke this program. Refused: re-entering settlement
    /// around a lamport-moving loop is not a property to leave to careful
    /// reading.
    SelfInvocationRefused = 23,
    /// The action payload exceeds the maximum a spend record can carry.
    PayloadTooLarge = 24,
    /// Two records in one settlement carried different relay fees.
    ///
    /// A member is paid `denomination - relay_fee`, and that payout is public.
    /// A batch whose members paid different fees settles into visibly different
    /// amounts, so an observer partitions it by value without breaking anything
    /// — which costs the members the exact property the batch exists to give
    /// them. Refused rather than documented.
    FeeNotUniform = 25,
    /// The batch is below the crowd size and the settler did not ask for that.
    ///
    /// Distinct from [`MirrorProgramError::CrowdTooSmall`], and the distinction
    /// is the whole point of the flag. `CrowdTooSmall` means the batch *cannot*
    /// settle yet — the timeout has not run. This means it could, and nobody
    /// said so: a settler who composes an under-floor batch by accident is
    /// refused here rather than handed a settlement that costs somebody their
    /// anonymity set. Two codes so that a rejection says which of the two
    /// happened without anyone having to guess.
    BelowFloorNotPermitted = 26,
}

impl From<MirrorProgramError> for ProgramError {
    fn from(e: MirrorProgramError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

impl From<mirror_core::MirrorError> for MirrorProgramError {
    fn from(e: mirror_core::MirrorError) -> Self {
        use mirror_core::MirrorError as E;
        match e {
            E::NonCanonicalField | E::BadFieldLength => MirrorProgramError::NonCanonicalField,
            E::Poseidon => MirrorProgramError::PoseidonFailed,
            E::TreeFull => MirrorProgramError::TreeFull,
            E::LeafIndexOutOfRange => MirrorProgramError::MalformedInstruction,
        }
    }
}
