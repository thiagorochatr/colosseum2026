//! The end-to-end assertion for the proving path: a proof produced by the host
//! prover must verify under the exact verifier the on-chain program runs, using
//! the exact verifying-key bytes we will bake into it.
//!
//! `groth16-solana` compiles for the host as well as for SBF, so this exercises
//! the real verifier rather than a stand-in. If the G2 component order or the
//! `proof_a` negation were wrong, this is where it surfaces — and it surfaces
//! as a clean failure here instead of as an opaque rejection on devnet.

use groth16_solana::groth16::{Groth16Verifier, Groth16Verifyingkey};
use mirror_circuit::{generate_reproducible, prove, Witness};
use mirror_core::{Field, MerkleTree, Note};

const SEED: &[u8] = b"mirror-pool-reproducible-dev-setup-v1";

struct Fixture {
    keys: mirror_circuit::Keys,
    tree: MerkleTree,
    notes: Vec<Note>,
}

fn fixture(count: u64) -> Fixture {
    let keys = generate_reproducible(SEED).expect("key generation");
    let mut tree = MerkleTree::new().unwrap();
    let denom = Field::from_u64(1);
    let mut notes = Vec::new();
    for i in 1..=count {
        let note = Note::new(Field::from_u64(i * 7), Field::from_u64(i * 13), denom);
        tree.insert(note.commitment().unwrap()).unwrap();
        notes.push(note);
    }
    Fixture { keys, tree, notes }
}

fn verify_onchain(
    svk: &mirror_circuit::SolanaVerifyingKey,
    p: &mirror_circuit::SolanaProof,
) -> Result<(), groth16_solana::errors::Groth16Error> {
    let vk = Groth16Verifyingkey {
        nr_pubinputs: svk.public_input_count(),
        vk_alpha_g1: svk.alpha_g1,
        vk_beta_g2: svk.beta_g2,
        vk_gamme_g2: svk.gamma_g2,
        vk_delta_g2: svk.delta_g2,
        vk_ic: &svk.ic,
    };
    let mut verifier =
        Groth16Verifier::<3>::new(&p.proof_a, &p.proof_b, &p.proof_c, &p.public_inputs, &vk)?;
    verifier.verify()
}

#[test]
fn a_host_proof_verifies_under_the_onchain_verifier() {
    let f = fixture(6);
    let svk = f.keys.solana_vk();
    assert_eq!(svk.public_input_count(), 3);
    assert_eq!(
        svk.ic.len(),
        4,
        "ic carries one more entry than public inputs"
    );

    let index = 3usize;
    let merkle_proof = f.tree.proof(index as u64).unwrap();
    let witness = Witness {
        note: f.notes[index],
        merkle_proof: &merkle_proof,
        root: f.tree.root().unwrap(),
        action_binding: Field::from_u64(0xfeed_beef),
    };
    let mut rng = ark_std::rand::rngs::StdRng::from_seed([7u8; 32]);
    let proof = prove(&f.keys, &witness, &mut rng).expect("proving");

    verify_onchain(&svk, &proof).expect("the on-chain verifier rejected an honest proof");
}

#[test]
fn a_tampered_public_input_is_rejected_by_the_onchain_verifier() {
    let f = fixture(6);
    let svk = f.keys.solana_vk();
    let merkle_proof = f.tree.proof(1).unwrap();
    let witness = Witness {
        note: f.notes[1],
        merkle_proof: &merkle_proof,
        root: f.tree.root().unwrap(),
        action_binding: Field::from_u64(1234),
    };
    let mut rng = ark_std::rand::rngs::StdRng::from_seed([9u8; 32]);
    let mut proof = prove(&f.keys, &witness, &mut rng).expect("proving");

    // Re-point the action binding, which is what a malicious relay would do.
    // Flip a low bit so the value stays a canonical scalar and the rejection
    // comes from the pairing rather than from the range check.
    proof.public_inputs[2][31] ^= 1;
    assert!(
        verify_onchain(&svk, &proof).is_err(),
        "a redirected action must not verify"
    );
}

#[test]
fn a_non_negated_proof_a_is_rejected() {
    // Guards the single easiest mistake in this integration. groth16-solana does
    // not negate proof_a for you; if a future refactor drops the negation, this
    // fails loudly instead of on devnet.
    let f = fixture(4);
    let svk = f.keys.solana_vk();
    let merkle_proof = f.tree.proof(0).unwrap();
    let witness = Witness {
        note: f.notes[0],
        merkle_proof: &merkle_proof,
        root: f.tree.root().unwrap(),
        action_binding: Field::from_u64(5),
    };
    let mut rng = ark_std::rand::rngs::StdRng::from_seed([3u8; 32]);
    let mut proof = prove(&f.keys, &witness, &mut rng).expect("proving");

    // Negating again returns the original, un-negated A.
    let a = ark_bn254::G1Affine::new_unchecked(
        ark_ff::PrimeField::from_be_bytes_mod_order(&proof.proof_a[..32]),
        ark_ff::PrimeField::from_be_bytes_mod_order(&proof.proof_a[32..]),
    );
    proof.proof_a = mirror_circuit::solana::g1_to_solana(&(-a));
    assert!(verify_onchain(&svk, &proof).is_err());
}

#[test]
fn the_setup_is_reproducible_from_its_seed() {
    // Anyone can re-derive the deployed verifying key from the committed seed.
    // Publishing the entropy while withholding the key is the failure this
    // guards against: insecure and unreproducible at the same time.
    let a = generate_reproducible(SEED).unwrap().solana_vk();
    let b = generate_reproducible(SEED).unwrap().solana_vk();
    assert_eq!(a, b);
    let c = generate_reproducible(b"a different seed")
        .unwrap()
        .solana_vk();
    assert_ne!(a, c);
}

use ark_std::rand::SeedableRng;
