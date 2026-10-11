// Tempo in the browser: read the pool from public RPC, deposit with the test
// wallet in one batch transaction, prove in the browser, withdraw through the relayer.

import { createPublicClient, http } from "viem";
import { tempoModerato } from "viem/chains";
import { generatePrivateKey, privateKeyToAddress } from "viem/accounts";
import { Account, createClient } from "viem/tempo";
import * as T from "compliance-sdk/tempo";
import { commitmentOf, nullifierOf, circuitNote } from "compliance-sdk/note";
import { prove, toSolidity } from "compliance-sdk/prove";
import { burner, createNote } from "./store.js";
import { post, PROVING, short, field, indices } from "./api.js";

export function tempoChain(cfg) {
  const publicClient = createPublicClient({ chain: tempoModerato, transport: http(cfg.rpc) });
  const key = burner("tempo", generatePrivateKey);
  const me = privateKeyToAddress(key);
  const wallet = createClient({ account: Account.fromSecp256k1(key), chain: tempoModerato, transport: http(cfg.rpc), feeToken: "0x20C0000000000000000000000000000000000000" });
  const read = (functionName, args = []) => publicClient.readContract({ address: cfg.pool, abi: T.POOL_ABI, functionName, args });
  const unit = (v) => `${(Number(v) / 1e6).toLocaleString()} dUSD`;

  const scope = { chain: "tempo", network: "moderato", pool: cfg.pool, denomination: BigInt(cfg.denomination) };

  return {
    id: "tempo",
    me,
    scope,
    unit,
    denominationLabel: unit(cfg.denomination),
    txLink: (h) => `${cfg.explorer}/tx/${h}`,
    addressLink: (a) => `${cfg.explorer}/address/${a}`,

    async balance() {
      const v = await publicClient.readContract({ address: cfg.token, abi: T.TIP20_ABI, functionName: "balanceOf", args: [me] });
      return unit(v);
    },
    async faucet(step) {
      step("Asking the demo service for test dUSD");
      const r = await post("/api/tempo/faucet", { address: me });
      step(r.note ? "This wallet already has enough test dUSD" : `The demo issuer minted ${unit(r.minted)} to this wallet`, { tx: r.tx, done: true });
    },

    /**
     * Every deposit, with its depositor, whether it is clean, and whether the
     * token's policy blocks its depositor now. A clean deposit of a blocked
     * depositor is waiting for a keeper to exclude it.
     */
    async pool() {
      const s = await T.syncPool(publicClient, { pool: cfg.pool, fromBlock: cfg.deployBlock });
      const { authorized } = await T.readSweep(publicClient, { pool: cfg.pool, token: cfg.token, synced: s });
      const deposits = s.leaves.map((leaf, i) => {
        const clean = !s.excluded.includes(i);
        const blocked = !authorized(s.depositors[i]);
        return { index: i, leaf, depositor: s.depositors[i], clean, blocked, pending: clean && blocked };
      });
      return { deposits, blocked: [...new Set(deposits.filter((d) => d.blocked).map((d) => d.depositor))], synced: s };
    },

    async noteStatus(note, pool) {
      const c = await commitmentOf(note);
      const index = pool.synced.leaves.findIndex((l) => l === c);
      const spent = index >= 0 && (await read("spent", [await nullifierOf(note)]));
      if (index < 0) return { index, label: "Not deposited" };
      if (spent) return { index, label: "Withdrawn", tone: "done" };
      if (pool.synced.excluded.includes(index)) return { index, label: "Excluded", tone: "bad" };
      if (pool.deposits[index]?.pending) return { index, label: "Blocked, not yet excluded", tone: "wait", canWithdraw: true };
      return { index, label: "Clean", tone: "good", canWithdraw: true };
    },

    /** `keep` stores the key before anything is sent, so a lost confirmation cannot lose it. */
    async deposit(step, keep) {
      const note = createNote(scope);
      keep(note);
      step("Created a withdrawal key in this browser", { detail: "Two random field elements, k and r. They stay in this browser unless you download the key.", done: true });
      const commitment = await commitmentOf(note);
      step("Computed the commitment", { detail: [["C = Poseidon(k, r, denomination)", field(commitment)]], done: true });
      step("Sending one Tempo batch transaction: approve the pool, then deposit");
      const calls = T.depositCalls({ pool: cfg.pool, token: cfg.token, denomination: cfg.denomination, commitment });
      const r = await wallet.sendTransactionSync({ calls });
      if (r.status !== "success") throw new Error("the deposit reverted");
      step(`The pool took ${unit(cfg.denomination)} and appended C to the pool tree and the clean tree`, {
        tx: r.transactionHash,
        detail: "The token's TIP-403 policy checked that this wallet is an authorized sender.",
        done: true,
      });
      return { note, tx: r.transactionHash };
    },

    freshRecipient: () => privateKeyToAddress(generatePrivateKey()),

    async withdraw(note, recipient, pool, step, retried = false) {
      const fee = BigInt(cfg.fee);
      await this.sweep(step);
      pool = await this.pool();
      const { index } = await this.noteStatus(note, pool);
      const { leaves, excluded } = pool.synced;
      step("Rebuilt both trees from the pool's events", {
        detail: [["deposits", leaves.length], ["excluded by the policy", indices(excluded)], ["your deposit", `#${index}`]],
        done: true,
      });
      if (excluded.includes(index)) {
        throw new Error(`Deposit #${index} is outside the clean set: the token's policy blocks its depositor. No valid proof exists for it.`);
      }
      const b = T.binding({ chainId: cfg.chainId, pool: cfg.pool, recipient, relayer: cfg.relayer, fee });
      step("Bound the proof to this exact withdrawal", {
        detail: [["recipient", short(recipient)], ["relayer", short(cfg.relayer)], ["fee", unit(fee)], ["binding", field(b)]],
        done: true,
      });
      step("Generating the Groth16 proof in this browser");
      const t0 = performance.now();
      const { proof, input } = await prove({ note: circuitNote(note), index, poolLeaves: leaves, excluded, binding: b }, PROVING);
      step(`Proof ready in ${((performance.now() - t0) / 1000).toFixed(1)} s. It reveals only its four public inputs`, {
        detail: [["pool root", field(input.poolRoot)], ["clean root", field(input.cleanRoot)], ["nullifier hash", field(input.nullifierHash)], ["binding", field(input.binding)]],
        done: true,
      });
      step("The relayer applies the policy again, then submits withdraw. The pool checks both roots, the nullifier and the proof");
      let out;
      try {
        out = await post("/api/tempo/relay", T.relayRequest({ proof: toSolidity(proof), input, recipient, fee }));
      } catch (e) {
        // An exclusion landed after this proof was made, so its clean root is
        // stale. Prove again against the new one, once.
        if (retried || !/StaleCleanRoot/.test(e.message)) throw e;
        step("The clean set changed while proving, so its root is stale. Proving again", { done: true });
        return this.withdraw(note, recipient, pool, step, true);
      }
      const received = unit(BigInt(cfg.denomination) - fee);
      step(`Accepted: ${received} paid to ${short(recipient)} and ${unit(fee)} to the relayer`, {
        tx: out.tx,
        detail: "The nullifier is now spent, so this withdrawal key can never be used again.",
        done: true,
      });
      return { tx: out.tx, received };
    },

    /** Anyone's keeper, run by the demo service: the clean set follows the token's current policy. */
    async sweep(step) {
      step("The keeper reads the token's TIP-403 policy for every depositor and applies it to the clean set");
      const r = await post("/api/tempo/sweep", {});
      const changed = r.excluded.length + r.reincluded.length > 0;
      step(changed ? "The keeper brought the clean set in line with the policy" : "The clean set already matches the policy", {
        tx: r.tx ?? undefined,
        detail: changed ? [["excluded", indices(r.excluded)], ["reincluded", indices(r.reincluded)]] : undefined,
        done: true,
      });
      return r;
    },

    async restrict(address, step) {
      step(`The issuer adds ${short(address)} to the TIP-403 blacklist, and nothing else`);
      const r = await post("/api/tempo/restrict", { address });
      step("Blocked in the token's policy. The token now refuses this address, but the pool's clean set still includes its deposits until a keeper applies the policy", {
        tx: r.tx,
        done: true,
      });
      return r;
    },

    async block(address, step) {
      step(`The issuer adds ${short(address)} to the TIP-403 blacklist and calls exclude, in one batch transaction`);
      const r = await post("/api/tempo/block", { address });
      step(`Excluded ${indices(r.excluded)} from the clean set`, {
        tx: r.tx,
        detail: "The clean epoch moved forward, so the pool now rejects every older clean root.",
        done: true,
      });
      return r;
    },

    async unblock(address, step) {
      step(`The issuer removes ${short(address)} from the blacklist and calls reinclude`);
      const r = await post("/api/tempo/unblock", { address });
      step(`Reincluded ${indices(r.reincluded)} in the clean set`, { tx: r.tx, done: true });
      return r;
    },
  };
}
