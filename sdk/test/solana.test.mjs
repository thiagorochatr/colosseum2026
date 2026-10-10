// The Solana helpers against values Rust produced: mirror-core's binding and
// snarkjs-bridge's conversion.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { bytesToHex } from "viem";
import { actionBinding, toSolana } from "../src/solana.mjs";

const root = new URL("../..", import.meta.url).pathname;
const vectors = JSON.parse(readFileSync(`${root}circuits/test/vectors.json`, "utf8"));
const snark = JSON.parse(readFileSync(`${root}solana/crates/snarkjs-bridge/tests/fixtures/proofs.json`, "utf8"));
const onchain = JSON.parse(readFileSync(`${root}solana/programs/compliance/tests/fixtures/proofs.json`, "utf8"));

test("the binding is mirror-core's", () => {
  // tools/vectors: action_binding(0, [0; 32], [7; 32], [9; 32], 5000, 0, [])
  const b = actionBinding({
    selector: 0n,
    targetProgram: new Uint8Array(32),
    beneficiary: new Uint8Array(32).fill(7),
    relay: new Uint8Array(32).fill(9),
    relayFee: 5000n,
  });
  assert.equal(b, BigInt(vectors.valid[0].binding));
});

test("the proof layout is snarkjs-bridge's", () => {
  for (let i = 0; i < snark.proofs.length; i++) {
    const s = toSolana(snark.proofs[i].proof);
    const want = onchain.proofs.find((p) => p.index === snark.proofs[i].index);
    assert.equal(bytesToHex(s.a).slice(2), want.a);
    assert.equal(bytesToHex(s.b).slice(2), want.b);
    assert.equal(bytesToHex(s.c).slice(2), want.c);
  }
});
