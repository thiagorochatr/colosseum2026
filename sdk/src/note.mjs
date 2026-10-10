// A note: the secret that releases one deposit. See docs/SPEC.md section 3.
//
// Works in Node and in the browser: randomness comes from crypto.getRandomValues.
// Whoever holds the note file can withdraw the deposit, so it is stored only
// where its owner keeps it.

import { FIELD, hasher } from "./tree.mjs";

export const NOTE_VERSION = 1;

/** A uniformly random field element, by rejection sampling over 254 bits. */
export function randomField() {
  for (;;) {
    const b = new Uint8Array(32);
    globalThis.crypto.getRandomValues(b);
    b[0] &= 0x3f; // 254 bits
    let x = 0n;
    for (const v of b) x = (x << 8n) | BigInt(v);
    if (x < FIELD) return x;
  }
}

/**
 * @param {object} p
 * @param {"tempo"|"solana"} p.chain
 * @param {string} p.network   e.g. "moderato", "devnet"
 * @param {string} p.pool      the pool's address
 * @param {bigint} p.denomination  in the pool's base units; also the note's denomTag
 */
export function createNote({ chain, network, pool, denomination }) {
  return { version: NOTE_VERSION, chain, network, pool, denomination: BigInt(denomination), k: randomField(), r: randomField() };
}

export async function commitmentOf(note) {
  const { h3 } = await hasher();
  return h3(note.k, note.r, BigInt(note.denomination));
}

export async function nullifierOf(note) {
  const { h1 } = await hasher();
  return h1(note.k);
}

/** The circuit's view of a note. */
export const circuitNote = (note) => ({ k: BigInt(note.k), r: BigInt(note.r), denomTag: BigInt(note.denomination) });

export function serializeNote(note) {
  return JSON.stringify(note, (_, v) => (typeof v === "bigint" ? v.toString() : v), 2);
}

export function parseNote(text) {
  const n = JSON.parse(text);
  if (n.version !== NOTE_VERSION) throw new Error(`unsupported note version ${n.version}`);
  for (const f of ["chain", "network", "pool", "denomination", "k", "r"]) {
    if (n[f] === undefined) throw new Error(`note is missing ${f}`);
  }
  return { ...n, denomination: BigInt(n.denomination), k: BigInt(n.k), r: BigInt(n.r) };
}
