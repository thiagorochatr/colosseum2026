// Tests for the withdraw circuit against vectors computed by mirror-core.
//
// Accepting a vector means the circuit agrees with mirror-core's Poseidon, zero
// ladder and path convention. Each rejection is checked at witness generation,
// where circom asserts every `===`, so an unsatisfiable input never yields a
// witness at all. The proving tests then check the Groth16 layer itself.

import { test, after } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import * as snarkjs from "snarkjs";

const here = new URL("..", import.meta.url).pathname;
const WASM = join(here, "build/withdraw_js/withdraw.wasm");
// The ceremony's final key when it exists, otherwise the development key.
const FINAL = existsSync(join(here, "ceremony/withdraw_final.zkey"));
const ZKEY = join(here, FINAL ? "ceremony/withdraw_final.zkey" : "build/withdraw_dev.zkey");
const VK = join(here, FINAL ? "ceremony/vk.json" : "build/vk_dev.json");
const vectors = JSON.parse(readFileSync(join(here, "test/vectors.json"), "utf8"));

const PUBLIC_ORDER = ["poolRoot", "cleanRoot", "nullifierHash", "binding"];
const P = 21888242871839275222246405745257275088548364400416034343698204186575808495617n;

// The circuit's input: hex strings from Rust become decimal strings, and the
// generator's bookkeeping fields (prefixed with "_") are dropped.
function toInput(v) {
  const dec = (x) => BigInt(x).toString();
  const out = {};
  for (const [key, value] of Object.entries(v)) {
    if (key.startsWith("_")) continue;
    out[key] = Array.isArray(value) ? value.map(dec) : dec(value);
  }
  return out;
}

const tmp = mkdtempSync(join(tmpdir(), "withdraw-"));
let n = 0;

// The circuit line holding each check, read from the source so the tests do not
// hard-code line numbers that an edit would silently shift.
const source = readFileSync(join(here, "withdraw.circom"), "utf8").split("\n");
const lineOf = (needle) => {
  const i = source.findIndex((l) => l.includes(needle));
  assert.notEqual(i, -1, `no line contains ${needle}`);
  return i + 1;
};
const CHECK = {
  nullifier: lineOf("nullifierHash === nullifierHasher.out"),
  bit: lineOf("pathIndices[j] * (pathIndices[j] - 1) === 0"),
  poolRoot: lineOf("poolRoot === poolTree.root"),
  cleanRoot: lineOf("cleanRoot === cleanTree.root"),
};

// Generates a witness. Returns null on success, or the circuit line of the
// first failed constraint, which circom's witness generator reports on the
// console.
async function witnessFailure(input) {
  const file = join(tmp, `w${n++}.wtns`);
  const seen = [];
  const keep = { log: console.log, error: console.error };
  console.log = console.error = (...a) => seen.push(a.join(" "));
  try {
    await snarkjs.wtns.calculate(input, WASM, file);
    return null;
  } catch (e) {
    const m = (seen.join("\n") + String(e?.message)).match(/line: (\d+)/);
    return m ? Number(m[1]) : -1;
  } finally {
    Object.assign(console, keep);
  }
}
const witnessOk = async (input) => (await witnessFailure(input)) === null;
async function rejectedAt(input, check, label = "") {
  assert.equal(await witnessFailure(input), CHECK[check], `${label} should fail at the ${check} check`);
}

// Poseidon in JavaScript, for trees the vectors do not contain: circomlib's
// reference permutation (poseidon_old.circom) with its constants. It equals the
// circuit's optimized one, and each test using it checks that with a witness.
const POSEIDON = readFileSync(join(here, "node_modules/circomlib/circuits/poseidon_constants_old.circom"), "utf8");
const constants = {};
function poseidonConstants(fn, t) {
  const body = POSEIDON.slice(POSEIDON.indexOf(`function ${fn}(t)`));
  const branch = body.slice(body.indexOf(`t == ${t})`));
  // Drop the leading match, which is t itself.
  return branch.slice(0, branch.indexOf("}")).match(/0x[0-9a-fA-F]+|\d+/g).slice(1).map(BigInt);
}
function poseidon(inputs) {
  const t = inputs.length + 1;
  const nF = 8;
  const nP = [56, 57, 56, 60][t - 2];
  constants[t] ??= { C: poseidonConstants("POSEIDON_C", t), M: poseidonConstants("POSEIDON_M", t) };
  const { C, M } = constants[t];
  const pow5 = (x) => (((x * x) % P) ** 2n % P) * x % P;
  let s = [0n, ...inputs.map(BigInt)];
  for (let i = 0; i < nF + nP; i++) {
    s = s.map((x, j) => (x + C[t * i + j]) % P);
    s = i < nF / 2 || i >= nF / 2 + nP ? s.map(pow5) : [pow5(s[0]), ...s.slice(1)];
    s = s.map((_, row) => s.reduce((acc, x, j) => (acc + M[row * t + j] * x) % P, 0n));
  }
  return s[0];
}

const valid = vectors.valid.map(toInput);

// snarkjs keeps the curve's worker threads alive, which stops Node from exiting
// once the tests are done. Terminate them explicitly.
after(async () => {
  if (globalThis.curve_bn128) await globalThis.curve_bn128.terminate();
});

test("every clean, deposited note is accepted", async () => {
  assert.equal(valid.length, 4);
  for (const input of valid) assert.ok(await witnessOk(input));
});

test("a note excluded from the clean set is rejected", async () => {
  for (const v of vectors.rejected.excluded_note) {
    await rejectedAt(toInput(v), "cleanRoot", `index ${v._index}`);
  }
});

test("a dishonest curator cannot add a note that was never deposited", async () => {
  const v = vectors.rejected.phantom_note_from_dishonest_curator;
  await rejectedAt(toInput(v), "poolRoot");
});

test("the excluded note does pass against the pool alone, so the clean tree is what rejects it", async () => {
  // Sanity check on the vector: proving the excluded note against a clean tree
  // equal to the pool tree succeeds. Without this, the rejection above could be
  // caused by a broken vector rather than by the mask.
  const v = toInput(vectors.rejected.excluded_note[0]);
  assert.ok(await witnessOk({ ...v, cleanRoot: v.poolRoot, cleanSiblings: v.poolSiblings }));
});

test("a wrong nullifier is rejected", async () => {
  const input = { ...valid[0], nullifierHash: ((BigInt(valid[0].nullifierHash) + 1n) % P).toString() };
  await rejectedAt(input, "nullifier");
});

test("another note's nullifier is rejected", async () => {
  await rejectedAt({ ...valid[0], nullifierHash: valid[1].nullifierHash }, "nullifier");
});

test("a wrong pool root or clean root is rejected", async () => {
  await rejectedAt({ ...valid[0], poolRoot: valid[0].cleanRoot }, "poolRoot");
  await rejectedAt({ ...valid[0], cleanRoot: valid[0].poolRoot }, "cleanRoot");
});

test("the note must sit at the same index in both trees", async () => {
  // Note 0 keeps its true pool path, with a genuine clean path for the same
  // leaf at index 1: a clean tree holding only that leaf, there.
  const v = valid[0];
  const j = 1;
  const bits = v.pathIndices.map((_, d) => String((j >> d) & 1));
  const cleanSiblings = [];
  let node = poseidon([v.k, v.r, v.denomTag]);
  let zero = 0n;
  for (const bit of bits) {
    cleanSiblings.push(zero.toString());
    node = bit === "1" ? poseidon([zero, node]) : poseidon([node, zero]);
    zero = poseidon([zero, zero]);
  }
  const moved = { cleanSiblings, cleanRoot: node.toString() };
  // The clean path is valid at index 1: with the pool tree moved too, it proves.
  assert.ok(await witnessOk({ ...v, ...moved, pathIndices: bits, poolSiblings: cleanSiblings, poolRoot: moved.cleanRoot }));
  await rejectedAt({ ...v, ...moved }, "cleanRoot");
});

test("a path index that is not a bit is rejected", async () => {
  const pathIndices = [...valid[0].pathIndices];
  pathIndices[0] = "2";
  await rejectedAt({ ...valid[0], pathIndices }, "bit");
});

test("a wrong denomination tag is rejected", async () => {
  const input = { ...valid[0], denomTag: (BigInt(valid[0].denomTag) + 1n).toString() };
  await rejectedAt(input, "poolRoot");
});

test("Groth16: proofs verify, public signals come out in the specified order", { skip: !existsSync(ZKEY) && "run npm run setup:dev first" }, async () => {
  const vk = JSON.parse(readFileSync(VK, "utf8"));
  assert.equal(vk.nPublic, 4);
  for (const input of valid) {
    const { proof, publicSignals } = await snarkjs.groth16.fullProve(input, WASM, ZKEY);
    assert.deepEqual(publicSignals, PUBLIC_ORDER.map((k) => input[k]));
    assert.ok(await snarkjs.groth16.verify(vk, publicSignals, proof));
  }
});

test("Groth16: changing any public input after proving breaks verification", { skip: !existsSync(ZKEY) && "run npm run setup:dev first" }, async () => {
  const vk = JSON.parse(readFileSync(VK, "utf8"));
  const { proof, publicSignals } = await snarkjs.groth16.fullProve(valid[0], WASM, ZKEY);
  for (let i = 0; i < 4; i++) {
    const tampered = [...publicSignals];
    tampered[i] = ((BigInt(tampered[i]) + 1n) % P).toString();
    assert.equal(await snarkjs.groth16.verify(vk, tampered, proof), false, PUBLIC_ORDER[i]);
  }
  // A relayer swapping in another action's binding is the case that matters most.
  const otherBinding = [...publicSignals];
  otherBinding[3] = "12345";
  assert.equal(await snarkjs.groth16.verify(vk, otherBinding, proof), false);
});
