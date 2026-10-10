// The curator, off-chain. See docs/SPEC.md section 8.3.
//
//   node curator.mjs init                    create the curator account, bound to the policy's hash
//   node curator.mjs publish                 rebuild the pool, apply the policy, publish the clean root
//   node curator.mjs revoke                  start a new epoch, invalidating every root so far
//
// The policy is deployments/<network>-policy.json: a blocklist of depositor
// addresses. Its sha256 is stored on-chain when the curator is created, and the
// clean set is published as deployments/<network>-curator.json.

import { existsSync } from "node:fs";
import { cleanTree } from "../../sdk/src/tree.mjs";
import {
  NETWORK, PublicKey, demoKeypair, deploymentFile, explorer, ixs, policyHash, readDeployment, readKeypair, send,
  syncPool, writeDeployment, connection, curatorAddress,
} from "./common.mjs";

export function curatorKeypair() {
  return process.env.CURATOR_KEYPAIR ? readKeypair(process.env.CURATOR_KEYPAIR) : demoKeypair("curator");
}

export function readPolicy() {
  const file = deploymentFile("policy");
  if (!existsSync(file)) throw new Error(`no policy at ${file}`);
  return readDeployment("policy");
}

export async function init() {
  const d = readDeployment();
  const curator = curatorKeypair();
  const compliance = new PublicKey(d.complianceProgram);
  const account = curatorAddress(compliance, curator.publicKey);
  if (await connection.getAccountInfo(account)) return { account, existed: true };
  const hash = policyHash(readPolicy());
  const sig = await send([ixs.initCurator(compliance, curator.publicKey, hash)], [curator]);
  console.log(`curator ${account.toBase58()} created: ${explorer(sig)}`);
  return { account, sig, existed: false };
}

/** Applies the policy to the pool's depositors and publishes the clean root. */
export async function publish() {
  const d = readDeployment();
  const policy = readPolicy();
  const curator = curatorKeypair();
  const compliance = new PublicKey(d.complianceProgram);
  const pool = new PublicKey(d.pool);
  const { leaves, depositors } = await syncPool(new PublicKey(d.poolProgram), pool);
  const blocked = new Set(policy.blocklist);
  const excluded = depositors.map((who, i) => (blocked.has(who) ? i : -1)).filter((i) => i >= 0);
  const clean = await cleanTree(leaves, excluded);
  const cleanRoot = clean.root();
  const sig = await send([ixs.publishRoot(compliance, curator.publicKey, cleanRoot)], [curator]);
  const file = {
    version: 1,
    pool: pool.toBase58(),
    curator: curator.publicKey.toBase58(),
    policy_hash: policyHash(policy).toString("hex"),
    deposit_count: leaves.length,
    excluded,
    clean_root: cleanRoot.toString(16).padStart(64, "0"),
    published_in: sig,
  };
  writeDeployment("curator", file);
  return { ...file, sig, leaves };
}

export async function revoke() {
  const d = readDeployment();
  const curator = curatorKeypair();
  const sig = await send([ixs.revoke(new PublicKey(d.complianceProgram), curator.publicKey)], [curator]);
  console.log(`revoked: ${explorer(sig)}`);
  return sig;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const cmd = process.argv[2];
  const run = { init, publish, revoke }[cmd];
  if (!run) {
    console.error("usage: node curator.mjs init|publish|revoke");
    process.exit(2);
  }
  console.log(`network: ${NETWORK}`);
  const out = await run();
  if (cmd === "publish") {
    console.log(`published clean root over ${out.deposit_count} deposits, excluding [${out.excluded}]: ${explorer(out.sig)}`);
  }
}
