// Proves every valid vector with a given key and writes the proofs as snarkjs
// JSON, for the on-chain verifier tests on both chains.
//
//   node scripts/prove-fixtures.mjs build/withdraw_dev.zkey build/proofs_dev.json

import { readFileSync, writeFileSync } from "node:fs";
import * as snarkjs from "snarkjs";

const [zkey, out] = process.argv.slice(2);
if (!zkey || !out) throw new Error("usage: prove-fixtures.mjs <zkey> <out.json>");

const here = new URL("..", import.meta.url).pathname;
const wasm = `${here}build/withdraw_js/withdraw.wasm`;
const vectors = JSON.parse(readFileSync(`${here}test/vectors.json`, "utf8"));

const dec = (x) => BigInt(x).toString();
const proofs = [];
for (const v of vectors.valid) {
  const input = {};
  for (const [k, val] of Object.entries(v)) {
    if (!k.startsWith("_")) input[k] = Array.isArray(val) ? val.map(dec) : dec(val);
  }
  const { proof, publicSignals } = await snarkjs.groth16.fullProve(input, wasm, zkey);
  proofs.push({ index: v._index, proof, publicSignals });
}
writeFileSync(out, JSON.stringify({ zkey, proofs }, null, 2));
console.log(`wrote ${proofs.length} proofs to ${out}`);
await globalThis.curve_bn128?.terminate();
