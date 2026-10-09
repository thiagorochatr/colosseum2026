//! Canonical BN254 scalar field elements.
//!
//! Everything that crosses the boundary between the circuit, the host and the
//! on-chain program is a `Field`: exactly 32 bytes, big-endian, strictly less
//! than the BN254 scalar modulus `r`.
//!
//! The canonicality check is not decoration. SIMD-0359 made the Poseidon
//! syscall reject inputs that are not full field elements, and the on-chain
//! Groth16 verifier rejects public inputs greater than or equal to `r`. Values
//! that fail either check must be rejected where they enter the system, not
//! silently reduced — reducing would let two distinct byte strings denote one
//! nullifier, which is a double-spend.

use crate::MirrorError;

/// The BN254 scalar field modulus `r`, big-endian.
///
/// 21888242871839275222246405745257275088548364400416034343698204186575808495617
pub const MODULUS_BE: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91, 0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

/// A canonical BN254 scalar, stored big-endian.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Field([u8; 32]);

impl Field {
    /// Additive identity.
    pub const ZERO: Field = Field([0u8; 32]);

    /// Wraps 32 big-endian bytes, rejecting anything at or above the modulus.
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, MirrorError> {
        if !is_canonical(&bytes) {
            return Err(MirrorError::NonCanonicalField);
        }
        Ok(Field(bytes))
    }

    /// Wraps a small integer, which is always canonical.
    pub fn from_u64(v: u64) -> Self {
        let mut bytes = [0u8; 32];
        bytes[24..].copy_from_slice(&v.to_be_bytes());
        Field(bytes)
    }

    /// Reads a canonical element from a slice of exactly 32 bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, MirrorError> {
        let arr: [u8; 32] = bytes.try_into().map_err(|_| MirrorError::BadFieldLength)?;
        Self::from_bytes(arr)
    }

    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 32]
    }
}

/// Constant-time-ish big-endian comparison against the modulus.
///
/// This runs on values that are already public (roots, nullifiers, bindings), so
/// timing is not a concern; the loop is written for clarity and to avoid pulling
/// a bignum dependency into the on-chain binary.
fn is_canonical(bytes: &[u8; 32]) -> bool {
    for i in 0..32 {
        match bytes[i].cmp(&MODULUS_BE[i]) {
            core::cmp::Ordering::Less => return true,
            core::cmp::Ordering::Greater => return false,
            core::cmp::Ordering::Equal => continue,
        }
    }
    // Exactly equal to the modulus is not a canonical field element.
    false
}

impl core::fmt::Debug for Field {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Field(0x")?;
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        write!(f, ")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_and_small_values_are_canonical() {
        assert!(Field::from_bytes([0u8; 32]).is_ok());
        assert_eq!(Field::from_u64(0), Field::ZERO);
        assert_eq!(Field::from_u64(1).to_bytes()[31], 1);
    }

    #[test]
    fn the_modulus_itself_is_rejected() {
        // r is not a member of the field; only 0..r-1 are.
        assert!(matches!(
            Field::from_bytes(MODULUS_BE),
            Err(MirrorError::NonCanonicalField)
        ));
    }

    #[test]
    fn one_below_the_modulus_is_accepted_and_one_above_is_not() {
        let mut below = MODULUS_BE;
        below[31] -= 1;
        assert!(Field::from_bytes(below).is_ok());

        let mut above = MODULUS_BE;
        above[31] += 1;
        assert!(Field::from_bytes(above).is_err());
    }

    #[test]
    fn all_ones_is_rejected() {
        // The classic non-canonical input an attacker reaches for first.
        assert!(Field::from_bytes([0xff; 32]).is_err());
    }

    #[test]
    fn a_wrong_length_slice_is_rejected_rather_than_padded() {
        assert!(matches!(
            Field::from_slice(&[1u8; 31]),
            Err(MirrorError::BadFieldLength)
        ));
        assert!(matches!(
            Field::from_slice(&[1u8; 33]),
            Err(MirrorError::BadFieldLength)
        ));
    }

    #[test]
    fn high_byte_boundary_is_respected() {
        // Differs from the modulus only in the first byte, one below: canonical.
        let mut b = MODULUS_BE;
        b[0] -= 1;
        assert!(Field::from_bytes(b).is_ok());

        // One above in the first byte: not canonical, even though later bytes are small.
        let mut b = MODULUS_BE;
        b[0] += 1;
        b[31] = 0;
        assert!(Field::from_bytes(b).is_err());
    }
}
