// The Solana client: configuration, PDAs, instruction builders, and the pool's
// leaves rebuilt from its history. See docs/SPEC.md section 8.
//
//   NETWORK=local   a solana-test-validator on http://127.0.0.1:8899
//   NETWORK=devnet  Solana devnet
//
// Program ids and the curator come from deployments/<network>.json.

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  sendAndConfirmTransaction,
} from "@solana/web3.js";
import { MerkleTree, hasher } from "../../sdk/src/tree.mjs";

export const ROOT = new URL("..", import.meta.url).pathname;
export const NETWORK = process.env.NETWORK ?? "local";
export const RPC = process.env.RPC ?? (NETWORK === "devnet" ? "https://api.devnet.solana.com" : "http://127.0.0.1:8899");
export const connection = new Connection(RPC, "confirmed");
export const explorer = (sig) =>
  NETWORK === "devnet" ? `https://explorer.solana.com/tx/${sig}?cluster=devnet` : sig;

const json = (path) => JSON.parse(readFileSync(path, "utf8"));
export const readKeypair = (path) => Keypair.fromSecretKey(Uint8Array.from(json(path)));
export const wallet = () => readKeypair(process.env.WALLET ?? `${homedir()}/.config/solana/id.json`);

export const deploymentFile = (kind = "") => `${ROOT}deployments/${NETWORK}${kind ? `-${kind}` : ""}.json`;
export const readDeployment = (kind) => json(deploymentFile(kind));
export const writeDeployment = (kind, v) =>
  writeFileSync(deploymentFile(kind), JSON.stringify(v, (_, x) => (typeof x === "bigint" ? x.toString() : x), 2) + "\n");

/** Demo keypairs for a test network, generated once into .demo/ (gitignored). */
export function demoKeypair(name) {
  const dir = `${ROOT}.demo/${NETWORK}`;
  const file = `${dir}/${name}.json`;
  if (!existsSync(file)) {
    mkdirSync(dir, { recursive: true });
    writeFileSync(file, JSON.stringify([...Keypair.generate().secretKey]), { mode: 0o600 });
  }
  return readKeypair(file);
}

// ------------------------------------------------------------------- encoding

const u64 = (v) => {
  const b = Buffer.alloc(8);
  b.writeBigUInt64LE(BigInt(v));
  return b;
};
const u32 = (v) => {
  const b = Buffer.alloc(4);
  b.writeUInt32LE(Number(v));
  return b;
};
export const be32 = (v) => Buffer.from(BigInt(v).toString(16).padStart(64, "0"), "hex");
const readBe = (buf) => BigInt(`0x${Buffer.from(buf).toString("hex")}`);

// ----------------------------------------------------------------------- PDAs

const pda = (seeds, program) => PublicKey.findProgramAddressSync(seeds, program)[0];
export const poolAddress = (program, denomination) => pda([Buffer.from("pool"), u64(denomination)], program);
export const vaultAddress = (program, pool) => pda([Buffer.from("vault"), pool.toBuffer()], program);
export const spendAddress = (program, pool, nullifier) => pda([Buffer.from("spend"), pool.toBuffer(), nullifier], program);
export const curatorAddress = (compliance, authority) => pda([Buffer.from("curator"), authority.toBuffer()], compliance);
export const clearanceAddress = (compliance, pool, nullifier) =>
  pda([Buffer.from("clearance"), pool.toBuffer(), nullifier], compliance);

// --------------------------------------------------------------- instructions

const ix = (programId, keys, data) => new TransactionInstruction({ programId, keys, data: Buffer.concat(data) });
const w = (pubkey, isSigner = false) => ({ pubkey, isSigner, isWritable: true });
const r = (pubkey, isSigner = false) => ({ pubkey, isSigner, isWritable: false });

export const ixs = {
  /** mirror-pool InitPool: denomination, entry fee 0, k_floor, settle timeout (0 = default). */
  initPool: (program, payer, denomination, kFloor = 2, settleTimeout = 0) => {
    const pool = poolAddress(program, denomination);
    return ix(program, [w(payer, true), w(pool), w(vaultAddress(program, pool)), r(SystemProgram.programId)], [
      Buffer.of(0), u64(denomination), u64(0), u32(kFloor), u32(settleTimeout),
    ]);
  },
  deposit: (program, depositor, pool, commitment) =>
    ix(program, [w(depositor, true), w(pool), w(vaultAddress(program, pool)), r(SystemProgram.programId)], [
      Buffer.of(1), be32(commitment),
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
      [Buffer.of(3, spends.length, allowBelowFloor ? 1 : 0)],
    ),
  initCurator: (compliance, authority, policyHash) =>
    ix(compliance, [w(authority, true), w(curatorAddress(compliance, authority)), r(SystemProgram.programId)], [
      Buffer.of(1), policyHash,
    ]),
  publishRoot: (compliance, authority, root) =>
    ix(compliance, [r(authority, true), w(curatorAddress(compliance, authority))], [Buffer.of(2), be32(root)]),
  revoke: (compliance, authority) => ix(compliance, [r(authority, true), w(curatorAddress(compliance, authority))], [Buffer.of(3)]),
  /** proof: { a, b, c } in groth16-solana layout; inputs: [poolRoot, cleanRoot, nullifierHash, binding] */
  verifyClearance: (compliance, payer, curatorAuthority, pool, proof, inputs) =>
    ix(
      compliance,
      [
        w(payer, true),
        r(curatorAddress(compliance, curatorAuthority)),
        r(pool),
        w(clearanceAddress(compliance, pool, be32(inputs[2]))),
        r(SystemProgram.programId),
      ],
      [Buffer.of(4), Buffer.from(proof.a), Buffer.from(proof.b), Buffer.from(proof.c), ...inputs.map(be32)],
    ),
  /** A transfer action: selector 0, no target, no payload. */
  submitClearedSpend: (program, compliance, relay, pool, nullifier, beneficiary, relayFee) =>
    ix(
      program,
      [
        w(relay, true),
        w(pool),
        w(spendAddress(program, pool, nullifier)),
        r(SystemProgram.programId),
        r(clearanceAddress(compliance, pool, nullifier)),
      ],
      [Buffer.of(4), nullifier, u64(0), Buffer.alloc(32), beneficiary.toBuffer(), u64(relayFee), Buffer.of(0), Buffer.alloc(2)],
    ),
};

export async function send(instructions, signers) {
  const tx = new Transaction().add(...instructions);
  return sendAndConfirmTransaction(connection, tx, signers, { commitment: "confirmed" });
}

// ------------------------------------------------------------------ pool sync

/** The pool account's deposit count and current root. */
export async function poolState(pool) {
  const info = await connection.getAccountInfo(pool, "confirmed");
  if (!info) throw new Error(`no pool at ${pool.toBase58()}`);
  const d = info.data;
  const depositCount = d.readBigUInt64LE(24);
  const rootPos = d.readUInt32LE(48);
  const currentRoot = readBe(d.subarray(696 + 32 * rootPos, 696 + 32 * rootPos + 32));
  return { depositCount, currentRoot };
}

/**
 * The pool's leaves and depositors, in deposit order, rebuilt from its
 * transaction history. Checked against the pool: the count must match and the
 * rebuilt root must be the pool's current root.
 */
export async function syncPool(program, pool) {
  const sigs = [];
  let before;
  for (;;) {
    const page = await connection.getSignaturesForAddress(pool, { before, limit: 1000 }, "confirmed");
    sigs.push(...page);
    if (page.length < 1000) break;
    before = page[page.length - 1].signature;
  }
  sigs.reverse(); // oldest first
  const deposits = [];
  for (const { signature, err } of sigs) {
    if (err) continue;
    const tx = await connection.getTransaction(signature, { commitment: "confirmed", maxSupportedTransactionVersion: 0 });
    const keys = tx.transaction.message.staticAccountKeys ?? tx.transaction.message.accountKeys;
    for (const ci of tx.transaction.message.compiledInstructions ?? tx.transaction.message.instructions) {
      if (!keys[ci.programIdIndex].equals(program)) continue;
      const data = Buffer.from(ci.data);
      const accounts = ci.accountKeyIndexes ?? ci.accounts;
      if (data[0] === 1 && data.length === 33 && keys[accounts[1]].equals(pool)) {
        deposits.push({ commitment: readBe(data.subarray(1, 33)), depositor: keys[accounts[0]].toBase58(), signature });
      }
    }
  }
  const state = await poolState(pool);
  if (BigInt(deposits.length) !== state.depositCount) {
    throw new Error(`history has ${deposits.length} deposits, the pool counts ${state.depositCount}`);
  }
  const tree = await MerkleTree.from(deposits.map((d) => d.commitment));
  if (tree.root() !== state.currentRoot) throw new Error("the rebuilt pool root is not the pool's current root");
  return { leaves: deposits.map((d) => d.commitment), depositors: deposits.map((d) => d.depositor), root: state.currentRoot };
}

/** sha256 of a policy document, as published in the curator account. */
export const policyHash = (doc) => createHash("sha256").update(JSON.stringify(doc)).digest();

export { hasher, PublicKey, Keypair, SystemProgram };
