// A keeper: keeps the pool's clean set in line with the token's TIP-403 policy,
// for issuers that block an address without calling exclude in the same batch.
//
//   NETWORK=moderato node keeper/keeper.mjs            (watch, until stopped)
//   NETWORK=moderato node keeper/keeper.mjs --once     (one sweep, then exit)
//
// It watches the registry's BlacklistUpdated and WhitelistUpdated events and
// sweeps as soon as one names a depositor, and sweeps every minute anyway, in
// case the issuer switched the token to another policy. exclude and reinclude
// are permissionless, and the pool checks each against the registry, so the
// keeper needs no role: any funded key will do. KEEPER_KEY, or the demo relayer.

import { parseAbi } from "viem";
import * as T from "compliance-sdk/tempo";
import { demoKeys, publicClient, readState, txLink, wallet } from "../e2e/common.mjs";

const POLICY_EVENTS = parseAbi([
  "event BlacklistUpdated(uint64 indexed policyId, address indexed updater, address indexed account, bool restricted)",
  "event WhitelistUpdated(uint64 indexed policyId, address indexed updater, address indexed account, bool allowed)",
]);
const EVERY = 60_000;

const { pool, deployBlock } = readState();
const { token } = readState("issuer");
const key = process.env.KEEPER_KEY ?? demoKeys().relayer;
const at = () => new Date().toISOString().slice(11, 19);

/** One sweep: read the plan, and send it in batches of up to four calls if it has work. */
export async function sweepOnce() {
  const plan = await T.readSweep(publicClient, { pool, token, fromBlock: deployBlock });
  const batches = T.sweepBatches({ pool, ...plan });
  if (!batches.length) return { plan, depositors: plan.synced.depositors };
  let tx;
  for (const calls of batches) {
    const receipt = await wallet(key).sendTransactionSync({ calls });
    if (receipt.status !== "success") throw new Error(`sweep reverted: ${receipt.transactionHash}`);
    tx = receipt.transactionHash;
    console.log(`${at()} sent ${calls.length} of the sweep's calls: ${txLink(tx)}`);
  }
  console.log(`${at()} excluded [${plan.exclude}] reincluded [${plan.reinclude}]`);
  return { plan, depositors: plan.synced.depositors, tx };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  let depositors = new Set();
  let running = Promise.resolve();
  let failed = false;
  // Sweeps run one at a time; a sweep that fails, say on a race with another
  // keeper, is logged and retried on the next event or tick.
  const sweep = (why) =>
    (running = running.then(async () => {
      try {
        const r = await sweepOnce();
        depositors = new Set(r.depositors.map((d) => d.toLowerCase()));
        if (!r.tx && why !== "tick") console.log(`${at()} ${why}: the clean set already matches the policy`);
      } catch (e) {
        failed = true;
        console.error(`${at()} sweep failed (${why}): ${e.shortMessage ?? e.message}`);
      }
    }));

  await sweep("start");
  // With --once, a failed sweep exits non-zero so a scheduler can see it.
  if (process.argv.includes("--once")) process.exit(failed ? 1 : 0);

  console.log(`${at()} watching the TIP-403 registry for ${depositors.size} depositors of ${pool}`);
  // Polls getLogs block by block: Moderato's RPC does not keep the event
  // filters viem's watchers rely on.
  // One poll at a time, and at most 100k blocks per query, Moderato's cap, so
  // that catching up after a long pause never wedges the loop.
  let last = await publicClient.getBlockNumber();
  let polling = false;
  setInterval(async () => {
    if (polling) return;
    polling = true;
    try {
      const head = await publicClient.getBlockNumber();
      if (head <= last) return;
      const to = head - last > 100_000n ? last + 100_000n : head;
      const logs = await publicClient.getLogs({ address: T.REGISTRY, events: POLICY_EVENTS, fromBlock: last + 1n, toBlock: to });
      last = to;
      const hit = logs.find((l) => depositors.has(l.args.account?.toLowerCase()));
      if (hit) sweep(`${hit.eventName} for ${hit.args.account} in block ${hit.blockNumber}`);
    } catch (e) {
      console.error(`${at()} watch: ${e.shortMessage ?? e.message}`);
    } finally {
      polling = false;
    }
  }, 2_000);
  setInterval(() => sweep("tick"), EVERY);
}
