// The curator, off-chain. See docs/SPEC.md section 8.3.
//
//   node curator.mjs init              create the curator account, bound to the genesis policy's hash
//   node curator.mjs publish           rebuild the pool, apply the current policy, publish the clean root
//   node curator.mjs block <address>   add a depositor to the blocklist: new epoch, new policy, new root
//   node curator.mjs unblock <address> remove a depositor from the blocklist
//   node curator.mjs revoke            start a new epoch, invalidating every root so far
//
// Every publication writes, in one transaction, the policy's hash, the clean
// root and a memo carrying the policy document and the clean set. The genesis
// policy is deployments/<network>-policy.json; after that the policy in force
// is the one in the latest memo the curator account stands behind.

import { cleanTree } from "../../sdk/src/tree.mjs";
import * as S from "../../sdk/src/solana.mjs";
import {
  NETWORK, PublicKey, connection, demoKeypair, explorer, ixs, readDeployment, readKeypair, send, syncPool,
  curatorAddress, poolAddress,
} from "./common.mjs";

export function curatorKeypair() {
  return process.env.CURATOR_KEYPAIR ? readKeypair(process.env.CURATOR_KEYPAIR) : demoKeypair("curator");
}

const programs = () => {
  const d = readDeployment();
  const poolProgram = new PublicKey(d.poolProgram);
  return { d, compliance: new PublicKey(d.complianceProgram), poolProgram, pool: poolAddress(poolProgram, BigInt(d.denomination)) };
};

/** The genesis policy: the blocklist in deployments/<network>-policy.json. */
export const genesisBlocklist = () => readDeployment("policy").blocklist;

export async function currentBlocklist() {
  const { compliance, pool } = programs();
  const set = await S.readCleanSet(connection, compliance, curatorKeypair().publicKey, pool);
  return set?.blocklist ?? genesisBlocklist();
}

export async function init() {
  const { compliance } = programs();
  const curator = curatorKeypair();
  const account = curatorAddress(compliance, curator.publicKey);
  if (await connection.getAccountInfo(account)) return { account, existed: true };
  const hash = S.policyHash(S.policyDocument(genesisBlocklist()));
  const sig = await send([ixs.initCurator(compliance, curator.publicKey, hash)], [curator]);
  return { account, sig, existed: false };
}

/** Applies `blocklist` (default: the policy in force) and publishes the clean root. */
export async function publish(blocklist, { revoke = false } = {}) {
  const { compliance, poolProgram, pool } = programs();
  const curator = curatorKeypair();
  const policy = S.policyDocument(blocklist ?? (await currentBlocklist()));
  const { leaves, depositors } = await syncPool(poolProgram, pool);
  const blocked = new Set(policy.blocklist);
  const excluded = depositors.map((who, i) => (blocked.has(who) ? i : -1)).filter((i) => i >= 0);
  const cleanRoot = (await cleanTree(leaves, excluded)).root();
  const sig = await send(
    S.publishInstructions({ compliance, curator: curator.publicKey, pool, depositCount: leaves.length, excluded, blocklist: policy.blocklist, cleanRoot, revoke }),
    [curator],
  );
  return { sig, depositCount: leaves.length, excluded, cleanRoot, policy, leaves };
}

export async function block(address) {
  return publish([...(await currentBlocklist()), address], { revoke: true });
}

export async function unblock(address) {
  return publish((await currentBlocklist()).filter((a) => a !== address));
}

export async function revoke() {
  const { compliance } = programs();
  const curator = curatorKeypair();
  return { sig: await send([ixs.revoke(compliance, curator.publicKey)], [curator]) };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const [cmd, arg] = process.argv.slice(2);
  const run = { init, publish: () => publish(), block: () => block(arg), unblock: () => unblock(arg), revoke }[cmd];
  if (!run || ((cmd === "block" || cmd === "unblock") && !arg)) {
    console.error("usage: node curator.mjs init | publish | block <address> | unblock <address> | revoke");
    process.exit(2);
  }
  console.log(`network: ${NETWORK}`);
  const out = await run();
  if (out.sig) console.log(`${cmd}: ${explorer(out.sig)}`);
  if (out.excluded) console.log(`clean root over ${out.depositCount} deposits, excluding [${out.excluded}]`);
}
