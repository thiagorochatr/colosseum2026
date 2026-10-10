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
export async function syncPool(publicClient, { pool, fromBlock }) {
  const range = { address: pool, fromBlock: BigInt(fromBlock), toBlock: "latest" };
  const [deposits, exclusions, reinclusions] = await Promise.all([
    publicClient.getLogs({ ...range, event: DEPOSIT }),
    publicClient.getLogs({ ...range, event: EXCLUDED }),
    publicClient.getLogs({ ...range, event: REINCLUDED }),
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
