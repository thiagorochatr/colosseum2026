// Tempo-side helpers: the action binding, and rebuilding a pool's trees from
// its events. See docs/SPEC.md sections 6 and 7.

import { encodeFunctionData, encodePacked, keccak256, parseAbi, parseAbiItem } from "viem";

export const BINDING_DOMAIN = "compliance-pool:tempo:withdraw:v1";
const MASK_248 = (1n << 248n) - 1n;

/** keccak256(abi.encodePacked(domain, chainId, pool, recipient, relayer, fee)) with its first byte cleared. */
export function binding({ chainId, pool, recipient, relayer, fee }) {
  const d = keccak256(
    encodePacked(
      ["string", "uint256", "address", "address", "address", "uint256"],
      [BINDING_DOMAIN, BigInt(chainId), pool, recipient, relayer, BigInt(fee)],
    ),
  );
  return BigInt(d) & MASK_248;
}

const DEPOSIT = parseAbiItem(
  "event Deposit(uint256 indexed index, uint256 commitment, address indexed depositor, uint256 poolRoot, uint256 cleanRoot)",
);
const EXCLUDED = parseAbiItem(
  "event Excluded(uint256 indexed index, address indexed depositor, uint256 cleanRoot, uint64 cleanEpoch)",
);
const REINCLUDED = parseAbiItem("event Reincluded(uint256 indexed index, uint256 cleanRoot)");

/**
 * Rebuilds the pool's leaves and its excluded set from events, in order.
 * Returns the last roots the events announced so a caller can check them.
 */
/** getLogs over [from, latest] in chunks, since public RPCs cap the block range. */
async function logsSince(publicClient, address, event, fromBlock, chunk = 100_000n) {
  const latest = await publicClient.getBlockNumber();
  const out = [];
  for (let from = BigInt(fromBlock); from <= latest; from += chunk) {
    const to = from + chunk - 1n < latest ? from + chunk - 1n : latest;
    out.push(...(await publicClient.getLogs({ address, event, fromBlock: from, toBlock: to })));
  }
  return out;
}

export async function syncPool(publicClient, { pool, fromBlock }) {
  const [deposits, exclusions, reinclusions] = await Promise.all([
    logsSince(publicClient, pool, DEPOSIT, fromBlock),
    logsSince(publicClient, pool, EXCLUDED, fromBlock),
    logsSince(publicClient, pool, REINCLUDED, fromBlock),
  ]);
  const leaves = [];
  for (const log of deposits) {
    const i = Number(log.args.index);
    if (i !== leaves.length) throw new Error(`deposit ${i} arrived out of order`);
    leaves.push(log.args.commitment);
  }
  // Exclusions and reinclusions in chain order decide the current mask.
  const order = (l) => [l.blockNumber, l.logIndex];
  const events = [
    ...exclusions.map((l) => ({ l, excluded: true })),
    ...reinclusions.map((l) => ({ l, excluded: false })),
  ].sort((a, b) => {
    const [ab, ai] = order(a.l);
    const [bb, bi] = order(b.l);
    return ab === bb ? ai - bi : ab < bb ? -1 : 1;
  });
  const excluded = new Set();
  for (const { l, excluded: ex } of events) {
    const i = Number(l.args.index);
    if (ex) excluded.add(i);
    else excluded.delete(i);
  }
  return { leaves, excluded: [...excluded].sort((a, b) => a - b), depositors: deposits.map((l) => l.args.depositor) };
}

// ------------------------------------------------------------- the pool's ABI


export const POOL_ABI = parseAbi([
  "function deposit(uint256 commitment) returns (uint256 index)",
  "function exclude(uint256 index)",
  "function reinclude(uint256 index)",
  "function withdraw(uint256[2] a, uint256[2][2] b, uint256[2] c, uint256 poolRoot, uint256 cleanRoot, uint256 nullifierHash, address recipient, address relayer, uint256 fee)",
  "function bindingFor(address recipient, address relayer, uint256 fee) view returns (uint256)",
  "function currentPoolRoot() view returns (uint256)",
  "function currentCleanRoot() view returns (uint256)",
  "function cleanLeaf(uint256 index) view returns (uint256)",
  "function cleanEpoch() view returns (uint64)",
  "function nextIndex() view returns (uint256)",
  "function spent(uint256 nullifierHash) view returns (bool)",
  "function denomination() view returns (uint256)",
  "function token() view returns (address)",
  "function depositor(uint256 index) view returns (address)",
  "event Deposit(uint256 indexed index, uint256 commitment, address indexed depositor, uint256 poolRoot, uint256 cleanRoot)",
  "event Excluded(uint256 indexed index, address indexed depositor, uint256 cleanRoot, uint64 cleanEpoch)",
  "event Reincluded(uint256 indexed index, uint256 cleanRoot)",
  "event Withdrawal(uint256 indexed nullifierHash, address indexed recipient, address indexed relayer, uint256 fee)",
  "error InvalidCommitment()",
  "error PoolFull()",
  "error TransferFailed()",
  "error UnknownIndex()",
  "error AlreadyExcluded()",
  "error NotExcluded()",
  "error StillAuthorized()",
  "error StillUnauthorized()",
  "error InvalidRecipient()",
  "error FeeTooHigh()",
  "error RelayerRequired()",
  "error NullifierSpent()",
  "error UnknownPoolRoot()",
  "error StaleCleanRoot()",
  "error InvalidProof()",
]);

export const TIP20_ABI = parseAbi([
  "function approve(address spender, uint256 amount) returns (bool)",
  "function balanceOf(address account) view returns (uint256)",
  "function transferPolicyId() view returns (uint64)",
  "function decimals() view returns (uint8)",
  "function symbol() view returns (string)",
]);

export const REGISTRY = "0x403c000000000000000000000000000000000000";
export const REGISTRY_ABI = parseAbi([
  "function modifyPolicyBlacklist(uint64 policyId, address account, bool restricted)",
  "function isAuthorizedSender(uint64 policyId, address user) view returns (bool)",
]);

// ------------------------------------------------------------------ the keeper

/**
 * The pool's rule, applied: which deposits to exclude and which to reinclude
 * so that the clean set matches the token's current policy. `authorized(a)`
 * says whether depositor `a` is an authorized sender under that policy.
 */
export function planSweep({ depositors, excluded }, authorized) {
  const out = new Set(excluded);
  const plan = { exclude: [], reinclude: [] };
  depositors.forEach((d, i) => {
    const ok = authorized(d);
    if (!ok && !out.has(i)) plan.exclude.push(i);
    if (ok && out.has(i)) plan.reinclude.push(i);
  });
  return plan;
}

/**
 * Reads the pool and the token's current policy and plans a sweep. The policy
 * is read from the token on every call, since the issuer can switch it. Every
 * read is a view call: only a plan with work in it costs gas, once sent.
 */
export async function readSweep(publicClient, { pool, token, fromBlock, synced }) {
  synced ??= await syncPool(publicClient, { pool, fromBlock });
  const policy = await publicClient.readContract({ address: token, abi: TIP20_ABI, functionName: "transferPolicyId" });
  const unique = [...new Set(synced.depositors.map((d) => d.toLowerCase()))];
  const answers = await Promise.all(
    unique.map((a) => publicClient.readContract({ address: REGISTRY, abi: REGISTRY_ABI, functionName: "isAuthorizedSender", args: [policy, a] })),
  );
  const ok = new Map(unique.map((a, i) => [a, answers[i]]));
  const authorized = (a) => ok.get(a.toLowerCase());
  return { ...planSweep(synced, authorized), policy, synced, authorized };
}

/** A sweep as one batch transaction: every exclude and reinclude it needs. */
export function sweepCalls({ pool, exclude, reinclude }) {
  const call = (functionName, i) => ({ to: pool, data: encodeFunctionData({ abi: POOL_ABI, functionName, args: [BigInt(i)] }) });
  return [...exclude.map((i) => call("exclude", i)), ...reinclude.map((i) => call("reinclude", i))];
}

/** Approve and deposit as ONE Tempo batch transaction: the user signs once. */
export function depositCalls({ pool, token, denomination, commitment }) {
  return [
    { to: token, data: encodeFunctionData({ abi: TIP20_ABI, functionName: "approve", args: [pool, BigInt(denomination)] }) },
    { to: pool, data: encodeFunctionData({ abi: POOL_ABI, functionName: "deposit", args: [BigInt(commitment)] }) },
  ];
}

/** The issuer's batch: block an address and exclude its deposits, atomically. */
export function blockAndExcludeCalls({ pool, policy, account, indices }) {
  return [
    { to: REGISTRY, data: encodeFunctionData({ abi: REGISTRY_ABI, functionName: "modifyPolicyBlacklist", args: [BigInt(policy), account, true] }) },
    ...indices.map((i) => ({ to: pool, data: encodeFunctionData({ abi: POOL_ABI, functionName: "exclude", args: [BigInt(i)] }) })),
  ];
}

/** The body the relayer's POST /relay expects, from a Solidity-layout proof. */
export function relayRequest({ proof, input, recipient, fee }) {
  const s = (x) => BigInt(x).toString();
  return {
    a: proof.a.map(s),
    b: proof.b.map((row) => row.map(s)),
    c: proof.c.map(s),
    poolRoot: s(input.poolRoot),
    cleanRoot: s(input.cleanRoot),
    nullifierHash: s(input.nullifierHash),
    recipient,
    fee: s(fee),
  };
}
