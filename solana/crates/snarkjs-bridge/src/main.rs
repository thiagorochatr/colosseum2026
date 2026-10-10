//! snarkjs-bridge vk <vk.json> <provenance>         prints the Rust module for the program
//! snarkjs-bridge proofs <vk.json> <proofs.json>    prints the proofs in on-chain layout
//!
//! Both commands verify with arkworks first and refuse to emit anything that fails.

use snarkjs_bridge as b;
use std::process::exit;

fn read(path: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        exit(1)
    });
    serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        exit(1)
    })
}

fn or_die<T>(r: Result<T, b::Error>) -> T {
    r.unwrap_or_else(|e| {
        eprintln!("error: {e}");
        exit(1)
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("vk") if args.len() == 4 => {
            let vk = or_die(b::verifying_key(&read(&args[2])));
            print!("{}", b::vk_to_rust(&vk, &args[3]));
        }
        Some("proofs") if args.len() == 4 => {
            let vk = or_die(b::verifying_key(&read(&args[2])));
            let file = read(&args[3]);
            let mut out = Vec::new();
            for entry in file["proofs"]
                .as_array()
                .unwrap_or_else(|| or_die(Err("no proofs".into())))
            {
                let p = or_die(b::proof(&entry["proof"]));
                let inputs = or_die(b::public_signals(&entry["publicSignals"]));
                if !or_die(b::verify(&vk, &p, &inputs)) {
                    or_die::<()>(Err(format!(
                        "proof for index {} does not verify",
                        entry["index"]
                    )));
                }
                let mut v = b::proof_to_hex(&p, &inputs);
                v["index"] = entry["index"].clone();
                out.push(v);
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({ "proofs": out })).unwrap()
            );
        }
        _ => {
            eprintln!("usage:\n  snarkjs-bridge vk <vk.json> <provenance>\n  snarkjs-bridge proofs <vk.json> <proofs.json>");
            exit(2);
        }
    }
}
