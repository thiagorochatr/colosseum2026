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
    consoleHow: "The token issuer's policy decides. Blocking a depositor adds them to the blacklist and excludes their deposits in the same Tempo transaction.",
  },
  solana: {
    tab: "Solana",
    sub: "Compliant mirror-pool",
    lead: "Solana has privacy without compliance: its privacy pools screen deposits through off-chain APIs, and nothing on chain says a withdrawal came from clean funds.",
    how: "Here a curator publishes a clean set of deposits. A withdrawal proves its deposit is in that set, without saying which one, and the pool pays only against that proof. The pool is mirror-pool, winner of Superteam Brasil's Noise bounty, unchanged.",
    consoleTitle: "Curator console",
    consoleHow: "The curator applies a published policy. Blocking a depositor starts a new epoch and publishes a clean root without them, in one transaction, with the clean set in its memo.",
  },
};

// Each chain has its own route, /tempo and /solana. The root opens the last one visited.
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

export default function App() {
  const [config, setConfig] = useState(null);
  const [error, setError] = useState(null);
  const [tab, setTab] = useState(() => fromPath() ?? lastVisited() ?? "tempo");
  // The protocol log of each chain survives switching routes.
  const [logs, setLogs] = useState(() => Object.fromEntries(CHAINS.map((id) => [id, []])));
  const updateLog = useCallback((id, fn) => setLogs((all) => ({ ...all, [id]: fn(all[id]) })), []);

  useEffect(() => {
    getConfig().then(setConfig, (e) => setError(e.message));
  }, []);
  useEffect(() => {
    if (fromPath() !== tab) window.history.replaceState(null, "", `/${tab}`);
    const onPop = () => setTab(fromPath() ?? "tempo");
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
    // Runs once: later changes go through go(), which pushes the route itself.
  }, []);
  useEffect(() => {
    document.title = `${COPY[tab].tab} · Private payments that keep the issuer's rules`;
    try {
      localStorage.setItem("tab", tab);
    } catch {
      // Storage can be blocked; remembering the route is only a convenience.
    }
  }, [tab]);

  const go = (id) => (event) => {
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) return;
    event.preventDefault();
    if (id === tab) return;
    window.history.pushState(null, "", `/${id}`);
    setTab(id);
  };

  const chains = useMemo(() => (config ? { tempo: tempoChain(config.tempo), solana: solanaChain(config.solana) } : null), [config]);

  return (
    <div className="page">
      <header className="hero">
        <p className="eyebrow">Colosseum World's Fair 2026 · test networks</p>
        <h1>Private payments that keep the issuer's rules</h1>
        <p className="thesis">
          Tempo has compliance without privacy. Solana has privacy without compliance. One zero-knowledge proof closes both gaps.
        </p>
      </header>

      <nav className="tabs" aria-label="Chain">
        {Object.entries(COPY).map(([id, c]) => (
          <a key={id} href={`/${id}`} aria-current={tab === id ? "page" : undefined} className={tab === id ? "tab on" : "tab"} onClick={go(id)}>
            <span className="tab-title">{c.tab}</span>
            <span className="tab-sub">{c.sub}</span>
          </a>
        ))}
      </nav>

      {error && <div className="card error">The demo service is not reachable: {error}</div>}
      {!config && !error && <div className="card muted">Loading…</div>}
      {chains && <ChainPanel key={tab} chain={chains[tab]} copy={COPY[tab]} log={logs[tab]} updateLog={updateLog} />}

      <footer className="foot">
        <span>Same circuit and verifying key on both chains · Groth16 over BN254 · keys from a public ceremony</span>
        <a href="https://github.com/thiagorochatr/colosseum2026" target="_blank" rel="noreferrer">Source on GitHub</a>
      </footer>
    </div>
  );
}
