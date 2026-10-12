// Tempo actions: test funds, the relayer, the keeper's sweep, and the issuer
// blocking or unblocking a depositor. The pool and the token's policy do the
// enforcing.

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

/**
 * The keeper's rule, sent by the relayer's key: exclude every deposit whose
 * depositor the token's current policy blocks, reinclude every one it allows
 * again. exclude and reinclude are permissionless and the pool checks each
 * against the registry, so any key could send this.
 */
async function sweep() {
  const plan = await T.readSweep(publicClient, { pool: TEMPO.pool, token: TEMPO.token, fromBlock: TEMPO.deployBlock });
  const txs = [];
  for (const calls of T.sweepBatches({ pool: TEMPO.pool, ...plan })) txs.push(await sendCalls(wallet(relayerKey()), calls));
  return { tx: txs.at(-1) ?? null, txs, excluded: plan.exclude, reincluded: plan.reinclude };
}

/**
 * A sweep that tolerates one race: if another keeper excluded the same deposit
 * first, the batch reverts with AlreadyExcluded, so it plans again from the
 * new state. A second failure refuses the withdrawal rather than relaying it
 * without applying the policy.
 */
async function sweepBeforeRelay() {
  try {
    return await sweep();
  } catch {
    try {
      return await sweep();
    } catch (e) {
      throw new Error(`could not apply the issuer's policy before relaying, so the withdrawal was not sent: ${e.shortMessage ?? e.message}`, { cause: e });
    }
  }
}

/** The token's policy in force, read on every call: the issuer can switch it. */
const policyId = () => publicClient.readContract({ address: TEMPO.token, abi: T.TIP20_ABI, functionName: "transferPolicyId" });

/** The demo's own accounts, which no visitor may block: doing so would stop the pool for everyone. */
const isProtected = (a) => [TEMPO.pool, TEMPO.issuer, relayerAddress()].some((p) => p.toLowerCase() === a.toLowerCase());

const isUint = (x) => /^[0-9]{1,78}$/.test(String(x));

const chunks = (list, size) => Array.from({ length: Math.ceil(list.length / size) }, (_, i) => list.slice(i * size, (i + 1) * size));

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
    // Validate everything before spending any gas.
    if (!isUint(req.fee)) throw new Error("malformed fee");
    const fee = BigInt(req.fee);
    if (fee < BigInt(TEMPO.fee)) throw new Error(`fee below ${TEMPO.fee}`);
    if (!isAddress(req.recipient)) throw new Error("recipient is not an address");
    const points = [req.a, ...(Array.isArray(req.b) ? req.b : [null]), req.c];
    if (!Array.isArray(req.b) || req.b.length !== 2 || !points.every((p) => Array.isArray(p) && p.length === 2 && p.every(isUint))) throw new Error("malformed proof");
    if (![req.poolRoot, req.cleanRoot, req.nullifierHash].every(isUint)) throw new Error("malformed public inputs");
    // Apply the issuer's current policy first: a depositor it blocks after the
    // proof was made is excluded before the withdrawal can land.
    const swept = await sweepBeforeRelay();
    const account = Account.fromSecp256k1(relayerKey());
    const args = [
      req.a.map(BigInt), req.b.map((row) => row.map(BigInt)), req.c.map(BigInt),
      BigInt(req.poolRoot), BigInt(req.cleanRoot), BigInt(req.nullifierHash), req.recipient, account.address, fee,
    ];
    try {
      await publicClient.simulateContract({ account, address: TEMPO.pool, abi: T.POOL_ABI, functionName: "withdraw", args });
    } catch (e) {
      // Name the pool's error, such as StaleCleanRoot, so the client can act on it.
      const name = e.walk?.((x) => x.data?.errorName)?.data?.errorName;
      throw new Error(name ? `the pool refused the withdrawal: ${name}` : (e.shortMessage ?? e.message), { cause: e });
    }
    const receipt = await wallet(relayerKey()).writeContractSync({ address: TEMPO.pool, abi: T.POOL_ABI, functionName: "withdraw", args });
    if (receipt.status !== "success") throw new Error(`reverted: ${receipt.transactionHash}`);
    return { tx: receipt.transactionHash, swept };
  },

  /** Anyone's keeper: brings the clean set in line with the token's current policy. */
  sweep: () => sweep(),

  /**
   * The issuer blocks an address in the TIP-403 registry only, the way an
   * issuer that has never heard of this pool would. The pool's clean set is
   * left to the keeper.
   */
  async restrict({ address }) {
    if (!isAddress(address)) throw new Error("not an address");
    if (isProtected(address)) throw new Error("the demo's own accounts cannot be blocked");
    const { encodeFunctionData } = await import("viem");
    const data = encodeFunctionData({ abi: T.REGISTRY_ABI, functionName: "modifyPolicyBlacklist", args: [await policyId(), address, true] });
    return { tx: await sendCalls(issuer(), [{ to: T.REGISTRY, data }]) };
  },

  /** The issuer blocks an address and excludes its clean deposits, in one batch transaction. */
  async block({ address }) {
    if (!isAddress(address)) throw new Error("not an address");
    if (isProtected(address)) throw new Error("the demo's own accounts cannot be blocked");
    const synced = await T.syncPool(publicClient, { pool: TEMPO.pool, fromBlock: TEMPO.deployBlock });
    const indices = synced.depositors
      .map((d, i) => (d.toLowerCase() === address.toLowerCase() && !synced.excluded.includes(i) ? i : -1))
      .filter((i) => i >= 0);
    // The block and the first three excludes go in one batch, closing the race
    // for them; any further excludes follow in batches of four, since each
    // costs about 1.5M gas.
    const [first, ...rest] = [indices.slice(0, 3), ...chunks(indices.slice(3), 4)];
    let tx = await sendCalls(issuer(), T.blockAndExcludeCalls({ pool: TEMPO.pool, policy: await policyId(), account: address, indices: first }));
    for (const part of rest) tx = await sendCalls(issuer(), T.sweepCalls({ pool: TEMPO.pool, exclude: part, reinclude: [] }));
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
    const unblock = { to: T.REGISTRY, data: encodeFunctionData({ abi: T.REGISTRY_ABI, functionName: "modifyPolicyBlacklist", args: [await policyId(), address, false] }) };
    const [first, ...rest] = [indices.slice(0, 3), ...chunks(indices.slice(3), 4)];
    let tx = await sendCalls(issuer(), [unblock, ...T.sweepCalls({ pool: TEMPO.pool, exclude: [], reinclude: first })]);
    for (const part of rest) tx = await sendCalls(issuer(), T.sweepCalls({ pool: TEMPO.pool, exclude: [], reinclude: part }));
    return { tx, reincluded: indices };
  },
};
