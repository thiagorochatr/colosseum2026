//! The R1CS Poseidon gadget.
//!
//! This mirrors `light_poseidon`'s permutation instruction for instruction,
//! reading its published round constants and MDS matrix rather than
//! re-deriving them. That is the whole design: the constants have exactly one
//! source, so the gadget cannot drift from the syscall.
//!
//! Two routes were rejected:
//!
//! * `ark_crypto_primitives`' `PoseidonSponge` is a sponge with rate and
//!   capacity; its squeeze does not agree with circomlib's fixed-arity
//!   compression. Reading `state[0]` after one permutation does agree, but that
//!   is an implementation detail of the sponge rather than part of its contract.
//!   The one open Solana project that made the sponge primary later migrated off
//!   it.
//! * `ark_crypto_primitives::crh::poseidon::TwoToOneCRH` is not usable here for
//!   the same reason.
//!
//! Several published Solana projects get this wrong in a way that only shows up
//! at proving time: their native hash and their in-circuit hash are different
//! functions, because the gadget was configured with home-rolled constants.
//! `matches_native_over_random_inputs` below is the test that would have caught
//! it.

use ark_bn254::Fr;
use ark_ff::PrimeField;
use ark_r1cs_std::{fields::fp::FpVar, fields::FieldVar};
use ark_relations::r1cs::SynthesisError;
use light_poseidon::{parameters::bn254_x5::get_poseidon_parameters, PoseidonParameters};

/// Poseidon over `inputs`, matching `light_poseidon::Poseidon::new_circom`.
///
/// The state is `[domain_tag, inputs...]` with `domain_tag = 0`, the width is
/// `inputs.len() + 1`, and the digest is `state[0]` after the permutation.
pub fn poseidon_var(inputs: &[FpVar<Fr>]) -> Result<FpVar<Fr>, SynthesisError> {
    let width = inputs.len() + 1;
    let params: PoseidonParameters<Fr> =
        get_poseidon_parameters::<Fr>(width as u8).map_err(|_| SynthesisError::Unsatisfiable)?;

    // state = [domain_tag = 0, inputs...]
    let mut state: Vec<FpVar<Fr>> = Vec::with_capacity(width);
    state.push(FpVar::<Fr>::constant(Fr::from(0u64)));
    state.extend_from_slice(inputs);

    let all_rounds = params.full_rounds + params.partial_rounds;
    let half_rounds = params.full_rounds / 2;

    for round in 0..half_rounds {
        apply_ark(&mut state, &params, round);
        apply_sbox_full(&mut state, params.alpha)?;
        state = apply_mds(&state, &params);
    }
    for round in half_rounds..half_rounds + params.partial_rounds {
        apply_ark(&mut state, &params, round);
        state[0] = pow_alpha(&state[0], params.alpha)?;
        state = apply_mds(&state, &params);
    }
    for round in half_rounds + params.partial_rounds..all_rounds {
        apply_ark(&mut state, &params, round);
        apply_sbox_full(&mut state, params.alpha)?;
        state = apply_mds(&state, &params);
    }

    Ok(state[0].clone())
}

/// Adds this round's constants. The constants are field constants, so this is
/// a linear operation and costs no constraints.
fn apply_ark(state: &mut [FpVar<Fr>], params: &PoseidonParameters<Fr>, round: usize) {
    for (i, slot) in state.iter_mut().enumerate() {
        let c = params.ark[round * params.width + i];
        *slot += FpVar::<Fr>::constant(c);
    }
}

fn apply_sbox_full(state: &mut [FpVar<Fr>], alpha: u64) -> Result<(), SynthesisError> {
    for slot in state.iter_mut() {
        *slot = pow_alpha(slot, alpha)?;
    }
    Ok(())
}

/// `x^alpha`. Alpha is 5 for every BN254 x5 instance, computed as
/// `((x^2)^2) * x` — three constraints, which is optimal for exponent 5.
fn pow_alpha(x: &FpVar<Fr>, alpha: u64) -> Result<FpVar<Fr>, SynthesisError> {
    debug_assert_eq!(alpha, 5, "the x5 parameter sets only support alpha = 5");
    let x2 = x.square()?;
    let x4 = x2.square()?;
    Ok(&x4 * x)
}

/// Multiplies the state by the MDS matrix. The matrix is constant, so each
/// output is a linear combination of the inputs and costs no constraints.
fn apply_mds(state: &[FpVar<Fr>], params: &PoseidonParameters<Fr>) -> Vec<FpVar<Fr>> {
    (0..params.width)
        .map(|i| {
            state
                .iter()
                .enumerate()
                .fold(FpVar::<Fr>::constant(Fr::from(0u64)), |acc, (j, a)| {
                    acc + a * FpVar::<Fr>::constant(params.mds[i][j])
                })
        })
        .collect()
}

/// Converts a canonical big-endian 32-byte element into `Fr`.
pub fn fr_from_be(bytes: &[u8; 32]) -> Fr {
    Fr::from_be_bytes_mod_order(bytes)
}

/// Converts `Fr` into canonical big-endian 32 bytes.
pub fn fr_to_be(f: &Fr) -> [u8; 32] {
    use ark_ff::BigInteger;
    let mut out = [0u8; 32];
    out.copy_from_slice(&f.into_bigint().to_bytes_be());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_r1cs_std::{alloc::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::rand::{RngCore, SeedableRng};

    fn native(inputs: &[Fr]) -> Fr {
        use light_poseidon::{Poseidon, PoseidonHasher};
        let mut h = Poseidon::<Fr>::new_circom(inputs.len()).unwrap();
        h.hash(inputs).unwrap()
    }

    fn in_circuit(inputs: &[Fr]) -> (Fr, usize) {
        let cs = ConstraintSystem::<Fr>::new_ref();
        let vars: Vec<FpVar<Fr>> = inputs
            .iter()
            .map(|v| FpVar::new_witness(cs.clone(), || Ok(*v)).unwrap())
            .collect();
        let out = poseidon_var(&vars).unwrap();
        assert!(cs.is_satisfied().unwrap(), "constraint system unsatisfied");
        (out.value().unwrap(), cs.num_constraints())
    }

    /// The parity test. If this ever fails, proofs stop verifying on-chain and
    /// the reason will not be obvious from the failure.
    #[test]
    fn matches_native_over_random_inputs() {
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(0x6d1_7707);
        for arity in 1..=4usize {
            for _ in 0..8 {
                let inputs: Vec<Fr> = (0..arity)
                    .map(|_| {
                        let mut b = [0u8; 32];
                        rng.fill_bytes(&mut b);
                        Fr::from_be_bytes_mod_order(&b)
                    })
                    .collect();
                let (got, _) = in_circuit(&inputs);
                assert_eq!(got, native(&inputs), "arity {arity} diverged");
            }
        }
    }

    /// Anchors the gadget to the same external constant `mirror-core` pins, so
    /// gadget and host are each checked against circomlib rather than against each
    /// other, and the syscall is checked against the host on-chain.
    #[test]
    fn matches_the_published_circomlib_vector() {
        let (got, _) = in_circuit(&[Fr::from(1u64), Fr::from(2u64)]);
        assert_eq!(
            fr_to_be(&got),
            [
                0x11, 0x5c, 0xc0, 0xf5, 0xe7, 0xd6, 0x90, 0x41, 0x3d, 0xf6, 0x4c, 0x6b, 0x96, 0x62,
                0xe9, 0xcf, 0x2a, 0x36, 0x17, 0xf2, 0x74, 0x32, 0x45, 0x51, 0x9e, 0x19, 0x60, 0x7a,
                0x44, 0x17, 0x18, 0x9a,
            ]
        );
    }

    #[test]
    fn constraint_cost_is_as_budgeted() {
        let (_, n) = in_circuit(&[Fr::from(1u64), Fr::from(2u64)]);
        // 8 full rounds x 3 lanes x 3 constraints + 57 partial x 3 = 243.
        assert!(
            (200..300).contains(&n),
            "arity-2 permutation cost {n} constraints, outside the expected band"
        );
    }
}
