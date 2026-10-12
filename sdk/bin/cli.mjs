#!/usr/bin/env node
// A thin command line over the SDK, for both chains.
//
//   compliance note new  --chain tempo|solana --network <net> --out note.json
//   compliance deposit   --note note.json  [--key 0x… | TEMPO_PRIVATE_KEY]  [--keypair id.json]
//   compliance status    --note note.json
//   compliance withdraw  --note note.json --to <address> [--relayer http://…] [--max-fee n]      (Tempo)
//   compliance withdraw  --note note.json --to <pubkey> --relay-keypair relay.json [--fee n]   (Solana)
//   compliance settle    --network <net> [--keypair id.json] [--fee n] [--allow-below-floor] (Solana: spends at --fee)
//
// Networks: tempo local|moderato, solana local|devnet. Addresses come from the
// repository's deployments/ files. Proving uses circuits/build/withdraw_js/withdraw.wasm
// and circuits/ceremony/withdraw_final.zkey, or WITHDRAW_WASM / WITHDRAW_ZKEY, both
// published in the ceremony-v1 release.

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { homedir } from "node:os";
import { parseArgs } from "node:util";
import { createNote, commitmentOf, nullifierOf, circuitNote, serializeNote, parseNote, prove, terminate } from "../src/index.mjs";
import * as T from "../src/tempo.mjs";
import * as S from "../src/solana.mjs";

const ROOT = new URL("../..", import.meta.url).pathname;
const json = (p) => JSON.parse(readFileSync(p, "utf8"));
const proving = {
  wasm: process.env.WITHDRAW_WASM ?? `${ROOT}circuits/build/withdraw_js/withdraw.wasm`,
  zkey: process.env.WITHDRAW_ZKEY ?? `${ROOT}circuits/ceremony/withdraw_final.zkey`,
};

const NETWORKS = {
  tempo: {
    local: { chainId: 31337, rpc: process.env.RPC ?? "http://127.0.0.1:8545", explorer: null },
    moderato: { chainId: 42431, rpc: "https://rpc.moderato.tempo.xyz", explorer: "https://explore.testnet.tempo.xyz/tx/" },
  },
  solana: {
    local: { rpc: process.env.RPC ?? "http://127.0.0.1:8899", explorer: null },
    devnet: { rpc: "https://api.devnet.solana.com", explorer: "https://explorer.solana.com/tx/" },
  },
};
const link = (net, sig) => (net.explorer ? `${net.explorer}${sig}${net.explorer.includes("solana") ? "?cluster=devnet" : ""}` : sig);

// --------------------------------------------------------------------- Tempo

async function tempoCtx(network) {
  const net = NETWORKS.tempo[network];
  if (!net) throw new Error(`unknown Tempo network ${network}`);
  const { createPublicClient, defineChain, http } = await import("viem");
  const { tempoLocalnet, tempoModerato } = await import("viem/chains");
  const chain = network === "moderato" ? tempoModerato : defineChain({ ...tempoLocalnet, id: 31337, rpcUrls: { default: { http: [net.rpc] } } });
  const dep = json(`${ROOT}tempo/deployments/${net.chainId}.json`);
  const publicClient = createPublicClient({ chain, transport: http(net.rpc) });
  const read = (functionName, args = []) => publicClient.readContract({ address: dep.pool, abi: T.POOL_ABI, functionName, args });
  return { net, chain, dep, publicClient, read };
}

async function tempoWallet(chain, key) {
  if (!key) throw new Error("pass --key or set TEMPO_PRIVATE_KEY");
  const { Account, createClient } = await import("viem/tempo");
  const { http } = await import("viem");
  return createClient({ account: Account.fromSecp256k1(key), chain, transport: http(), feeToken: "0x20C0000000000000000000000000000000000000" });
}

async function tempoFind(ctx, note) {
  const synced = await T.syncPool(ctx.publicClient, { pool: ctx.dep.pool, fromBlock: ctx.dep.deployBlock });
  const c = await commitmentOf(note);
  return { synced, index: synced.leaves.findIndex((l) => l === c) };
}

const tempo = {
  async deposit(note, opts) {
    const ctx = await tempoCtx(note.network);
    if (note.pool.toLowerCase() !== ctx.dep.pool.toLowerCase()) throw new Error("this note is for another pool than the network's");
    // A second deposit of one note shares its nullifier, so it could never be withdrawn.
    if ((await tempoFind(ctx, note)).index >= 0) throw new Error("this note is already deposited; create a new one");
    const token = await ctx.read("token");
    const wallet = await tempoWallet(ctx.chain, opts.key ?? process.env.TEMPO_PRIVATE_KEY);
    const calls = T.depositCalls({ pool: ctx.dep.pool, token, denomination: note.denomination, commitment: await commitmentOf(note) });
    const receipt = await wallet.sendTransactionSync({ calls });
    if (receipt.status !== "success") throw new Error(`deposit reverted: ${receipt.transactionHash}`);
    return { tx: link(ctx.net, receipt.transactionHash), note: "approve and deposit in one Tempo batch transaction" };
  },
  async status(note) {
    const ctx = await tempoCtx(note.network);
    const { synced, index } = await tempoFind(ctx, note);
    const spent = await ctx.read("spent", [await nullifierOf(note)]);
    return { deposited: index >= 0, index, clean: index >= 0 && !synced.excluded.includes(index), spent };
  },
  async withdraw(note, opts) {
    const ctx = await tempoCtx(note.network);
    const relayerUrl = opts.relayer ?? "http://127.0.0.1:8787";
    const info = await (await fetch(`${relayerUrl}/info`)).json();
    const fee = BigInt(opts.fee ?? info.fee);
    // The relayer names its own fee, and the pool only caps it at the whole deposit.
    const maxFee = opts["max-fee"] !== undefined ? BigInt(opts["max-fee"]) : BigInt(note.denomination) / 20n;
    if (fee > maxFee) throw new Error(`the relayer asks ${fee}, above the ${maxFee} --max-fee allows`);
    const { synced, index } = await tempoFind(ctx, note);
    if (index < 0) throw new Error("this note was never deposited");
    if (synced.excluded.includes(index)) throw new Error("this note is outside the clean set: its depositor is no longer authorized by the issuer");
    const binding = T.binding({ chainId: ctx.chain.id, pool: ctx.dep.pool, recipient: opts.to, relayer: info.relayer, fee });
    const { toSolidity } = await import("../src/prove.mjs");
    const { proof, input } = await prove({ note: circuitNote(note), index, poolLeaves: synced.leaves, excluded: synced.excluded, binding }, proving);
    const res = await fetch(`${relayerUrl}/relay`, { method: "POST", body: JSON.stringify(T.relayRequest({ proof: toSolidity(proof), input, recipient: opts.to, fee })) });
    const out = await res.json();
    if (!res.ok) throw new Error(`relayer: ${out.error}`);
    return { tx: link(ctx.net, out.txHash), recipient: opts.to, received: (note.denomination - fee).toString() };
  },
};

// -------------------------------------------------------------------- Solana

async function solanaCtx(network) {
  const net = NETWORKS.solana[network];
  if (!net) throw new Error(`unknown Solana network ${network}`);
  const web3 = await import("@solana/web3.js");
  const dep = json(`${ROOT}solana/deployments/${network}.json`);
  const connection = new web3.Connection(net.rpc, "confirmed");
  const program = new web3.PublicKey(dep.poolProgram);
  const compliance = new web3.PublicKey(dep.complianceProgram);
  const curatorAuthority = new web3.PublicKey(dep.curatorAuthority);
  const pool = S.poolAddress(program, BigInt(dep.denomination));
  // The clean set in force, read from the curator's memo and checked against its account.
  const set = await S.readCleanSet(connection, compliance, curatorAuthority, pool);
  return { net, web3, dep, connection, program, compliance, curatorAuthority, set };
}

const keypairFile = (web3, path) => web3.Keypair.fromSecretKey(Uint8Array.from(json(path ?? `${homedir()}/.config/solana/id.json`)));
const solanaSend = (ctx, ixs, signers) =>
  ctx.web3.sendAndConfirmTransaction(ctx.connection, new ctx.web3.Transaction().add(...ixs), signers, { commitment: "confirmed" });

async function solanaFind(ctx, note) {
  const pool = new ctx.web3.PublicKey(note.pool);
  // The clean set in ctx is the network pool's: a note from another pool cannot use it.
  if (!pool.equals(S.poolAddress(ctx.program, BigInt(ctx.dep.denomination)))) throw new Error("this note is for another pool than the network's");
  const synced = await S.syncPool(ctx.connection, ctx.program, pool);
  const c = await commitmentOf(note);
  return { pool, synced, index: synced.leaves.findIndex((l) => l === c) };
}

const solana = {
  async deposit(note, opts) {
    const ctx = await solanaCtx(note.network);
    const kp = keypairFile(ctx.web3, opts.keypair);
    // A second deposit of one note shares its nullifier, so it could never be withdrawn.
    if ((await solanaFind(ctx, note)).index >= 0) throw new Error("this note is already deposited; create a new one");
    const pool = new ctx.web3.PublicKey(note.pool);
    const sig = await solanaSend(ctx, [S.instructions.deposit(ctx.program, kp.publicKey, pool, await commitmentOf(note))], [kp]);
    return { tx: link(ctx.net, sig) };
  },
  async status(note) {
    const ctx = await solanaCtx(note.network);
    const { pool, synced, index } = await solanaFind(ctx, note);
    const excluded = ctx.set?.excluded ?? null;
    const spend = S.spendAddress(ctx.program, pool, S.fieldBytes(await nullifierOf(note)));
    const spent = (await ctx.connection.getAccountInfo(spend)) !== null;
    const curatedUpTo = ctx.set?.depositCount ?? 0;
    return {
      deposited: index >= 0,
      index,
      clean: index >= 0 && excluded !== null && index < curatedUpTo && !excluded.includes(index),
      curatedDeposits: curatedUpTo,
      totalDeposits: synced.leaves.length,
      spent,
    };
  },
  async withdraw(note, opts) {
    const ctx = await solanaCtx(note.network);
    if (!ctx.set) throw new Error("the curator has no clean set in force for this pool; run the curator first");
    // The relay signs the withdrawal in public. Defaulting to the wallet that
    // deposited would sign both ends with one key and undo the privacy.
    if (!opts["relay-keypair"]) throw new Error("pass --relay-keypair: a key that never touched this deposit");
    const relay = keypairFile(ctx.web3, opts["relay-keypair"]);
    let wallet = null;
    try {
      wallet = keypairFile(ctx.web3, undefined).publicKey;
    } catch {
      // No default wallet on this machine: nothing to compare against.
    }
    if (wallet && relay.publicKey.equals(wallet)) throw new Error("the relay key is your default wallet; use a separate one");
    const fee = BigInt(opts.fee ?? 1_000_000);
    const { pool, synced, index } = await solanaFind(ctx, note);
    if (index < 0) throw new Error("this note was never deposited");
    if (index >= ctx.set.depositCount) throw new Error("the curator has not published a clean root covering this deposit yet");
    if (ctx.set.excluded.includes(index)) throw new Error("this note is outside the curator's clean set");
    // Prove against the pool as the curator saw it, so the clean root is the published one.
    const leaves = synced.leaves.slice(0, ctx.set.depositCount);
    const beneficiary = new ctx.web3.PublicKey(opts.to);
    const binding = S.actionBinding({ beneficiary: beneficiary.toBytes(), relay: relay.publicKey.toBytes(), relayFee: fee });
    const { proof, input } = await prove({ note: circuitNote(note), index, poolLeaves: leaves, excluded: ctx.set.excluded, binding }, proving);
    // The memo's clean set must rebuild to the root the curator holds, before any fee is paid.
    if (BigInt(input.cleanRoot) !== ctx.set.cleanRoot) throw new Error("the curator's memo does not rebuild to its clean root");
    const inputs = [input.poolRoot, input.cleanRoot, input.nullifierHash, input.binding];
    const nullifier = S.fieldBytes(await nullifierOf(note));
    const sig = await solanaSend(ctx, [
      S.instructions.verifyClearance(ctx.compliance, relay.publicKey, ctx.curatorAuthority, pool, S.toSolana(proof), inputs),
      S.instructions.submitClearedSpend(ctx.program, ctx.compliance, ctx.curatorAuthority, relay.publicKey, pool, nullifier, beneficiary, fee),
    ], [relay]);
    return { tx: link(ctx.net, sig), beneficiary: opts.to, pending: "settles with the next batch: compliance settle" };
  },
  /** Settles every pending spend of the network's pool in one batch. */
  async settle(opts) {
    const ctx = await solanaCtx(opts.network);
    const pool = S.poolAddress(ctx.program, BigInt(ctx.dep.denomination));
    const accounts = await ctx.connection.getProgramAccounts(ctx.program, { filters: [{ memcmp: { offset: 96, bytes: pool.toBase58() } }] });
    // Spend records of this pool that are still pending (version 1, status 1, the pool at offset 96)
    // and settle together: plain transfers (selector 0, at 8) at one relay fee (at 16), ten at most.
    const fee = BigInt(opts.fee ?? 1_000_000);
    const pending = accounts
      .filter((a) => a.account.data.length >= 160 && a.account.data[0] === 1 && a.account.data[1] === 1)
      .filter((a) => a.account.data.readBigUInt64LE(8) === 0n && a.account.data.readBigUInt64LE(16) === fee)
      .slice(0, 10);
    if (!pending.length) return { settled: 0 };
    const kp = keypairFile(ctx.web3, opts.keypair);
    const metas = [
      { pubkey: kp.publicKey, isSigner: true, isWritable: true },
      { pubkey: pool, isSigner: false, isWritable: true },
      { pubkey: S.vaultAddress(ctx.program, pool), isSigner: false, isWritable: true },
      ...pending.flatMap((a) => [
        { pubkey: a.pubkey, isSigner: false, isWritable: true },
        { pubkey: new ctx.web3.PublicKey(a.account.data.subarray(32, 64)), isSigner: false, isWritable: true },
        { pubkey: new ctx.web3.PublicKey(a.account.data.subarray(64, 96)), isSigner: false, isWritable: true },
      ]),
    ];
    const ix = new ctx.web3.TransactionInstruction({
      programId: ctx.program,
      keys: metas,
      data: Buffer.from([3, pending.length, opts["allow-below-floor"] ? 1 : 0]),
    });
    const sig = await solanaSend(ctx, [ix], [kp]);
    return { settled: pending.length, tx: link(ctx.net, sig) };
  },
};

// ---------------------------------------------------------------------- main

const { positionals, values } = parseArgs({
  allowPositionals: true,
  options: {
    chain: { type: "string" }, network: { type: "string" }, out: { type: "string" }, note: { type: "string" },
    key: { type: "string" }, keypair: { type: "string" }, to: { type: "string" }, relayer: { type: "string" },
    "relay-keypair": { type: "string" }, fee: { type: "string" }, "max-fee": { type: "string" }, "allow-below-floor": { type: "boolean" },
  },
});
const [cmd, sub] = positionals;
const chains = { tempo, solana };
const loadNote = () => parseNote(readFileSync(values.note ?? "note.json", "utf8"));
const print = (o) => console.log(JSON.stringify(o, (_, v) => (typeof v === "bigint" ? v.toString() : v), 2));

try {
  if (cmd === "note" && sub === "new") {
    const { chain, network } = values;
    let pool, denomination;
    if (chain === "tempo") {
      const ctx = await tempoCtx(network);
      pool = ctx.dep.pool;
      denomination = BigInt(ctx.dep.denomination);
    } else if (chain === "solana") {
      const ctx = await solanaCtx(network);
      denomination = BigInt(ctx.dep.denomination);
      pool = S.poolAddress(ctx.program, denomination).toBase58();
    } else throw new Error("--chain tempo|solana");
    const out = values.out ?? "note.json";
    if (existsSync(out)) throw new Error(`${out} exists; a note is never overwritten`);
    writeFileSync(out, serializeNote(createNote({ chain, network, pool, denomination })), { mode: 0o600 });
    print({ wrote: out, chain, network, pool, denomination, warning: "this file is the deposit: whoever holds it can withdraw" });
  } else if (["deposit", "status", "withdraw"].includes(cmd)) {
    const note = loadNote();
    print(await chains[note.chain][cmd](note, values));
  } else if (cmd === "settle") {
    print(await solana.settle(values));
  } else {
    console.error(readFileSync(new URL(import.meta.url), "utf8").split("\n").slice(1, 9).join("\n"));
    process.exitCode = 2;
  }
} catch (e) {
  console.error(`error: ${e.shortMessage ?? e.message}`);
  process.exitCode = 1;
} finally {
  await terminate();
}
