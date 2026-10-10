//! Reads snarkjs verifying keys and proofs into arkworks, checks every proof
//! with arkworks before trusting the conversion, and converts both into the
//! byte layout `groth16-solana` expects.
//!
//! The conversion itself is mirror-circuit's, which was validated on devnet and
//! handles the three known pitfalls: G2 stores `c1` before `c0`, `proof_a` must
//! be negated, and coordinates are serialised rather than points.

use ark_bn254::{Bn254, Fq, Fq2, Fr, G1Affine, G2Affine};
use ark_ff::PrimeField;
use ark_groth16::{prepare_verifying_key, Groth16, Proof, VerifyingKey};
use mirror_circuit::solana::{fr_to_be, proof_to_solana, vk_to_solana, SolanaVerifyingKey};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub type Error = String;

/// A decimal string that is a canonical field element. arkworks' `from_str`
/// reduces modulo the field's order, so `p` would silently read as `0`. The
/// value is accepted only if writing it back gives the exact same string.
fn num<F: PrimeField>(v: &Value, what: &str) -> Result<F, Error> {
    let s = v
        .as_str()
        .ok_or_else(|| format!("{what}: expected a decimal string"))?;
    let f = F::from_str(s).map_err(|_| format!("{what}: not a decimal number: {s}"))?;
    if f.into_bigint().to_string() != s {
        return Err(format!("{what}: not a canonical field element: {s}"));
    }
    Ok(f)
}

fn at<'a>(v: &'a Value, i: usize, what: &str) -> Result<&'a Value, Error> {
    v.get(i).ok_or_else(|| format!("{what}: missing index {i}"))
}

/// A snarkjs G1 point, `[x, y, "1"]`, checked to be on the curve.
pub fn g1(v: &Value, what: &str) -> Result<G1Affine, Error> {
    if at(v, 2, what)?.as_str() != Some("1") {
        return Err(format!("{what}: not in affine form"));
    }
    let p = G1Affine::new_unchecked(
        num::<Fq>(at(v, 0, what)?, what)?,
        num::<Fq>(at(v, 1, what)?, what)?,
    );
    if !p.is_on_curve() || !p.is_in_correct_subgroup_assuming_on_curve() {
        return Err(format!("{what}: not a valid G1 point"));
    }
    Ok(p)
}

/// A snarkjs G2 point, `[[x.c0, x.c1], [y.c0, y.c1], ["1", "0"]]`, checked to be
/// on the curve and in the prime-order subgroup. Reading the coordinates in the
/// wrong order gives a point that fails this check, so a swap cannot slip by.
pub fn g2(v: &Value, what: &str) -> Result<G2Affine, Error> {
    let fq2 = |p: &Value, part: &str| -> Result<Fq2, Error> {
        Ok(Fq2::new(
            num(at(p, 0, part)?, part)?,
            num(at(p, 1, part)?, part)?,
        ))
    };
    let z = at(v, 2, what)?;
    if at(z, 0, what)?.as_str() != Some("1") || at(z, 1, what)?.as_str() != Some("0") {
        return Err(format!("{what}: not in affine form"));
    }
    let p = G2Affine::new_unchecked(fq2(at(v, 0, what)?, what)?, fq2(at(v, 1, what)?, what)?);
    if !p.is_on_curve() || !p.is_in_correct_subgroup_assuming_on_curve() {
        return Err(format!("{what}: not a valid G2 point"));
    }
    Ok(p)
}

/// A snarkjs `verification_key.json`.
pub fn verifying_key(v: &Value) -> Result<VerifyingKey<Bn254>, Error> {
    if v["protocol"] != "groth16" || v["curve"] != "bn128" {
        return Err("not a groth16 key on bn128".into());
    }
    let ic = v["IC"].as_array().ok_or("IC: missing")?;
    let n_public = v["nPublic"].as_u64().ok_or("nPublic: missing")? as usize;
    if ic.len() != n_public + 1 {
        return Err(format!(
            "IC has {} points for {n_public} public inputs",
            ic.len()
        ));
    }
    Ok(VerifyingKey {
        alpha_g1: g1(&v["vk_alpha_1"], "vk_alpha_1")?,
        beta_g2: g2(&v["vk_beta_2"], "vk_beta_2")?,
        gamma_g2: g2(&v["vk_gamma_2"], "vk_gamma_2")?,
        delta_g2: g2(&v["vk_delta_2"], "vk_delta_2")?,
        gamma_abc_g1: ic
            .iter()
            .enumerate()
            .map(|(i, p)| g1(p, &format!("IC[{i}]")))
            .collect::<Result<_, _>>()?,
    })
}

/// A snarkjs proof.
pub fn proof(v: &Value) -> Result<Proof<Bn254>, Error> {
    if v["protocol"] != "groth16" {
        return Err("not a groth16 proof".into());
    }
    Ok(Proof {
        a: g1(&v["pi_a"], "pi_a")?,
        b: g2(&v["pi_b"], "pi_b")?,
        c: g1(&v["pi_c"], "pi_c")?,
    })
}

/// snarkjs public signals, each checked to be a canonical field element.
pub fn public_signals(v: &Value) -> Result<Vec<Fr>, Error> {
    v.as_array()
        .ok_or("public signals: expected an array")?
        .iter()
        .enumerate()
        .map(|(i, s)| num::<Fr>(s, &format!("public signal {i}")))
        .collect()
}

/// Verifies with arkworks. Every conversion is gated on this.
pub fn verify(
    vk: &VerifyingKey<Bn254>,
    proof: &Proof<Bn254>,
    inputs: &[Fr],
) -> Result<bool, Error> {
    let pvk = prepare_verifying_key(vk);
    Groth16::<Bn254>::verify_proof(&pvk, proof, inputs).map_err(|e| format!("arkworks: {e:?}"))
}

/// sha256 over the key in on-chain layout, in a fixed order. Both chains print
/// the digest of the key they deploy, so equal digests mean the same statement.
pub fn vk_digest(vk: &SolanaVerifyingKey) -> [u8; 32] {
    Sha256::digest(vk.digest_preimage()).into()
}

/// The verifying key as a Rust module for the compliance program.
pub fn vk_to_rust(vk: &VerifyingKey<Bn254>, provenance: &str) -> String {
    use std::fmt::Write;
    let sv = vk_to_solana(vk);
    let mut s = String::new();
    let _ = writeln!(
        s,
        "//! Generated by snarkjs-bridge from {provenance}. Do not edit."
    );
    let _ = writeln!(s, "//!");
    let _ = writeln!(
        s,
        "//! Verifies {} public inputs: poolRoot, cleanRoot, nullifierHash, binding.",
        sv.public_input_count()
    );
    let _ = writeln!(
        s,
        "//! Key digest (sha256, on-chain layout): {}\n",
        hex::encode(vk_digest(&sv))
    );
    let _ = writeln!(s, "use groth16_solana::groth16::Groth16Verifyingkey;\n");
    let _ = writeln!(s, "pub const VK_DIGEST: [u8; 32] = {:?};\n", vk_digest(&sv));
    let emit = |s: &mut String, name: &str, bytes: &[u8]| {
        let _ = writeln!(s, "pub const {name}: [u8; {}] = [", bytes.len());
        for chunk in bytes.chunks(16) {
            let line: Vec<String> = chunk.iter().map(|b| b.to_string()).collect();
            let _ = writeln!(s, "    {},", line.join(", "));
        }
        let _ = writeln!(s, "];\n");
    };
    emit(&mut s, "VK_ALPHA_G1", &sv.alpha_g1);
    emit(&mut s, "VK_BETA_G2", &sv.beta_g2);
    emit(&mut s, "VK_GAMMA_G2", &sv.gamma_g2);
    emit(&mut s, "VK_DELTA_G2", &sv.delta_g2);
    let _ = writeln!(s, "pub const VK_IC: [[u8; 64]; {}] = [", sv.ic.len());
    for point in &sv.ic {
        let line: Vec<String> = point.iter().map(|b| b.to_string()).collect();
        let _ = writeln!(s, "    [{}],", line.join(", "));
    }
    let _ = writeln!(s, "];\n");
    let _ = writeln!(
        s,
        "pub const VERIFYING_KEY: Groth16Verifyingkey<'static> = Groth16Verifyingkey {{\n    nr_pubinputs: {},\n    vk_alpha_g1: VK_ALPHA_G1,\n    vk_beta_g2: VK_BETA_G2,\n    vk_gamme_g2: VK_GAMMA_G2,\n    vk_delta_g2: VK_DELTA_G2,\n    vk_ic: &VK_IC,\n}};",
        sv.public_input_count()
    );
    s
}

/// A proof and its public inputs in on-chain layout, as hex. `a` is negated.
pub fn proof_to_hex(proof: &Proof<Bn254>, inputs: &[Fr]) -> Value {
    let (a, b, c) = proof_to_solana(proof);
    serde_json::json!({
        "a": hex::encode(a),
        "b": hex::encode(b),
        "c": hex::encode(c),
        "inputs": inputs.iter().map(|f| hex::encode(fr_to_be(f))).collect::<Vec<_>>(),
    })
}
