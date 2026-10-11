import { useCallback, useEffect, useState } from "react";
import { addNote, loadNotes, downloadNote } from "./lib/store.js";
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
              <span>{x.text}</span> {x.tx && <Link href={chain.txLink(x.tx)}>transaction</Link>}
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

  // Each action opens a log entry; the chain adapter narrates its steps into it.
  function begin(title) {
    const id = `${Date.now()}-${Math.random()}`;
    const edit = (fn) => updateLog(chain.id, (l) => l.map((e) => (e.id === id ? fn(e) : e)));
    updateLog(chain.id, (l) => [{ id, title, at: now(), state: "running", steps: [] }, ...l].slice(0, 20));
    return {
      step(text, { detail, tx, done = false } = {}) {
        if (!done) setBusy(text);
        edit((e) => ({ ...e, steps: [...settle(e.steps, "done"), { text, detail, tx, state: done ? "done" : "running" }] }));
      },
      finish: () => edit((e) => ({ ...e, state: "done", steps: settle(e.steps, "done") })),
      fail: (error) => edit((e) => ({ ...e, state: "failed", error, steps: settle(e.steps, "failed") })),
    };
  }

  const refresh = useCallback(async () => {
    try {
      const [b, p] = await Promise.all([chain.balance(), chain.pool()]);
      setBalance(b);
      setPool(p);
      const n = loadNotes(chain.id);
      setNotes(n);
      setStatuses(await Promise.all(n.map((note) => chain.noteStatus(note, p))));
    } catch (e) {
      setError(e.message);
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
      await refresh();
    } catch (e) {
      entry.fail(e.message);
      setError(e.message);
    } finally {
      setBusy(null);
    }
  }

  const faucet = () => run("Test funds", (step) => chain.faucet(step));

  const deposit = () =>
    run(`Deposit ${chain.denominationLabel}`, async (step) => {
      const { note } = await chain.deposit(step);
      addNote(chain.id, note);
    });

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

  const block = (address, blocked) =>
    run(blocked ? `Unblock ${short(address)}` : `Block ${short(address)}`, (step) => (blocked ? chain.unblock(address, step) : chain.block(address, step)));

  const blockedAddresses = new Set(
    chain.id === "solana"
      ? pool?.blocklist ?? []
      : (pool?.deposits ?? []).filter((d) => !d.clean).map((d) => d.depositor.toLowerCase()),
  );
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
            <p className="hint">This creates a secret note in your browser. Whoever holds the note can withdraw the deposit.</p>
          </section>

          <section className="card">
            <h2>3 · Your notes</h2>
            {notes.length === 0 && <p className="muted">No notes yet.</p>}
            <ul className="notes">
              {notes.map((note, i) => {
                const st = statuses[i];
                return (
                  <li key={i}>
                    <div className="row">
                      <span>Deposit #{st?.index >= 0 ? st.index : "…"}</span>
                      {st && <Chip tone={st.tone}>{st.label}</Chip>}
                      <button className="link" onClick={() => downloadNote(note)}>Download note</button>
                    </div>
                    {st?.canWithdraw && withdrawing !== i && (
                      <button onClick={() => startWithdraw(i)} disabled={!!busy}>Withdraw…</button>
                    )}
                    {withdrawing === i && (
                      <div className="withdraw">
                        <label>
                          Pay to
                          <input className="mono" value={recipient} onChange={(e) => setRecipient(e.target.value)} />
                        </label>
                        <p className="hint">Prefilled with a brand-new address that has never been on chain. The relayer pays the fee and takes it from the withdrawal.</p>
                        <div className="row">
                          <button className="primary" onClick={() => withdraw(note)} disabled={!!busy}>Prove and withdraw</button>
                          <button className="link" onClick={() => setWithdrawing(null)}>Cancel</button>
                        </div>
                      </div>
                    )}
                    {st?.label === "Waiting for the batch" && chain.settle && (
                      <>
                        <p className="hint">The pool pays withdrawals in batches of at least two, so a single withdrawal cannot be timed back to its deposit.</p>
                        <button onClick={settleBatch} disabled={!!busy}>Settle the batch</button>
                      </>
                    )}
                    {st?.label === "Excluded" && <p className="hint bad">The depositor was blocked, so this deposit is outside the clean set. No valid proof exists for it.</p>}
                  </li>
                );
              })}
            </ul>
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
                        <td>{d.uncurated ? <Chip tone="wait">pending</Chip> : d.clean ? <Chip tone="good">in</Chip> : <Chip tone="bad">excluded</Chip>}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                <p className="hint">Anyone can see who deposited. Nobody can see which deposit a withdrawal spends.</p>
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
                  <button className={isBlocked(a) ? "" : "danger"} onClick={() => block(a, isBlocked(a))} disabled={!!busy}>
                    {isBlocked(a) ? "Unblock" : "Block"}
                  </button>
                </li>
              ))}
            </ul>
          </section>

        </div>
      </div>
    </main>
  );
}
