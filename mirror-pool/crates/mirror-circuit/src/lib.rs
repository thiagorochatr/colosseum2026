//! Groth16 membership circuit over BN254, its key generation, the host prover,
//! and export into the byte layout the on-chain verifier consumes.
//!
//! Host-only: never linked into the on-chain program.
#![forbid(unsafe_code)]

pub mod circuit;
pub mod poseidon_gadget;
pub mod prover;
pub mod solana;

pub use circuit::MembershipCircuit;
pub use prover::{generate, generate_reproducible, prove, Keys, SolanaProof, Witness};
pub use solana::{vk_to_rust_source, vk_to_solana, SolanaVerifyingKey};
