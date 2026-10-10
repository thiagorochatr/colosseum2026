// The SDK against mirror-core's vectors and snarkjs's own calldata exporter.

import { test, after } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import * as snarkjs from "snarkjs";
import { MerkleTree, cleanTree, hasher, zeroLadder } from "../src/tree.mjs";
import { witnessInput, toSolidity, terminate } from "../src/prove.mjs";

const root = new URL("../..", import.meta.url).pathname;
const vectors = JSON.parse(readFileSync(`${root}circuits/test/vectors.json`, "utf8"));
const proofs = JSON.parse(readFileSync(`${root}solana/crates/snarkjs-bridge/tests/fixtures/proofs.json`, "utf8"));
const B = (x) => BigInt(x);

after(terminate);

// All six notes: the valid ones and the excluded ones, by index.
const notes = new Map();
for (const v of [...vectors.valid, ...vectors.rejected.excluded_note]) {
  notes.set(v._index, { k: B(v.k), r: B(v.r), denomTag: B(v.denomTag) });
}

async function poolLeaves() {
  const { h3 } = await hasher();
  return [...Array(vectors.deposits).keys()].map((i) => {
    const n = notes.get(i);
    return h3(n.k, n.r, n.denomTag);
  });
}

test("the zero ladder is mirror-core's", async () => {
  const z = await zeroLadder();
  assert.equal(z[1], 0x2098f5fb9e239eab3ceac3f27b81e481dc3124d55ffed523a839ee8446b64864n);
  assert.equal(z[20], 0x2134e76ac5d21aab186c2be1dd8f84ee880a1e46eaf712f9d371b6df22191f3en);
});

test("trees rebuilt from the notes match mirror-core's roots and paths", async () => {
  const leaves = await poolLeaves();
  const pool = await MerkleTree.from(leaves);
  const clean = await cleanTree(leaves, vectors.excluded);
  for (const v of vectors.valid) {
    assert.equal(pool.root(), B(v.poolRoot));
    assert.equal(clean.root(), B(v.cleanRoot));
    assert.deepEqual(pool.siblings(v._index), v.poolSiblings.map(B));
    assert.deepEqual(clean.siblings(v._index), v.cleanSiblings.map(B));
  }
});

test("the circuit input is exactly the vector's", async () => {
  const leaves = await poolLeaves();
  for (const v of vectors.valid) {
    const input = await witnessInput({
      note: notes.get(v._index),
      index: v._index,
      poolLeaves: leaves,
      excluded: vectors.excluded,
      binding: B(v.binding),
    });
    for (const key of ["poolRoot", "cleanRoot", "nullifierHash", "binding", "k", "r", "denomTag"]) {
      assert.equal(B(input[key]), B(v[key]), key);
    }
    assert.deepEqual(input.pathIndices, v.pathIndices);
    assert.deepEqual(input.poolSiblings.map(B), v.poolSiblings.map(B));
    assert.deepEqual(input.cleanSiblings.map(B), v.cleanSiblings.map(B));
  }
});

test("a note that is not the leaf at its index is refused", async () => {
  const leaves = await poolLeaves();
  await assert.rejects(
    witnessInput({ note: notes.get(0), index: 2, poolLeaves: leaves, excluded: [], binding: 1n }),
    /is not this note's commitment/,
  );
});

test("the Solidity layout matches snarkjs's own exporter", async () => {
  for (const { proof, publicSignals } of proofs.proofs) {
    const [a, b, c] = JSON.parse(`[${await snarkjs.groth16.exportSolidityCallData(proof, publicSignals)}]`);
    const mine = toSolidity(proof);
    assert.deepEqual(mine.a, a.map(B));
    assert.deepEqual(mine.b, b.map((row) => row.map(B)));
    assert.deepEqual(mine.c, c.map(B));
  }
});
