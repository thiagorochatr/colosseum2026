// Called by the Foundry tests through vm.ffi. Proves a withdrawal for the pool
// state the test built, and prints the result ABI-encoded for abi.decode:
//
//   (bool ok, uint256[2] a, uint256[2][2] b, uint256[2] c,
//    uint256 poolRoot, uint256 cleanRoot, uint256 nullifierHash)
//
// ok is false when the note cannot be proved against these trees, for example
// because it is excluded. The circuit refuses to produce a witness then.
//
//   node script/ffi-prove.mjs <k> <r> <denomTag> <index> <binding> <excluded,csv> <leaves,csv>

import { prove, toSolidity, terminate } from "../../sdk/src/prove.mjs";

const root = new URL("../..", import.meta.url).pathname;
const wasm = process.env.WITHDRAW_WASM ?? `${root}circuits/build/withdraw_js/withdraw.wasm`;
const zkey = process.env.WITHDRAW_ZKEY ?? `${root}circuits/ceremony/withdraw_final.zkey`;

const [k, r, denomTag, index, binding, excludedCsv, leavesCsv] = process.argv.slice(2);
const list = (csv) => (csv && csv !== "-" ? csv.split(",").map(BigInt) : []);
const word = (x) => BigInt(x).toString(16).padStart(64, "0");

let words;
try {
  // The witness generator reports a failed constraint on the console; keep it quiet.
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
  const s = toSolidity(proof);
  words = [1n, ...s.a, ...s.b.flat(), ...s.c, input.poolRoot, input.cleanRoot, input.nullifierHash];
} catch {
  words = Array(12).fill(0n);
}
process.stdout.write("0x" + words.map(word).join(""));
await terminate();
