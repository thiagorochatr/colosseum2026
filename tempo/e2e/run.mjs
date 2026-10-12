// The whole flow on a real Tempo node, after setup-issuer.mjs and Deploy.s.sol.
//
//   NETWORK=local node e2e/run.mjs
//
// 1. Alice, Bob and Carol each deposit one note.
// 2. The issuer blocks Bob and excludes his deposit in ONE Tempo batch transaction.
// 3. Alice and Carol withdraw through the relayer to fresh, unfunded addresses.
// 4. Bob's note cannot be proved any more.
//
// Every transaction is recorded in deployments/<chainId>-e2e.json.

import { randomBytes } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { encodeFunctionData } from "viem";
import { generatePrivateKey, privateKeyToAddress } from "viem/accounts";
import { hasher, FIELD } from "../../sdk/src/tree.mjs";
import { prove, toSolidity, terminate } from "../../sdk/src/prove.mjs";
import { binding, syncPool } from "../../sdk/src/tempo.mjs";
import { serve } from "../relayer/relayer.mjs";
import { abi, address, chain, demoKeys, NETWORK, publicClient, readState, REGISTRY, ROOT, send, txLink, wallet, writeState } from "./common.mjs";

const keys = demoKeys();
const dep = readState();
const iss = readState("issuer");
const POOL = abi("CompliantPool");
const ITIP20 = abi("ITIP20");
const REGISTRY_ABI = abi("ITIP403Registry");
const FEE = 1_000_000n;
const proving = {
  wasm: `${ROOT}../circuits/build/withdraw_js/withdraw.wasm`,
  zkey: `${ROOT}../circuits/ceremony/withdraw_final.zkey`,
};
const read = (functionName, args = []) => publicClient.readContract({ address: dep.pool, abi: POOL, functionName, args });
const log = [];
const record = (step, receipt, extra = {}) => {
  log.push({ step, tx: receipt.transactionHash, link: txLink(receipt.transactionHash), gasUsed: receipt.gasUsed, ...extra });
  console.log(`${step}: ${txLink(receipt.transactionHash)} (gas ${receipt.gasUsed})`);
};

// ------------------------------------------------------------------ 1. deposits
const { h3 } = await hasher();
const rand = () => BigInt(`0x${randomBytes(32).toString("hex")}`) % FIELD;
const notes = {};
for (const user of ["alice", "bob", "carol"]) {
  const note = { k: rand(), r: rand(), denomTag: BigInt(dep.denomination) };
  const commitment = h3(note.k, note.r, note.denomTag);
  await send(keys[user], { address: iss.token, abi: ITIP20, functionName: "approve", args: [dep.pool, BigInt(dep.denomination)] });
  const { result: index, receipt } = await send(keys[user], { address: dep.pool, abi: POOL, functionName: "deposit", args: [commitment] });
  notes[user] = { ...note, index: Number(index) };
  record(`deposit by ${user}`, receipt, { index: Number(index) });
}
// The notes are the secrets that release the escrow; they stay on this machine.
mkdirSync(`${ROOT}.demo`, { recursive: true });
writeFileSync(`${ROOT}.demo/${chain.id}-notes.json`, JSON.stringify(notes, (_, v) => (typeof v === "bigint" ? v.toString() : v), 2), { mode: 0o600 });

// ------------------------------------------- 2. block and exclude, atomically
const epochBefore = await read("cleanEpoch");
const batch = await wallet(keys.issuer).sendTransactionSync({
  calls: [
    { to: REGISTRY, data: encodeFunctionData({ abi: REGISTRY_ABI, functionName: "modifyPolicyBlacklist", args: [iss.policy, address(keys.bob), true] }) },
    { to: dep.pool, data: encodeFunctionData({ abi: POOL, functionName: "exclude", args: [BigInt(notes.bob.index)] }) },
  ],
});
if (batch.status !== "success") throw new Error("the batch reverted");
record("issuer blocks Bob and excludes his deposit, one batch transaction", batch);
if ((await read("cleanLeaf", [BigInt(notes.bob.index)])) !== 0n) throw new Error("Bob's leaf is still clean");
if ((await read("cleanEpoch")) !== epochBefore + 1n) throw new Error("the clean epoch did not move");

// ---------------------------------------- 3. withdraw through the relayer
const { server, relayer } = await serve({ key: keys.relayer, pool: dep.pool, minFee: FEE, port: 0 });
const relayUrl = `http://127.0.0.1:${server.address().port}`;
const synced = await syncPool(publicClient, { pool: dep.pool, fromBlock: dep.deployBlock });
if (!synced.excluded.includes(notes.bob.index)) throw new Error("the SDK did not see the exclusion");

async function withdrawFor(user) {
  const recipient = privateKeyToAddress(generatePrivateKey()); // fresh, never funded
  const b = binding({ chainId: chain.id, pool: dep.pool, recipient, relayer, fee: FEE });
  if (b !== (await read("bindingFor", [recipient, relayer, FEE]))) throw new Error("SDK and pool disagree on the binding");
  const n = notes[user];
  const { proof, input } = await prove(
    { note: n, index: n.index, poolLeaves: synced.leaves, excluded: synced.excluded, binding: b },
    proving,
  );
  if (BigInt(input.poolRoot) !== (await read("currentPoolRoot"))) throw new Error("pool root mismatch");
  if (BigInt(input.cleanRoot) !== (await read("currentCleanRoot"))) throw new Error("clean root mismatch");
  const s = toSolidity(proof);
  const body = { ...s, poolRoot: input.poolRoot, cleanRoot: input.cleanRoot, nullifierHash: input.nullifierHash, recipient, fee: FEE };
  const res = await fetch(`${relayUrl}/relay`, { method: "POST", body: JSON.stringify(body, (_, v) => (typeof v === "bigint" ? v.toString() : v)) });
  const out = await res.json();
  if (!res.ok) throw new Error(`relayer: ${out.error}`);
  const receipt = await publicClient.getTransactionReceipt({ hash: out.txHash });
  const paid = await publicClient.readContract({ address: iss.token, abi: ITIP20, functionName: "balanceOf", args: [recipient] });
  if (paid !== BigInt(dep.denomination) - FEE) throw new Error(`recipient got ${paid}`);
  record(`withdrawal of ${user}'s note to a fresh address, via the relayer`, receipt, { recipient, received: paid });
}
await withdrawFor("alice");
await withdrawFor("carol");

// ------------------------------------------------ 4. Bob's note is not provable
// The witness generator reports the failed constraint's line. Only a failure
// at the clean-root check counts: a missing file or a typo must not pass.
const circuit = readFileSync(new URL("../../circuits/withdraw.circom", import.meta.url), "utf8").split("\n");
const cleanRootLine = circuit.findIndex((l) => l.includes("cleanRoot === cleanTree.root")) + 1;
let refused = false;
const seen = [];
const quiet = { log: console.log, error: console.error };
console.log = console.error = (...a) => seen.push(a.join(" "));
try {
  await prove(
    { note: notes.bob, index: notes.bob.index, poolLeaves: synced.leaves, excluded: synced.excluded, binding: 1n },
    proving,
  );
} catch (e) {
  const line = Number((seen.join("\n") + String(e?.message)).match(/line: (\d+)/)?.[1]);
  if (line !== cleanRootLine) throw new Error(`Bob's proof failed, but not at the clean-root check: ${e?.message}`, { cause: e });
  refused = true;
} finally {
  Object.assign(console, quiet);
}
if (!refused) throw new Error("Bob's excluded note produced a proof");
console.log("Bob's excluded note: the circuit refuses to prove it");
server.close();

writeState("e2e", {
  network: NETWORK,
  chainId: chain.id,
  pool: dep.pool,
  token: iss.token,
  policy: iss.policy,
  relayer,
  steps: log,
  excludedNoteRefused: refused,
  finalPoolRoot: await read("currentPoolRoot"),
  finalCleanRoot: await read("currentCleanRoot"),
  cleanEpoch: await read("cleanEpoch"),
});
console.log(`recorded in deployments/${chain.id}-e2e.json`);
await terminate();
