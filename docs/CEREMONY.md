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
| 1 | Thiago Rocha | *pending* |
| | Beacon | *pending* |

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

The output lists every contribution hash, which must match the table above. `ceremony/transcript.txt` holds the output recorded when the ceremony closed.
