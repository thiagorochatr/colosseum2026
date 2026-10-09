//! Key generation and proving.

use crate::{
    circuit::MembershipCircuit,
    poseidon_gadget::fr_from_be,
    solana::{proof_to_solana, vk_to_solana, SolanaVerifyingKey},
};
use ark_bn254::{Bn254, Fr};
use ark_groth16::{Groth16, Proof, ProvingKey, VerifyingKey};
use ark_snark::SNARK;
use ark_std::rand::{RngCore, SeedableRng};
use mirror_core::{Field, MerkleProof, Note};

/// A Groth16 keypair for the membership circuit.
pub struct Keys {
    pub pk: ProvingKey<Bn254>,
    pub vk: VerifyingKey<Bn254>,
}

impl Keys {
    pub fn solana_vk(&self) -> SolanaVerifyingKey {
        vk_to_solana(&self.vk)
    }
}

/// Generates keys from a caller-supplied RNG.
///
/// The security of the resulting keys is exactly the security of this RNG's
/// entropy: whoever knows it can forge proofs. `generate_reproducible` below
/// makes that trade explicit rather than hiding it.
pub fn generate<R: RngCore + ark_std::rand::CryptoRng>(rng: &mut R) -> Result<Keys, SetupError> {
    let (pk, vk) = Groth16::<Bn254>::circuit_specific_setup(MembershipCircuit::blank(), rng)
        .map_err(|_| SetupError::KeyGeneration)?;
    Ok(Keys { pk, vk })
}

/// Generates keys deterministically from a public seed.
///
/// This trades ceremony secrecy for reproducibility, deliberately and in the
/// open. Anyone can re-derive byte-identical keys from the committed seed and
/// check that the deployed program's verifying key is the one the circuit in
/// this repository produces.
///
/// It is **not** a secure setup: the seed is public, so the toxic waste is
/// public, so proofs are forgeable. It is the honest development posture, and
/// the multi-party path that replaces it for production is a separate,
/// documented procedure.
///
/// The reproducibility is scoped, and the scope is the committed `Cargo.lock`.
/// `StdRng` is explicitly not guaranteed portable across `rand` releases, and
/// arkworks' key generation consumes randomness in an order that is an
/// implementation detail. Re-deriving the same bytes therefore requires the same
/// locked dependency graph, which is why the lockfile is committed and why the
/// setup transcript records a digest of the resulting key rather than trusting
/// the seed alone.
///
/// There are two ways to get this wrong, and they are opposites. Publishing the
/// entropy string *and* withholding the proving key is the worst of both: the
/// setup is insecure, because the toxic waste is public, and unreproducible,
/// because no third party can regenerate the key to check it — so nobody can
/// produce a valid proof for the deployed program at all. Running a real
/// ceremony but verifying only `delta` in the transcript is the other: the
/// process looks rigorous while certifying a key that may belong to a different
/// circuit entirely.
pub fn generate_reproducible(seed: &[u8]) -> Result<Keys, SetupError> {
    let mut expanded = [0u8; 32];
    for (i, slot) in expanded.iter_mut().enumerate() {
        *slot = seed.get(i).copied().unwrap_or(0);
    }
    let mut rng = ark_std::rand::rngs::StdRng::from_seed(expanded);
    generate(&mut rng)
}

/// Everything the prover needs that is not already public.
pub struct Witness<'a> {
    pub note: Note,
    pub merkle_proof: &'a MerkleProof,
    pub root: Field,
    pub action_binding: Field,
}

/// A proof plus the public inputs it was made against, in on-chain layout.
pub struct SolanaProof {
    pub proof_a: [u8; 64],
    pub proof_b: [u8; 128],
    pub proof_c: [u8; 64],
    /// `[root, nullifier, action_binding]`, big-endian, in circuit order.
    pub public_inputs: [[u8; 32]; 3],
}

/// Builds an assigned circuit from a witness.
fn assign(w: &Witness<'_>) -> Result<(MembershipCircuit, Field), ProveError> {
    let nullifier = w.note.nullifier().map_err(|_| ProveError::Hash)?;
    let circuit = MembershipCircuit {
        root: Some(fr_from_be(w.root.as_bytes())),
        nullifier: Some(fr_from_be(nullifier.as_bytes())),
        action_binding: Some(fr_from_be(w.action_binding.as_bytes())),
        k: Some(fr_from_be(w.note.k.as_bytes())),
        r: Some(fr_from_be(w.note.r.as_bytes())),
        denom_tag: Some(fr_from_be(w.note.denom_tag.as_bytes())),
        leaf_index: Some(w.merkle_proof.leaf_index),
        siblings: Some(
            w.merkle_proof
                .siblings
                .iter()
                .map(|s| fr_from_be(s.as_bytes()))
                .collect(),
        ),
    };
    Ok((circuit, nullifier))
}

/// Proves membership, returning the proof in on-chain layout.
pub fn prove<R: RngCore + ark_std::rand::CryptoRng>(
    keys: &Keys,
    witness: &Witness<'_>,
    rng: &mut R,
) -> Result<SolanaProof, ProveError> {
    let (circuit, nullifier) = assign(witness)?;
    let proof: Proof<Bn254> =
        Groth16::<Bn254>::prove(&keys.pk, circuit, rng).map_err(|_| ProveError::Proving)?;

    // Self-check before handing the proof out. A proof that does not verify
    // against its own verifying key will not verify on-chain either, and
    // failing here gives a diagnosable error instead of an opaque
    // ProofVerificationFailed from the syscall.
    let public: Vec<Fr> = vec![
        fr_from_be(witness.root.as_bytes()),
        fr_from_be(nullifier.as_bytes()),
        fr_from_be(witness.action_binding.as_bytes()),
    ];
    let ok =
        Groth16::<Bn254>::verify(&keys.vk, &public, &proof).map_err(|_| ProveError::Proving)?;
    if !ok {
        return Err(ProveError::SelfCheckFailed);
    }

    let (proof_a, proof_b, proof_c) = proof_to_solana(&proof);
    Ok(SolanaProof {
        proof_a,
        proof_b,
        proof_c,
        public_inputs: [
            witness.root.to_bytes(),
            nullifier.to_bytes(),
            witness.action_binding.to_bytes(),
        ],
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SetupError {
    #[error("groth16 key generation failed")]
    KeyGeneration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProveError {
    #[error("hashing failed while assigning the witness")]
    Hash,
    #[error("groth16 proving failed")]
    Proving,
    #[error("the proof did not verify against its own verifying key")]
    SelfCheckFailed,
}
