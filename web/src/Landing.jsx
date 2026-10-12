const REPO = "https://github.com/thiagorochatr/colosseum2026";

const DEMOS = [
  {
    id: "tempo",
    name: "Tempo",
    title: "A private stablecoin pool",
    text: "The issuer's own TIP-403 policy decides who can withdraw. Block a depositor and their deposit can no longer leave the pool.",
  },
  {
    id: "solana",
    name: "Solana",
    title: "Compliance for any privacy pool",
    text: "A curator publishes a clean set. The pool pays only against a proof of membership, checked at withdrawal, not only at deposit.",
  },
];

/** The front door: what Alibi is, and one way into each chain's demo. */
export default function Landing() {
  return (
    <div className="landing">
      <header className="landing-top">
        <span className="brand">
          <img className="brand-mark" src="/favicon.svg" alt="" width="24" height="24" />
          <span className="brand-name">Alibi</span>
        </span>
        <nav className="landing-links">
          <a href="https://x.com/alibi_zk" target="_blank" rel="noreferrer">
            @alibi_zk ↗
          </a>
          <a href={REPO} target="_blank" rel="noreferrer">
            Source ↗
          </a>
        </nav>
      </header>

      <main className="landing-main">
        <h1 className="landing-title">Private payments that keep the issuer's rules.</h1>
        <p className="landing-lead">
          Prove your deposit is in the clean set without revealing which one is yours. One zero-knowledge proof, verified on Tempo and on Solana.
        </p>

        <div className="landing-demos">
          {DEMOS.map((d) => (
            <a key={d.id} className="landing-demo" href={`/${d.id}`}>
              <span className="landing-chain">{d.name}</span>
              <span className="landing-demo-title">{d.title}</span>
              <span className="landing-demo-text">{d.text}</span>
              <span className="landing-go">Open the {d.name} demo →</span>
            </a>
          ))}
        </div>

        <p className="landing-note">
          Testnet only, not audited. A throwaway wallet is created in your browser.{" "}
          <a href={`${REPO}#how-it-works`} target="_blank" rel="noreferrer">
            How it works
          </a>
          {" · "}
          <a href={`${REPO}/blob/main/docs/PROOF.md`} target="_blank" rel="noreferrer">
            Proof
          </a>
        </p>
      </main>
    </div>
  );
}
