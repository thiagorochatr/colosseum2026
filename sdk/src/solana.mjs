// Solana-side helpers: mirror-pool's action binding and the proof in
// groth16-solana's layout. See docs/SPEC.md section 6.

import { keccak256, concatBytes, hexToBytes, bytesToHex } from "viem";

export const ACTION_DOMAIN = new TextEncoder().encode("mirror-pool:action:v2");
/** The BN254 base field modulus: negating a G1 point maps y to Q - y. */
export const Q = 21888242871839275222246405745257275088696311157297823662689037894645226208583n;
const MASK_248 = (1n << 248n) - 1n;

const le = (v, bytes) => {
  const out = new Uint8Array(bytes);
  let x = BigInt(v);
  for (let i = 0; i < bytes; i++) {
    out[i] = Number(x & 0xffn);
    x >>= 8n;
  }
  return out;
};
const be32 = (v) => hexToBytes(`0x${BigInt(v).toString(16).padStart(64, "0")}`);
const bytes32 = (b) => {
  const u = typeof b === "string" ? hexToBytes(b.startsWith("0x") ? b : `0x${b}`) : Uint8Array.from(b);
  if (u.length !== 32) throw new Error("expected 32 bytes");
  return u;
};

/**
 * mirror-core's action_binding: keccak over the domain and the action's fields,
 * with its first byte cleared. Pubkeys are 32 raw bytes.
 */
export function actionBinding({ selector = 0n, targetProgram, beneficiary, relay, relayFee, actionAccounts = 0, payload = new Uint8Array() }) {
  const d = keccak256(
    concatBytes([
      ACTION_DOMAIN,
      le(selector, 8),
      bytes32(targetProgram ?? new Uint8Array(32)),
      bytes32(beneficiary),
      bytes32(relay),
      le(relayFee, 8),
      Uint8Array.of(actionAccounts),
      Uint8Array.from(payload),
    ]),
  );
  return BigInt(d) & MASK_248;
}

/** A snarkjs proof in groth16-solana's layout: A negated, G2 as c1 before c0, big-endian. */
export function toSolana(proof) {
  const [ax, ay] = [BigInt(proof.pi_a[0]), BigInt(proof.pi_a[1])];
  const a = concatBytes([be32(ax), be32((Q - ay) % Q)]);
  const [[x0, x1], [y0, y1]] = proof.pi_b;
  const b = concatBytes([be32(x1), be32(x0), be32(y1), be32(y0)]);
  const c = concatBytes([be32(proof.pi_c[0]), be32(proof.pi_c[1])]);
  return { a, b, c };
}

export const fieldBytes = be32;
export { bytesToHex };
