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
  const sel = (abi, item) => (item.type === "event" ? toEventSelector(item) : toFunctionSelector(item));
  const known = new Set(compiled.filter((i) => i.type === "function" || i.type === "event").map((i) => sel(compiled, i)));
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

test("the Solana PDAs are the ones already on devnet", () => {
  const d = json("solana/deployments/devnet.json");
  const e = json("solana/deployments/devnet-e2e.json");
  const program = new PublicKey(d.poolProgram);
  assert.equal(solana.poolAddress(program, BigInt(d.denomination)).toBase58(), e.pool);
  const curator = json("solana/deployments/devnet-curator.json");
  assert.equal(curator.pool, e.pool);
});

test("the policy hash is the one the curator published", () => {
  const policy = json("solana/deployments/devnet-policy.json");
  const curator = json("solana/deployments/devnet-curator.json");
  assert.equal(Buffer.from(solana.policyHash(policy)).toString("hex"), curator.policy_hash);
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
