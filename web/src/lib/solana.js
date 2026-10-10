// Solana in the browser: the pool rebuilt from devnet history, the curator's
// clean set read from its memo, deposit with the test wallet, prove in the
// browser, and clear-and-spend through the relay.

import { Connection, Keypair, PublicKey, Transaction } from "@solana/web3.js";
import * as S from "compliance-sdk/solana";
import { commitmentOf, nullifierOf, circuitNote } from "compliance-sdk/note";
import { prove } from "compliance-sdk/prove";
import { burner, createNote } from "./store.js";
import { post, PROVING } from "./api.js";

const hex = (u) => Array.from(u, (b) => b.toString(16).padStart(2, "0")).join("");

export function solanaChain(cfg) {
  const connection = new Connection(cfg.rpc, "confirmed");
  const secret = burner("solana", () => JSON.stringify([...Keypair.generate().secretKey]));
  const kp = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(secret)));
  const me = kp.publicKey.toBase58();
  const program = new PublicKey(cfg.poolProgram);
  const compliance = new PublicKey(cfg.complianceProgram);
  const curator = new PublicKey(cfg.curatorAuthority);
  const pool = new PublicKey(cfg.pool);
  const unit = (lamports) => `${(Number(lamports) / 1e9).toLocaleString(undefined, { maximumFractionDigits: 4 })} SOL`;
  const cluster = "?cluster=devnet";

  return {
    id: "solana",
    me,
    unit,
    denominationLabel: unit(cfg.denomination),
    txLink: (s) => `${cfg.explorer}/tx/${s}${cluster}`,
    addressLink: (a) => `${cfg.explorer}/address/${a}${cluster}`,

    balance: async () => unit(await connection.getBalance(kp.publicKey)),
    faucet: () => post("/api/solana/faucet", { address: me }),

    async pool() {
      const [s, set] = await Promise.all([S.syncPool(connection, program, pool), S.readCleanSet(connection, compliance, curator, pool)]);
      const covered = set?.depositCount ?? 0;
      const excluded = set?.excluded ?? [];
      return {
        deposits: s.leaves.map((leaf, i) => ({
          index: i,
          leaf,
          depositor: s.depositors[i],
          clean: i < covered && !excluded.includes(i),
          uncurated: i >= covered,
        })),
        synced: s,
        cleanSet: set,
        blocklist: set?.blocklist ?? [],
      };
    },

    async noteStatus(note, p) {
      const c = await commitmentOf(note);
      const index = p.synced.leaves.findIndex((l) => l === c);
      if (index < 0) return { index, label: "Not deposited" };
      const record = await connection.getAccountInfo(S.spendAddress(program, pool, S.fieldBytes(await nullifierOf(note))));
      if (record) return record.data[1] === 2 ? { index, label: "Withdrawn", tone: "done" } : { index, label: "Waiting for the batch", tone: "wait" };
      const d = p.deposits[index];
      if (d.uncurated) return { index, label: "Awaiting the curator", tone: "wait" };
      if (!d.clean) return { index, label: "Excluded", tone: "bad" };
      return { index, label: "Clean", tone: "good", canWithdraw: true };
    },

    async deposit() {
      const note = createNote({ chain: "solana", network: "devnet", pool: cfg.pool, denomination: BigInt(cfg.denomination) });
      const tx = new Transaction().add(S.instructions.deposit(program, kp.publicKey, pool, await commitmentOf(note)));
      const sig = await connection.sendTransaction(tx, [kp]);
      await connection.confirmTransaction(sig, "confirmed");
      // The curator publishes a clean root that covers the new deposit.
      await post("/api/solana/publish", {});
      return { note, tx: sig };
    },

    freshRecipient: () => Keypair.generate().publicKey.toBase58(),

    async withdraw(note, recipient, p, onStage) {
      const fee = BigInt(cfg.fee);
      const { index } = await this.noteStatus(note, p);
      const set = p.cleanSet;
      onStage("Proving in your browser");
      const binding = S.actionBinding({ beneficiary: new PublicKey(recipient).toBytes(), relay: new PublicKey(cfg.relay).toBytes(), relayFee: fee });
      const { proof, input } = await prove(
        { note: circuitNote(note), index, poolLeaves: p.synced.leaves.slice(0, set.depositCount), excluded: set.excluded, binding },
        PROVING,
      );
      onStage("Clearing and spending through the relay");
      const s = S.toSolana(proof);
      const out = await post("/api/solana/relay", {
        proof: { a: hex(s.a), b: hex(s.b), c: hex(s.c) },
        inputs: [input.poolRoot, input.cleanRoot, input.nullifierHash, input.binding],
        beneficiary: recipient,
        fee: fee.toString(),
      });
      return { tx: out.tx, settled: out.settled, pending: out.pending, received: unit(BigInt(cfg.denomination) - fee) };
    },

    settle: () => post("/api/solana/settle", {}),

    block: (address) => post("/api/solana/block", { address }),
    unblock: (address) => post("/api/solana/unblock", { address }),
  };
}
