// The whole Solana flow on a real cluster, after the programs are deployed and
// deployments/<network>.json names them.
//
//   NETWORK=local node e2e.mjs
//
// 1. Alice, Bob and Carol each deposit one note into the compliant pool.
// 2. The policy blocks Bob; the curator publishes the clean root without him.
// 3. Alice's and Carol's notes are proved, cleared and spent through a relay,
//    VerifyClearance and SubmitClearedSpend in one transaction each.
// 4. mirror-pool's own SettleEpoch pays both fresh beneficiaries in one batch.
// 5. Bob's note cannot be proved.

import { randomBytes } from "node:crypto";
import { writeFileSync, mkdirSync } from "node:fs";
import { FIELD, hasher } from "../../sdk/src/tree.mjs";
import { prove, terminate } from "../../sdk/src/prove.mjs";
import { actionBinding, toSolana } from "../../sdk/src/solana.mjs";
import {
  NETWORK, ROOT, PublicKey, Keypair, SystemProgram, be32, connection, demoKeypair, explorer, ixs, poolAddress,
  readDeployment, send, syncPool, wallet, writeDeployment,
} from "./common.mjs";
import { curatorKeypair, init as initCurator, publish } from "./curator.mjs";

const d = readDeployment();
const poolProgram = new PublicKey(d.poolProgram);
const compliance = new PublicKey(d.complianceProgram);
const denomination = BigInt(d.denomination);
const pool = poolAddress(poolProgram, denomination);
const FEE = 1_000_000n;
const proving = {
  wasm: `${ROOT}../circuits/build/withdraw_js/withdraw.wasm`,
  zkey: `${ROOT}../circuits/ceremony/withdraw_final.zkey`,
};
const steps = [];
const record = (step, sig, extra = {}) => {
  steps.push({ step, signature: sig, link: explorer(sig), ...extra });
  console.log(`${step}: ${explorer(sig)}`);
};

// ---------------------------------------------------------------- 0. accounts
const funder = wallet();
const users = { alice: demoKeypair("alice"), bob: demoKeypair("bob"), carol: demoKeypair("carol") };
const relay = demoKeypair("relay");
const curator = curatorKeypair();
async function fund(pubkey, lamports) {
  const have = BigInt(await connection.getBalance(pubkey));
  if (have >= lamports) return;
  await send([SystemProgram.transfer({ fromPubkey: funder.publicKey, toPubkey: pubkey, lamports: lamports - have })], [funder]);
}
for (const u of Object.values(users)) await fund(u.publicKey, denomination + 50_000_000n);
await fund(relay.publicKey, 100_000_000n);
await fund(curator.publicKey, 50_000_000n);

if (!(await connection.getAccountInfo(pool))) {
  record("init pool", await send([ixs.initPool(poolProgram, funder.publicKey, denomination, 2)], [funder]));
}
writeDeployment("", { ...d, pool: pool.toBase58() });
writeDeployment("policy", { version: 1, rule: "exclude every deposit whose depositor is on the blocklist", blocklist: [users.bob.publicKey.toBase58()] });
const created = await initCurator();
if (!created.existed) steps.push({ step: "curator created", signature: created.sig, link: explorer(created.sig) });

// --------------------------------------------------------------- 1. deposits
const { h3, h1 } = await hasher();
const rand = () => BigInt(`0x${randomBytes(32).toString("hex")}`) % FIELD;
const notes = {};
for (const [name, kp] of Object.entries(users)) {
  const note = { k: rand(), r: rand(), denomTag: denomination };
  const sig = await send([ixs.deposit(poolProgram, kp.publicKey, pool, h3(note.k, note.r, denomination))], [kp]);
  notes[name] = note;
  record(`deposit by ${name}`, sig);
}
mkdirSync(`${ROOT}.demo/${NETWORK}`, { recursive: true });
writeFileSync(`${ROOT}.demo/${NETWORK}/notes.json`, JSON.stringify(notes, (_, v) => (typeof v === "bigint" ? v.toString() : v), 2), { mode: 0o600 });

// ------------------------------------------------- 2. the curator publishes
const published = await publish();
record(`curator publishes the clean root, excluding [${published.excluded}]`, published.sig);

// -------------------------------------- 3. prove, clear and spend via the relay
const synced = await syncPool(poolProgram, pool);
const indexOf = (note) => synced.leaves.findIndex((l) => l === h3(note.k, note.r, denomination));
const spends = [];
async function clearAndSpend(name) {
  const note = notes[name];
  const beneficiary = Keypair.generate().publicKey; // fresh, never funded
  const binding = actionBinding({ beneficiary: beneficiary.toBytes(), relay: relay.publicKey.toBytes(), relayFee: FEE });
  const { proof, input } = await prove(
    { note, index: indexOf(note), poolLeaves: synced.leaves, excluded: published.excluded, binding },
    proving,
  );
  const inputs = [input.poolRoot, input.cleanRoot, input.nullifierHash, input.binding];
  const nullifier = be32(h1(note.k));
  const sig = await send(
    [
      ixs.verifyClearance(compliance, relay.publicKey, curator.publicKey, pool, toSolana(proof), inputs),
      ixs.submitClearedSpend(poolProgram, compliance, relay.publicKey, pool, nullifier, beneficiary, FEE),
    ],
    [relay],
  );
  spends.push({ nullifier, beneficiary, relay: relay.publicKey });
  record(`${name}'s note cleared and spent, one transaction`, sig, { beneficiary: beneficiary.toBase58() });
}
await clearAndSpend("alice");
await clearAndSpend("carol");

// ------------------------------------------- 4. mirror-pool settles the batch
const settled = await send([ixs.settle(poolProgram, relay.publicKey, pool, spends)], [relay]);
record("mirror-pool settles both spends in one batch", settled);
for (const s of spends) {
  const got = BigInt(await connection.getBalance(s.beneficiary));
  if (got !== denomination - FEE) throw new Error(`beneficiary ${s.beneficiary.toBase58()} got ${got}`);
}

// ------------------------------------------------- 5. Bob's note is refused
let refused = false;
const quiet = { log: console.log, error: console.error };
console.log = console.error = () => {};
try {
  await prove({ note: notes.bob, index: indexOf(notes.bob), poolLeaves: synced.leaves, excluded: published.excluded, binding: 1n }, proving);
} catch {
  refused = true;
} finally {
  Object.assign(console, quiet);
}
if (!refused) throw new Error("Bob's excluded note produced a proof");
console.log("Bob's excluded note: the circuit refuses to prove it");

writeDeployment("e2e", {
  network: NETWORK,
  pool: pool.toBase58(),
  poolProgram: d.poolProgram,
  complianceProgram: d.complianceProgram,
  curator: curator.publicKey.toBase58(),
  steps,
  excluded: published.excluded,
  cleanRoot: published.clean_root,
  beneficiariesReceived: (denomination - FEE).toString(),
  excludedNoteRefused: refused,
});
console.log(`recorded in deployments/${NETWORK}-e2e.json`);
await terminate();
