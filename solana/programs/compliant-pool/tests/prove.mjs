// Called by the Rust tests. Proves a withdrawal for the pool state the test
// built and prints it in groth16-solana's layout, as JSON:
//   {"ok": true, "a": hex, "b": hex, "c": hex, "inputs": [poolRoot, cleanRoot, nullifierHash, binding]}
// ok is false only when the witness fails at the circuit's clean-root check,
// as for an excluded note. Any other failure exits non-zero, so a missing file
// or a bad argument cannot pass for a refusal.
//
//   node prove.mjs <k> <r> <denomTag> <index> <binding> <excluded,csv|-> <leaves,csv>

import { readFileSync } from "node:fs";
import { prove, terminate } from "../../../../sdk/src/prove.mjs";
import { toSolana, fieldBytes, bytesToHex } from "../../../../sdk/src/solana.mjs";

const root = new URL("../../../..", import.meta.url).pathname;
const wasm = `${root}circuits/build/withdraw_js/withdraw.wasm`;
const zkey = `${root}circuits/ceremony/withdraw_final.zkey`;
const [k, r, denomTag, index, binding, excludedCsv, leavesCsv] = process.argv.slice(2);
const list = (csv) => (csv && csv !== "-" ? csv.split(",").map(BigInt) : []);
const hex = (u) => bytesToHex(u).slice(2);
// The witness generator reports the failed constraint's line.
const circuit = readFileSync(`${root}circuits/withdraw.circom`, "utf8").split("\n");
const cleanRootLine = circuit.findIndex((l) => l.includes("cleanRoot === cleanTree.root")) + 1;
if (!cleanRootLine) throw new Error("the clean-root check is not in circuits/withdraw.circom");

let out;
const seen = [];
const quiet = { log: console.log, error: console.error };
try {
  console.log = console.error = (...a) => seen.push(a.join(" "));
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
} catch (e) {
  const line = Number((seen.join("\n") + String(e?.message)).match(/line: (\d+)/)?.[1]);
  if (line !== cleanRootLine) throw new Error(`proving failed, but not at the clean-root check: ${e?.message}`, { cause: e });
  out = { ok: false };
} finally {
  Object.assign(console, quiet);
}
process.stdout.write(JSON.stringify(out));
await terminate();
