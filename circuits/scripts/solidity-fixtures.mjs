// Converts snarkjs proofs into the calldata layout of the snarkjs Solidity
// verifier, using snarkjs's own exporter, so the G2 coordinate order matches
// the verifier exactly.
//
//   node scripts/solidity-fixtures.mjs build/proofs_dev.json ../tempo/test/fixtures/proofs_dev.json

import { readFileSync, writeFileSync } from "node:fs";
import * as snarkjs from "snarkjs";

const [input, out] = process.argv.slice(2);
if (!input || !out) throw new Error("usage: solidity-fixtures.mjs <proofs.json> <out.json>");

const { proofs } = JSON.parse(readFileSync(input, "utf8"));
const converted = [];
for (const { index, proof, publicSignals } of proofs) {
  const calldata = await snarkjs.groth16.exportSolidityCallData(proof, publicSignals);
  const [a, b, c, inputs] = JSON.parse(`[${calldata}]`);
  converted.push({ index, a, b, c, inputs });
}
writeFileSync(out, JSON.stringify({ proofs: converted }, null, 2));
console.log(`wrote ${converted.length} proofs to ${out}`);
await globalThis.curve_bn128?.terminate();
