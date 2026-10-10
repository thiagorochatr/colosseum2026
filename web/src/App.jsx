import { useEffect, useMemo, useState } from "react";
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

export default function App() {
  const [config, setConfig] = useState(null);
  const [error, setError] = useState(null);
  const [tab, setTab] = useState(() => {
    try {
      return localStorage.getItem("tab") ?? "tempo";
    } catch {
      return "tempo";
    }
  });

  useEffect(() => {
    getConfig().then(setConfig, (e) => setError(e.message));
  }, []);
  useEffect(() => {
    try {
      localStorage.setItem("tab", tab);
    } catch {
      // Storage can be blocked; remembering the tab is only a convenience.
    }
  }, [tab]);

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

      <nav className="tabs" role="tablist">
        {Object.entries(COPY).map(([id, c]) => (
          <button key={id} role="tab" aria-selected={tab === id} className={tab === id ? "tab on" : "tab"} onClick={() => setTab(id)}>
            <span className="tab-title">{c.tab}</span>
            <span className="tab-sub">{c.sub}</span>
          </button>
        ))}
      </nav>

      {error && <div className="card error">The demo service is not reachable: {error}</div>}
      {!config && !error && <div className="card muted">Loading…</div>}
      {chains && <ChainPanel key={tab} chain={chains[tab]} copy={COPY[tab]} />}

      <footer className="foot">
        <span>Same circuit and verifying key on both chains · Groth16 over BN254 · keys from a public ceremony</span>
        <a href="https://github.com/thiagorochatr/colosseum2026" target="_blank" rel="noreferrer">Source on GitHub</a>
      </footer>
    </div>
  );
}
