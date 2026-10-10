// Puts the proving files where the browser fetches them, web/public/zk/, and
// checks them against the ceremony's published hashes. Uses the local build if
// present, otherwise downloads the ceremony-v1 release.

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";

const ROOT = new URL("../..", import.meta.url).pathname;
const OUT = `${ROOT}web/public/zk`;
const RELEASE = "https://github.com/thiagorochatr/colosseum2026/releases/download/ceremony-v1";
const FILES = {
  "withdraw.wasm": { local: "circuits/build/withdraw_js/withdraw.wasm", sha256: "dfe3ab63c33a4784c241d0723ed2487e80affe9b16ab5e3bd42fb5ba4ce9709a" },
  "withdraw_final.zkey": { local: "circuits/ceremony/withdraw_final.zkey", sha256: "a3217e83f95632a27e27d7593167cdfe5bb9ecaa0539e926cdce38248ce180f5" },
};
const sha = (b) => createHash("sha256").update(b).digest("hex");

mkdirSync(OUT, { recursive: true });
for (const [name, { local, sha256 }] of Object.entries(FILES)) {
  const target = `${OUT}/${name}`;
  if (existsSync(target) && sha(readFileSync(target)) === sha256) continue;
  let bytes;
  if (existsSync(`${ROOT}${local}`)) {
    bytes = readFileSync(`${ROOT}${local}`);
  } else {
    const res = await fetch(`${RELEASE}/${name}`);
    if (!res.ok) throw new Error(`${name}: HTTP ${res.status}`);
    bytes = Buffer.from(await res.arrayBuffer());
  }
  if (sha(bytes) !== sha256) throw new Error(`${name}: sha256 does not match the ceremony's`);
  writeFileSync(target, bytes);
  console.log(`${name}: ok`);
}
