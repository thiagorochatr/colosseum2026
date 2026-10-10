use solana_program::program_error::ProgramError;

/// Custom error codes. Each failure has its own code so a rejection says which
/// check closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ComplianceError {
    UnknownInstruction = 0,
    /// The instruction data has the wrong length.
    MalformedInstruction = 1,
    /// A public input is not a canonical field element.
    NonCanonicalInput = 2,
    /// The Groth16 pairing check failed, or a proof point was malformed.
    ProofVerificationFailed = 3,
}

impl From<ComplianceError> for ProgramError {
    fn from(e: ComplianceError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
