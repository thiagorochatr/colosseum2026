//! Groth16 verification of the withdraw circuit, over the four public inputs
//! in the order fixed by docs/SPEC.md section 5.

use crate::{vk::VERIFYING_KEY, ComplianceError};
use groth16_solana::groth16::Groth16Verifier;

pub const PUBLIC_INPUTS: usize = 4;
pub const REQUEST_LEN: usize = 64 + 128 + 64 + 32 * PUBLIC_INPUTS;

/// The BN254 scalar field modulus, big-endian.
const MODULUS_BE: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91, 0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

/// A proof in groth16-solana's layout (`a` already negated) and its inputs:
/// poolRoot, cleanRoot, nullifierHash, binding.
pub struct ProofRequest {
    pub a: [u8; 64],
    pub b: [u8; 128],
    pub c: [u8; 64],
    pub inputs: [[u8; 32]; PUBLIC_INPUTS],
}

impl ProofRequest {
    pub fn decode(data: &[u8]) -> Result<Self, ComplianceError> {
        if data.len() != REQUEST_LEN {
            return Err(ComplianceError::MalformedInstruction);
        }
        let mut a = [0u8; 64];
        let mut b = [0u8; 128];
        let mut c = [0u8; 64];
        a.copy_from_slice(&data[..64]);
        b.copy_from_slice(&data[64..192]);
        c.copy_from_slice(&data[192..256]);
        let mut inputs = [[0u8; 32]; PUBLIC_INPUTS];
        for (i, input) in inputs.iter_mut().enumerate() {
            input.copy_from_slice(&data[256 + 32 * i..288 + 32 * i]);
        }
        Ok(ProofRequest { a, b, c, inputs })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(REQUEST_LEN);
        out.extend_from_slice(&self.a);
        out.extend_from_slice(&self.b);
        out.extend_from_slice(&self.c);
        for input in &self.inputs {
            out.extend_from_slice(input);
        }
        out
    }
}

/// Big-endian `x < p`. A non-canonical input would alias a canonical one, so
/// it is refused before it reaches the verifier.
fn is_canonical(x: &[u8; 32]) -> bool {
    x < &MODULUS_BE
}

pub fn verify(req: &ProofRequest) -> Result<(), ComplianceError> {
    if !req.inputs.iter().all(is_canonical) {
        return Err(ComplianceError::NonCanonicalInput);
    }
    let mut verifier =
        Groth16Verifier::<PUBLIC_INPUTS>::new(&req.a, &req.b, &req.c, &req.inputs, &VERIFYING_KEY)
            .map_err(|_| ComplianceError::ProofVerificationFailed)?;
    verifier
        .verify()
        .map_err(|_| ComplianceError::ProofVerificationFailed)
}
