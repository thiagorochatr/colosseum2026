// Tempo actions: test funds, the relayer, and the issuer blocking or
// unblocking a depositor. The pool and the token's policy do the enforcing.

import { createPublicClient, defineChain, http } from "viem";
import { tempoModerato } from "viem/chains";
import { Account, createClient } from "viem/tempo";
import * as T from "compliance-sdk/tempo";
import { TEMPO, secret, isAddress } from "./common.mjs";

const chain = TEMPO.rpc === tempoModerato.rpcUrls.default.http[0] ? tempoModerato : defineChain({ ...tempoModerato, rpcUrls: { default: { http: [TEMPO.rpc] } } });
const publicClient = createPublicClient({ chain, transport: http(TEMPO.rpc) });
const wallet = (key) => createClient({ account: Account.fromSecp256k1(key), chain, transport: http(TEMPO.rpc), feeToken: "0x20C0000000000000000000000000000000000000" });
const issuer = () => wallet(secret("TEMPO_ISSUER_KEY"));
const relayerKey = () => secret("TEMPO_RELAYER_KEY");
export const relayerAddress = () => Account.fromSecp256k1(relayerKey()).address;
const MINT = 300_000_000n;

const balance = (who) => publicClient.readContract({ address: TEMPO.token, abi: T.TIP20_ABI, functionName: "balanceOf", args: [who] });

async function sendCalls(client, calls) {
  const receipt = await client.sendTransactionSync({ calls });
  if (receipt.status !== "success") throw new Error(`reverted: ${receipt.transactionHash}`);
  return receipt.transactionHash;
}

export const tempoActions = {
  /** Gas stablecoins from Moderato's faucet, and Demo USD from the issuer. */
  async faucet({ address }) {
    if (!isAddress(address)) throw new Error("not an address");
    await publicClient.request({ method: "tempo_fundAddress", params: [address] });
    if ((await balance(address)) >= 2n * BigInt(TEMPO.denomination)) return { minted: "0", note: "already funded" };
    const tx = await sendCalls(issuer(), [
      { to: TEMPO.token, data: (await import("viem")).encodeFunctionData({ abi: [{ type: "function", name: "mint", inputs: [{ type: "address" }, { type: "uint256" }], outputs: [], stateMutability: "nonpayable" }], functionName: "mint", args: [address, MINT] }) },
    ]);
    return { minted: MINT.toString(), tx };
  },

  /** Submits a withdrawal and pays the gas; the pool pays the fee bound into the proof. */
  async relay(req) {
    const fee = BigInt(req.fee);
    if (fee < BigInt(TEMPO.fee)) throw new Error(`fee below ${TEMPO.fee}`);
    if (!isAddress(req.recipient)) throw new Error("recipient is not an address");
    const account = Account.fromSecp256k1(relayerKey());
    const args = [
      req.a.map(BigInt), req.b.map((row) => row.map(BigInt)), req.c.map(BigInt),
      BigInt(req.poolRoot), BigInt(req.cleanRoot), BigInt(req.nullifierHash), req.recipient, account.address, fee,
    ];
    await publicClient.simulateContract({ account, address: TEMPO.pool, abi: T.POOL_ABI, functionName: "withdraw", args });
    const receipt = await wallet(relayerKey()).writeContractSync({ address: TEMPO.pool, abi: T.POOL_ABI, functionName: "withdraw", args });
    if (receipt.status !== "success") throw new Error(`reverted: ${receipt.transactionHash}`);
    return { tx: receipt.transactionHash };
  },

  /** The issuer blocks an address and excludes its clean deposits, in one batch transaction. */
  async block({ address }) {
    if (!isAddress(address)) throw new Error("not an address");
    const synced = await T.syncPool(publicClient, { pool: TEMPO.pool, fromBlock: TEMPO.deployBlock });
    const indices = synced.depositors
      .map((d, i) => (d.toLowerCase() === address.toLowerCase() && !synced.excluded.includes(i) ? i : -1))
      .filter((i) => i >= 0);
    const tx = await sendCalls(issuer(), T.blockAndExcludeCalls({ pool: TEMPO.pool, policy: TEMPO.policy, account: address, indices }));
    return { tx, excluded: indices };
  },

  /** The issuer unblocks an address and its deposits are reincluded, in one batch transaction. */
  async unblock({ address }) {
    if (!isAddress(address)) throw new Error("not an address");
    const synced = await T.syncPool(publicClient, { pool: TEMPO.pool, fromBlock: TEMPO.deployBlock });
    const indices = synced.depositors
      .map((d, i) => (d.toLowerCase() === address.toLowerCase() && synced.excluded.includes(i) ? i : -1))
      .filter((i) => i >= 0);
    const { encodeFunctionData } = await import("viem");
    const calls = [
      { to: T.REGISTRY, data: encodeFunctionData({ abi: T.REGISTRY_ABI, functionName: "modifyPolicyBlacklist", args: [BigInt(TEMPO.policy), address, false] }) },
      ...indices.map((i) => ({ to: TEMPO.pool, data: encodeFunctionData({ abi: T.POOL_ABI, functionName: "reinclude", args: [BigInt(i)] }) })),
    ];
    return { tx: await sendCalls(issuer(), calls), reincluded: indices };
  },
};
