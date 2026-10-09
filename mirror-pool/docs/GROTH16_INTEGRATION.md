# arkworks 0.5 → `groth16-solana` 0.2: integration reference

Every claim here was verified by execution against the pinned versions, not taken
from documentation. The published `groth16-solana` docs are stale (they show an
API removed in 0.0.3 and arkworks 0.3 traits that no longer exist), so this file
is the authority for our conversion code.

Pinned: `ark-groth16 0.5`, `ark-bn254 0.5`, `groth16-solana 0.2`,
`solana-program 4.0`, `solana-bn254 2.2.2`.

## Byte layouts

| Item | Layout | Size |
|---|---|---|
| G1 | `x_be ‖ y_be` | 64 |
| G2 | `x.c1_be ‖ x.c0_be ‖ y.c1_be ‖ y.c0_be` | 128 |
| Public input | canonical `Fr`, big-endian | 32 |
| Point at infinity | all zero bytes | 64 / 128 |

The G2 **cross order** is the single easiest thing to get wrong. `solana-bn254`'s
`PodG2::from_be_bytes` reads `c1` before `c0` (its own source comments it "note
the cross order"). arkworks serializes `Fq2` as `c0_le ‖ c1_le`, so reversing the
whole 64-byte block flips limb endianness *and* swaps the components in one step.
G1 therefore reverses 32-byte chunks; G2 reverses **64-byte** chunks.

## `proof_a` must be pre-negated

`Groth16Verifier::new` performs length checks only — no arithmetic — and
`verify_common` feeds `proof_a` directly into the four-pair product
`e(A,B)·e(Σic,γ)·e(C,δ)·e(α,β) == 1`, which holds only when `A = −A_proof`.
Serialize `-proof.a`. Nothing negates it for you; the crate's own negative test
passes a non-negated `proof_a` specifically to assert failure.

## Verifying key

```rust
pub struct Groth16Verifyingkey<'a> {
    pub nr_pubinputs: usize,
    pub vk_alpha_g1: [u8; 64],
    pub vk_beta_g2: [u8; 128],
    pub vk_gamme_g2: [u8; 128],   // the typo is the real field name
    pub vk_delta_g2: [u8; 128],
    pub vk_ic: &'a [[u8; 64]],
}
```

`vk_ic.len() == n + 1` for `n` public inputs; `vk_ic[0]` is the constant term and
`vk_ic[i+1]` multiplies input `i`, in the order inputs were allocated in the
circuit. `nr_pubinputs` is **never read** by any verification path, and the
upstream JS generator sets it to `n + 1` rather than `n` — do not rely on it. The
const generic `NR_INPUTS` on `Groth16Verifier` is the real input count, and
`public_inputs` is `&[[u8; 32]; NR_INPUTS]`, a fixed-size array reference rather
than a slice.

`verify()` returns `Result<(), Groth16Error>`; there is no `Ok(false)`.

## Compute units

Measured in LiteSVM against a real SBF build, with a scratch circuit per input
count that is not part of this repository. The figure this repository does
reproduce is the whole `submit_spend` instruction at three inputs — about
101,000 CU, printed by `cargo test -p mirror-pool-program --test end_to_end --
--nocapture`.

The instruction's cost is not identical run to run. Each run derives the spend
record's address from a freshly generated nullifier, and `find_program_address`
searches a different number of bumps to get there, which moves the total by a
few thousand CU. So the test asserts a bound — under 110,000 CU, against the
200,000 a single instruction gets by default — rather than pinning a figure that
would fail on nothing.

| Public inputs | `verify()` |
|---|---|
| 1 | 79,840 |
| 2 | 85,500 |
| 4 | 96,822 |
| 8 | 119,466 |

`verify() ≈ 74,179 + 5,661·N`. The pairing itself is a fixed 73,612 CU; each
additional public input costs one G1 multiplication (3,840), one addition (334),
allocation overhead (~510) and a canonical-range check (~977).

**Design consequence:** public inputs are the only knob. Every input we can fold
into a hash saves ~5.7k CU, so the circuit binds a single digest wherever several
values would otherwise be exposed separately.

`verify_unchecked` saves ~977 CU per input by skipping the `< r` range check. It
is safe only for inputs we constructed ourselves from `Fr`; never for
attacker-supplied bytes. We use the checked form on anything that crosses the
instruction boundary.

## Pitfalls

1. **Serialize coordinates, not the point.** arkworks packs a y-sign flag into
   the top bit of y's most significant byte even under `Compress::No` — it fires
   on roughly half of all points. Verification still succeeds (deserialization
   masks flags), but the bytes are non-canonical, which silently breaks anything
   that hashes or compares serialized proofs and keys. Write `p.x` and `p.y`
   separately.
2. **Infinity encodes as all zeros** in the syscall ABI, but arkworks writes flag
   `0x40`. An explicit infinity branch is required. Honest keys never contain
   infinity; adversarial input can.
3. **Uncompressed sizes are 64 and 128**, not 65/129. A trailing byte appears in
   some upstream examples as arkworks-0.3 residue; it is vestigial.
4. **Compressed input is not accepted.** Decompression costs 398 CU for G1 but
   **13,610** for G2, so compressing to save transaction bytes is rarely worth it.
5. **`ark-bn254` needs `features = ["curve"]`** when `default-features = false`;
   `scalar_field` alone gives `Fr` but not the curve groups.
6. **Duplicate arkworks in the lockfile is expected.** `groth16-solana` pulls
   `solana-bn254`, which pins arkworks 0.4 — but only under
   `cfg(not(target_os = "solana"))`, so it is host-only and never linked into the
   `.so`. Types from different arkworks majors never unify; keep ours on 0.5.
7. **Subgroup and curve checks happen inside the syscall**, not in `new()`. They
   cannot be skipped and need not be duplicated.
8. **`cargo-build-sbf` resolves dependencies with the host cargo.** A host
   toolchain older than 1.85 fails on transitive `edition2024` crates. Our
   `rust-toolchain.toml` pins 1.97.1, which covers this.
