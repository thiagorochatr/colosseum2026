//! The withdraw proof verified on-chain, in a real SVM, by the built program.
//!
//! The fixtures are snarkjs proofs of mirror-core's vectors, converted by
//! snarkjs-bridge, which refused to emit any proof arkworks did not verify.

use compliance_program::{tag, verify::ProofRequest};
use litesvm::LiteSVM;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_program::{instruction::Instruction, pubkey::Pubkey};
use solana_signer::Signer;
use solana_transaction::Transaction;

/// The BN254 base field modulus, big-endian: negating a G1 point maps y to q - y.
const Q_BE: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x97, 0x81, 0x6a, 0x91, 0x68, 0x71, 0xca, 0x8d, 0x3c, 0x20, 0x8c, 0x16, 0xd8, 0x7c, 0xfd, 0x47,
];
/// The BN254 scalar field modulus, big-endian.
const P_BE: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91, 0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

fn program_bytes() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/sbf/deploy/compliance_program.so"
    );
    std::fs::read(path)
        .unwrap_or_else(|e| panic!("could not read {path}: {e}\n\nRun `make build-sbf` first."))
}

fn arr<const N: usize>(hex_str: &str) -> [u8; N] {
    hex::decode(hex_str).unwrap().try_into().unwrap()
}

fn fixtures() -> Vec<ProofRequest> {
    let text = include_str!("fixtures/proofs.json");
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    v["proofs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let ins: Vec<[u8; 32]> = p["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| arr(x.as_str().unwrap()))
                .collect();
            ProofRequest {
                a: arr(p["a"].as_str().unwrap()),
                b: arr(p["b"].as_str().unwrap()),
                c: arr(p["c"].as_str().unwrap()),
                inputs: ins.try_into().unwrap(),
            }
        })
        .collect()
}

struct Env {
    svm: LiteSVM,
    program_id: Pubkey,
    payer: Keypair,
}

impl Env {
    fn new() -> Self {
        let mut svm = LiteSVM::new();
        let program_id = Pubkey::new_unique();
        svm.add_program(program_id, &program_bytes())
            .expect("loading the built program");
        let payer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();
        Env {
            svm,
            program_id,
            payer,
        }
    }

    /// Sends raw instruction data. Ok(compute units) or Err(debug of the error).
    fn send(&mut self, data: Vec<u8>) -> Result<u64, String> {
        let ix = Instruction::new_with_bytes(self.program_id, &data, vec![]);
        let msg = Message::new(&[ix], Some(&self.payer.pubkey()));
        // A fresh blockhash per send, so two identical requests are two
        // transactions rather than a rejected duplicate.
        self.svm.expire_blockhash();
        let tx = Transaction::new(&[&self.payer], msg, self.svm.latest_blockhash());
        self.svm
            .send_transaction(tx)
            .map(|m| m.compute_units_consumed)
            .map_err(|e| format!("{:?}", e.err))
    }

    fn verify(&mut self, req: &ProofRequest) -> Result<u64, String> {
        let mut data = vec![tag::VERIFY_PROOF];
        data.extend(req.encode());
        self.send(data)
    }
}

fn custom(code: u32) -> String {
    format!("Custom({code})")
}

fn add_one(x: &mut [u8; 32]) {
    for byte in x.iter_mut().rev() {
        let (v, carry) = byte.overflowing_add(1);
        *byte = v;
        if !carry {
            break;
        }
    }
}

/// q - y over 32-byte big-endian integers.
fn negate_y(a: &mut [u8; 64]) {
    let mut out = [0u8; 32];
    let mut borrow = 0i16;
    for i in (0..32).rev() {
        let d = Q_BE[i] as i16 - a[32 + i] as i16 - borrow;
        borrow = if d < 0 { 1 } else { 0 };
        out[i] = (d + 256 * borrow) as u8;
    }
    a[32..].copy_from_slice(&out);
}

#[test]
fn every_valid_proof_verifies_on_chain_within_budget() {
    let mut env = Env::new();
    let proofs = fixtures();
    assert_eq!(proofs.len(), 4);
    let mut worst = 0;
    for req in &proofs {
        let cu = env.verify(req).expect("a valid proof must verify");
        worst = worst.max(cu);
    }
    println!("VerifyProof, 4 public inputs: worst case {worst} compute units");
    assert!(
        worst < 200_000,
        "must fit the default per-instruction budget, used {worst}"
    );
}

#[test]
fn changing_any_public_input_fails_verification() {
    let mut env = Env::new();
    let base = &fixtures()[0];
    for i in 0..4 {
        let mut req = ProofRequest {
            a: base.a,
            b: base.b,
            c: base.c,
            inputs: base.inputs,
        };
        add_one(&mut req.inputs[i]);
        assert_eq!(
            env.verify(&req),
            Err(format!("InstructionError(0, {})", custom(3))),
            "input {i}"
        );
    }
}

#[test]
fn the_input_order_is_part_of_the_statement() {
    let mut env = Env::new();
    let base = &fixtures()[0];
    let mut swapped = base.inputs;
    swapped.swap(0, 1);
    let req = ProofRequest {
        a: base.a,
        b: base.b,
        c: base.c,
        inputs: swapped,
    };
    assert!(env.verify(&req).unwrap_err().contains(&custom(3)));
}

#[test]
fn one_notes_proof_does_not_verify_another_notes_inputs() {
    let mut env = Env::new();
    let proofs = fixtures();
    let req = ProofRequest {
        a: proofs[0].a,
        b: proofs[0].b,
        c: proofs[0].c,
        inputs: proofs[1].inputs,
    };
    assert!(env.verify(&req).unwrap_err().contains(&custom(3)));
}

#[test]
fn an_unnegated_a_fails_so_the_conversion_negates_it() {
    let mut env = Env::new();
    let base = &fixtures()[0];
    let mut a = base.a;
    negate_y(&mut a);
    let req = ProofRequest {
        a,
        b: base.b,
        c: base.c,
        inputs: base.inputs,
    };
    assert!(env.verify(&req).unwrap_err().contains(&custom(3)));
    // And negating twice restores it.
    negate_y(&mut a);
    assert!(env
        .verify(&ProofRequest {
            a,
            b: base.b,
            c: base.c,
            inputs: base.inputs
        })
        .is_ok());
}

#[test]
fn a_non_canonical_input_is_refused_before_verification() {
    let mut env = Env::new();
    let base = &fixtures()[0];
    let mut inputs = base.inputs;
    inputs[3] = P_BE;
    let req = ProofRequest {
        a: base.a,
        b: base.b,
        c: base.c,
        inputs,
    };
    assert!(env.verify(&req).unwrap_err().contains(&custom(2)));
}

#[test]
fn malformed_and_unknown_instructions_are_refused() {
    let mut env = Env::new();
    let base = &fixtures()[0];
    let mut short = vec![tag::VERIFY_PROOF];
    short.extend(&base.encode()[..100]);
    assert!(env.send(short).unwrap_err().contains(&custom(1)));
    assert!(env.send(vec![42]).unwrap_err().contains(&custom(0)));
    assert!(env.send(vec![]).unwrap_err().contains(&custom(0)));
}
