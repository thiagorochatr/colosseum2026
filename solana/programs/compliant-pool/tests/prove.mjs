// Called by the Rust tests. Proves a withdrawal for the pool state the test
// built and prints it in groth16-solana's layout, as JSON:
//   {"ok": true, "a": hex, "b": hex, "c": hex, "inputs": [poolRoot, cleanRoot, nullifierHash, binding]}
// ok is false when the note cannot be proved against these trees.
//
//   node prove.mjs <k> <r> <denomTag> <index> <binding> <excluded,csv|-> <leaves,csv>

import { prove, terminate } from "../../../../sdk/src/prove.mjs";
import { toSolana, fieldBytes, bytesToHex } from "../../../../sdk/src/solana.mjs";

const root = new URL("../../../..", import.meta.url).pathname;
const wasm = `${root}circuits/build/withdraw_js/withdraw.wasm`;
const zkey = `${root}circuits/ceremony/withdraw_final.zkey`;
const [k, r, denomTag, index, binding, excludedCsv, leavesCsv] = process.argv.slice(2);
const list = (csv) => (csv && csv !== "-" ? csv.split(",").map(BigInt) : []);
const hex = (u) => bytesToHex(u).slice(2);

let out = { ok: false };
try {
  console.log = console.error = () => {};
  const { proof, input } = await prove(
    {
      note: { k: BigInt(k), r: BigInt(r), denomTag: BigInt(denomTag) },
      index: Number(index),
      poolLeaves: list(leavesCsv),
      excluded: list(excludedCsv).map(Number),
      binding: BigInt(binding),
    },
    { wasm, zkey },
  );
  const s = toSolana(proof);
  out = {
    ok: true,
    a: hex(s.a),
    b: hex(s.b),
    c: hex(s.c),
    inputs: [input.poolRoot, input.cleanRoot, input.nullifierHash, input.binding].map((x) => hex(fieldBytes(x))),
  };
} catch {
  // The circuit refuses a witness for a note it cannot prove: ok stays false.
}
process.stdout.write(JSON.stringify(out));
await terminate();
