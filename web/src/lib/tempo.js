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
import { post, PROVING } from "./api.js";

export function tempoChain(cfg) {
  const publicClient = createPublicClient({ chain: tempoModerato, transport: http(cfg.rpc) });
  const key = burner("tempo", generatePrivateKey);
  const me = privateKeyToAddress(key);
  const wallet = createClient({ account: Account.fromSecp256k1(key), chain: tempoModerato, transport: http(cfg.rpc), feeToken: "0x20C0000000000000000000000000000000000000" });
  const read = (functionName, args = []) => publicClient.readContract({ address: cfg.pool, abi: T.POOL_ABI, functionName, args });
  const unit = (v) => `${(Number(v) / 1e6).toLocaleString()} dUSD`;

  return {
    id: "tempo",
    me,
    unit,
    denominationLabel: unit(cfg.denomination),
    txLink: (h) => `${cfg.explorer}/tx/${h}`,
    addressLink: (a) => `${cfg.explorer}/address/${a}`,

    async balance() {
      const v = await publicClient.readContract({ address: cfg.token, abi: T.TIP20_ABI, functionName: "balanceOf", args: [me] });
      return unit(v);
    },
    faucet: () => post("/api/tempo/faucet", { address: me }),

    /** Every deposit, with its depositor and whether it is clean. */
    async pool() {
      const s = await T.syncPool(publicClient, { pool: cfg.pool, fromBlock: cfg.deployBlock });
      return {
        deposits: s.leaves.map((leaf, i) => ({ index: i, leaf, depositor: s.depositors[i], clean: !s.excluded.includes(i) })),
        synced: s,
      };
    },

    async noteStatus(note, pool) {
      const c = await commitmentOf(note);
      const index = pool.synced.leaves.findIndex((l) => l === c);
      const spent = index >= 0 && (await read("spent", [await nullifierOf(note)]));
      if (index < 0) return { index, label: "Not deposited" };
      if (spent) return { index, label: "Withdrawn", tone: "done" };
      if (pool.synced.excluded.includes(index)) return { index, label: "Excluded", tone: "bad" };
      return { index, label: "Clean", tone: "good", canWithdraw: true };
    },

    async deposit() {
      const note = createNote({ chain: "tempo", network: "moderato", pool: cfg.pool, denomination: BigInt(cfg.denomination) });
      const calls = T.depositCalls({ pool: cfg.pool, token: cfg.token, denomination: cfg.denomination, commitment: await commitmentOf(note) });
      const r = await wallet.sendTransactionSync({ calls });
      if (r.status !== "success") throw new Error("the deposit reverted");
      return { note, tx: r.transactionHash };
    },

    freshRecipient: () => privateKeyToAddress(generatePrivateKey()),

    async withdraw(note, recipient, pool, onStage) {
      const fee = BigInt(cfg.fee);
      const { index } = await this.noteStatus(note, pool);
      onStage("Proving in your browser");
      const b = T.binding({ chainId: cfg.chainId, pool: cfg.pool, recipient, relayer: cfg.relayer, fee });
      const { proof, input } = await prove({ note: circuitNote(note), index, poolLeaves: pool.synced.leaves, excluded: pool.synced.excluded, binding: b }, PROVING);
      onStage("Sending to the relayer");
      const out = await post("/api/tempo/relay", T.relayRequest({ proof: toSolidity(proof), input, recipient, fee }));
      return { tx: out.tx, received: unit(BigInt(cfg.denomination) - fee) };
    },

    block: (address) => post("/api/tempo/block", { address }),
    unblock: (address) => post("/api/tempo/unblock", { address }),
  };
}
