// Shared by the demo service's functions. They hold the test-network keys, so
// they only act: the browser reads chain state straight from public RPCs.

import tempoDeployment from "../../tempo/deployments/42431.json" with { type: "json" };
import tempoIssuer from "../../tempo/deployments/42431-issuer.json" with { type: "json" };
import solanaDeployment from "../../solana/deployments/devnet.json" with { type: "json" };
import solanaE2e from "../../solana/deployments/devnet-e2e.json" with { type: "json" };
import solanaPolicy from "../../solana/deployments/devnet-policy.json" with { type: "json" };

export const TEMPO = {
  chainId: 42431,
  rpc: process.env.TEMPO_RPC ?? "https://rpc.moderato.tempo.xyz",
  explorer: "https://explore.testnet.tempo.xyz",
  pool: tempoDeployment.pool,
  deployBlock: tempoDeployment.deployBlock,
  token: tempoIssuer.token,
  policy: tempoIssuer.policy,
  issuer: tempoIssuer.issuer,
  denomination: String(tempoDeployment.denomination),
  fee: "1000000",
};

export const SOLANA = {
  rpc: process.env.SOLANA_RPC ?? "https://api.devnet.solana.com",
  explorer: "https://explorer.solana.com",
  poolProgram: solanaDeployment.poolProgram,
  complianceProgram: solanaDeployment.complianceProgram,
  curatorAuthority: solanaDeployment.curatorAuthority,
  pool: solanaE2e.pool,
  denomination: solanaDeployment.denomination,
  fee: "1000000",
  initialBlocklist: solanaPolicy.blocklist,
};

export function secret(name) {
  const v = process.env[name];
  if (!v) throw new Error(`the demo service is missing ${name}`);
  return v;
}

/** The parsed JSON body, whether the platform parsed it already or not. */
export async function body(req) {
  if (req.body && typeof req.body === "object") return req.body;
  if (typeof req.body === "string") return JSON.parse(req.body || "{}");
  let raw = "";
  for await (const chunk of req) raw += chunk;
  return raw ? JSON.parse(raw) : {};
}

export function reply(res, code, value) {
  res.statusCode = code;
  res.setHeader("content-type", "application/json");
  res.setHeader("cache-control", "no-store");
  res.end(JSON.stringify(value, (_, v) => (typeof v === "bigint" ? v.toString() : v)));
}

/** Wraps an action table into a handler: POST /api/<chain>/<action>. */
export function actions(table) {
  return async function handler(req, res) {
    const action = req.query?.action ?? new URL(req.url, "http://x").pathname.split("/").pop();
    const run = table[action];
    if (!run) return reply(res, 404, { error: `unknown action ${action}` });
    if (req.method !== "POST") return reply(res, 405, { error: "POST only" });
    try {
      reply(res, 200, await run(await body(req)));
    } catch (e) {
      reply(res, 400, { error: e.shortMessage ?? e.message });
    }
  };
}

export const isAddress = (a) => typeof a === "string" && /^0x[0-9a-fA-F]{40}$/.test(a);
