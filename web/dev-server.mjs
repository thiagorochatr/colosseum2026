// Local development: the web app through Vite, and /api routed to the same
// functions Vercel deploys. Test-network keys come from the gitignored .demo/
// files when they are not already in the environment.
//
//   npm run dev     (from the repository root)

import { createServer } from "node:http";
import { existsSync, readFileSync } from "node:fs";
import { createServer as vite } from "vite";

const ROOT = new URL("..", import.meta.url).pathname;
const read = (p) => (existsSync(`${ROOT}${p}`) ? readFileSync(`${ROOT}${p}`, "utf8") : null);

// Variables pulled from Vercel (`vercel env pull`), such as SOLANA_RPC.
for (const line of (read(".env.local") ?? "").split("\n")) {
  const m = line.match(/^([A-Z0-9_]+)=(.*)$/);
  if (m && process.env[m[1]] === undefined) process.env[m[1]] = m[2].replace(/^"(.*)"$/, "$1");
}
const tempoKeys = read("tempo/.demo/42431-keys.json");
if (tempoKeys) {
  const k = JSON.parse(tempoKeys);
  process.env.TEMPO_ISSUER_KEY ??= k.issuer;
  process.env.TEMPO_RELAYER_KEY ??= k.relayer;
}
process.env.SOLANA_RELAY_KEY ??= read("solana/.demo/devnet/relay.json") ?? "";
process.env.SOLANA_CURATOR_KEY ??= read("solana/.demo/devnet/curator.json") ?? "";

const routes = [
  [/^\/api\/config$/, "api/config.mjs", () => ({})],
  [/^\/api\/tempo\/([a-z]+)$/, "api/tempo/[action].mjs", (m) => ({ action: m[1] })],
  [/^\/api\/solana\/([a-z]+)$/, "api/solana/[action].mjs", (m) => ({ action: m[1] })],
];

const app = await vite({ root: `${ROOT}web`, server: { middlewareMode: true }, appType: "spa" });
const port = Number(process.env.PORT ?? 5173);
createServer(async (req, res) => {
  const path = new URL(req.url, "http://x").pathname;
  for (const [re, file, query] of routes) {
    const m = path.match(re);
    if (m) {
      req.query = query(m);
      const { default: handler } = await import(`${ROOT}${file}`);
      return handler(req, res);
    }
  }
  app.middlewares(req, res);
  // Local only: this server holds the test networks' keys.
}).listen(port, "127.0.0.1", () => console.log(`http://localhost:${port}`));
