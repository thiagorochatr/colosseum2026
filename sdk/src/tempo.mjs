// Tempo-side helpers: the action binding, and rebuilding a pool's trees from
// its events. See docs/SPEC.md sections 6 and 7.

import { encodePacked, keccak256, parseAbiItem } from "viem";

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
