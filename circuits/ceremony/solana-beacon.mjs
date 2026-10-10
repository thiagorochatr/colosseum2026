// Prints the blockhash, in hex, of the first Solana mainnet block at or after a slot.
//
//   node ceremony/solana-beacon.mjs 455095000
//
// A slot can be skipped, so the rule announced in docs/CEREMONY.md is "the
// first slot at or after S that has a block".

const slot = Number(process.argv[2]);
if (!Number.isInteger(slot)) throw new Error("usage: solana-beacon.mjs <slot>");
const RPC = process.env.SOLANA_RPC ?? "https://api.mainnet-beta.solana.com";

async function rpc(method, params) {
  const res = await fetch(RPC, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
  });
  const body = await res.json();
  if (body.error) throw new Error(`${method}: ${JSON.stringify(body.error)}`);
  return body.result;
}

const ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
function base58ToHex(s) {
  let n = 0n;
  for (const ch of s) {
    const v = ALPHABET.indexOf(ch);
    if (v < 0) throw new Error(`not base58: ${s}`);
    n = n * 58n + BigInt(v);
  }
  const hex = n.toString(16).padStart(64, "0");
  if (hex.length !== 64) throw new Error(`blockhash is not 32 bytes: ${s}`);
  return hex;
}

const blocks = await rpc("getBlocks", [slot, slot + 200, { commitment: "finalized" }]);
if (!blocks.length) throw new Error(`no finalized block in [${slot}, ${slot + 200}] yet`);
const first = blocks[0];
const block = await rpc("getBlock", [first, { commitment: "finalized", transactionDetails: "none", rewards: false, maxSupportedTransactionVersion: 1 }]);
console.log(JSON.stringify({ announcedSlot: slot, slot: first, blockhash: block.blockhash, hex: base58ToHex(block.blockhash) }));
