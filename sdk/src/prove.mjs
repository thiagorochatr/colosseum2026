// Builds the withdraw circuit's input from a note and the two trees, and proves it.

import * as snarkjs from "snarkjs";
import { MerkleTree, cleanTree, hasher, pathIndices } from "./tree.mjs";

/**
 * @param {object} p
 * @param {{k: bigint, r: bigint, denomTag: bigint}} p.note
 * @param {number} p.index        the note's leaf index in the pool
 * @param {bigint[]} p.poolLeaves every pool leaf, in deposit order
 * @param {number[]} p.excluded   indices outside the clean set
 * @param {bigint} p.binding      the chain's action binding, already in the field
 */
export async function witnessInput({ note, index, poolLeaves, excluded, binding }) {
  const { h1, h3 } = await hasher();
  const leaf = h3(note.k, note.r, note.denomTag);
  if (BigInt(poolLeaves[index]) !== leaf) throw new Error(`leaf ${index} is not this note's commitment`);
  const pool = await MerkleTree.from(poolLeaves);
  const clean = await cleanTree(poolLeaves, excluded);
  const s = (x) => BigInt(x).toString();
  return {
    poolRoot: s(pool.root()),
    cleanRoot: s(clean.root()),
    nullifierHash: s(h1(note.k)),
    binding: s(binding),
    k: s(note.k),
    r: s(note.r),
    denomTag: s(note.denomTag),
    pathIndices: pathIndices(index).map(String),
    poolSiblings: pool.siblings(index).map(s),
    cleanSiblings: clean.siblings(index).map(s),
  };
}

/** Proves, or throws if the note is not provable against these trees. */
export async function prove(args, { wasm, zkey }) {
  const input = await witnessInput(args);
  const { proof, publicSignals } = await snarkjs.groth16.fullProve(input, wasm, zkey);
  return { input, proof, publicSignals };
}

/** The proof as the Solidity verifier takes it, with G2 coordinates swapped. */
export function toSolidity(proof) {
  const b = (x) => BigInt(x);
  return {
    a: [b(proof.pi_a[0]), b(proof.pi_a[1])],
    b: [
      [b(proof.pi_b[0][1]), b(proof.pi_b[0][0])],
      [b(proof.pi_b[1][1]), b(proof.pi_b[1][0])],
    ],
    c: [b(proof.pi_c[0]), b(proof.pi_c[1])],
  };
}

export async function terminate() {
  await globalThis.curve_bn128?.terminate();
}
