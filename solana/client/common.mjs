// The Solana client: configuration, PDAs, instruction builders, and the pool's
// leaves rebuilt from its history. See docs/SPEC.md section 8.
//
//   NETWORK=local   a solana-test-validator on http://127.0.0.1:8899
//   NETWORK=devnet  Solana devnet
//
// Program ids and the curator come from deployments/<network>.json.

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { Connection, Keypair, PublicKey, SystemProgram, Transaction, sendAndConfirmTransaction } from "@solana/web3.js";
import { hasher } from "../../sdk/src/tree.mjs";

export const ROOT = new URL("..", import.meta.url).pathname;
export const NETWORK = process.env.NETWORK ?? "local";
export const RPC = process.env.RPC ?? (NETWORK === "devnet" ? "https://api.devnet.solana.com" : "http://127.0.0.1:8899");
export const connection = new Connection(RPC, "confirmed");
export const explorer = (sig) =>
  NETWORK === "devnet" ? `https://explorer.solana.com/tx/${sig}?cluster=devnet` : sig;

const json = (path) => JSON.parse(readFileSync(path, "utf8"));
export const readKeypair = (path) => Keypair.fromSecretKey(Uint8Array.from(json(path)));
export const wallet = () => readKeypair(process.env.WALLET ?? `${homedir()}/.config/solana/id.json`);

export const deploymentFile = (kind = "") => `${ROOT}deployments/${NETWORK}${kind ? `-${kind}` : ""}.json`;
export const readDeployment = (kind) => json(deploymentFile(kind));
export const writeDeployment = (kind, v) =>
  writeFileSync(deploymentFile(kind), JSON.stringify(v, (_, x) => (typeof x === "bigint" ? x.toString() : x), 2) + "\n");

/** Demo keypairs for a test network, generated once into .demo/ (gitignored). */
export function demoKeypair(name) {
  const dir = `${ROOT}.demo/${NETWORK}`;
  const file = `${dir}/${name}.json`;
  if (!existsSync(file)) {
    mkdirSync(dir, { recursive: true });
    writeFileSync(file, JSON.stringify([...Keypair.generate().secretKey]), { mode: 0o600 });
  }
  return readKeypair(file);
}

// Encoding, PDAs, instruction builders and pool sync live in the SDK, which
// the browser app uses too. This module adds what only Node has: key files,
// deployment files and sending.
import * as sdk from "../../sdk/src/solana.mjs";

export const be32 = sdk.fieldBytes;
export const { poolAddress, vaultAddress, spendAddress, curatorAddress, clearanceAddress, policyHash } = sdk;
export const ixs = sdk.instructions;
export const poolState = (pool) => sdk.poolState(connection, pool);
export const syncPool = (program, pool) => sdk.syncPool(connection, program, pool);

export async function send(instructions, signers) {
  const tx = new Transaction().add(...instructions);
  return sendAndConfirmTransaction(connection, tx, signers, { commitment: "confirmed" });
}

export { hasher, PublicKey, Keypair, SystemProgram };
