// The depth-20 Poseidon Merkle tree of docs/SPEC.md section 4, in JavaScript.
//
// Same hash, zero ladder and path convention as mirror-core: leaf index bit j
// is 0 when the running node is a left child. The clean tree is a mask over
// the pool tree: the same leaves at the same indices, with non-clean ones zero.

import { buildPoseidon } from "circomlibjs";

export const DEPTH = 20;
export const FIELD = 21888242871839275222246405745257275088548364400416034343698204186575808495617n;

let poseidonPromise;
async function poseidon() {
  poseidonPromise ??= buildPoseidon();
  return poseidonPromise;
}

/** H1, H2 and H3 over BigInts. */
export async function hasher() {
  const P = await poseidon();
  const h = (...xs) => P.F.toObject(P(xs));
  return { h1: (a) => h(a), h2: (a, b) => h(a, b), h3: (a, b, c) => h(a, b, c) };
}

export async function zeroLadder() {
  const { h2 } = await hasher();
  const z = [0n];
  for (let i = 0; i < DEPTH; i++) z.push(h2(z[i], z[i]));
  return z;
}

export class MerkleTree {
  /** @param {bigint[]} leaves */
  static async from(leaves) {
    const t = new MerkleTree();
    t.h = await hasher();
    t.zeros = await zeroLadder();
    t.leaves = leaves.map(BigInt);
    return t;
  }

  /** The node at `level`, `index`. Built on demand, memoised per call. */
  node(level, index, memo = new Map()) {
    if (level === 0) return index < this.leaves.length ? this.leaves[index] : 0n;
    if (index * 2 ** level >= this.leaves.length) return this.zeros[level];
    const key = level * 2 ** DEPTH + index;
    if (memo.has(key)) return memo.get(key);
    const v = this.h.h2(this.node(level - 1, 2 * index, memo), this.node(level - 1, 2 * index + 1, memo));
    memo.set(key, v);
    return v;
  }

  root() {
    return this.node(DEPTH, 0);
  }

  /** Siblings from the leaf up, as mirror-core's MerkleProof. */
  siblings(index) {
    const memo = new Map();
    const out = [];
    let i = index;
    for (let level = 0; level < DEPTH; level++) {
      out.push(this.node(level, i ^ 1, memo));
      i >>= 1;
    }
    return out;
  }
}

/** The clean tree: the pool's leaves with every excluded index set to zero. */
export async function cleanTree(poolLeaves, excluded) {
  const ex = new Set([...excluded].map(Number));
  return MerkleTree.from(poolLeaves.map((leaf, i) => (ex.has(i) ? 0n : BigInt(leaf))));
}

export function pathIndices(index) {
  return Array.from({ length: DEPTH }, (_, j) => (index >> j) & 1);
}
