export async function post(path, body) {
  const res = await fetch(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  // A timeout or a platform error page is not JSON: report the status, not a parse error.
  const out = await res.json().catch(() => null);
  if (!res.ok) throw new Error(out?.error ?? `the demo service answered HTTP ${res.status}`);
  if (!out) throw new Error("the demo service sent an unreadable answer");
  return out;
}

export async function getConfig() {
  const res = await fetch("/api/config");
  if (!res.ok) throw new Error("the demo service is not reachable");
  return res.json();
}

export const PROVING = { wasm: "/zk/withdraw.wasm", zkey: "/zk/withdraw_final.zkey" };
export const short = (s, n = 4) => (s ? `${s.slice(0, n + 2)}…${s.slice(-n)}` : "");

/** A field element as 0x-prefixed hex, shortened for the protocol log. */
export const field = (x) => short(`0x${BigInt(x).toString(16).padStart(64, "0")}`, 6);
export const indices = (list) => (list.length ? list.map((i) => `#${i}`).join(", ") : "none");
