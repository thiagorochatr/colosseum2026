use solana_program::program_error::ProgramError;

/// Custom error codes. Each failure has its own code so a rejection says which
/// check closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ComplianceError {
    UnknownInstruction = 0,
    /// The instruction data has the wrong length.
    MalformedInstruction = 1,
    /// A public input or a root is not a canonical field element.
    NonCanonicalInput = 2,
    /// The Groth16 pairing check failed, or a proof point was malformed.
    ProofVerificationFailed = 3,
    /// A required signature is missing.
    MissingSignature = 4,
    /// An account is not the one this instruction derives or expects.
    InvalidAccount = 5,
    /// The curator account already exists.
    CuratorExists = 6,
    /// The signer is not this curator's authority.
    WrongAuthority = 7,
    /// The pool does not have this root in its history.
    UnknownPoolRoot = 8,
    /// The curator never published this root, or revoked it.
    StaleCleanRoot = 9,
    /// A clearance for this pool and nullifier already exists.
    ClearanceExists = 10,
    /// A counter would overflow.
    ArithmeticOverflow = 11,
}

impl From<ComplianceError> for ProgramError {
    fn from(e: ComplianceError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
