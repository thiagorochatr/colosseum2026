# Trusted setup ceremony

The withdraw circuit uses Groth16, so it needs a trusted setup in two phases. A proof system set up this way is sound as long as **at least one** contribution in each phase was honest, meaning its random entropy was destroyed.

## Phase 1

The [Perpetual Powers of Tau](https://github.com/privacy-scaling-explorations/perpetualpowersoftau) from Privacy & Scaling Explorations, after 80 contributions, truncated to `2^14`. See [`circuits/ptau/README.md`](../circuits/ptau/README.md) for the download and its sha256. `snarkjs powersoftau verify` accepts the file.

## Phase 2

**Inputs**, all reproducible from this repository:

| | sha256 |
|---|---|
| `circuits/withdraw.circom` | `dae0a392866f1548f80e7d4e08dc3d42a92ff6bfc6b211b9e6e61c61aedc14a9` |
| `withdraw.r1cs`, built with `--O2` | `4ef6cdab5240650c3a755fddb5f8868efa2334506761c0aa5954f5b1d3fedcae` |
| `ppot_0080_14.ptau` | `3ca1149e9349b22b0ee0649399cfb787677129b7b1189d1899fc0d615d9583db` |
| `withdraw_0000.zkey`, the starting key | `c247eeb9bcfb2f150cacb66b4382a159a21e4b595ddaa29aaf21268586cf41fc` |

The starting key has no contribution yet and is deterministic:

```sh
cd circuits && npm run build
npx snarkjs groth16 setup build/withdraw.r1cs ptau/ppot_0080_14.ptau withdraw_0000.zkey
```

### The beacon, announced in advance

The last step is a public random beacon, so the final key cannot be chosen by whoever contributes last. It is announced here, in a commit published before it exists:

> **The blockhash of the first finalized Solana mainnet block at or after slot `455,095,000`**, as 32 bytes in hex, applied with `snarkjs zkey beacon` and `numIterationsExp = 10`.

When this was written, mainnet was near slot 455,087,000. The slot was expected around 2026-10-10 03:12 UTC. `circuits/ceremony/solana-beacon.mjs` reads the blockhash, and anyone can check it on any Solana explorer.

### Contributions

| # | Contributor | Contribution hash |
|---|---|---|
| 1 | Thiago Rocha | `e35b607f dd19c43a c3367064 6568d4e4 2ad745e4 13ea84fb f42a2787 dda14856 30fd849c 362f2e8b 83416ff2 845324ff 8447ec82 0629fbcd ea55d22a 94a29eb5` |
| 2 | Beacon | `c309be62 0f3b075d f9e3e3cd 4868b87f d52c5cdc e66a126a e01e8b73 37117270 f8581872 4e529ff2 c9a6389a 3f88684e d2f98403 af717679 60d70f86 a99a5e6d` |

### The beacon, as applied

- **Slot used:** `455,095,000`, the announced slot itself. It has a finalized block.
- **Blockhash:** `AEyQPgBgGwQ4ELNHYbSkaccXjbLfgSpjoeNrE4gKeBcw`, identical on two independent RPC providers.
- **As hex, the beacon generator:** `894d821d67644065f5210c9ed3fa1a7b9c884c244d80ee940e8397dc449c9c14`.
- **Iterations exponent:** `10`.

`circuits/ceremony/beacon.json` records these values.

### Result

| | sha256 |
|---|---|
| `withdraw_final.zkey`, the proving key | `a3217e83f95632a27e27d7593167cdfe5bb9ecaa0539e926cdce38248ce180f5` |
| `circuits/ceremony/vk.json`, the verifying key | `e5d67dab6cb8819d8b6e11f1f36d460d68f542392049f86fc5c0b0b5bec07cf0` |
| The verifying key in Solana's on-chain layout | `e453d98a0fea776c6cbc7a307532e44947a0f13aafeae3bebf8793897b455c4a` |

The proving key, the witness generator and the verifying key are published in the [ceremony-v1 release](https://github.com/thiagorochatr/colosseum2026/releases/tag/ceremony-v1), with a `SHA256SUMS` file. The Solana module and the Solidity verifier are both generated from this key by `circuits/scripts/regenerate-verifiers.sh`. A check confirms that every constant of `vk.json` appears in the Solidity verifier. The circuit, Solana and Tempo test suites all pass under it.

### Two discarded attempts

Both were the script's fault, not the contributor's, and both printed the entropy on the contributor's screen:

1. The first passed the entropy as a command-line argument through `npx`. npm 12 prints the full command line. Discarded key sha256: `bee6056ca7dfd1c294e215b8a12bb6962b6b2c0bbfdf14b96ea8a3de2c717cea`.
2. The second piped the entropy into snarkjs's interactive prompt. That prompt echoes its input when the output is a terminal. The script's test captured the output instead of using a terminal, so it missed the echo. Discarded key sha256: `555e216bbb21ecc1dc928185121a9ee0bf12b9de5c754f13a1c95c3dc965c6ee`.

snarkjs mixes 64 bytes of its own OS randomness into every contribution, and those were never shown, so the printed text alone does not reveal either discarded secret. Both were discarded anyway, because the script promises the entropy is never printed. Neither key is part of the chain.

The script now calls snarkjs's `zKey.contribute` API from `contribute.mjs`, with the entropy read from stdin into memory. It never passes through a command line, an environment variable or a prompt, and any log line containing it is refused. It was tested inside a pseudo-terminal, where the same test does catch the old prompt's echo. The real contribution was made after this fix and before the beacon slot.

### Trust model, stated plainly

Phase 2 has a single human contributor. The final key is sound if that contribution's entropy was destroyed, which only the contributor can attest to. The beacon does not change that: it stops the last contributor from choosing the result, but it does not replace an honest contribution. This is acceptable for test networks and is not enough for real value.

The ceremony is open. Anyone can add a contribution on top of the final key with `circuits/ceremony/contribute.sh`, apply a new beacon, and the verifiers on both chains are regenerated from the result.

## Verifying

```sh
cd circuits
npx snarkjs zkey verify build/withdraw.r1cs ptau/ppot_0080_14.ptau ceremony/withdraw_final.zkey
```

The output lists every contribution hash, which must match the table above. `circuits/ceremony/transcript.txt` holds the output recorded when the ceremony closed, on 2026-10-10.
