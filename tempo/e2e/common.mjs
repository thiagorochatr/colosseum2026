// Shared configuration for the Tempo scripts: the network, demo keys, clients
// and contract ABIs.
//
//   NETWORK=local     a local `anvil --network tempo` on RPC (default http://127.0.0.1:8545)
//   NETWORK=moderato  Tempo's public testnet
//
// Demo keys are for test networks only. On local they are anvil's well-known
// development keys. On Moderato they are generated once, stored in
// tempo/.demo/ (gitignored), and funded from the faucet.

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createPublicClient, defineChain, http } from "viem";
import { generatePrivateKey, privateKeyToAddress } from "viem/accounts";
import { tempoLocalnet, tempoModerato } from "viem/chains";
import { Account, createClient } from "viem/tempo";
import { RETRY } from "compliance-sdk/tempo";

export const ROOT = new URL("..", import.meta.url).pathname;
export const NETWORK = process.env.NETWORK ?? "local";
export const PATH_USD = "0x20C0000000000000000000000000000000000000";
export const REGISTRY = "0x403c000000000000000000000000000000000000";
export const FACTORY = "0x20Fc000000000000000000000000000000000000";

const local = defineChain({
  ...tempoLocalnet,
  id: 31337,
  rpcUrls: { default: { http: [process.env.RPC ?? "http://127.0.0.1:8545"] } },
});
export const chain = NETWORK === "moderato" ? tempoModerato : local;
export const EXPLORER = NETWORK === "moderato" ? "https://explore.testnet.tempo.xyz" : null;
export const txLink = (hash) => (EXPLORER ? `${EXPLORER}/tx/${hash}` : hash);

// anvil's public development keys 1..5; key 0 is left to the deployer.
const ANVIL = [
  "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
  "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a",
  "0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6",
  "0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a",
  "0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba",
];
export const ROLES = ["issuer", "alice", "bob", "carol", "relayer"];

export function demoKeys() {
  if (NETWORK !== "moderato") return Object.fromEntries(ROLES.map((r, i) => [r, ANVIL[i]]));
  const dir = `${ROOT}.demo`;
  const file = `${dir}/${chain.id}-keys.json`;
  if (existsSync(file)) return JSON.parse(readFileSync(file, "utf8"));
  mkdirSync(dir, { recursive: true });
  const keys = Object.fromEntries(ROLES.map((r) => [r, generatePrivateKey()]));
  writeFileSync(file, JSON.stringify(keys, null, 2), { mode: 0o600 });
  return keys;
}

export const address = (pk) => privateKeyToAddress(pk);
export const publicClient = createPublicClient({ chain, transport: http(undefined, RETRY) });
export const wallet = (pk) =>
  createClient({ account: Account.fromSecp256k1(pk), chain, transport: http(undefined, RETRY), feeToken: PATH_USD });

export function abi(name, file = name) {
  return JSON.parse(readFileSync(`${ROOT}out/${file}.sol/${name}.json`, "utf8")).abi;
}

const stateFile = (kind) => `${ROOT}deployments/${chain.id}${kind ? `-${kind}` : ""}.json`;
export const readState = (kind) => JSON.parse(readFileSync(stateFile(kind), "utf8"));
export const writeState = (kind, value) =>
  writeFileSync(stateFile(kind), JSON.stringify(value, (_, v) => (typeof v === "bigint" ? v.toString() : v), 2) + "\n");

/** Simulates a write, then sends it and waits. Returns the simulated result and the receipt. */
export async function send(pk, { address: to, abi, functionName, args }) {
  const account = Account.fromSecp256k1(pk);
  const { result } = await publicClient.simulateContract({ account, address: to, abi, functionName, args });
  const receipt = await wallet(pk).writeContractSync({ address: to, abi, functionName, args });
  if (receipt.status !== "success") throw new Error(`${functionName} reverted: ${receipt.transactionHash}`);
  return { result, receipt };
}

/** Asks the Moderato faucet to fund an address. */
export async function fund(addr) {
  if (NETWORK !== "moderato") return;
  await publicClient.request({ method: "tempo_fundAddress", params: [addr] });
}
