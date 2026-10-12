# SDK

JavaScript (ES modules) shared by the command line, the end-to-end scripts and the web app. Nothing under `src/` imports a Node built-in, so it runs in the browser.

| Module | What it does |
|---|---|
| `note.mjs` | Creates a note from `crypto.getRandomValues`, computes its commitment and nullifier, and reads and writes the note file. |
| `tree.mjs` | The depth-20 Poseidon tree of mirror-core: zero ladder, roots, siblings, and the clean tree as a mask. |
| `prove.mjs` | Builds the circuit input from a note and both trees, proves with snarkjs, and converts the proof to Solidity calldata. |
| `tempo.mjs` | The pool's ABI, the action binding, `syncPool` from events, the approve-and-deposit batch, the issuer's block-and-exclude batch, and the relayer request. |
| `solana.mjs` | mirror-pool's action binding, the proof in `groth16-solana` layout, the PDAs, instruction builders for both programs, and `syncPool` from history. |

## What the tests pin

`npm test` runs 16 tests, most of them against values computed elsewhere:

- trees, paths and circuit inputs equal mirror-core's vectors;
- the Solana binding equals mirror-core's `action_binding`;
- the Solana proof layout equals snarkjs-bridge's output, and the Solidity layout equals snarkjs's exporter;
- the Tempo ABI matches the compiled pool;
- the Solana PDAs and the policy hash are the ones already on devnet.

## Command line

```sh
node bin/cli.mjs note new  --chain tempo  --network moderato --out note.json
node bin/cli.mjs deposit   --note note.json --key 0x…          # Tempo: approve and deposit in one batch transaction
node bin/cli.mjs status    --note note.json                    # deposited? clean? spent?
node bin/cli.mjs withdraw  --note note.json --to 0x… --relayer http://127.0.0.1:8787

node bin/cli.mjs note new  --chain solana --network devnet --out note.json
node bin/cli.mjs deposit   --note note.json --keypair ~/.config/solana/id.json
node bin/cli.mjs withdraw  --note note.json --to <pubkey> --relay-keypair relay.json
node bin/cli.mjs settle    --network devnet
```

**The note file is the deposit.** Whoever holds it can withdraw, so the command line writes it with mode 600 and never overwrites one.

**Proving files.** Proving needs `withdraw.wasm` and `withdraw_final.zkey` from the [ceremony-v1 release](https://github.com/thiagorochatr/colosseum2026/releases/tag/ceremony-v1). Put them at the paths in `circuits/`, or point `WITHDRAW_WASM` and `WITHDRAW_ZKEY` at them.

**What a withdrawal does on each chain:**
- **Tempo:** it goes through the relayer, `tempo/relayer/relayer.mjs`.
- **Solana:** the relay keypair signs `VerifyClearance` and `SubmitClearedSpend` in one transaction, and `settle` pays the batch.

**Tested on local networks.** On a local Tempo node, a note deposited, withdrew through the relayer and showed as spent. A note whose depositor the issuer then blocked showed as not clean, and the command line refused to withdraw it. On a local Solana validator, two notes became clean once the curator published, then withdrew and settled, and each beneficiary received the denomination minus the fee.
