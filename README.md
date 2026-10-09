# colosseum2026

A compliance layer for private payments, built for the Colosseum World's Fair hackathon.

> Tempo has compliance without privacy. Solana has privacy without compliance.
> One zero-knowledge proof closes both gaps.

A user proves that their deposit belongs to a **clean set** without revealing which deposit is theirs. The same circuit and the same verifying key run on both chains:

- **Tempo:** a private stablecoin pool whose clean set is derived on-chain from the token issuer's own TIP-403 policy. A depositor who is blocklisted after depositing falls out of the set and cannot withdraw.
- **Solana:** a compliance program that verifies the proof and records a clearance any program can check. mirror-pool is the reference pool.

Work in progress.

## Layout

| Path | What it is |
|---|---|
| `mirror-pool/` | Prior work, imported unchanged. See below. |

## Prior work

`mirror-pool/` is [mirror-pool](https://github.com/solanabr/mirror-pool), the winning submission of Superteam Brasil's Noise bounty, written before this hackathon. It was imported at commit `3600c97`, which adds the judge's follow-up recommendations (upstream PR #6) on top of the merged version. The first commit of this repository is that import, byte for byte. Every commit after it is work done during the hackathon window, and the diff against that first commit is exactly what was built here.

## License

MIT. `mirror-pool/` keeps its own MIT license.
