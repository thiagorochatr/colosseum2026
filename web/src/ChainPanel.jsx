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

export default function ChainPanel({ chain, copy }) {
  const [balance, setBalance] = useState("…");
  const [pool, setPool] = useState(null);
  const [notes, setNotes] = useState(() => loadNotes(chain.id));
  const [statuses, setStatuses] = useState([]);
  const [busy, setBusy] = useState(null);
  const [log, setLog] = useState([]);
  const [error, setError] = useState(null);
  const [withdrawing, setWithdrawing] = useState(null);
  const [recipient, setRecipient] = useState("");

  const say = (text, tx) => setLog((l) => [{ text, tx, at: new Date().toLocaleTimeString() }, ...l].slice(0, 12));

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
    try {
      await fn();
      await refresh();
    } catch (e) {
      setError(e.message);
    } finally {
      setBusy(null);
    }
  }

  const faucet = () =>
    run("Requesting test funds", async () => {
      const r = await chain.faucet();
      say(r.note ?? "Test funds received", r.tx);
    });

  const deposit = () =>
    run("Depositing", async () => {
      const { note, tx } = await chain.deposit();
      addNote(chain.id, note);
      say(`Deposited ${chain.denominationLabel}. The note is saved in this browser.`, tx);
    });

  const startWithdraw = (i) => {
    setWithdrawing(i);
    setRecipient(chain.freshRecipient());
  };

  const withdraw = (note) =>
    run("Withdrawing", async () => {
      const out = await chain.withdraw(note, recipient, pool, (stage) => setBusy(stage));
      setWithdrawing(null);
      if (chain.id === "solana" && !out.settled) {
        say(`Spent. It pays ${short(recipient)} once another withdrawal joins the batch: the pool settles crowds of at least two.`, out.tx);
      } else {
        say(`Withdrawn: ${out.received} paid to ${short(recipient)}, a fresh address.`, out.tx);
        if (out.settled) say("The batch settled, paying every pending withdrawal at once.", out.settled.tx);
      }
    });

  const block = (address, blocked) =>
    run(blocked ? "Unblocking" : "Blocking", async () => {
      const r = blocked ? await chain.unblock(address) : await chain.block(address);
      say(blocked ? `Unblocked ${short(address)}.` : `Blocked ${short(address)}: their deposits left the clean set.`, r.tx);
    });

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
                    {st?.label === "Excluded" && <p className="hint bad">The depositor was blocked, so this deposit is outside the clean set. No valid proof exists for it.</p>}
                  </li>
                );
              })}
            </ul>
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

          <section className="card">
            <h2>Activity</h2>
            {log.length === 0 && <p className="muted">Transactions from this session appear here.</p>}
            <ul className="log">
              {log.map((l, i) => (
                <li key={i}>
                  <span className="muted">{l.at}</span> {l.text} {l.tx && <Link href={chain.txLink(l.tx)}>view transaction</Link>}
                </li>
              ))}
            </ul>
          </section>
        </div>
      </div>
    </main>
  );
}
