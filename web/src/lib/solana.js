// Solana in the browser: the pool rebuilt from devnet history, the curator's
// clean set read from its memo, deposit with the test wallet, prove in the
// browser, and clear-and-spend through the relay.

import { Connection, Keypair, PublicKey, Transaction } from "@solana/web3.js";
import * as S from "compliance-sdk/solana";
import { commitmentOf, nullifierOf, circuitNote } from "compliance-sdk/note";
import { prove } from "compliance-sdk/prove";
import { burner, createNote } from "./store.js";
import { post, PROVING, short, field, indices } from "./api.js";

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
  const curatorAccount = [["curator account", S.curatorAddress(compliance, curator).toBase58()]];
  const unit = (lamports) => `${(Number(lamports) / 1e9).toLocaleString(undefined, { maximumFractionDigits: 4 })} SOL`;
  const cluster = "?cluster=devnet";

  const scope = { chain: "solana", network: "devnet", pool: cfg.pool, denomination: BigInt(cfg.denomination) };

  return {
    id: "solana",
    me,
    scope,
    unit,
    denominationLabel: unit(cfg.denomination),
    txLink: (s) => `${cfg.explorer}/tx/${s}${cluster}`,
    addressLink: (a) => `${cfg.explorer}/address/${a}${cluster}`,

    balance: async () => unit(await connection.getBalance(kp.publicKey)),
    async faucet(step) {
      step("Asking the demo service for devnet SOL");
      const r = await post("/api/solana/faucet", { address: me });
      step(r.note ? "This wallet already has enough devnet SOL" : `The demo relay sent ${unit(r.sent)} to this wallet`, { tx: r.tx, done: true });
    },

    async pool() {
      // The memo must rebuild, from the pool's own leaves, to the root it names.
      const s = await S.syncPool(connection, program, pool);
      const set = await S.readCleanSet(connection, compliance, curator, pool, { leaves: s.leaves });
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
      // Prefer a clean index if the same commitment was deposited more than once.
      const matches = p.synced.leaves.flatMap((l, i) => (l === c ? [i] : []));
      const index = matches.find((i) => p.deposits[i]?.clean) ?? matches[0] ?? -1;
      if (index < 0) return { index, label: "Not deposited" };
      const record = await connection.getAccountInfo(S.spendAddress(program, pool, S.fieldBytes(await nullifierOf(note))));
      if (record) return record.data[1] === 2 ? { index, label: "Withdrawn", tone: "done" } : { index, label: "Waiting for the batch", tone: "wait" };
      const d = p.deposits[index];
      if (d.uncurated) return { index, label: "Awaiting the curator", tone: "wait" };
      if (!d.clean) return { index, label: "Excluded", tone: "bad" };
      return { index, label: "Clean", tone: "good", canWithdraw: true };
    },

    /** `keep` stores the key before anything is sent, so a lost confirmation cannot lose it. */
    async deposit(step, keep) {
      // The deposit plus fees and the spend record's rent.
      const have = await connection.getBalance(kp.publicKey);
      if (have < Number(cfg.denomination) + 10_000_000) throw new Error(`This wallet holds ${unit(have)}. Get test funds first.`);
      const note = createNote(scope);
      keep(note);
      step("Created a withdrawal key in this browser", { detail: "Two random field elements, k and r. They stay in this browser unless you download the key.", done: true });
      const commitment = await commitmentOf(note);
      step("Computed the commitment", { detail: [["C = Poseidon(k, r, denomination)", field(commitment)]], done: true });
      step("Sending Deposit to the compliant pool, which runs mirror-pool's own deposit");
      const tx = new Transaction().add(S.instructions.deposit(program, kp.publicKey, pool, commitment));
      const sig = await connection.sendTransaction(tx, [kp]);
      await connection.confirmTransaction(sig, "confirmed");
      step(`The pool escrowed ${unit(cfg.denomination)} and appended C to its tree`, { tx: sig, done: true });
      step("The curator applies its policy and publishes a clean root that covers the new deposit");
      const r = await post("/api/solana/publish", {});
      step(`Curator published the clean root for ${r.depositCount} deposits`, {
        tx: r.tx,
        accounts: curatorAccount,
        detail: [["excluded by the policy", indices(r.excluded)], ["one transaction", "UpdatePolicy, PublishRoot, and a memo with the clean set and the policy"]],
        done: true,
      });
      return { note, tx: sig };
    },

    freshRecipient: () => Keypair.generate().publicKey.toBase58(),

    async withdraw(note, recipient, p, step, retried = false) {
      const fee = BigInt(cfg.fee);
      // Read the chain again: the curator may have published since the page loaded.
      p = await this.pool();
      const status = await this.noteStatus(note, p);
      const { index } = status;
      if (!status.canWithdraw) throw new Error(`${index >= 0 ? `Deposit #${index}` : "This withdrawal key's deposit"} cannot be withdrawn now: ${status.label.toLowerCase()}.`);
      const set = p.cleanSet;
      step("Rebuilt the pool from devnet history and checked the curator's memo", {
        detail: [
          ["deposits covered", set.depositCount],
          ["excluded by the policy", indices(set.excluded)],
          ["policy", "its sha256 matches the hash in the curator account"],
          ["your deposit", `#${index}`],
        ],
        done: true,
      });
      const binding = S.actionBinding({ beneficiary: new PublicKey(recipient).toBytes(), relay: new PublicKey(cfg.relay).toBytes(), relayFee: fee });
      step("Bound the proof to this exact withdrawal", {
        detail: [["beneficiary", short(recipient)], ["relay", short(cfg.relay)], ["fee", unit(fee)], ["binding", field(binding)]],
        done: true,
      });
      step("Generating the Groth16 proof in this browser");
      const t0 = performance.now();
      const { proof, input } = await prove(
        { note: circuitNote(note), index, poolLeaves: p.synced.leaves.slice(0, set.depositCount), excluded: set.excluded, binding },
        PROVING,
      );
      step(`Proof ready in ${((performance.now() - t0) / 1000).toFixed(1)} s. It reveals only its four public inputs`, {
        detail: [["pool root", field(input.poolRoot)], ["clean root", field(input.cleanRoot)], ["nullifier hash", field(input.nullifierHash)], ["binding", field(input.binding)]],
        done: true,
      });
      step("The relay sends one transaction: VerifyClearance, then SubmitClearedSpend");
      const s = S.toSolana(proof);
      let out;
      try {
        out = await post("/api/solana/relay", {
          proof: { a: hex(s.a), b: hex(s.b), c: hex(s.c) },
          inputs: [input.poolRoot, input.cleanRoot, input.nullifierHash, input.binding],
          beneficiary: recipient,
          fee: fee.toString(),
        });
      } catch (e) {
        // The curator revoked this clean root while the proof was being made. Prove again, once.
        if (retried || !/StaleCleanRoot|UnknownPoolRoot/.test(e.message)) throw e;
        step("The curator published a new clean set while proving, so this root is stale. Proving again", { done: true });
        return this.withdraw(note, recipient, p, step, true);
      }
      const received = unit(BigInt(cfg.denomination) - fee);
      step("The compliance program verified the proof and wrote a public clearance. The pool accepted the spend", {
        tx: out.tx,
        accounts: [["clearance", S.clearanceAddress(compliance, pool, curator, S.fieldBytes(input.nullifierHash)).toBase58()]],
        detail: "The clearance and the spend share one slot, so a clearance cannot be kept and used after a curator's revocation.",
        done: true,
      });
      if (out.settleError) {
        step(`The spend is recorded, but settling the batch failed: ${out.settleError}. Retry with "Settle the batch"`, { done: true });
      } else if (out.settled && out.paid) {
        step(`The batch settled: ${out.settled.settled} withdrawals paid at once, ${received} of them to ${short(recipient)}`, {
          tx: out.settled.tx,
          accounts: [["beneficiary", recipient]],
          done: true,
        });
      } else if (out.settled) {
        step(`A batch of ${out.settled.settled} settled without this spend, which waits for the next one`, { tx: out.settled.tx, done: true });
      } else {
        step(`Waiting for the batch: ${out.pending} ${out.pending === 1 ? "spend" : "spends"} pending. The pool pays crowds of at least two, all at one timestamp`, { done: true });
      }
      return { tx: out.tx, settled: out.settled, pending: out.pending, received };
    },

    async settle(step) {
      step("Asking the relay to settle the pending batch");
      // Below the floor, the pool itself decides: it pays a lone spend only once the pool's timeout has passed.
      const r = await post("/api/solana/settle", { allowBelowFloor: true });
      if (r.settled) step(`The batch settled: ${r.settled} withdrawals paid at once`, { tx: r.tx, done: true });
      else if (r.waiting) step(`Not yet: ${r.pending} spend pending. The pool pays fewer than ${r.floor} only after its timeout, so try again later or wait for another withdrawal`, { done: true });
      else step(`Nothing to settle: ${r.pending} spend pending`, { done: true });
      return r;
    },

    /** Asks the curator to publish a clean root that covers every deposit so far. */
    async publish(step) {
      step("Asking the curator to apply its policy to every deposit so far");
      const r = await post("/api/solana/publish", {});
      step(`Curator published the clean root for ${r.depositCount} deposits`, {
        tx: r.tx,
        accounts: curatorAccount,
        detail: [["excluded by the policy", indices(r.excluded)]],
        done: true,
      });
      return r;
    },

    async block(address, step) {
      step(`The curator blocks ${short(address)}: Revoke, UpdatePolicy, PublishRoot and a memo, in one transaction`);
      const r = await post("/api/solana/block", { address });
      step(`New epoch: every older clean root is revoked. Excluded ${indices(r.excluded)}`, {
        tx: r.tx,
        accounts: curatorAccount,
        detail: [["clean set", `${r.depositCount} deposits, excluded ${indices(r.excluded)}`]],
        done: true,
      });
      return r;
    },

    async unblock(address, step) {
      step(`The curator publishes a policy without ${short(address)} and a new clean root`);
      const r = await post("/api/solana/unblock", { address });
      step(`Published: excluded ${indices(r.excluded)}`, { tx: r.tx, accounts: curatorAccount, done: true });
      return r;
    },
  };
}
