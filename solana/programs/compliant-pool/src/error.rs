use solana_program::program_error::ProgramError;

/// Codes start at 100, clear of mirror-pool's own, which this program also
/// returns unchanged for every instruction it forwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum CompliantPoolError {
    /// A plain SubmitSpend: this pool pays only against a clearance.
    ClearanceRequired = 100,
    /// The clearance account is not a clearance from the compliance program.
    InvalidClearance = 101,
    /// The clearance names another pool.
    ClearanceForAnotherPool = 102,
    /// The clearance names another nullifier.
    ClearanceForAnotherNullifier = 103,
    /// The clearance was issued under another curator.
    WrongCurator = 104,
    /// The clearance was issued in an earlier slot.
    StaleClearance = 105,
    /// The clearance's binding is not this spend's.
    BindingMismatch = 106,
}

impl From<CompliantPoolError> for ProgramError {
    fn from(e: CompliantPoolError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
