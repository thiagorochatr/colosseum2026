import { useCallback, useEffect, useRef, useState } from "react";
import { addNote, removeNote, loadNotes, downloadNote, importNote, markSaved } from "./lib/store.js";
import { short } from "./lib/api.js";

function Chip({ tone, children }) {
  return <span className={`chip ${tone ?? ""}`}>{children}</span>;
}

function Link({ href, children, className }) {
  return (
    <a href={href} target="_blank" rel="noreferrer" className={className}>
      {children}
    </a>
  );
}

const now = () => new Date().toLocaleTimeString();

/** An error as one readable sentence: viem's short message, without its request dump. */
function plain(e) {
  const text = e.shortMessage ?? e.message ?? String(e);
  if (/PolicyForbids/.test(text)) return "The token's TIP-403 policy refused the transfer: the issuer blocks one of its addresses.";
  return text;
}
const settle = (steps, state) => steps.map((x) => (x.state === "running" ? { ...x, state } : x));
// The page reads the chain again on its own this often, while it is visible and idle.
const POLL = 30_000;

/** One entry of the live log: an action and every step it went through. */
function LogEntry({ entry, chain }) {
  return (
    <li className={`entry ${entry.state}`}>
      <div className="entry-head">
        <span className="entry-time">{entry.at}</span>
        <strong>{entry.title}</strong>
        {entry.state === "running" && <span className="spinner small" />}
        {entry.state === "failed" && <Chip tone="bad">failed</Chip>}
      </div>
      <ol className="steps">
        {entry.steps.map((x, i) => (
          <li key={i} className={`step ${x.state}`}>
            <span className="mark" aria-hidden="true">{x.state === "running" ? "•" : x.state === "failed" ? "✕" : "✓"}</span>
            <div>
              <span>{x.text}</span>
              {(x.tx || x.accounts) && (
                <p className="links">
                  {x.tx && <Link href={chain.txLink(x.tx)}>Transaction {short(x.tx, 6)} ↗</Link>}
                  {(x.accounts ?? []).map(([label, a]) => (
                    <Link key={a} href={chain.addressLink(a)}>{label} {short(a)} ↗</Link>
                  ))}
                </p>
              )}
              {typeof x.detail === "string" && <p className="detail">{x.detail}</p>}
              {Array.isArray(x.detail) && (
                <dl className="kv">
                  {x.detail.map(([k, v]) => (
                    <div key={k}>
                      <dt>{k}</dt>
                      <dd className="mono">{String(v)}</dd>
                    </div>
                  ))}
                </dl>
              )}
            </div>
          </li>
        ))}
        {entry.error && (
          <li className="step failed">
            <span className="mark">✕</span>
            <div>{entry.error}</div>
          </li>
        )}
      </ol>
    </li>
  );
}

/** Where a deposit stands, for its tile in the pool and its label. */
function depositState(d) {
  if (d.uncurated) return { tone: "wait", label: "awaiting the curator" };
  if (d.pending) return { tone: "wait", label: "blocked, not yet excluded" };
  return d.clean ? { tone: "good", label: "in the clean set" } : { tone: "bad", label: "excluded" };
}

export default function ChainPanel({ chain, copy, log, updateLog, onStatus }) {
  const [balance, setBalance] = useState("…");
  const [pool, setPool] = useState(null);
  const [notes, setNotes] = useState(() => loadNotes(chain.id));
  const [statuses, setStatuses] = useState([]);
  const [busy, setBusy] = useState(null);
  const [read, setRead] = useState(null);
  const [error, setError] = useState(null);
  const [withdrawing, setWithdrawing] = useState(null);
  const [recipient, setRecipient] = useState("");
  const [importing, setImporting] = useState(false);
  const [pasted, setPasted] = useState("");
  const [importError, setImportError] = useState(null);
  // Keys offered as a file in this visit, awaiting the user's word that the file saved.
  const [offered, setOffered] = useState(() => new Set());

  useEffect(() => onStatus({ busy, read, network: chain.network }), [busy, read, chain, onStatus]);

  // Each action opens a log entry; the chain adapter narrates its steps into it.
  function begin(title) {
    const id = `${Date.now()}-${Math.random()}`;
    const edit = (fn) => updateLog(chain.id, (l) => l.map((e) => (e.id === id ? fn(e) : e)));
    updateLog(chain.id, (l) => [{ id, title, at: now(), state: "running", steps: [] }, ...l].slice(0, 30));
    return {
      step(text, { detail, tx, accounts, done = false } = {}) {
        if (!done) setBusy(text);
        edit((e) => ({ ...e, steps: [...settle(e.steps, "done"), { text, detail, tx, accounts, state: done ? "done" : "running" }] }));
      },
      finish: () => edit((e) => ({ ...e, state: "done", steps: settle(e.steps, "done") })),
      fail: (error) => edit((e) => ({ ...e, state: "failed", error, steps: settle(e.steps, "failed") })),
    };
  }

  // `quiet` keeps a failed refresh from replacing the error of the action that just ran.
  // A slow refresh that finishes after a newer one is dropped, so it cannot put back an older state.
  const latest = useRef(0);
  const refresh = useCallback(async ({ quiet = false } = {}) => {
    const mine = ++latest.current;
    try {
      const [b, p] = await Promise.all([chain.balance(), chain.pool()]);
      const n = loadNotes(chain.id);
      const s = await Promise.all(n.map((note) => chain.noteStatus(note, p)));
      if (mine !== latest.current) return;
      setBalance(b);
      setPool(p);
      setNotes(n);
      setStatuses(s);
      setRead(now());
    } catch (e) {
      if (!quiet && mine === latest.current) setError(e.message);
    }
  }, [chain]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Others use the pool too: keep reading it while the page is open, but never during an action.
  const idle = useRef(true);
  idle.current = !busy;
  useEffect(() => {
    const timer = setInterval(() => {
      if (idle.current && document.visibilityState === "visible") refresh({ quiet: true });
    }, POLL);
    return () => clearInterval(timer);
  }, [refresh]);

  async function run(label, fn) {
    setBusy(label);
    setError(null);
    const entry = begin(label);
    try {
      await fn(entry.step);
      entry.finish();
    } catch (e) {
      const message = plain(e);
      entry.fail(message);
      setError(message);
    } finally {
      // A failed action can still have changed the chain, as when the keeper
      // excludes a deposit before its withdrawal is refused.
      setBusy("Reading the chain again");
      await refresh({ quiet: true });
      setBusy(null);
    }
  }

  const faucet = () => run("Test funds", (step) => chain.faucet(step));

  const deposit = () =>
    run(`Deposit ${chain.denominationLabel}`, async (step) => {
      await chain.deposit(step, (note) => {
        addNote(chain.id, note);
        return () => removeNote(chain.id, note);
      });
      step("Download the withdrawal key in step 3. It is the only way to withdraw this deposit, from this browser or any other", { done: true });
    });

  const save = (note) => {
    downloadNote(note);
    setOffered((s) => new Set(s).add(String(note.k)));
  };

  const confirmSaved = (note) => {
    try {
      markSaved(note);
    } catch (e) {
      setError(e.message);
    }
    setNotes(loadNotes(chain.id));
  };

  function importKey(text) {
    try {
      importNote(text.trim(), chain.scope);
      setImporting(false);
      setPasted("");
      setImportError(null);
      setNotes(loadNotes(chain.id));
      refresh();
    } catch (e) {
      setImportError(e.message);
    }
  }

  async function importFile(e) {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    try {
      importKey(await file.text());
    } catch (err) {
      setImportError(`Could not read ${file.name}: ${err.message}`);
    }
  }

  const startWithdraw = (i) => {
    setWithdrawing(i);
    setRecipient(chain.freshRecipient());
  };

  const withdraw = (note) =>
    run("Withdraw", async (step) => {
      await chain.withdraw(note, recipient, pool, step);
      setWithdrawing(null);
    });

  const settleBatch = () => run("Settle the batch", (step) => chain.settle(step));
  const publishRoot = () => run("Curator publication", (step) => chain.publish(step));

  const block = (address, blocked) =>
    run(blocked ? `Unblock ${short(address)}` : `Block ${short(address)}`, (step) => (blocked ? chain.unblock(address, step) : chain.block(address, step)));

  const blockedAddresses = new Set(
    chain.id === "solana" ? pool?.blocklist ?? [] : (pool?.blocked ?? []).map((a) => a.toLowerCase()),
  );
  const isBlocked = (a) => blockedAddresses.has(chain.id === "solana" ? a : a.toLowerCase());
  const deposits = pool?.deposits ?? [];
  // Tempo addresses compare without case; Solana keys are base58, where case matters.
  const mine = (a) => (chain.id === "solana" ? a === chain.me : a.toLowerCase() === chain.me.toLowerCase());
  const depositors = [...new Set(deposits.map((d) => d.depositor))].map((a) => ({ address: a, indices: deposits.filter((d) => d.depositor === a).map((d) => d.index) }));
  const count = (f) => deposits.filter(f).length;
  const pending = count((d) => d.pending);

  return (
    <div className="board">
      {/* Left: what this is, and the three steps a user takes. */}
      <section className="col left">
        <div className="intro">
          <p className="eyebrow">{copy.tab} · {copy.sub}</p>
          <h1>{copy.headline}</h1>
          <p className="lead">{copy.lead}</p>
          <p className="muted">{copy.how}</p>
        </div>

        {error && (
          <div className="notice error">
            <span>{error}</span>
            <button className="link" onClick={() => setError(null)}>Dismiss</button>
          </div>
        )}

        <ol className="flow">
          <li className="card">
            <h2><span className="num">1</span> Your test wallet</h2>
            <div className="row spread">
              <Link className="mono" href={chain.addressLink(chain.me)}>{short(chain.me, 6)} ↗</Link>
              <span className="big">{balance}</span>
            </div>
            <button onClick={faucet} disabled={!!busy}>Get test funds</button>
            <p className="hint">A throwaway wallet that lives in this browser, on a test network.</p>
          </li>

          <li className="card">
            <h2><span className="num">2</span> Deposit</h2>
            <p className="hint">Every deposit is exactly {chain.denominationLabel}, so withdrawals cannot be matched to deposits by amount. It creates a withdrawal key in your browser: whoever holds the key can withdraw, from any browser.</p>
            <button className="primary" onClick={deposit} disabled={!!busy}>Deposit {chain.denominationLabel}</button>
          </li>

          <li className="card">
            <h2><span className="num">3</span> Withdraw</h2>
            {notes.length === 0 && <p className="hint">Your withdrawal keys appear here after a deposit.</p>}
            <ul className="notes">
              {notes.map((note, i) => {
                const st = statuses[i];
                return (
                  <li key={i}>
                    <div className="row spread">
                      <span className="row">
                        <strong>{!st ? "Deposit #…" : st.index >= 0 ? `Deposit #${st.index}` : "Withdrawal key"}</strong>
                        {st && <Chip tone={st.tone}>{st.label}</Chip>}
                      </span>
                      <span className="row">
                        {st?.canWithdraw && withdrawing !== i && (
                          <button className="primary small" onClick={() => startWithdraw(i)} disabled={!!busy}>Withdraw</button>
                        )}
                        <button className="small" onClick={() => save(note)}>Download key</button>
                      </span>
                    </div>
                    {/* Kept for "Not deposited" too: a deposit whose confirmation was lost can still land. */}
                    {!note.saved && st?.label !== "Withdrawn" && (
                      offered.has(String(note.k)) ? (
                        <div className="row">
                          <span className="hint">Did the file save? Check your downloads first.</span>
                          <button className="link" onClick={() => confirmSaved(note)}>Yes, I have the file</button>
                        </div>
                      ) : (
                        <p className="hint bad">Not backed up: this key exists only in this browser. Download it and keep the file safe.</p>
                      )
                    )}
                    {withdrawing === i && (
                      <div className="form">
                        <label>
                          Pay to
                          <input className="mono" value={recipient} onChange={(e) => setRecipient(e.target.value)} />
                        </label>
                        <p className="hint">A brand-new address that has never been on chain; paste one you control to keep the payment. The relayer pays the network fee and takes its fee from the withdrawal.</p>
                        <div className="row">
                          <button className="primary" onClick={() => withdraw(note)} disabled={!!busy}>Prove and withdraw</button>
                          <button className="link" onClick={() => setWithdrawing(null)}>Cancel</button>
                        </div>
                      </div>
                    )}
                    {st?.label === "Waiting for the batch" && chain.settle && (
                      <div className="row">
                        <span className="hint">The pool pays in batches of at least two, so a withdrawal cannot be timed back to its deposit.</span>
                        <button className="small" onClick={settleBatch} disabled={!!busy}>Settle the batch</button>
                      </div>
                    )}
                    {st?.label === "Awaiting the curator" && chain.publish && (
                      <div className="row">
                        <span className="hint">No clean root covers this deposit yet.</span>
                        <button className="small" onClick={publishRoot} disabled={!!busy}>Ask the curator to publish</button>
                      </div>
                    )}
                    {st?.label === "Blocked, not yet excluded" && (
                      <p className="hint bad">The issuer blocked this depositor. A withdrawal applies the policy first, so it will be refused.</p>
                    )}
                    {st?.label === "Not deposited" && (
                      <div className="row">
                        <span className="hint">No deposit with this key is on chain. If its deposit failed, remove it.</span>
                        <button className="link" onClick={() => { removeNote(chain.id, note); setNotes(loadNotes(chain.id)); refresh(); }} disabled={!!busy}>Remove</button>
                      </div>
                    )}
                    {st?.label === "Excluded" && <p className="hint bad">The depositor was blocked, so this deposit is outside the clean set. No valid proof exists for it.</p>}
                  </li>
                );
              })}
            </ul>
            {!importing && (
              <button className="link" onClick={() => setImporting(true)}>Import a withdrawal key…</button>
            )}
            {importing && (
              <div className="form">
                <p className="hint">Deposited from another browser? Choose its key file, or paste its contents.</p>
                <input type="file" accept=".json,application/json" onChange={importFile} />
                <textarea className="mono" rows={3} value={pasted} onChange={(e) => setPasted(e.target.value)} placeholder='{ "version": 1, "chain": … }' />
                {importError && <p className="hint bad">{importError}</p>}
                <div className="row">
                  <button className="primary small" onClick={() => importKey(pasted)} disabled={!pasted.trim()}>Import</button>
                  <button className="link" onClick={() => { setImporting(false); setImportError(null); }}>Cancel</button>
                </div>
              </div>
            )}
          </li>
        </ol>

        <p className="foot">Same circuit and verifying key on both chains · Groth16 over BN254 · keys from a public ceremony</p>
      </section>

      {/* Center: the pool everyone shares, and who decides what is clean. */}
      <section className="col center">
        <div className="card">
          <div className="row spread">
            <h2>The pool</h2>
            <Link className="small-link" href={chain.addressLink(chain.scope.pool)}>{short(chain.scope.pool)} ↗</Link>
          </div>
          {!pool && <p className="muted">Reading the chain…</p>}
          {pool && (
            <>
              <div className="stats">
                <div><span className="stat">{deposits.length}</span><span className="stat-label">deposits</span></div>
                <div><span className="stat good">{count((d) => d.clean && !d.uncurated)}</span><span className="stat-label">in the clean set</span></div>
                <div><span className="stat bad">{count((d) => !d.clean && !d.uncurated)}</span><span className="stat-label">excluded</span></div>
                <div><span className="stat accent">{count((d) => mine(d.depositor))}</span><span className="stat-label">yours</span></div>
              </div>
              <div className="tiles">
                {deposits.map((d) => {
                  const s = depositState(d);
                  return (
                    <a
                      key={d.index}
                      className={`tile ${s.tone}${mine(d.depositor) ? " mine" : ""}`}
                      href={chain.addressLink(d.depositor)}
                      target="_blank"
                      rel="noreferrer"
                      title={`#${d.index} · deposited by ${d.depositor}${mine(d.depositor) ? " (you)" : ""} · ${s.label}`}
                    >
                      {d.index}
                    </a>
                  );
                })}
              </div>
              <div className="legend">
                <span><i className="sw good" /> in the clean set</span>
                <span><i className="sw bad" /> excluded</span>
                {count((d) => d.uncurated || d.pending) > 0 && <span><i className="sw wait" /> waiting</span>}
                <span><i className="sw mine" /> yours</span>
              </div>
              <p className="hint">Anyone can see who deposited. Nobody can see which deposit a withdrawal spends.</p>
              {pending > 0 && (
                <p className="hint bad">
                  {pending === 1 ? "One deposit's depositor is" : `${pending} deposits' depositors are`} blocked by the token's policy but still in the clean set. Every withdrawal applies the policy first.
                </p>
              )}
            </>
          )}
        </div>

        <div className="card">
          <h2>{copy.consoleTitle}</h2>
          <p className="hint">{copy.consoleHow}</p>
          <ul className="depositors">
            {depositors.map(({ address, indices }) => (
              <li key={address} className="row spread">
                <span className="row">
                  <Link className="mono" href={chain.addressLink(address)}>{short(address)}</Link>
                  {mine(address) && <span className="you">you</span>}
                  <span className="hint inline">{indices.map((i) => `#${i}`).join(" ")}</span>
                </span>
                <button className={`small ${isBlocked(address) ? "" : "danger"}`} onClick={() => block(address, isBlocked(address))} disabled={!!busy}>
                  {isBlocked(address) ? "Unblock" : "Block"}
                </button>
              </li>
            ))}
          </ul>
        </div>
      </section>

      {/* Right: everything that happens, as it happens, with a link to every transaction. */}
      <section className="col right">
        <div className="card log-card">
          <h2 className="row"><span className={`dot ${busy ? "busy" : "live"}`} /> Live log</h2>
          {log.length === 0 && (
            <p className="hint">Every step of every action appears here as it happens: what this browser computed, what each transaction did, and what the chain checked, with a link to the block explorer for each transaction and account.</p>
          )}
          <ol className="log">
            {log.map((entry) => (
              <LogEntry key={entry.id} entry={entry} chain={chain} />
            ))}
          </ol>
        </div>
      </section>
    </div>
  );
}
