// Solana-side helpers: mirror-pool's action binding and the proof in
// groth16-solana's layout. See docs/SPEC.md section 6.

import { keccak256, concatBytes, hexToBytes, bytesToHex, sha256 } from "viem";
import { PublicKey, SystemProgram, TransactionInstruction } from "@solana/web3.js";
import { MerkleTree } from "./tree.mjs";

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

// ------------------------------------------------------------------- encoding

const u64le = (v) => le(v, 8);
const u32le = (v) => le(v, 4);
const readBe = (u) => BigInt(bytesToHex(Uint8Array.from(u)));
const readU64 = (u, at) => {
  let x = 0n;
  for (let i = 7; i >= 0; i--) x = (x << 8n) | BigInt(u[at + i]);
  return x;
};
const readU32 = (u, at) => u[at] | (u[at + 1] << 8) | (u[at + 2] << 16) | (u[at + 3] * 2 ** 24);
const text = (s) => new TextEncoder().encode(s);

// ----------------------------------------------------------------------- PDAs

const pda = (seeds, program) => PublicKey.findProgramAddressSync(seeds, program)[0];
export const poolAddress = (program, denomination) => pda([text("pool"), u64le(denomination)], program);
export const vaultAddress = (program, pool) => pda([text("vault"), pool.toBytes()], program);
export const spendAddress = (program, pool, nullifier) => pda([text("spend"), pool.toBytes(), bytes32(nullifier)], program);
export const curatorAddress = (compliance, authority) => pda([text("curator"), authority.toBytes()], compliance);
export const clearanceAddress = (compliance, pool, nullifier) =>
  pda([text("clearance"), pool.toBytes(), bytes32(nullifier)], compliance);

// --------------------------------------------------------------- instructions

const ix = (programId, keys, parts) => new TransactionInstruction({ programId, keys, data: Buffer.from(concatBytes(parts)) });
const w = (pubkey, isSigner = false) => ({ pubkey, isSigner, isWritable: true });
const ro = (pubkey, isSigner = false) => ({ pubkey, isSigner, isWritable: false });

/** Instruction builders for the compliant pool and the compliance program. */
export const instructions = {
  initPool: (program, payer, denomination, kFloor = 2, settleTimeout = 0) => {
    const pool = poolAddress(program, denomination);
    return ix(program, [w(payer, true), w(pool), w(vaultAddress(program, pool)), ro(SystemProgram.programId)], [
      Uint8Array.of(0), u64le(denomination), u64le(0), u32le(kFloor), u32le(settleTimeout),
    ]);
  },
  deposit: (program, depositor, pool, commitment) =>
    ix(program, [w(depositor, true), w(pool), w(vaultAddress(program, pool)), ro(SystemProgram.programId)], [
      Uint8Array.of(1), be32(commitment),
    ]),
  /** spends: [{ nullifier, beneficiary, relay }] */
  settle: (program, settler, pool, spends, allowBelowFloor = false) =>
    ix(
      program,
      [
        w(settler, true),
        w(pool),
        w(vaultAddress(program, pool)),
        ...spends.flatMap((s) => [w(spendAddress(program, pool, s.nullifier)), w(s.beneficiary), w(s.relay)]),
      ],
      [Uint8Array.of(3, spends.length, allowBelowFloor ? 1 : 0)],
    ),
  initCurator: (compliance, authority, policyHash) =>
    ix(compliance, [w(authority, true), w(curatorAddress(compliance, authority)), ro(SystemProgram.programId)], [
      Uint8Array.of(1), bytes32(policyHash),
    ]),
  publishRoot: (compliance, authority, root) =>
    ix(compliance, [ro(authority, true), w(curatorAddress(compliance, authority))], [Uint8Array.of(2), be32(root)]),
  revoke: (compliance, authority) =>
    ix(compliance, [ro(authority, true), w(curatorAddress(compliance, authority))], [Uint8Array.of(3)]),
  /** proof in groth16-solana layout; inputs: [poolRoot, cleanRoot, nullifierHash, binding] */
  verifyClearance: (compliance, payer, curatorAuthority, pool, proof, inputs) =>
    ix(
      compliance,
      [
        w(payer, true),
        ro(curatorAddress(compliance, curatorAuthority)),
        ro(pool),
        w(clearanceAddress(compliance, pool, be32(inputs[2]))),
        ro(SystemProgram.programId),
      ],
      [Uint8Array.of(4), proof.a, proof.b, proof.c, ...inputs.map(be32)],
    ),
  /** A transfer action: selector 0, no target program, no payload. */
  submitClearedSpend: (program, compliance, relay, pool, nullifier, beneficiary, relayFee) =>
    ix(
      program,
      [
        w(relay, true),
        w(pool),
        w(spendAddress(program, pool, nullifier)),
        ro(SystemProgram.programId),
        ro(clearanceAddress(compliance, pool, nullifier)),
      ],
      [Uint8Array.of(4), bytes32(nullifier), u64le(0), new Uint8Array(32), beneficiary.toBytes(), u64le(relayFee), Uint8Array.of(0), new Uint8Array(2)],
    ),
};

/** The policy document's hash, as stored in a curator account. */
export const policyHash = (doc) => hexToBytes(sha256(new TextEncoder().encode(JSON.stringify(doc))));

// ------------------------------------------------------------------ pool sync

/** The pool account's deposit count and current root. */
export async function poolState(connection, pool) {
  const info = await connection.getAccountInfo(pool, "confirmed");
  if (!info) throw new Error(`no pool at ${pool.toBase58()}`);
  const d = Uint8Array.from(info.data);
  const rootPos = readU32(d, 48);
  return { depositCount: readU64(d, 24), spendCount: readU64(d, 32), currentRoot: readBe(d.subarray(696 + 32 * rootPos, 728 + 32 * rootPos)) };
}

/**
 * The pool's leaves and depositors in deposit order, rebuilt from its history.
 * The count must match the pool's and the rebuilt root must be its current root.
 */
export async function syncPool(connection, program, pool) {
  const sigs = [];
  let before;
  for (;;) {
    const page = await connection.getSignaturesForAddress(pool, { before, limit: 1000 }, "confirmed");
    sigs.push(...page);
    if (page.length < 1000) break;
    before = page[page.length - 1].signature;
  }
  sigs.reverse();
  const deposits = [];
  for (const { signature, err } of sigs) {
    if (err) continue;
    const tx = await connection.getTransaction(signature, { commitment: "confirmed", maxSupportedTransactionVersion: 0 });
    const msg = tx.transaction.message;
    const keys = msg.staticAccountKeys ?? msg.accountKeys;
    for (const ci of msg.compiledInstructions ?? msg.instructions) {
      if (!keys[ci.programIdIndex].equals(program)) continue;
      const data = Uint8Array.from(ci.data);
      const accounts = ci.accountKeyIndexes ?? ci.accounts;
      if (data[0] === 1 && data.length === 33 && keys[accounts[1]].equals(pool)) {
        deposits.push({ commitment: readBe(data.subarray(1, 33)), depositor: keys[accounts[0]].toBase58(), signature });
      }
    }
  }
  const state = await poolState(connection, pool);
  if (BigInt(deposits.length) !== state.depositCount) {
    throw new Error(`history has ${deposits.length} deposits, the pool counts ${state.depositCount}`);
  }
  const tree = await MerkleTree.from(deposits.map((x) => x.commitment));
  if (tree.root() !== state.currentRoot) throw new Error("the rebuilt pool root is not the pool's current root");
  return { leaves: deposits.map((x) => x.commitment), depositors: deposits.map((x) => x.depositor), deposits, root: state.currentRoot };
}
