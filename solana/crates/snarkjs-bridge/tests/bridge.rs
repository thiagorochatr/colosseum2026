//! The bridge reads snarkjs output correctly, and refuses what it cannot trust.

use serde_json::Value;
use snarkjs_bridge as b;

fn load(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn every_snarkjs_proof_verifies_under_arkworks() {
    let vk = b::verifying_key(&load("vk_dev.json")).unwrap();
    let proofs = load("proofs_dev.json");
    for entry in proofs["proofs"].as_array().unwrap() {
        let p = b::proof(&entry["proof"]).unwrap();
        let inputs = b::public_signals(&entry["publicSignals"]).unwrap();
        assert_eq!(inputs.len(), 4);
        assert!(b::verify(&vk, &p, &inputs).unwrap());
    }
}

#[test]
fn reading_g2_coordinates_in_the_wrong_order_is_caught() {
    let mut entry = load("proofs_dev.json")["proofs"][0]["proof"].clone();
    let x = entry["pi_b"][0].as_array().unwrap().clone();
    entry["pi_b"][0] = Value::Array(vec![x[1].clone(), x[0].clone()]);
    let err = b::proof(&entry).unwrap_err();
    assert!(err.contains("not a valid G2 point"), "{err}");
}

#[test]
fn a_tampered_public_signal_does_not_verify() {
    let vk = b::verifying_key(&load("vk_dev.json")).unwrap();
    let entry = &load("proofs_dev.json")["proofs"][0];
    let p = b::proof(&entry["proof"]).unwrap();
    let mut inputs = b::public_signals(&entry["publicSignals"]).unwrap();
    inputs[3] += ark_bn254::Fr::from(1u64);
    assert!(!b::verify(&vk, &p, &inputs).unwrap());
}

#[test]
fn a_key_with_the_wrong_number_of_ic_points_is_refused() {
    let mut vk = load("vk_dev.json");
    vk["nPublic"] = Value::from(3);
    assert!(b::verifying_key(&vk)
        .unwrap_err()
        .contains("IC has 5 points for 3"));
}

#[test]
fn a_non_canonical_public_signal_is_refused() {
    let p = "21888242871839275222246405745257275088548364400416034343698204186575808495617";
    let p_minus_1 = "21888242871839275222246405745257275088548364400416034343698204186575808495616";
    assert!(b::public_signals(&Value::from(vec![p]))
        .unwrap_err()
        .contains("not a canonical"));
    assert!(b::public_signals(&Value::from(vec!["007"]))
        .unwrap_err()
        .contains("not a canonical"));
    assert_eq!(
        b::public_signals(&Value::from(vec![p_minus_1, "0", "7"]))
            .unwrap()
            .len(),
        3
    );
}
