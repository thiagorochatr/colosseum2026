// Checks that the contracts on Moderato run the bytecode this repository builds.
// Run `forge build` first.
//
//   node script/check-deployed.mjs        (RPC from $TEMPO_RPC, else Moderato's)
//
// Two kinds of bytes legitimately differ from the build, and each is checked
// on its own instead of being trusted:
//   - immutables, which the constructor writes: read back through the pool's
//     getters and compared with deployments/42431.json;
//   - linked library addresses (Poseidon), placeholders in the build: the code
//     at each linked address is compared with the library's own build.
// Every other byte, including the metadata hash of the exact source, must match.
import { readFileSync } from "node:fs";
import { createPublicClient, http, parseAbi, getAddress } from "viem";

const here = new URL("..", import.meta.url).pathname;
const client = createPublicClient({ transport: http(process.env.TEMPO_RPC ?? "https://rpc.moderato.tempo.xyz") });
const deployment = JSON.parse(readFileSync(`${here}deployments/42431.json`, "utf8"));
const failures = [];

function artifact(name) {
  const { deployedBytecode } = JSON.parse(readFileSync(`${here}out/${name}.sol/${name}.json`, "utf8"));
  const hex = deployedBytecode.object.slice(2).replace(/__\$[0-9a-f]{34}\$__/g, "00".repeat(20));
  const links = Object.values(deployedBytecode.linkReferences ?? {}).flatMap((file) =>
    Object.entries(file).flatMap(([library, refs]) => refs.map((r) => ({ library, ...r }))),
  );
  const immutables = Object.values(deployedBytecode.immutableReferences ?? {}).flat();
  return { code: Buffer.from(hex, "hex"), links, immutables };
}

async function deployedCode(address) {
  return Buffer.from((await client.getCode({ address })).slice(2), "hex");
}

// Compares after zeroing the given byte ranges in both.
function same(label, onchain, built, masked) {
  for (const { start, length } of masked) {
    built.fill(0, start, start + length);
    if (start < onchain.length) onchain.fill(0, start, Math.min(onchain.length, start + length));
  }
  if (onchain.equals(built)) return true;
  const at = built.findIndex((b, i) => b !== onchain[i]);
  failures.push(`${label}: differs from the build (lengths ${onchain.length}/${built.length}, first difference at byte ${at})`);
  return false;
}

async function check(name, address) {
  const { code, links, immutables } = artifact(name);
  const onchain = await deployedCode(address);

  // Each linked library: the address the deployed code calls, and its code.
  const libraries = new Map();
  for (const { library, start } of links) {
    libraries.set(library, getAddress(`0x${onchain.subarray(start, start + 20).toString("hex")}`));
  }
  for (const [library, at] of libraries) {
    const lib = artifact(library);
    // A deployed library starts with PUSH20 <its own address>, a call guard the
    // build leaves as zeros.
    const guard = lib.code[0] === 0x73 ? [{ start: 1, length: 20 }] : [];
    if (same(`${library} ${at}`, await deployedCode(at), lib.code, guard)) console.log(`  ${library} ${at}: runs this build`);
  }

  if (same(`${name} ${address}`, onchain, code, [...immutables, ...links])) console.log(`${name} ${address}: runs this build`);
}

await check("Groth16Verifier", deployment.verifier);
await check("CompliantPool", deployment.pool);

// The pool's immutables, which the comparison above masked.
const abi = parseAbi([
  "function token() view returns (address)",
  "function denomination() view returns (uint256)",
  "function verifier() view returns (address)",
]);
const read = (functionName) => client.readContract({ address: deployment.pool, abi, functionName });
const expected = { token: getAddress(deployment.token), denomination: BigInt(deployment.denomination), verifier: getAddress(deployment.verifier) };
for (const [key, want] of Object.entries(expected)) {
  const got = await read(key);
  if (got === want) console.log(`  CompliantPool.${key}() = ${got}`);
  else failures.push(`CompliantPool.${key}() is ${got}, deployments/42431.json says ${want}`);
}

for (const f of failures) console.error(f);
process.exit(failures.length ? 1 : 0);
