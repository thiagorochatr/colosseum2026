import { useCallback, useEffect, useMemo, useState } from "react";
import { getConfig } from "./lib/api.js";
import { tempoChain } from "./lib/tempo.js";
import { solanaChain } from "./lib/solana.js";
import ChainPanel from "./ChainPanel.jsx";

const COPY = {
  tempo: {
    tab: "Tempo",
    sub: "Private stablecoin pool",
    headline: "Tempo has compliance without privacy.",
    lead: "Every TIP-20 transfer is public, and the issuer's blocklist sits on chain.",
    how: "This pool hides which deposit a withdrawal spends. Its clean set is derived from the issuer's own TIP-403 policy, on chain, with no curator: if the issuer blocks a depositor, their deposit leaves the clean set and can no longer be withdrawn.",
    consoleTitle: "Issuer console",
    consoleHow: "The token issuer's policy decides. Blocking a depositor adds them to the blacklist and excludes their deposits in the same Tempo batch transaction.",
  },
  solana: {
    tab: "Solana",
    sub: "Compliant mirror-pool",
    headline: "Solana has privacy without compliance.",
    lead: "Its privacy pools screen deposits through off-chain APIs, and nothing on chain says a withdrawal came from clean funds.",
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

/** The bar across the top: what this is, and what the page is doing right now. */
function TopBar({ copy, status }) {
  return (
    <header className="top">
      <div className="brand">
        <img className="brand-mark" src="/favicon.svg" alt="" width="24" height="24" />
        <span className="brand-name">Alibi</span>
        <span className="brand-tag">Private payments that keep the issuer's rules</span>
        <span className="brand-chain">{copy.tab}</span>
      </div>
      <div className="status" aria-live="polite">
        {status.busy ? (
          <>
            <span className="spinner small" /> <span className="status-text">{status.busy}…</span>
          </>
        ) : status.read ? (
          <>
            <span className="dot live" /> <span className="status-text">Live on {status.network} · read at {status.read}</span>
          </>
        ) : (
          <>
            <span className="dot" /> <span className="status-text">Connecting…</span>
          </>
        )}
      </div>
      <a className="source" href="https://github.com/thiagorochatr/colosseum2026" target="_blank" rel="noreferrer">
        Source ↗
      </a>
    </header>
  );
}

export default function App() {
  const [config, setConfig] = useState(null);
  const [error, setError] = useState(null);
  const [log, setLog] = useState([]);
  const [status, setStatus] = useState({});
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
    <div className="app">
      <TopBar copy={copy} status={status} />
      {error && <div className="notice error">The demo cannot start: {error}</div>}
      {!chain && !error && <div className="notice">Loading…</div>}
      {chain && <ChainPanel chain={chain} copy={copy} log={log} updateLog={updateLog} onStatus={setStatus} />}
    </div>
  );
}
