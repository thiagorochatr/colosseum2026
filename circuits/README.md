# Withdraw circuit

The statement from [`docs/SPEC.md`](../docs/SPEC.md) section 5, in circom 2.2.3 with circomlib 2.0.5.

## Frozen

The circuit is frozen. Any change to `withdraw.circom` invalidates the ceremony and every deployed verifier.

| | |
|---|---|
| `withdraw.circom` sha256 | `dae0a392866f1548f80e7d4e08dc3d42a92ff6bfc6b211b9e6e61c61aedc14a9` |
| `withdraw.r1cs` sha256, built with `--O2` | `4ef6cdab5240650c3a755fddb5f8868efa2334506761c0aa5954f5b1d3fedcae` |
| Constraints | 10,175, all non-linear |
| Public inputs, in order | `poolRoot`, `cleanRoot`, `nullifierHash`, `binding` |
| Private inputs | 63 |

The R1CS is reproducible: rebuilding from the same source gives the same hash.

## Build and test

```sh
npm install
npm run build        # compile into build/
npm run setup:dev    # development key; needs ptau/, see ptau/README.md
npm test
```

`npm run vectors` regenerates `test/vectors.json` with [`tools/vectors`](../tools/vectors), which computes every value with `mirror-core`, the code the Solana program links. The output is deterministic.

## What the tests check

Each rejection must fail at the specific constraint that should catch it, not merely fail.

| Case | Expected | Rejected at |
|---|---|---|
| Clean, deposited notes | accepted | |
| Note excluded from the clean set | rejected | clean root |
| Note a dishonest curator added without a deposit | rejected | pool root |
| Excluded note against an unmasked clean tree | accepted, which shows the mask is what rejects it | |
| Wrong nullifier, or another note's nullifier | rejected | nullifier |
| Wrong pool root / wrong clean root | rejected | pool root / clean root |
| Same note claiming another index | rejected | pool root |
| A path index that is not a bit | rejected | bit check |
| Wrong denomination tag | rejected | pool root |
| Groth16 proofs | verify, with public signals in the specified order | |
| Any public input changed after proving | verification fails | |

## Development key

`npm run setup:dev` makes a key from one local contribution. It is for testing only and is never deployed as final. The final key comes from the multi-party ceremony.
