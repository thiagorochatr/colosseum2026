// The issuer's side, before the pool exists: create a USD stablecoin, put it
// under a blacklist policy the issuer administers, and mint to three users.
//
//   NETWORK=local node e2e/setup-issuer.mjs

import { abi, address, demoKeys, FACTORY, fund, NETWORK, PATH_USD, publicClient, REGISTRY, send, txLink, writeState } from "./common.mjs";

const keys = demoKeys();
const issuer = keys.issuer;
const ITIP20 = abi("ITIP20");
const FACTORY_ABI = abi("ITIP20Factory");
const REGISTRY_ABI = abi("ITIP403Registry");
const ROLES = abi("ITIP20RolesAuth");

for (const role of Object.keys(keys)) await fund(address(keys[role]));
if (NETWORK === "moderato") await new Promise((r) => setTimeout(r, 3000));

const salt = `0x${Date.now().toString(16).padStart(64, "0")}`;
const { result: token, receipt: created } = await send(issuer, {
  address: FACTORY,
  abi: FACTORY_ABI,
  functionName: "createToken",
  args: ["Demo USD", "dUSD", "USD", PATH_USD, address(issuer), salt],
});
console.log(`token ${token}: ${txLink(created.transactionHash)}`);

const issuerRole = await publicClient.readContract({
  address: token,
  abi: ITIP20,
  functionName: "ISSUER_ROLE",
});
await send(issuer, { address: token, abi: ROLES, functionName: "grantRole", args: [issuerRole, address(issuer)] });

const { result: policy, receipt: policyTx } = await send(issuer, {
  address: REGISTRY,
  abi: REGISTRY_ABI,
  functionName: "createPolicy",
  args: [address(issuer), 1],
});
console.log(`blacklist policy ${policy}: ${txLink(policyTx.transactionHash)}`);
await send(issuer, { address: token, abi: ITIP20, functionName: "changeTransferPolicyId", args: [policy] });

for (const user of ["alice", "bob", "carol"]) {
  await send(issuer, { address: token, abi: ITIP20, functionName: "mint", args: [address(keys[user]), 1_000_000_000n] });
}

writeState("issuer", {
  network: NETWORK,
  token,
  policy,
  issuer: address(issuer),
  users: Object.fromEntries(["alice", "bob", "carol", "relayer"].map((u) => [u, address(keys[u])])),
  createTokenTx: created.transactionHash,
  createPolicyTx: policyTx.transactionHash,
});
console.log("issuer ready; deploy the pool with POOL_TOKEN=" + token);
