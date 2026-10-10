// Solana actions: test funds, the relay (which settles the batch once it
// reaches the pool's floor), and the curator publishing, blocking or
// unblocking. The curator's clean set travels in a memo with each root.

import { Connection, Keypair, PublicKey, SystemProgram, Transaction, sendAndConfirmTransaction } from "@solana/web3.js";
import * as S from "compliance-sdk/solana";
import { cleanTree } from "compliance-sdk/tree";
import { SOLANA, secret } from "./common.mjs";

const connection = new Connection(SOLANA.rpc, "confirmed");
const keypair = (name) => Keypair.fromSecretKey(Uint8Array.from(JSON.parse(secret(name))));
const relay = () => keypair("SOLANA_RELAY_KEY");
const curator = () => keypair("SOLANA_CURATOR_KEY");
export const relayAddress = () => relay().publicKey.toBase58();
const program = new PublicKey(SOLANA.poolProgram);
const compliance = new PublicKey(SOLANA.complianceProgram);
const pool = new PublicKey(SOLANA.pool);
const K_FLOOR = 2;

const send = (ixs, signers) => sendAndConfirmTransaction(connection, new Transaction().add(...ixs), signers, { commitment: "confirmed" });

const isPending = (data) => data.length >= 160 && data[0] === 1 && data[1] === 1;

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

async function settle(allowBelowFloor = false, known = []) {
  const pending = await pendingSpends(known);
  if (!pending.length) return null;
  const { TransactionInstruction } = await import("@solana/web3.js");
  const r = relay();
  const keys = [
    { pubkey: r.publicKey, isSigner: true, isWritable: true },
    { pubkey: pool, isSigner: false, isWritable: true },
    { pubkey: S.vaultAddress(program, pool), isSigner: false, isWritable: true },
    ...pending.flatMap((a) => [
      { pubkey: a.pubkey, isSigner: false, isWritable: true },
      { pubkey: new PublicKey(a.account.data.subarray(32, 64)), isSigner: false, isWritable: true },
      { pubkey: new PublicKey(a.account.data.subarray(64, 96)), isSigner: false, isWritable: true },
    ]),
  ];
  const ix = new TransactionInstruction({ programId: program, keys, data: Buffer.from([3, pending.length, allowBelowFloor ? 1 : 0]) });
  return { tx: await send([ix], [r]), settled: pending.length };
}

/** Publishes the clean root of the whole pool under `blocklist`, with the set in a memo. */
async function publish(blocklist, { revoke = false } = {}) {
  const { leaves, depositors } = await S.syncPool(connection, program, pool);
  const blocked = new Set(blocklist);
  const excluded = depositors.map((d, i) => (blocked.has(d) ? i : -1)).filter((i) => i >= 0);
  const cleanRoot = (await cleanTree(leaves, excluded)).root();
  const c = curator();
  const ixs = [
    ...(revoke ? [S.instructions.revoke(compliance, c.publicKey)] : []),
    S.instructions.publishRoot(compliance, c.publicKey, cleanRoot),
    S.cleanSetMemo({ pool, depositCount: leaves.length, excluded, blocklist: [...blocked], cleanRoot }),
  ];
  return { tx: await send(ixs, [c]), depositCount: leaves.length, excluded, revoked: revoke };
}

async function currentBlocklist() {
  const set = await S.readCleanSet(connection, compliance, curator().publicKey, pool);
  return set?.blocklist ?? SOLANA.initialBlocklist;
}

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
    const fee = BigInt(req.fee);
    if (fee < BigInt(SOLANA.fee)) throw new Error(`fee below ${SOLANA.fee}`);
    if (!isPubkey(req.beneficiary)) throw new Error("beneficiary is not a public key");
    const hex = (h) => Uint8Array.from(Buffer.from(h, "hex"));
    const proof = { a: hex(req.proof.a), b: hex(req.proof.b), c: hex(req.proof.c) };
    const inputs = req.inputs.map(BigInt);
    const r = relay();
    const tx = await send(
      [
        S.instructions.verifyClearance(compliance, r.publicKey, new PublicKey(SOLANA.curatorAuthority), pool, proof, inputs),
        S.instructions.submitClearedSpend(program, compliance, r.publicKey, pool, S.fieldBytes(inputs[2]), new PublicKey(req.beneficiary), fee),
      ],
      [r],
    );
    const spend = S.spendAddress(program, pool, S.fieldBytes(inputs[2]));
    const pending = (await pendingSpends([spend])).length;
    const settled = pending >= K_FLOOR ? await settle(false, [spend]) : null;
    return { tx, pending: settled ? 0 : pending, settled };
  },

  /** Settles whatever is pending; below the floor only once the pool's timeout has run. */
  async settle({ allowBelowFloor = false }) {
    const pending = (await pendingSpends()).length;
    if (!allowBelowFloor && pending < K_FLOOR) return { settled: 0, pending, floor: K_FLOOR };
    return (await settle(Boolean(allowBelowFloor))) ?? { settled: 0, pending, floor: K_FLOOR };
  },

  /** The curator publishes a root covering every deposit so far. */
  async publish() {
    return publish(await currentBlocklist());
  },

  /** The curator blocks a depositor: a new epoch, then the root without them, in one transaction. */
  async block({ address }) {
    if (!isPubkey(address)) throw new Error("not a public key");
    const list = new Set(await currentBlocklist());
    list.add(address);
    return publish([...list], { revoke: true });
  },

  async unblock({ address }) {
    if (!isPubkey(address)) throw new Error("not a public key");
    const list = new Set(await currentBlocklist());
    list.delete(address);
    return publish([...list]);
  },
};
