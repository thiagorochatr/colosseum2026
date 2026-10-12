// Solana actions: test funds, the relay (which settles the batch once it
// reaches the pool's floor), and the curator publishing, blocking or
// unblocking. The curator's clean set travels in a memo with each root.

import { Connection, Keypair, PublicKey, SystemProgram, Transaction, TransactionInstruction, sendAndConfirmTransaction } from "@solana/web3.js";
import * as S from "compliance-sdk/solana";
import { cleanTree } from "compliance-sdk/tree";
import { SOLANA, secret } from "./common.mjs";

const connection = new Connection(SOLANA.rpc, "confirmed");
// A parse error would quote part of the key, and errors reach the client.
function keypair(name) {
  try {
    return Keypair.fromSecretKey(Uint8Array.from(JSON.parse(secret(name))));
  } catch (e) {
    if (/missing/.test(e.message)) throw e;
    // eslint-disable-next-line preserve-caught-error -- the cause would quote the key
    throw new Error(`the demo service's ${name} is malformed`);
  }
}
const relay = () => keypair("SOLANA_RELAY_KEY");
const curator = () => keypair("SOLANA_CURATOR_KEY");
export const relayAddress = () => relay().publicKey.toBase58();
const program = new PublicKey(SOLANA.poolProgram);
const compliance = new PublicKey(SOLANA.complianceProgram);
const pool = new PublicKey(SOLANA.pool);
const K_FLOOR = 2;

const send = (ixs, signers) => sendAndConfirmTransaction(connection, new Transaction().add(...ixs), signers, { commitment: "confirmed" });

const isPending = (data) => data.length >= 160 && data[0] === 1 && data[1] === 1;
// Three accounts per spend: past this the settle transaction outgrows a packet.
const MAX_BATCH = 10;

// Names for the custom errors of the programs a relay's transaction runs.
const COMPLIANCE_ERRORS = ["UnknownInstruction", "MalformedInstruction", "NonCanonicalInput", "ProofVerificationFailed", "MissingSignature", "InvalidAccount", "CuratorExists", "WrongAuthority", "UnknownPoolRoot", "StaleCleanRoot", "ClearanceExists", "ArithmeticOverflow"];
const POOL_ERRORS = { 100: "ClearanceRequired", 101: "InvalidClearance", 102: "ClearanceForAnotherPool", 103: "ClearanceForAnotherNullifier", 104: "WrongCurator", 105: "StaleClearance", 106: "BindingMismatch", 107: "PoolNotServed" };
// mirror-pool's own codes, which the compliant pool returns unchanged.
const MIRROR_ERRORS = { 1: "MalformedInstruction", 2: "InvalidPda", 3: "MissingSignature", 4: "InvalidOwner", 5: "InvalidPoolAccount", 6: "AlreadyInitialised", 7: "InvalidParameter", 8: "NonCanonicalField", 9: "PoseidonFailed", 10: "TreeFull", 12: "ArithmeticOverflow", 13: "InsolventVault", 14: "InvalidSpendAccount", 15: "NullifierAlreadySpent", 16: "AlreadySettled", 17: "UnknownRoot", 18: "ProofVerificationFailed", 19: "BelowAnonymityFloor", 20: "RelayFeeTooLarge", 21: "CrowdTooSmall", 22: "UnknownSelector", 23: "SelfInvocationRefused", 24: "PayloadTooLarge", 25: "FeeNotUniform", 26: "BelowFloorNotPermitted" };

/**
 * The program error behind a failed transaction, by name, so the client can
 * act on it. The logs name the program that failed first, the innermost one:
 * a system-program failure inside the compliance program's account creation
 * is the system program's, not the compliance program's. Without logs, the
 * instruction index decides: 0 is VerifyClearance, 1 is the spend.
 */
function programError(e) {
  const logs = (e.logs ?? e.transactionLogs ?? []).join("\n");
  const failed = logs.match(/Program (\w+) failed: custom program error: 0x([0-9a-f]+)/i);
  let who;
  let code;
  if (failed) {
    who = failed[1];
    code = parseInt(failed[2], 16);
  } else {
    const text = String(e.message);
    const pre = text.match(/Instruction (\d+): custom program error: 0x([0-9a-f]+)/i);
    const exec = text.match(/"InstructionError":\[(\d+),\{"Custom":(\d+)\}/);
    if (!pre && !exec) return null;
    who = (pre ? pre[1] : exec[1]) === "0" ? compliance.toBase58() : program.toBase58();
    code = pre ? parseInt(pre[2], 16) : Number(exec[2]);
  }
  if (who === compliance.toBase58()) return { by: "the compliance program", name: COMPLIANCE_ERRORS[code] ?? `error ${code}`, code };
  if (who === program.toBase58()) return { by: "the pool", name: (code >= 100 ? POOL_ERRORS : MIRROR_ERRORS)[code] ?? `error ${code}`, code };
  if (who === SystemProgram.programId.toBase58()) return { by: "the system program", name: `error ${code}, likely too few lamports`, code };
  return { by: `program ${who}`, name: `error ${code}`, code };
}

function relayError(e) {
  const p = programError(e);
  return p ? new Error(`${p.by} refused the withdrawal: ${p.name}`, { cause: e }) : e;
}

/**
 * Every pending spend record of the pool. `known` are spend records this
 * request just created: a public RPC node may not index them yet, so they are
 * read directly and merged in.
 */
async function pendingSpends(known = []) {
  const accounts = await S.withBackoff(() =>
    connection.getProgramAccounts(program, { filters: [{ memcmp: { offset: 96, bytes: pool.toBase58() } }] }),
  );
  const found = accounts.filter((a) => isPending(a.account.data));
  for (const pubkey of known) {
    if (found.some((a) => a.pubkey.equals(pubkey))) continue;
    for (let i = 0; i < 10; i++) {
      const account = await connection.getAccountInfo(pubkey, "confirmed");
      if (account) {
        if (isPending(account.data)) found.push({ pubkey, account });
        break;
      }
      await new Promise((r) => setTimeout(r, 400));
    }
  }
  return found;
}

/**
 * The pending spends this relay can settle together: its own plain transfers
 * at its fee, its `known` spends first. A spend paying another fee, invoking a
 * program, or naming an account the pool cannot pay would make the whole batch
 * fail, so it is left out. Anyone can still settle those with the CLI.
 */
async function settleable(known = []) {
  const fee = BigInt(SOLANA.fee);
  const banned = new Set(protectedKeys());
  // Only spends this relay sent: it checked their beneficiaries. A spend sent
  // straight to the program by anyone else could name one the pool cannot pay.
  const mine = relay().publicKey.toBase58();
  const first = new Set(known.map((k) => k.toBase58()));
  const ok = (await pendingSpends(known)).filter(({ account: { data } }) => {
    const d = Buffer.from(data);
    return (
      d.readBigUInt64LE(16) === fee &&
      d.readBigUInt64LE(8) === 0n &&
      new PublicKey(d.subarray(64, 96)).toBase58() === mine &&
      !banned.has(new PublicKey(d.subarray(32, 64)).toBase58())
    );
  });
  // The runtime refuses to change an executable account's balance, so a
  // beneficiary a program was deployed to after its spend was recorded can
  // never be paid. Leaving those out keeps them from stalling every batch.
  const beneficiaries = await S.withBackoff(() => connection.getMultipleAccountsInfo(ok.map((a) => new PublicKey(a.account.data.subarray(32, 64))), "confirmed"));
  const payable = ok.filter((_, i) => !beneficiaries[i]?.executable);
  payable.sort((a, b) => Number(first.has(b.pubkey.toBase58())) - Number(first.has(a.pubkey.toBase58())));
  return payable.slice(0, MAX_BATCH);
}

function settleInstruction(spends, allowBelowFloor) {
  const r = relay();
  const keys = [
    { pubkey: r.publicKey, isSigner: true, isWritable: true },
    { pubkey: pool, isSigner: false, isWritable: true },
    { pubkey: S.vaultAddress(program, pool), isSigner: false, isWritable: true },
    ...spends.flatMap((a) => [
      { pubkey: a.pubkey, isSigner: false, isWritable: true },
      { pubkey: new PublicKey(a.account.data.subarray(32, 64)), isSigner: false, isWritable: true },
      { pubkey: new PublicKey(a.account.data.subarray(64, 96)), isSigner: false, isWritable: true },
    ]),
  ];
  return new TransactionInstruction({ programId: program, keys, data: Buffer.from([3, spends.length, allowBelowFloor ? 1 : 0]) });
}

async function simulates(spends, allowBelowFloor) {
  const r = relay();
  const tx = new Transaction({ feePayer: r.publicKey, recentBlockhash: (await connection.getLatestBlockhash()).blockhash }).add(settleInstruction(spends, allowBelowFloor));
  const out = await connection.simulateTransaction(tx, [r]);
  return out.value.err ? { err: out.value.err, logs: out.value.logs ?? [] } : null;
}

/**
 * Settles the settleable spends. A beneficiary that became unpayable after its
 * spend was recorded, such as an address a program was later deployed to,
 * would fail the whole batch, so the batch is simulated first and, if it
 * fails, one spend at a time is left out until it passes.
 */
async function settle(allowBelowFloor = false, known = []) {
  let pending = await settleable(known);
  // Another settle may have paid some since the caller counted.
  if (!pending.length || (!allowBelowFloor && pending.length < K_FLOOR)) return null;
  const failure = await simulates(pending, allowBelowFloor);
  if (failure) {
    const p = programError({ logs: failure.logs, message: JSON.stringify(failure.err) });
    // Too few spends before the pool's timeout: nothing to drop, just wait.
    if (p?.code === 21 && p.by === "the pool") throw new Error("CrowdTooSmall");
    let found = null;
    for (let i = 0; i < pending.length && !found; i++) {
      const without = pending.filter((_, j) => j !== i);
      if (without.length && !(await simulates(without, allowBelowFloor))) found = without;
    }
    if (!found) throw new Error(`the batch does not settle: ${p ? `${p.by}: ${p.name}` : JSON.stringify(failure.err)}`);
    pending = found;
  }
  let tx;
  try {
    tx = await send([settleInstruction(pending, allowBelowFloor)], [relay()]);
  } catch (e) {
    // Another settle paid one of these between the simulation and this send.
    if (programError(e)?.code === 16) return null;
    throw e;
  }
  return { tx, settled: pending.length, included: pending.map((a) => a.pubkey.toBase58()) };
}

/**
 * Publishes the clean root of the whole pool under `blocklist`, in one
 * transaction with the policy's hash and the memo carrying the policy document.
 */
async function publish(blocklist, { revoke = false, base } = {}) {
  const { leaves, depositors } = await S.syncPool(connection, program, pool);
  const policy = S.policyDocument(blocklist);
  const blocked = new Set(policy.blocklist);
  const excluded = depositors.map((d, i) => (blocked.has(d) ? i : -1)).filter((i) => i >= 0);
  const cleanRoot = (await cleanTree(leaves, excluded)).root();
  const c = curator();
  // The same clean set, already held in the current epoch under the same
  // policy: publishing it again would only push older roots out of the
  // curator's ring, staling proofs made against them.
  if (!revoke) {
    const state = await S.curatorState(connection, compliance, c.publicKey);
    if (state && state.roots.includes(cleanRoot) && state.policyHash === S.bytesToHex(S.policyHash(policy))) {
      return { tx: null, depositCount: leaves.length, excluded, revoked: false, unchanged: true };
    }
  }
  const ixs = S.publishInstructions({ compliance, curator: c.publicKey, pool, depositCount: leaves.length, excluded, blocklist: policy.blocklist, cleanRoot, revoke });
  // Another publication may have landed since `base` was read: publishing over
  // it would silently undo its block or unblock.
  if (base && !sameList(base, await currentBlocklist())) throw new Error("the curator's policy changed meanwhile; try again");
  // The memo carries the whole policy and clean set, and one transaction holds
  // at most 1232 bytes. Refuse a change that would make it unpublishable.
  // serialize() itself throws once a transaction passes the limit.
  let fits = true;
  try {
    new Transaction({ feePayer: c.publicKey, recentBlockhash: PublicKey.default.toBase58() }).add(...ixs).serialize({ requireAllSignatures: false, verifySignatures: false });
  } catch (e) {
    if (!/too large/i.test(e.message)) throw e;
    fits = false;
  }
  if (!fits) throw new Error("the curator's policy and clean set would not fit in one transaction's memo: unblock someone first");
  return { tx: await send(ixs, [c]), depositCount: leaves.length, excluded, revoked: revoke };
}

const sameList = (a, b) => JSON.stringify([...a].sort()) === JSON.stringify([...b].sort());

/**
 * Every curator change goes through one queue, so two changes from this
 * instance never interleave; publish re-reads the policy before sending to
 * catch one from another instance.
 */
let curatorQueue = Promise.resolve();
function curatorChange(edit, options = {}) {
  const run = curatorQueue.then(async () => {
    const base = await currentBlocklist();
    return publish(edit(new Set(base)), { ...options, base });
  });
  curatorQueue = run.catch(() => {});
  return run;
}

async function currentBlocklist() {
  return S.blocklistInForce(connection, compliance, curator().publicKey, pool, SOLANA.initialBlocklist);
}

/** The demo's own accounts, which no visitor may block. */
const protectedKeys = () => [relay().publicKey.toBase58(), curator().publicKey.toBase58(), pool.toBase58(), S.vaultAddress(program, pool).toBase58()];

const isPubkey = (a) => {
  try {
    return new PublicKey(a).toBase58() === a;
  } catch {
    return false;
  }
};

export const solanaActions = {
  async faucet({ address }) {
    if (!isPubkey(address)) throw new Error("not a public key");
    const to = new PublicKey(address);
    const have = await connection.getBalance(to);
    const want = 250_000_000;
    if (have >= 150_000_000) return { sent: "0", note: "already funded" };
    const r = relay();
    const tx = await send([SystemProgram.transfer({ fromPubkey: r.publicKey, toPubkey: to, lamports: want - have })], [r]);
    return { sent: String(want - have), tx };
  },

  /** VerifyClearance and SubmitClearedSpend in one transaction, then settles if the batch reached the floor. */
  async relay(req) {
    // Every spend this relay sends pays the same fee, so they can always settle together.
    if (String(req.fee) !== SOLANA.fee) throw new Error(`the relay fee is exactly ${SOLANA.fee}`);
    const fee = BigInt(SOLANA.fee);
    if (!isPubkey(req.beneficiary)) throw new Error("beneficiary is not a public key");
    if (protectedKeys().includes(req.beneficiary)) throw new Error("the beneficiary cannot be one of the demo's own accounts");
    // A wallet address is a point on the curve. Program-derived addresses,
    // sysvars and reserved accounts are not wallets, and the pool may not be able to pay them.
    // Sysvars and builtin programs have reserved, padded addresses, which the runtime keeps read-only.
    if (!PublicKey.isOnCurve(new PublicKey(req.beneficiary).toBytes()) || /1{16}$/.test(req.beneficiary)) {
      throw new Error("the beneficiary must be an ordinary wallet address");
    }
    const target = await connection.getAccountInfo(new PublicKey(req.beneficiary));
    if (target && (target.executable || !target.owner.equals(SystemProgram.programId))) {
      throw new Error("the beneficiary must be an ordinary wallet address: the pool cannot pay a program or a program's account");
    }
    const hex = (h, bytes) => {
      if (typeof h !== "string" || !new RegExp(`^[0-9a-f]{${2 * bytes}}$`).test(h)) throw new Error("malformed proof");
      return Uint8Array.from(Buffer.from(h, "hex"));
    };
    const proof = { a: hex(req.proof?.a, 64), b: hex(req.proof?.b, 128), c: hex(req.proof?.c, 64) };
    if (!Array.isArray(req.inputs) || req.inputs.length !== 4 || !req.inputs.every((x) => /^[0-9]{1,78}$/.test(String(x)))) throw new Error("malformed public inputs");
    const inputs = req.inputs.map(BigInt);
    const r = relay();
    let tx;
    try {
      tx = await send(
        [
          S.instructions.verifyClearance(compliance, r.publicKey, new PublicKey(SOLANA.curatorAuthority), pool, proof, inputs),
          S.instructions.submitClearedSpend(program, compliance, new PublicKey(SOLANA.curatorAuthority), r.publicKey, pool, S.fieldBytes(inputs[2]), new PublicKey(req.beneficiary), fee),
        ],
        [r],
      );
    } catch (e) {
      throw relayError(e);
    }
    // The spend has landed. Settling is a separate step that can fail on its
    // own, and anyone can retry it, so a failure here does not fail the relay.
    const spend = S.spendAddress(program, pool, S.fieldBytes(inputs[2]));
    try {
      const pending = (await settleable([spend])).length;
      const settled = pending >= K_FLOOR ? await settle(false, [spend]) : null;
      if (settled) return { tx, pending: 0, settled, paid: settled.included.includes(spend.toBase58()) };
      // Another settle may have paid this spend meanwhile.
      const record = await connection.getAccountInfo(spend, "confirmed");
      if (record?.data[1] === 2) return { tx, pending: 0, settled: null, paid: true };
      return { tx, pending: (await settleable([spend])).length, settled: null, paid: false };
    } catch (e) {
      return { tx, pending: null, settled: null, settleError: e.message };
    }
  },

  /** Settles whatever is pending; below the floor only once the pool's timeout has run. */
  async settle({ allowBelowFloor = false }) {
    const pending = (await settleable()).length;
    const below = pending < K_FLOOR;
    if (below && !allowBelowFloor) return { settled: 0, pending, floor: K_FLOOR };
    try {
      return (await settle(below)) ?? { settled: 0, pending, floor: K_FLOOR };
    } catch (e) {
      // Below the floor, mirror-pool pays only once the oldest spend has waited out the pool's timeout.
      if (e.message === "CrowdTooSmall") return { settled: 0, pending, floor: K_FLOOR, waiting: true };
      throw e;
    }
  },

  /** The curator publishes a root covering every deposit so far. */
  async publish() {
    return curatorChange((list) => [...list]);
  },

  /** The curator blocks a depositor: a new epoch, then the root without them, in one transaction. */
  async block({ address }) {
    if (!isPubkey(address)) throw new Error("not a public key");
    if (protectedKeys().includes(address)) throw new Error("the demo's own accounts cannot be blocked");
    // The memo holds only so many addresses, and the console lists depositors:
    // an address with no deposit here would take a place no one could see to free.
    const { depositors } = await S.syncPool(connection, program, pool);
    if (!depositors.includes(address)) throw new Error("this address has no deposit in the pool");
    return curatorChange(
      (list) => {
        if (list.has(address)) throw new Error("this address is blocked already");
        return [...list.add(address)];
      },
      { revoke: true },
    );
  },

  async unblock({ address }) {
    if (!isPubkey(address)) throw new Error("not a public key");
    return curatorChange((list) => (list.delete(address), [...list]));
  },
};
