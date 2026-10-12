import { useCallback, useEffect, useState } from "react";
import { addNote, loadNotes, downloadNote, importNote, markSaved } from "./lib/store.js";
import { short } from "./lib/api.js";

function Chip({ tone, children }) {
  return <span className={`chip ${tone ?? ""}`}>{children}</span>;
}

function Link({ href, children }) {
  return (
    <a href={href} target="_blank" rel="noreferrer">
      {children}
    </a>
  );
}

const now = () => new Date().toLocaleTimeString();
const settle = (steps, state) => steps.map((x) => (x.state === "running" ? { ...x, state } : x));

/** One entry of the protocol log: an action and every step it went through. */
function LogEntry({ entry, chain }) {
  return (
    <li className={`entry ${entry.state}`}>
      <div className="row entry-head">
        <strong>{entry.title}</strong>
        <span className="muted">{entry.at}</span>
        {entry.state === "failed" && <Chip tone="bad">failed</Chip>}
      </div>
      <ol className="steps">
        {entry.steps.map((x, i) => (
          <li key={i} className={`step ${x.state}`}>
            <span className="mark" aria-hidden="true">{x.state === "running" ? <span className="spinner small" /> : x.state === "failed" ? "✕" : "✓"}</span>
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
              {typeof x.detail === "string" && <p className="hint">{x.detail}</p>}
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
        {entry.error && <li className="step failed"><span className="mark">✕</span><div>{entry.error}</div></li>}
      </ol>
    </li>
  );
}

export default function ChainPanel({ chain, copy, log, updateLog }) {
  const [balance, setBalance] = useState("…");
  const [pool, setPool] = useState(null);
  const [notes, setNotes] = useState(() => loadNotes(chain.id));
  const [statuses, setStatuses] = useState([]);
  const [busy, setBusy] = useState(null);
  const [error, setError] = useState(null);
  const [withdrawing, setWithdrawing] = useState(null);
  const [recipient, setRecipient] = useState("");
  const [importing, setImporting] = useState(false);
  const [pasted, setPasted] = useState("");
  const [importError, setImportError] = useState(null);
  // Keys offered as a file in this visit, awaiting the user's word that the file saved.
  const [offered, setOffered] = useState(() => new Set());

  // Each action opens a log entry; the chain adapter narrates its steps into it.
  function begin(title) {
    const id = `${Date.now()}-${Math.random()}`;
    const edit = (fn) => updateLog(chain.id, (l) => l.map((e) => (e.id === id ? fn(e) : e)));
    updateLog(chain.id, (l) => [{ id, title, at: now(), state: "running", steps: [] }, ...l].slice(0, 20));
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
  const refresh = useCallback(async ({ quiet = false } = {}) => {
    try {
      const [b, p] = await Promise.all([chain.balance(), chain.pool()]);
      setBalance(b);
      setPool(p);
      const n = loadNotes(chain.id);
      setNotes(n);
      setStatuses(await Promise.all(n.map((note) => chain.noteStatus(note, p))));
    } catch (e) {
      if (!quiet) setError(e.message);
    }
  }, [chain]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function run(label, fn) {
    setBusy(label);
    setError(null);
    const entry = begin(label);
    try {
      await fn(entry.step);
      entry.finish();
    } catch (e) {
      entry.fail(e.message);
      setError(e.message);
    } finally {
      // A failed action can still have changed the chain, as when the keeper
      // excludes a deposit before its withdrawal is refused.
      await refresh({ quiet: true });
      setBusy(null);
    }
  }

  const faucet = () => run("Test funds", (step) => chain.faucet(step));

  const deposit = () =>
    run(`Deposit ${chain.denominationLabel}`, async (step) => {
      await chain.deposit(step, (note) => addNote(chain.id, note));
      step("Download the withdrawal key below. It is the only way to withdraw this deposit, from this browser or any other", { done: true });
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
    if (file) importKey(await file.text());
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
  const restrict = (address) => run(`Block ${short(address)} in TIP-403 only`, (step) => chain.restrict(address, step));
  const sweep = () => run("Keeper", (step) => chain.sweep(step));

  const blockedAddresses = new Set(
    chain.id === "solana" ? pool?.blocklist ?? [] : (pool?.blocked ?? []).map((a) => a.toLowerCase()),
  );
  const pending = (pool?.deposits ?? []).filter((d) => d.pending).length;
  const isBlocked = (a) => blockedAddresses.has(chain.id === "solana" ? a : a.toLowerCase());
  const depositors = [...new Set((pool?.deposits ?? []).map((d) => d.depositor))];

  return (
    <main className="panel">
      <section className="intro">
        <p>{copy.lead}</p>
        <p className="muted">{copy.how}</p>
      </section>

      {busy && <div className="card busy"><span className="spinner" /> {busy}…</div>}
      {error && <div className="card error">{error}</div>}

      <div className="grid">
        <div className="col">
          <section className="card">
            <h2>1 · Your test wallet</h2>
            <p className="mono">
              <Link href={chain.addressLink(chain.me)}>{short(chain.me, 6)}</Link>
            </p>
            <p className="big">{balance}</p>
            <button onClick={faucet} disabled={!!busy}>Get test funds</button>
            <p className="hint">A throwaway wallet that lives in this browser, on a test network.</p>
          </section>

          <section className="card">
            <h2>2 · Deposit</h2>
            <p>Every deposit is exactly {chain.denominationLabel}, so withdrawals cannot be matched to deposits by amount.</p>
            <button className="primary" onClick={deposit} disabled={!!busy}>Deposit {chain.denominationLabel}</button>
            <p className="hint">This creates a withdrawal key in your browser. Whoever holds the key can withdraw the deposit, from any browser.</p>
          </section>

          <section className="card">
            <h2>3 · Your withdrawal keys</h2>
            {notes.length === 0 && <p className="muted">No withdrawal keys yet.</p>}
            <ul className="notes">
              {notes.map((note, i) => {
                const st = statuses[i];
                return (
                  <li key={i}>
                    <div className="row">
                      <span>{!st ? "Deposit #…" : st.index >= 0 ? `Deposit #${st.index}` : "Withdrawal key"}</span>
                      {st && <Chip tone={st.tone}>{st.label}</Chip>}
                      <button className={note.saved ? "link" : ""} onClick={() => save(note)}>Download key</button>
                    </div>
                    {!note.saved && !["Withdrawn", "Not deposited"].includes(st?.label) && (
                      offered.has(String(note.k)) ? (
                        <div className="row">
                          <p className="hint">Did the file save? Check your downloads before you confirm.</p>
                          <button onClick={() => confirmSaved(note)}>Yes, I have the file</button>
                        </div>
                      ) : (
                        <p className="hint bad">Not backed up. This key exists only in this browser: if its data is cleared, the deposit cannot be withdrawn. Download it and keep the file safe.</p>
                      )
                    )}
                    {st?.canWithdraw && withdrawing !== i && (
                      <button onClick={() => startWithdraw(i)} disabled={!!busy}>Withdraw…</button>
                    )}
                    {withdrawing === i && (
                      <div className="withdraw">
                        <label>
                          Pay to
                          <input className="mono" value={recipient} onChange={(e) => setRecipient(e.target.value)} />
                        </label>
                        <p className="hint">Prefilled with a brand-new address that has never been on chain. Its key is not kept anywhere, so on this test network the payment only shows that it arrived; paste an address you control to keep it. The relayer pays the fee and takes it from the withdrawal.</p>
                        <div className="row">
                          <button className="primary" onClick={() => withdraw(note)} disabled={!!busy}>Prove and withdraw</button>
                          <button className="link" onClick={() => setWithdrawing(null)}>Cancel</button>
                        </div>
                      </div>
                    )}
                    {st?.label === "Waiting for the batch" && chain.settle && (
                      <>
                        <p className="hint">The pool pays withdrawals in batches of at least two, so a single withdrawal cannot be timed back to its deposit. A lone one is paid once the pool's timeout has passed.</p>
                        <button onClick={settleBatch} disabled={!!busy}>Settle the batch</button>
                      </>
                    )}
                    {st?.label === "Awaiting the curator" && chain.publish && (
                      <>
                        <p className="hint">The curator has not yet published a clean root that covers this deposit.</p>
                        <button onClick={publishRoot} disabled={!!busy}>Ask the curator to publish</button>
                      </>
                    )}
                    {st?.label === "Blocked, not yet excluded" && (
                      <p className="hint bad">The issuer blocked this depositor, but no keeper has excluded the deposit yet. A withdrawal runs the keeper first, so it will be refused.</p>
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
              <div className="withdraw">
                <p className="hint">Deposited from another browser or device? Choose its key file, or paste the file's contents.</p>
                <input type="file" accept=".json,application/json" onChange={importFile} />
                <textarea className="mono" rows={4} value={pasted} onChange={(e) => setPasted(e.target.value)} placeholder='{ "version": 1, "chain": … }' />
                {importError && <p className="hint bad">{importError}</p>}
                <div className="row">
                  <button className="primary" onClick={() => importKey(pasted)} disabled={!pasted.trim()}>Import</button>
                  <button className="link" onClick={() => { setImporting(false); setImportError(null); }}>Cancel</button>
                </div>
              </div>
            )}
          </section>
          <section className="card">
            <h2>Protocol log</h2>
            {log.length === 0 && <p className="muted">Every step of every action appears here: what was computed in this browser, what each transaction did, and what the chain checked.</p>}
            <ol className="log">
              {log.map((entry) => (
                <LogEntry key={entry.id} entry={entry} chain={chain} />
              ))}
            </ol>
          </section>
        </div>

        <div className="col">
          <section className="card">
            <h2>The pool</h2>
            {!pool && <p className="muted">Reading the chain…</p>}
            {pool && (
              <>
                <p>
                  {pool.deposits.length} deposits · {pool.deposits.filter((d) => d.clean).length} in the clean set
                </p>
                <table>
                  <thead>
                    <tr><th>#</th><th>Depositor</th><th>Clean set</th></tr>
                  </thead>
                  <tbody>
                    {pool.deposits.map((d) => (
                      <tr key={d.index}>
                        <td>{d.index}</td>
                        <td className="mono">
                          <Link href={chain.addressLink(d.depositor)}>{short(d.depositor)}</Link>
                          {d.depositor === chain.me && <span className="you"> you</span>}
                        </td>
                        <td>
                          {d.uncurated ? <Chip tone="wait">pending</Chip> : d.pending ? <Chip tone="wait">blocked, still in</Chip> : d.clean ? <Chip tone="good">in</Chip> : <Chip tone="bad">excluded</Chip>}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                <p className="hint">Anyone can see who deposited. Nobody can see which deposit a withdrawal spends.</p>
                {pending > 0 && (
                  <p className="hint bad">
                    {pending === 1 ? "One deposit's depositor is" : `${pending} deposits' depositors are`} blocked by the token's policy but still in the clean set. Any keeper can exclude them, and the relayer does before every withdrawal.
                  </p>
                )}
              </>
            )}
          </section>

          <section className="card console">
            <h2>{copy.consoleTitle}</h2>
            <p className="muted">{copy.consoleHow}</p>
            <ul className="depositors">
              {depositors.map((a) => (
                <li key={a} className="row">
                  <span className="mono">{short(a)}{a === chain.me && <span className="you"> you</span>}</span>
                  <span className="row">
                    {chain.restrict && !isBlocked(a) && (
                      <button className="link" onClick={() => restrict(a)} disabled={!!busy}>TIP-403 only</button>
                    )}
                    <button className={isBlocked(a) ? "" : "danger"} onClick={() => block(a, isBlocked(a))} disabled={!!busy}>
                      {isBlocked(a) ? "Unblock" : "Block"}
                    </button>
                  </span>
                </li>
              ))}
            </ul>
            {chain.sweep && (
              <>
                <p className="hint">
                  "TIP-403 only" blocks the address in the token's policy and leaves the pool alone, as an issuer that has never heard of this pool would. The keeper then applies the policy: anyone can run it, since the pool checks every exclusion against the registry.
                </p>
                <button onClick={sweep} disabled={!!busy}>Run the keeper</button>
              </>
            )}
          </section>

        </div>
      </div>
    </main>
  );
}
