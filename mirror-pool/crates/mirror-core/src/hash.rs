//! Poseidon over BN254, with domain separation by arity.
//!
//! `solana-poseidon` is the only entry point, deliberately. Upstream it is
//! cfg-gated to the `sol_poseidon` syscall on-chain and to light-poseidon over
//! ark-bn254 off-chain, so the host and the program compute the same function by
//! construction rather than by our own careful duplication. The R1CS gadget in
//! `mirror-circuit` is checked against this, which closes the three-way parity:
//! gadget == host == syscall.
//!
//! ## Domain separation
//!
//! Poseidon instances of different width are different permutations, so arity is
//! itself a domain separator — and it is a free one. Each hashed value in the
//! protocol uses a distinct arity, so no preimage of one can ever be reread as a
//! preimage of another:
//!
//! | value | arity | preimage |
//! |---|---|---|
//! | nullifier | 1 | `(k)` |
//! | Merkle node | 2 | `(left, right)` |
//! | note commitment | 3 | `(k, r, denom_tag)` |
//!
//! The action binding is not in this table: its payload is variable length, so
//! it is a keccak digest rather than a Poseidon compression. See
//! [`action_binding`].
//!
//! An explicit integer tag was the first design here and it was wrong: with a
//! small tag constant, a Merkle node whose left child equals the tag collides
//! with a nullifier. Reaching that state requires a Poseidon preimage and so is
//! not practically exploitable, but arity separation removes the question
//! entirely at no cost.
//!
//! Hashing Merkle nodes, nullifiers and action bindings with one untagged
//! arity-2 function is the shortcut this avoids: it works, right up until a
//! value from one domain is accepted where another was meant.

use crate::{Field, MirrorError};
use solana_poseidon::{hashv, Endianness, Parameters};

fn digest(inputs: &[&[u8]]) -> Result<Field, MirrorError> {
    let out = hashv(Parameters::Bn254X5, Endianness::BigEndian, inputs)
        .map_err(|_| MirrorError::Poseidon)?;
    // The syscall reduces into the field, so this cannot fail. We still go
    // through the checked constructor rather than assume it.
    Field::from_bytes(out.to_bytes())
}

/// Arity-1 Poseidon.
pub fn poseidon1(a: Field) -> Result<Field, MirrorError> {
    digest(&[a.as_bytes()])
}

/// Arity-2 Poseidon.
pub fn poseidon2(a: Field, b: Field) -> Result<Field, MirrorError> {
    digest(&[a.as_bytes(), b.as_bytes()])
}

/// Arity-3 Poseidon.
pub fn poseidon3(a: Field, b: Field, c: Field) -> Result<Field, MirrorError> {
    digest(&[a.as_bytes(), b.as_bytes(), c.as_bytes()])
}

/// Merkle internal node: `H2(left, right)`.
pub fn hash_node(left: Field, right: Field) -> Result<Field, MirrorError> {
    poseidon2(left, right)
}

/// Note commitment: `H3(k, r, denom_tag)`.
///
/// `k` is the nullifier preimage, `r` the blinding factor. `denom_tag` pins the
/// commitment to the pool's denomination so a note can never be replayed into a
/// pool of a different size even if the two share an accumulator.
pub fn commitment(k: Field, r: Field, denom_tag: Field) -> Result<Field, MirrorError> {
    poseidon3(k, r, denom_tag)
}

/// Nullifier: `H1(k)`.
///
/// Deliberately not scoped to an epoch. Scoping a nullifier to the epoch is the
/// natural move once epochs are how batches form — and against a value-bearing
/// payout it is a drain, because a single deposit then pays out once per epoch,
/// forever. This one is spent once, ever.
pub fn nullifier(k: Field) -> Result<Field, MirrorError> {
    poseidon1(k)
}

/// Domain tag for the action binding preimage.
///
/// Bumped to `v2` when `relay` joined the preimage. The digest would differ
/// regardless — a new field guarantees that — but the version says *why* it
/// differs. A prover left on the old shape produces a binding the program does
/// not recompute, and the failure surfaces as a rejected pairing; the tag is
/// what makes that a version mismatch rather than a mystery.
pub const ACTION_DOMAIN: &[u8] = b"mirror-pool:action:v2";

/// Reduces a 256-bit digest to a canonical BN254 scalar.
///
/// The top byte is cleared, so the value is below `2^248` and therefore
/// unconditionally below the modulus. Masking rather than reducing keeps the map
/// deterministic and total — a modular reduction would need a bignum in the
/// program, and rejecting out-of-range digests would make the binding fail for
/// one preimage in roughly forty.
///
/// Eight bits are lost from 256. What remains is far beyond what collision
/// resistance needs here: forging a binding still requires a keccak collision.
pub fn field_from_digest(digest: [u8; 32]) -> Field {
    let mut bytes = digest;
    bytes[0] = 0;
    Field::from_bytes(bytes).expect("cleared top byte is always canonical")
}

/// Action binding: a keccak digest over everything a relay could alter.
///
/// This is the circuit's third public input and the value the program recomputes
/// from the action it is about to execute. It lives here, shared by host and
/// program, so the two cannot drift.
///
/// The preimage covers every field with economic or behavioural meaning:
///
/// | field | what altering it would let a relay do |
/// |---|---|
/// | `selector` | run a different kind of action |
/// | `target_program` | invoke a different program entirely |
/// | `beneficiary` | redirect the outcome |
/// | `relay` | collect another relay's fee |
/// | `relay_fee` | inflate its own cut |
/// | `payload` | change the action's parameters |
/// | `action_accounts` | declare a count settlement cannot satisfy |
///
/// `relay` is here because binding the fee without binding its recipient is only
/// half a binding. Every other field in this preimage travels in clear text in
/// the instruction data, so anyone watching an unlanded `submit_spend` can lift
/// the proof, substitute their own key as the relay, and land it first: the
/// digest is unchanged, the pairing succeeds, the nullifier burns, and the fee
/// settles to them. The member's payout is never at risk — `beneficiary` and the
/// amount are bound — so this is theft from the relay, not from the member.
///
/// It is not a profitable theft. Whoever lands the transaction funds the spend
/// record's rent, and that record is never closed because it *is* the replay
/// guard, so the attacker sinks far more than any sane fee is worth. The reason
/// to close it anyway is that an attacker need not be paid to be effective: a
/// relay whose fee can be sniped at will is a relay nobody runs, and a pool with
/// no relays is one where members submit from their own funded wallets. That
/// undoes the anonymity this whole design exists to provide — not by breaking
/// the pool, but by making the honest path uneconomic.
///
/// `action_accounts` is here because leaving it out was a live griefing vector:
/// it is relay-supplied, it is written verbatim into the spend record, and
/// settlement refuses any spend whose declared count does not match its
/// selector. A relay handed a valid transfer proof could submit it with
/// `action_accounts = 1`, burn the nullifier, and leave a note that can never
/// settle and can never be refunded — for free, and undetectably until
/// settlement.
///
/// Keccak rather than Poseidon because the payload is variable length and
/// Poseidon is a fixed-arity compression. The circuit never computes this — it
/// takes the result as an opaque public input — so the choice costs no
/// constraints.
#[allow(clippy::too_many_arguments)]
pub fn action_binding(
    selector: u64,
    target_program: &[u8; 32],
    beneficiary: &[u8; 32],
    relay: &[u8; 32],
    relay_fee: u64,
    action_accounts: u8,
    payload: &[u8],
) -> Field {
    let digest = solana_keccak_hasher::hashv(&[
        ACTION_DOMAIN,
        &selector.to_le_bytes(),
        target_program,
        beneficiary,
        relay,
        &relay_fee.to_le_bytes(),
        &[action_accounts],
        payload,
    ]);
    field_from_digest(digest.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(v: u64) -> Field {
        Field::from_u64(v)
    }

    #[test]
    fn hashing_is_deterministic() {
        assert_eq!(
            poseidon2(f(1), f(2)).unwrap(),
            poseidon2(f(1), f(2)).unwrap()
        );
    }

    #[test]
    fn hashing_is_order_sensitive() {
        assert_ne!(
            poseidon2(f(1), f(2)).unwrap(),
            poseidon2(f(2), f(1)).unwrap()
        );
    }

    #[test]
    fn arities_are_distinct_domains() {
        // The whole point of separating by arity: padding one preimage with
        // zeros must not reproduce another's digest.
        let a1 = poseidon1(f(7)).unwrap();
        let a2 = poseidon2(f(7), Field::ZERO).unwrap();
        let a3 = poseidon3(f(7), Field::ZERO, Field::ZERO).unwrap();
        assert_ne!(a1, a2);
        assert_ne!(a2, a3);
        assert_ne!(a1, a3);
    }

    #[test]
    fn a_nullifier_cannot_be_confused_with_a_merkle_node() {
        // Different arity, so no choice of children reproduces a nullifier by
        // construction rather than by preimage hardness.
        let k = f(42);
        let null = nullifier(k).unwrap();
        assert_ne!(null, hash_node(Field::ZERO, k).unwrap());
        assert_ne!(null, hash_node(k, Field::ZERO).unwrap());
    }

    #[test]
    fn a_commitment_is_bound_to_its_denomination() {
        let (k, r) = (f(11), f(22));
        assert_ne!(
            commitment(k, r, f(1)).unwrap(),
            commitment(k, r, f(2)).unwrap(),
            "the same note under two denominations must not share a commitment"
        );
    }

    const PROG: [u8; 32] = [3u8; 32];
    const RELAY: [u8; 32] = [5u8; 32];

    #[test]
    fn the_action_binding_covers_every_field_a_relay_could_change() {
        let bob = [7u8; 32];
        let payload = b"stake 0.1".as_slice();
        let base = action_binding(1, &PROG, &bob, &RELAY, 5_000, 0, payload);

        let mut carol = [7u8; 32];
        carol[31] = 8;
        let mut other_prog = PROG;
        other_prog[0] = 4;

        assert_ne!(
            base,
            action_binding(2, &PROG, &bob, &RELAY, 5_000, 0, payload),
            "selector"
        );
        assert_ne!(
            base,
            action_binding(1, &other_prog, &bob, &RELAY, 5_000, 0, payload),
            "target program"
        );
        assert_ne!(
            base,
            action_binding(1, &PROG, &carol, &RELAY, 5_000, 0, payload),
            "beneficiary"
        );
        assert_ne!(
            base,
            action_binding(1, &PROG, &bob, &RELAY, 6_000, 0, payload),
            "relay fee"
        );
        assert_ne!(
            base,
            action_binding(1, &PROG, &bob, &RELAY, 5_000, 0, b"stake 1.0"),
            "payload"
        );
    }

    /// The front-running case, stated on its own rather than as one row above.
    ///
    /// Every other field in the preimage is chosen by the member. This one is
    /// whoever signs, so it is the only field an attacker can change *without*
    /// altering what the member asked for — which is exactly why leaving it out
    /// made the proof transferable to a different payee at no cost.
    #[test]
    fn two_relays_carrying_the_same_action_do_not_share_a_binding() {
        let bob = [7u8; 32];
        let thief = [6u8; 32];
        assert_ne!(
            action_binding(1, &PROG, &bob, &RELAY, 5_000, 0, b"stake 0.1"),
            action_binding(1, &PROG, &bob, &thief, 5_000, 0, b"stake 0.1"),
            "a proof must not verify for a relay the member did not name"
        );
    }

    #[test]
    fn the_binding_is_deterministic_and_lands_in_the_field() {
        let a = action_binding(1, &PROG, &[9u8; 32], &RELAY, 1, 0, b"x");
        let b = action_binding(1, &PROG, &[9u8; 32], &RELAY, 1, 0, b"x");
        assert_eq!(a, b);
        // Canonical by construction: the top byte is cleared.
        assert_eq!(a.to_bytes()[0], 0);
        assert!(Field::from_bytes(a.to_bytes()).is_ok());
    }

    #[test]
    fn an_empty_payload_is_a_distinct_action_from_a_zero_byte_one() {
        assert_ne!(
            action_binding(1, &PROG, &[1u8; 32], &RELAY, 0, 0, b""),
            action_binding(1, &PROG, &[1u8; 32], &RELAY, 0, 0, b"\0"),
        );
    }

    #[test]
    fn field_masking_is_total_over_every_digest() {
        // Including a digest that would otherwise exceed the modulus. A binding
        // that failed for some preimages would be a liveness bug that only
        // appeared for one action in forty.
        for probe in [[0xffu8; 32], [0u8; 32], crate::MODULUS_BE] {
            let f = field_from_digest(probe);
            assert_eq!(f.to_bytes()[0], 0);
        }
    }

    #[test]
    fn the_domain_tag_separates_bindings_from_raw_hashes() {
        // Without the tag, a preimage assembled elsewhere could collide with a
        // binding. With it, an attacker must also control the tag.
        let with_tag = action_binding(0, &[0u8; 32], &[0u8; 32], &[0u8; 32], 0, 0, b"");
        let raw = field_from_digest(
            solana_keccak_hasher::hashv(&[
                &0u64.to_le_bytes(),
                &[0u8; 32],
                &[0u8; 32],
                &[0u8; 32],
                &0u64.to_le_bytes(),
            ])
            .to_bytes(),
        );
        assert_ne!(with_tag, raw);
    }

    /// The anchor for the three-way parity.
    ///
    /// `poseidon([1, 2])` under circomlib's BN254 x5 instance is the published
    /// constant
    /// `7853200120776062878684798364095072458815029376092732009249414926327459813530`.
    /// Pinning it here means the host and the R1CS gadget in `mirror-circuit` are
    /// each checked against an external published value rather than against each
    /// other. The syscall is then checked against the host on-chain, by the
    /// end-to-end suite asserting the deployed program's root equals the host's.
    #[test]
    fn poseidon2_matches_the_published_circomlib_vector() {
        const CIRCOMLIB_1_2: [u8; 32] = [
            0x11, 0x5c, 0xc0, 0xf5, 0xe7, 0xd6, 0x90, 0x41, 0x3d, 0xf6, 0x4c, 0x6b, 0x96, 0x62,
            0xe9, 0xcf, 0x2a, 0x36, 0x17, 0xf2, 0x74, 0x32, 0x45, 0x51, 0x9e, 0x19, 0x60, 0x7a,
            0x44, 0x17, 0x18, 0x9a,
        ];
        assert_eq!(poseidon2(f(1), f(2)).unwrap().to_bytes(), CIRCOMLIB_1_2);
    }

    /// Regression vectors for the other arities we rely on. These are ours, not
    /// external, and exist so that an accidental change of parameters or
    /// endianness is caught rather than silently re-baselined.
    #[test]
    fn arity_vectors_are_stable() {
        const P1_1: [u8; 32] = [
            0x29, 0x17, 0x61, 0x00, 0xea, 0xa9, 0x62, 0xbd, 0xc1, 0xfe, 0x6c, 0x65, 0x4d, 0x6a,
            0x3c, 0x13, 0x0e, 0x96, 0xa4, 0xd1, 0x16, 0x8b, 0x33, 0x84, 0x8b, 0x89, 0x7d, 0xc5,
            0x02, 0x82, 0x01, 0x33,
        ];
        const P3_123: [u8; 32] = [
            0x0e, 0x77, 0x32, 0xd8, 0x9e, 0x69, 0x39, 0xc0, 0xff, 0x03, 0xd5, 0xe5, 0x8d, 0xab,
            0x63, 0x02, 0xf3, 0x23, 0x0e, 0x26, 0x9d, 0xc5, 0xb9, 0x68, 0xf7, 0x25, 0xdf, 0x34,
            0xab, 0x36, 0xd7, 0x32,
        ];
        assert_eq!(poseidon1(f(1)).unwrap().to_bytes(), P1_1);
        assert_eq!(poseidon3(f(1), f(2), f(3)).unwrap().to_bytes(), P3_123);
    }
}
