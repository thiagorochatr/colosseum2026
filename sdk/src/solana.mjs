// Solana-side helpers: mirror-pool's action binding and the proof in
// groth16-solana's layout. See docs/SPEC.md section 6.

import { keccak256, concatBytes, hexToBytes, bytesToHex, sha256 } from "viem";
import { PublicKey, SystemProgram, TransactionInstruction } from "@solana/web3.js";
import { MerkleTree, cleanTree } from "./tree.mjs";

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
/** One clearance per pool, curator and nullifier. */
export const clearanceAddress = (compliance, pool, curatorAuthority, nullifier) =>
  pda([text("clearance"), pool.toBytes(), curatorAuthority.toBytes(), bytes32(nullifier)], compliance);

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
  updatePolicy: (compliance, authority, policyHash) =>
    ix(compliance, [ro(authority, true), w(curatorAddress(compliance, authority))], [Uint8Array.of(5), bytes32(policyHash)]),
  /** proof in groth16-solana layout; inputs: [poolRoot, cleanRoot, nullifierHash, binding] */
  verifyClearance: (compliance, payer, curatorAuthority, pool, proof, inputs) =>
    ix(
      compliance,
      [
        w(payer, true),
        ro(curatorAddress(compliance, curatorAuthority)),
        ro(pool),
        w(clearanceAddress(compliance, pool, curatorAuthority, be32(inputs[2]))),
        ro(SystemProgram.programId),
      ],
      [Uint8Array.of(4), proof.a, proof.b, proof.c, ...inputs.map(be32)],
    ),
  /** A transfer action: selector 0, no target program, no payload. */
  submitClearedSpend: (program, compliance, curatorAuthority, relay, pool, nullifier, beneficiary, relayFee) =>
    ix(
      program,
      [
        w(relay, true),
        w(pool),
        w(spendAddress(program, pool, nullifier)),
        ro(SystemProgram.programId),
        ro(clearanceAddress(compliance, pool, curatorAuthority, nullifier)),
      ],
      [Uint8Array.of(4), bytes32(nullifier), u64le(0), new Uint8Array(32), beneficiary.toBytes(), u64le(relayFee), Uint8Array.of(0), new Uint8Array(2)],
    ),
};

/** The policy document's hash, as stored in a curator account. */
export const policyHash = (doc) => hexToBytes(sha256(new TextEncoder().encode(JSON.stringify(doc))));

// ------------------------------------------------------------- careful reads
//
// Public RPC endpoints rate-limit hard. Transactions never change once
// confirmed, so each is fetched once and cached, sequentially, with backoff.

const txCache = new Map();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const limited = (e) => /429|Too many requests|rate limit/i.test(String(e?.message ?? e));

export async function withBackoff(fn, attempts = 7) {
  for (let i = 0; ; i++) {
    try {
      return await fn();
    } catch (e) {
      if (!limited(e) || i >= attempts - 1) throw e;
      await sleep(500 * 2 ** i);
    }
  }
}

async function fetchTx(connection, signature) {
  if (txCache.has(signature)) return txCache.get(signature);
  const tx = await withBackoff(() => connection.getTransaction(signature, { commitment: "confirmed", maxSupportedTransactionVersion: 0 }));
  if (tx) txCache.set(signature, tx);
  return tx;
}

// ------------------------------------------------------------------ pool sync

/** The pool account's deposit count and current root. */
export async function poolState(connection, pool) {
  const info = await withBackoff(() => connection.getAccountInfo(pool, "confirmed"));
  if (!info) throw new Error(`no pool at ${pool.toBase58()}`);
  const d = Uint8Array.from(info.data);
  const rootPos = readU32(d, 48);
  return { depositCount: readU64(d, 24), spendCount: readU64(d, 32), currentRoot: readBe(d.subarray(696 + 32 * rootPos, 728 + 32 * rootPos)) };
}

const BASE58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/** Base58 to bytes: inner instructions come back from RPC with base58 data. */
function fromBase58(s) {
  let n = 0n;
  for (const ch of s) {
    const v = BASE58.indexOf(ch);
    if (v < 0) throw new Error("not base58");
    n = n * 58n + BigInt(v);
  }
  const hex = n === 0n ? "" : n.toString(16);
  const body = hexToBytes(`0x${hex.length % 2 ? "0" : ""}${hex}`);
  let zeros = 0;
  while (s[zeros] === "1") zeros++;
  return Uint8Array.from([...new Uint8Array(zeros), ...(n === 0n ? [] : body)]);
}

/**
 * Every instruction of a confirmed transaction in execution order, each with
 * its program and account keys resolved: top-level ones and, right after each,
 * the instructions it invoked. Keys loaded from lookup tables are included, so
 * a deposit made through another program or a v0 transaction is still seen.
 */
export function executedInstructions(tx) {
  const msg = tx.transaction.message;
  const loaded = tx.meta?.loadedAddresses;
  const keys = [...(msg.staticAccountKeys ?? msg.accountKeys), ...(loaded?.writable ?? []), ...(loaded?.readonly ?? [])].map((k) =>
    typeof k === "string" ? new PublicKey(k) : k,
  );
  const resolve = (ci, data) => ({
    program: keys[ci.programIdIndex],
    accounts: (ci.accountKeyIndexes ?? ci.accounts).map((i) => keys[i]),
    data,
  });
  const inner = new Map((tx.meta?.innerInstructions ?? []).map((x) => [x.index, x.instructions]));
  const out = [];
  (msg.compiledInstructions ?? msg.instructions).forEach((ci, i) => {
    out.push(resolve(ci, typeof ci.data === "string" ? fromBase58(ci.data) : Uint8Array.from(ci.data)));
    for (const x of inner.get(i) ?? []) out.push(resolve(x, typeof x.data === "string" ? fromBase58(x.data) : Uint8Array.from(x.data)));
  });
  return out;
}

/**
 * The pool's leaves and depositors in deposit order, rebuilt from its history.
 * The count must match the pool's and the rebuilt root must be its current root.
 */
export async function syncPool(connection, program, pool) {
  const sigs = [];
  let before;
  for (;;) {
    const page = await withBackoff(() => connection.getSignaturesForAddress(pool, { before, limit: 1000 }, "confirmed"));
    sigs.push(...page);
    if (page.length < 1000) break;
    before = page[page.length - 1].signature;
  }
  sigs.reverse();
  const deposits = [];
  for (const { signature, err } of sigs) {
    if (err) continue;
    const tx = await fetchTx(connection, signature);
    if (!tx) throw new Error(`transaction ${signature} is not available from this RPC yet`);
    for (const ix of executedInstructions(tx)) {
      if (!ix.program?.equals(program)) continue;
      const { data, accounts } = ix;
      if (data[0] === 1 && data.length === 33 && accounts[1]?.equals(pool)) {
        deposits.push({ commitment: readBe(data.subarray(1, 33)), depositor: accounts[0].toBase58(), signature });
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

// ------------------------------------------------- the curator's clean set, on-chain
//
// The curator account holds roots and the hash of the policy it applies. The
// policy document and the clean set behind the latest root travel in a Memo
// instruction of the same transaction that publishes them, so any client can
// rebuild the clean tree with nothing but an RPC endpoint. A client accepts a
// memo only if its root is held in the curator's current epoch AND the sha256
// of its policy document is the hash stored in the curator account.

export const MEMO_PROGRAM = new PublicKey("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
export const POLICY_RULE = "exclude every deposit whose depositor is on the blocklist";

/** The canonical policy document: fixed key order, sorted and deduplicated blocklist. */
export function policyDocument(blocklist) {
  return { version: 1, rule: POLICY_RULE, blocklist: [...new Set(blocklist)].sort() };
}

/**
 * Everything one publication writes, in one transaction: an optional new
 * epoch, the policy's hash, the clean root, and the memo carrying the policy
 * document and the clean set.
 */
export function publishInstructions({ compliance, curator, pool, depositCount, excluded, blocklist, cleanRoot, revoke = false }) {
  const policy = policyDocument(blocklist);
  const body = { v: 2, pool: pool.toBase58(), n: Number(depositCount), x: excluded, r: BigInt(cleanRoot).toString(16), policy };
  return [
    ...(revoke ? [instructions.revoke(compliance, curator)] : []),
    instructions.updatePolicy(compliance, curator, policyHash(policy)),
    instructions.publishRoot(compliance, curator, cleanRoot),
    new TransactionInstruction({ programId: MEMO_PROGRAM, keys: [], data: Buffer.from(JSON.stringify(body)) }),
  ];
}

/** The curator account's epoch, policy hash and the roots of its current epoch. */
export async function curatorState(connection, compliance, authority) {
  const info = await withBackoff(() => connection.getAccountInfo(curatorAddress(compliance, authority), "confirmed"));
  if (!info) return null;
  return parseCuratorAccount(Uint8Array.from(info.data));
}

export function parseCuratorAccount(d) {
  const epoch = readU64(d, 2);
  const roots = [];
  for (let i = 0; i < 32; i++) {
    const root = readBe(d.subarray(78 + 32 * i, 110 + 32 * i));
    if (root !== 0n && readU64(d, 1102 + 8 * i) === epoch) roots.push(root);
  }
  return { epoch, roots, policyHash: bytesToHex(d.subarray(46, 78)) };
}

/**
 * A memo body as the clean set it describes, or null if it does not describe
 * `pool` under this curator state: malformed, wrong version, a root the
 * curator does not hold now, or a policy document whose hash is not the
 * curator's. Never throws, since anyone can put any text in a memo.
 */
export function acceptMemo(text, pool, state) {
  try {
    const body = JSON.parse(text);
    if (body?.v !== 2 || body.pool !== pool.toBase58()) return null;
    const { n, x, r, policy } = body;
    if (!Number.isSafeInteger(n) || n < 0) return null;
    if (!Array.isArray(x) || !x.every((i, j) => Number.isSafeInteger(i) && i >= 0 && i < n && (j === 0 || i > x[j - 1]))) return null;
    if (typeof r !== "string" || !/^[0-9a-f]{1,64}$/.test(r)) return null;
    if (policy?.version !== 1 || policy.rule !== POLICY_RULE || !Array.isArray(policy.blocklist)) return null;
    if (!policy.blocklist.every((a) => typeof a === "string")) return null;
    const cleanRoot = BigInt(`0x${r}`);
    if (!state.roots.includes(cleanRoot)) return null;
    if (bytesToHex(policyHash(policy)) !== state.policyHash) return null;
    return { depositCount: n, excluded: x, policy, blocklist: policy.blocklist, cleanRoot };
  } catch {
    return null;
  }
}

/** True if `authority` signed this transaction. Signers come first among the static keys. */
function signedBy(tx, authority) {
  const msg = tx.transaction.message;
  const keys = msg.staticAccountKeys ?? msg.accountKeys;
  return keys.slice(0, msg.header.numRequiredSignatures).some((k) => k.equals(authority));
}

/**
 * The latest clean set the curator published for `pool` and still stands
 * behind, or null if none is found among the curator account's most recent
 * `maxScan` transactions of the curator's authority. Anyone can reference
 * that key, so this pages back through its history and reads only memos in
 * transactions the authority signed. With `leaves`, a memo must also rebuild
 * to its own root.
 */
export async function readCleanSet(connection, compliance, authority, pool, { leaves, maxScan = 2000 } = {}) {
  const state = await curatorState(connection, compliance, authority);
  if (!state) return null;
  // The authority's own history: withdrawals touch the curator account, not
  // the authority, so this is mostly publications. A transaction with no memo
  // is skipped without fetching it.
  let before;
  for (let scanned = 0; scanned < maxScan; ) {
    const sigs = await withBackoff(() => connection.getSignaturesForAddress(authority, { limit: 100, before }, "confirmed"));
    if (!sigs.length) return null;
    for (const { signature, err, memo } of sigs) {
      if (err || !memo) continue;
      const tx = await fetchTx(connection, signature);
      if (!tx || !signedBy(tx, authority)) continue;
      const msg = tx.transaction.message;
      const keys = msg.staticAccountKeys ?? msg.accountKeys;
      for (const ci of msg.compiledInstructions ?? msg.instructions) {
        if (!keys[ci.programIdIndex]?.equals(MEMO_PROGRAM)) continue;
        const set = acceptMemo(new TextDecoder().decode(Uint8Array.from(ci.data)), pool, state);
        if (!set) continue;
        if (leaves && (leaves.length < set.depositCount || (await cleanTree(leaves.slice(0, set.depositCount), set.excluded)).root() !== set.cleanRoot)) continue;
        return { ...set, signature, epoch: state.epoch };
      }
    }
    scanned += sigs.length;
    before = sigs.at(-1).signature;
  }
  return null;
}

/**
 * The blocklist in force: the latest accepted memo's, or `genesis` while the
 * curator has never published a root. A curator that has published but whose
 * memo cannot be found fails closed: falling back to `genesis` would silently
 * unblock everyone it blocked since.
 */
export async function blocklistInForce(connection, compliance, authority, pool, genesis) {
  const set = await readCleanSet(connection, compliance, authority, pool);
  if (set) return set.blocklist;
  const state = await curatorState(connection, compliance, authority);
  if (!state || (state.epoch === 0n && state.roots.length === 0)) return genesis;
  throw new Error("the curator's latest clean set could not be found, so its blocklist is unknown");
}
