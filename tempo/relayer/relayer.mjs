// A minimal relayer: it submits a withdrawal and pays the gas, and the pool
// pays it the fee bound into the proof. The recipient needs no balance and no
// key, so it can be any address, including a supplier's existing one.
//
//   NETWORK=local RELAYER_FEE=1000000 node relayer/relayer.mjs     (HTTP on :8787)
//
// POST /relay {a, b, c, poolRoot, cleanRoot, nullifierHash, recipient, fee}
// GET  /info  {relayer, fee, pool}

import { createServer } from "node:http";
import { abi, address, demoKeys, readState, send } from "../e2e/common.mjs";

export function createRelayer({ key, pool, minFee }) {
  const POOL = abi("CompliantPool");
  const relayer = address(key);
  async function relay(req) {
    const fee = BigInt(req.fee);
    if (fee < minFee) throw new Error(`fee below ${minFee}`);
    const args = [
      req.a.map(BigInt),
      req.b.map((row) => row.map(BigInt)),
      req.c.map(BigInt),
      BigInt(req.poolRoot),
      BigInt(req.cleanRoot),
      BigInt(req.nullifierHash),
      req.recipient,
      relayer,
      fee,
    ];
    // send() simulates first, so a bad proof or a stale root never costs gas.
    const { receipt } = await send(key, { address: pool, abi: POOL, functionName: "withdraw", args });
    return { txHash: receipt.transactionHash, gasUsed: receipt.gasUsed };
  }
  return { relayer, relay };
}

export function serve({ key, pool, minFee, port = 8787 }) {
  const { relayer, relay } = createRelayer({ key, pool, minFee });
  const json = (res, code, body) => {
    res.writeHead(code, { "content-type": "application/json", "access-control-allow-origin": "*" });
    res.end(JSON.stringify(body, (_, v) => (typeof v === "bigint" ? v.toString() : v)));
  };
  const server = createServer(async (req, res) => {
    if (req.method === "OPTIONS") {
      // A browser's POST with a JSON body is preflighted for its content-type.
      res.writeHead(204, {
        "access-control-allow-origin": "*",
        "access-control-allow-methods": "GET, POST",
        "access-control-allow-headers": "content-type",
      });
      return res.end();
    }
    if (req.method === "GET" && req.url === "/info") return json(res, 200, { relayer, fee: minFee.toString(), pool });
    if (req.method === "POST" && req.url === "/relay") {
      let body = "";
      for await (const chunk of req) body += chunk;
      try {
        return json(res, 200, await relay(JSON.parse(body)));
      } catch (e) {
        return json(res, 400, { error: e.shortMessage ?? e.message });
      }
    }
    json(res, 404, { error: "not found" });
  });
  return new Promise((resolve) => server.listen(port, () => resolve({ server, relayer })));
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const { pool } = readState();
  const minFee = BigInt(process.env.RELAYER_FEE ?? 1_000_000);
  const { relayer } = await serve({ key: demoKeys().relayer, pool, minFee, port: Number(process.env.PORT ?? 8787) });
  console.log(`relayer ${relayer} for pool ${pool}, fee ${minFee}, on :${process.env.PORT ?? 8787}`);
}
