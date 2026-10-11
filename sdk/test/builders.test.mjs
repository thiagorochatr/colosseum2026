// Notes, and each chain's builders, against what the chains already hold.

import { test, after } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { toFunctionSelector, toEventSelector } from "viem";
import { PublicKey } from "@solana/web3.js";
import { createNote, commitmentOf, nullifierOf, serializeNote, parseNote, randomField, FIELD, hasher, terminate } from "../src/index.mjs";
import * as tempo from "../src/tempo.mjs";
import * as solana from "../src/solana.mjs";

const root = new URL("../..", import.meta.url).pathname;
const json = (p) => JSON.parse(readFileSync(`${root}${p}`, "utf8"));
after(terminate);

test("a note's commitment and nullifier are H3(k, r, denomination) and H1(k)", async () => {
  const note = createNote({ chain: "tempo", network: "moderato", pool: "0xabc", denomination: 100_000_000n });
  const { h1, h3 } = await hasher();
  assert.equal(await commitmentOf(note), h3(note.k, note.r, 100_000_000n));
  assert.equal(await nullifierOf(note), h1(note.k));
});

test("notes round-trip through their file format and are never reused", () => {
  const a = createNote({ chain: "solana", network: "devnet", pool: "P", denomination: 1n });
  const b = createNote({ chain: "solana", network: "devnet", pool: "P", denomination: 1n });
  assert.deepEqual(parseNote(serializeNote(a)), a);
  assert.notEqual(a.k, b.k);
  assert.throws(() => parseNote('{"version": 9}'), /unsupported note version/);
  for (let i = 0; i < 200; i++) assert.ok(randomField() < FIELD);
});

test("the Tempo ABI matches the compiled pool", { skip: !existsSync(`${root}tempo/out/CompliantPool.sol/CompliantPool.json`) && "build tempo/ first" }, () => {
  const compiled = json("tempo/out/CompliantPool.sol/CompliantPool.json").abi;
  const sel = (abi, item) =>
    item.type === "event" ? toEventSelector(item) : toFunctionSelector(`${item.name}(${item.inputs.map((x) => x.type).join(",")})`);
  const known = new Set(compiled.filter((i) => ["function", "event", "error"].includes(i.type)).map((i) => sel(compiled, i)));
  for (const item of tempo.POOL_ABI) assert.ok(known.has(sel(tempo.POOL_ABI, item)), `${item.name} is not in the compiled pool`);
});

test("the Tempo batch builders encode approve+deposit and block+exclude", () => {
  const pool = "0x7228B24c607151E74DBCb4Ad818B87cd993a64E9";
  const calls = tempo.depositCalls({ pool, token: "0x20C0000000000000000000002Bc6afccdA61E264", denomination: 100_000_000n, commitment: 5n });
  assert.equal(calls.length, 2);
  assert.equal(calls[0].data.slice(0, 10), toFunctionSelector("approve(address,uint256)"));
  assert.equal(calls[1].data.slice(0, 10), toFunctionSelector("deposit(uint256)"));
  const block = tempo.blockAndExcludeCalls({ pool, policy: 1253146n, account: pool, indices: [1, 4] });
  assert.equal(block.length, 3);
  assert.equal(block[0].to, tempo.REGISTRY);
});

test("a sweep excludes newly blocked depositors and reincludes unblocked ones, and nothing else", () => {
  const alice = "0xA11cE00000000000000000000000000000000001";
  const bob = "0xB0B0000000000000000000000000000000000002";
  const carol = "0xCa20100000000000000000000000000000000003";
  // Bob is blocked now; Carol was excluded and is authorized again; deposit 3 is Bob's, already excluded.
  const synced = { depositors: [alice, bob, carol, bob, alice], excluded: [2, 3] };
  const blocked = new Set([bob.toLowerCase()]);
  const plan = tempo.planSweep(synced, (a) => !blocked.has(a.toLowerCase()));
  assert.deepEqual(plan, { exclude: [1], reinclude: [2] });
  assert.deepEqual(tempo.planSweep(synced, () => true), { exclude: [], reinclude: [2, 3] });
  assert.deepEqual(tempo.planSweep({ depositors: [], excluded: [] }, () => false), { exclude: [], reinclude: [] });

  const pool = "0x7228B24c607151E74DBCb4Ad818B87cd993a64E9";
  const calls = tempo.sweepCalls({ pool, ...plan });
  assert.deepEqual(calls.map((c) => c.to), [pool, pool]);
  assert.equal(calls[0].data.slice(0, 10), toFunctionSelector("exclude(uint256)"));
  assert.equal(calls[1].data.slice(0, 10), toFunctionSelector("reinclude(uint256)"));
  assert.equal(BigInt(`0x${calls[0].data.slice(10)}`), 1n);
});

test("the Solana PDAs are the ones already on devnet", () => {
  const d = json("solana/deployments/devnet.json");
  const e = json("solana/deployments/devnet-e2e.json");
  assert.equal(solana.poolAddress(new PublicKey(d.poolProgram), BigInt(d.denomination)).toBase58(), e.pool);
});

test("the policy document is canonical: sorted, deduplicated, fixed key order", () => {
  const a = solana.policyDocument(["b", "a", "b"]);
  assert.deepEqual(a, { version: 1, rule: solana.POLICY_RULE, blocklist: ["a", "b"] });
  assert.deepEqual(solana.policyHash(a), solana.policyHash(solana.policyDocument(["a", "b"])));
  // The genesis policy on devnet is already in canonical form.
  const genesis = json("solana/deployments/devnet-policy.json");
  assert.deepEqual(genesis, solana.policyDocument(genesis.blocklist));
});

test("a curator memo is accepted only with a held root and the curator's policy hash", () => {
  const compliance = new PublicKey("HHapM3yA1JkhFJ1msTc7n3CEQ7PMZvxHMvQz7G7orQ2j");
  const curator = new PublicKey("AdUihuLoEVQyyirSHLrrLMRvctXeFihqTQyGyV1sv5fA");
  const pool = new PublicKey("4RwTZq6ucSEDWEQZ6RSfa1ruhqEayS4QsF6LVPA7yvGP");
  const blocklist = ["9vZ1Lv", "E73QdV"];
  const ixs = solana.publishInstructions({ compliance, curator, pool, depositCount: 4, excluded: [1, 3], blocklist, cleanRoot: 77n, revoke: true });
  assert.deepEqual(ixs.map((i) => i.data[0]), [3, 5, 2, ixs[3].data[0]], "revoke, update policy, publish root, then the memo");
  const memo = new TextDecoder().decode(ixs[3].data);
  const hash = Buffer.from(solana.policyHash(solana.policyDocument(blocklist))).toString("hex");
  const state = { roots: [77n], policyHash: `0x${hash}` };

  const set = solana.acceptMemo(memo, pool, state);
  assert.deepEqual({ n: set.depositCount, x: set.excluded, b: set.blocklist, r: set.cleanRoot }, { n: 4, x: [1, 3], b: blocklist, r: 77n });
  assert.equal(solana.acceptMemo(memo, pool, { ...state, policyHash: `0x${"00".repeat(32)}` }), null, "another policy hash");
  assert.equal(solana.acceptMemo(memo, pool, { ...state, roots: [78n] }), null, "a root the curator no longer holds");
  assert.equal(solana.acceptMemo(memo, PublicKey.default, state), null, "another pool");
  const v1 = JSON.stringify({ v: 1, pool: pool.toBase58(), n: 4, x: [1, 3], b: blocklist, r: "4d" });
  assert.equal(solana.acceptMemo(v1, pool, state), null, "the old memo, with no policy document");
});

test("the Solana instruction data have the programs' exact lengths", () => {
  const P = new PublicKey("Fg1FxuHcjj54ERfdQQL9tzZq9wDmNfjGmFSkAXxVYdzB");
  const C = new PublicKey("HHapM3yA1JkhFJ1msTc7n3CEQ7PMZvxHMvQz7G7orQ2j");
  const me = PublicKey.default;
  const pool = solana.poolAddress(P, 100_000_000n);
  assert.equal(solana.instructions.initPool(P, me, 100_000_000n).data.length, 25);
  assert.equal(solana.instructions.deposit(P, me, pool, 7n).data.length, 33);
  assert.equal(solana.instructions.settle(P, me, pool, []).data.length, 3);
  // ProofRequest: tag + a, b, c + four inputs; ClearedSpend: tag + compliant-pool's BASE_LEN of 115.
  const proof = { a: new Uint8Array(64), b: new Uint8Array(128), c: new Uint8Array(64) };
  assert.equal(solana.instructions.verifyClearance(C, me, me, pool, proof, [1n, 2n, 3n, 4n]).data.length, 1 + 256 + 128);
  assert.equal(solana.instructions.submitClearedSpend(P, C, me, pool, new Uint8Array(32), me, 5n).data.length, 1 + 115);
});
