# Security

This is hackathon software running on test networks: Tempo's Moderato testnet and Solana devnet. It has not been audited. Do not use it with real value.

## What a deployment does and does not rest on

- **The trusted setup.** Groth16 is sound if at least one contribution in each phase of the setup was honest. Phase 1 is the Perpetual Powers of Tau, with 80 contributions. Phase 2 has one human contributor plus a public randomness beacon, so it rests on that one contributor having destroyed their entropy. That is acceptable on a test network and is not enough for real value. [docs/CEREMONY.md](docs/CEREMONY.md) has the full transcript.
- **Upgradeable programs.** The Solana programs are deployed upgradeable, and the upgrade keys are test keys.
- **Test-network keys.** The demo service holds an issuer key, a curator key and relayer keys for the test networks. They control nothing outside the demo.
- **The rest of the model** is in [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md): who can do what, and the known limits.

## What CI checks on every push

- The circuit compiles to the R1CS and witness generator the ceremony used, by sha256.
- A fresh verification of the ceremony, from phase 1 to the final key, reproduces the committed transcript exactly.
- Both chains' verifiers are exactly what the final key generates, and the circuit agrees with the reference implementation's test vectors.
- The Tempo contracts pass against Tempo's real precompiles, and the Solana programs pass under rustfmt, clippy and their LiteSVM suites.
- The Rust dependency tree passes `cargo deny` under [solana/deny.toml](solana/deny.toml), where every exception carries its reason.

A second workflow checks the networks themselves, daily and on every push to `main`. Moderato and devnet run byte for byte what this repository builds. The ceremony's beacon is the announced Solana mainnet blockhash.

## Known dependency advisories

`npm audit` still reports advisories in two dependency chains. None of them is reached with input an attacker controls.

- **elliptic and the ethers v5 packages**, through circomlibjs, and elliptic again through crypto-browserify in the web app's Node polyfills. This project uses circomlibjs only for the Poseidon hash, and nothing in it signs or verifies with elliptic: viem and @solana/web3.js bring their own curve code. elliptic has no patched release.
- **uuid and stream-json**, through jayson in @solana/web3.js 1.x. They parse JSON-RPC responses from this project's own RPC endpoint. Only @solana/web3.js 3 drops them, and that is a rewrite of the client API.

The Rust tree is checked by `cargo deny` instead, and its one exception is written down in [solana/deny.toml](solana/deny.toml).

## Reporting a vulnerability

Please report privately, through GitHub's **Report a vulnerability** button on this repository's Security tab. Do not open a public issue. Reports in English or Portuguese are welcome.
