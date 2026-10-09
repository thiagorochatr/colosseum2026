//! The verifying key compiled into the program is the one the published seed
//! produces — checked here, not by an operator remembering to run a command.
//!
//! `README.md` publishes a digest and tells a reader they can re-derive the key
//! from the committed seed and compare. That claim is only worth anything if
//! something enforces it: `vk.rs` is a generated file of plain byte arrays, and
//! a wrong byte in it is not a compile error, not a test failure anywhere else,
//! and not visible in a diff anyone reads carefully. It is a silent break of the
//! one property this setup does offer — that it is *reproducible*, since it is
//! openly not *secure*.
//!
//! What a drift here would mean is worse than a broken build. The program would
//! keep verifying proofs, against a key nobody can derive, and every claim about
//! reproducibility in this repository would be false while every test stayed
//! green.
//!
//! So this compares element by element rather than by digest alone. A digest
//! tells you *that* something moved; the element comparison tells you *which*
//! part of the key did, which is the difference between a five-minute fix and an
//! afternoon.

use mirror_circuit::generate_reproducible;
use mirror_pool_program::vk;

/// The same seed `mirror setup` and `mirror verify-setup` use. Committed in
/// plain sight, because the whole point is that anyone can run the setup again.
const SEED: &[u8] = b"mirror-pool-reproducible-dev-setup-v1";

/// The digest `README.md` publishes.
///
/// Pinned here so that changing the number in the documentation without changing
/// the key — or the reverse — fails rather than passes. The preimage is the
/// concatenation of every element in the order below, which is what makes this
/// a statement about the whole key rather than about one point of it.
const PUBLISHED_DIGEST: &str = "b0165d5eac6fe8273b6564c78e8ba548c97e6050ae785e9142de63c81aa905b7";

#[test]
fn the_compiled_verifying_key_is_the_one_the_published_seed_derives() {
    let derived = generate_reproducible(SEED)
        .expect("the committed seed must produce a setup")
        .solana_vk();

    assert_eq!(
        derived.alpha_g1,
        vk::VK_ALPHA_G1,
        "alpha_g1 in the program does not match the seed"
    );
    assert_eq!(
        derived.beta_g2,
        vk::VK_BETA_G2,
        "beta_g2 in the program does not match the seed"
    );
    assert_eq!(
        derived.gamma_g2,
        vk::VK_GAMMA_G2,
        "gamma_g2 in the program does not match the seed"
    );
    assert_eq!(
        derived.delta_g2,
        vk::VK_DELTA_G2,
        "delta_g2 in the program does not match the seed"
    );

    // The IC points are per public input, so a mismatch in their *count* means
    // the circuit's shape changed and the program is verifying a different
    // statement than the one this repository documents.
    assert_eq!(
        derived.ic.len(),
        vk::VK_IC.len(),
        "the program expects {} public inputs and the circuit produces {}",
        vk::VK_IC.len(),
        derived.ic.len()
    );
    for (i, point) in derived.ic.iter().enumerate() {
        assert_eq!(*point, vk::VK_IC[i], "IC point {i} does not match the seed");
    }
}

/// The number in the README, enforced.
///
/// Deliberately a separate test from the element comparison above. If both fail,
/// the key moved; if only this one fails, the documentation is stale and the key
/// is fine — and knowing which without reading any code is the point.
#[test]
fn the_published_digest_is_the_digest_of_the_derived_key() {
    use sha2::{Digest, Sha256};
    let derived = generate_reproducible(SEED)
        .expect("the committed seed must produce a setup")
        .solana_vk();
    let mut hasher = Sha256::new();
    hasher.update(derived.digest_preimage());
    let digest = hex::encode(hasher.finalize());
    assert_eq!(
        digest, PUBLISHED_DIGEST,
        "the digest published in README.md is not the digest of the key this seed derives"
    );
}
