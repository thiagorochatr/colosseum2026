import { useCallback, useEffect, useMemo, useState } from "react";
import { getConfig } from "./lib/api.js";
import { tempoChain } from "./lib/tempo.js";
import { solanaChain } from "./lib/solana.js";
import ChainPanel from "./ChainPanel.jsx";

const COPY = {
  tempo: {
    tab: "Tempo",
    sub: "Private stablecoin pool",
    lead: "Tempo has compliance without privacy: every TIP-20 transfer is public, and the issuer's blocklist sits on chain.",
    how: "This pool hides which deposit a withdrawal spends. Its clean set is derived from the issuer's own TIP-403 policy, on chain, with no curator: if the issuer blocks a depositor, their deposit leaves the clean set and can no longer be withdrawn.",
    consoleTitle: "Issuer console",
    consoleHow: "The token issuer's policy decides. Blocking a depositor adds them to the blacklist and excludes their deposits in the same Tempo batch transaction, up to three of them; any more follow at once.",
  },
  solana: {
    tab: "Solana",
    sub: "Compliant mirror-pool",
    lead: "Solana has privacy without compliance: its privacy pools screen deposits through off-chain APIs, and nothing on chain says a withdrawal came from clean funds.",
    how: "Here a curator publishes a clean set of deposits. A withdrawal proves its deposit is in that set, without saying which one, and the pool pays only against that proof. This is a layer, not another pool: any Solana privacy pool can require the same public clearance. Here it wraps mirror-pool, one of the winners of Superteam Brasil's Noise bounty, unchanged.",
    consoleTitle: "Curator console",
    consoleHow: "The curator applies a published policy. Blocking a depositor starts a new epoch and publishes a clean root without them, in one transaction, with the clean set in its memo.",
  },
};

// Each chain is its own page, /tempo and /solana, with no switch between them.
// The root opens the last one visited, or Tempo.
const CHAINS = Object.keys(COPY);
const fromPath = () => {
  const id = window.location.pathname.replace(/^\/+|\/+$/g, "");
  return CHAINS.includes(id) ? id : null;
};
function lastVisited() {
  try {
    const id = localStorage.getItem("tab");
    return CHAINS.includes(id) ? id : null;
  } catch {
    return null;
  }
}
const CHAIN = fromPath() ?? lastVisited() ?? "tempo";
const ADAPTERS = { tempo: (c) => tempoChain(c.tempo), solana: (c) => solanaChain(c.solana) };

export default function App() {
  const [config, setConfig] = useState(null);
  const [error, setError] = useState(null);
  const [log, setLog] = useState([]);
  const updateLog = useCallback((_id, fn) => setLog(fn), []);

  useEffect(() => {
    getConfig().then(
      (c) => (c[CHAIN]?.error ? setError(`the ${COPY[CHAIN].tab} demo is not configured (${c[CHAIN].error})`) : setConfig(c)),
      (e) => setError(e.message),
    );
    if (fromPath() !== CHAIN) window.history.replaceState(null, "", `/${CHAIN}`);
    document.title = `${COPY[CHAIN].tab} · Private payments that keep the issuer's rules`;
    try {
      localStorage.setItem("tab", CHAIN);
    } catch {
      // Storage can be blocked; remembering the page is only a convenience.
    }
  }, []);

  const chain = useMemo(() => (config ? ADAPTERS[CHAIN](config) : null), [config]);
  const copy = COPY[CHAIN];

  return (
    <div className="page">
      <header className="hero">
        <p className="eyebrow">Colosseum World's Fair 2026 · test networks</p>
        <h1>Private payments that keep the issuer's rules</h1>
        <p className="thesis">
          Tempo has compliance without privacy. Solana has privacy without compliance. One zero-knowledge proof closes both gaps.
        </p>
      </header>

      <div className="chain-head">
        <span className="tab-title">{copy.tab}</span>
        <span className="tab-sub">{copy.sub}</span>
      </div>

      {error && <div className="card error">The demo cannot start: {error}</div>}
      {!config && !error && <div className="card muted">Loading…</div>}
      {chain && <ChainPanel chain={chain} copy={copy} log={log} updateLog={updateLog} />}

      <footer className="foot">
        <span>Same circuit and verifying key on both chains · Groth16 over BN254 · keys from a public ceremony</span>
        <a href="https://github.com/thiagorochatr/colosseum2026" target="_blank" rel="noreferrer">Source on GitHub</a>
      </footer>
    </div>
  );
}
