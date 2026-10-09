//! `mirror` — the operator and member CLI.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

mod chain;
mod client;
mod crowd;
mod disclose;
mod history;
mod lookup;
mod note;
mod soak;
mod stake;

/// The public devnet cluster, so the common case needs no flag.
const DEFAULT_URL: &str = "https://api.devnet.solana.com";
/// Where the Solana CLI keeps its default keypair.
const DEFAULT_KEYPAIR: &str = "~/.config/solana/id.json";

/// The seed the committed verifying key was generated from.
///
/// Public on purpose. It makes the setup reproducible — anyone can re-derive the
/// deployed key and check it against the circuit in this repository — and it
/// makes the setup insecure in exactly the way a public seed implies: the toxic
/// waste is public, so proofs are forgeable. That trade is stated rather than
/// hidden, and the production path is a multi-party ceremony.
///
/// Reproducible-and-insecure is a coherent position for an unaudited tool. The
/// incoherent one is publishing the entropy *and* withholding the proving key,
/// which is insecure and unreproducible at once: the toxic waste is public, and
/// no third party can regenerate the key to produce a valid proof at all.
pub const DEV_SETUP_SEED: &str = "mirror-pool-reproducible-dev-setup-v1";

#[derive(Parser)]
#[command(
    name = "mirror",
    about = "mirror-pool: a behavioral anonymity set on Solana",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generates the Groth16 keys and writes the verifying key as Rust source
    /// for the on-chain program.
    Setup {
        #[arg(long, default_value = DEV_SETUP_SEED)]
        seed: String,
        #[arg(long, default_value = "programs/mirror-pool/src/vk.rs")]
        out: PathBuf,
    },
    /// Checks that an RPC endpoint can actually serve the history a provenance
    /// measurement depends on, and refuses it otherwise.
    ///
    /// Run this before trusting any number a collection produces. A truncated
    /// endpoint does not error on old data — it returns nothing — so a collector
    /// pointed at one reports every old funding event as absent and produces a
    /// graph that looks like a finding.
    CheckEndpoint {
        #[arg(long, default_value = "https://api.mainnet-beta.solana.com")]
        endpoint: String,
        /// Requests per second. The public endpoint sustains a measured
        /// 0.28-0.55, far below its documented allowance.
        #[arg(long, default_value_t = 0.4)]
        rps: f64,
    },
    /// Builds a member-weighted frame from a pool's depositors.
    ///
    /// Each depositor appears once however often they transact, and the sample
    /// is spread across the pool's whole signature history rather than its most
    /// recent minute. Nothing is dropped for looking hard to trace.
    Seeds {
        /// The pool program to enumerate depositors of.
        #[arg(long)]
        program: String,
        /// How many distinct depositors to collect.
        #[arg(long, default_value_t = 40)]
        n: usize,
        /// Signature pages to walk back through. More pages means a sample
        /// spread over more of the pool's lifetime.
        #[arg(long, default_value_t = 8)]
        pages: u32,
        /// Transactions to fetch in total, spread across the pages.
        ///
        /// Sized from the measured deposit rate rather than guessed: about one
        /// program transaction in eight is a deposit, so reaching N depositors
        /// costs roughly 8N fetches.
        #[arg(long, default_value_t = 400)]
        scan: usize,
        /// Minimum lamports a payer must part with for it to count as a deposit.
        #[arg(long, default_value_t = 10_000_000)]
        min_deposit: u64,
        #[arg(long, default_value = "seeds.txt")]
        out: PathBuf,
        #[arg(long, default_value = "https://api.mainnet-beta.solana.com")]
        endpoint: String,
        #[arg(long, default_value_t = 0.4)]
        rps: f64,
    },
    /// Pass one: walks each seed's funding chain and writes a sample file.
    ///
    /// The only step that touches the network. Everything it observes goes into
    /// the sample, so the number can be recomputed later by anyone holding it.
    Collect {
        /// File with one seed address per line.
        #[arg(long)]
        seeds: PathBuf,
        #[arg(long, default_value = "sample.json")]
        out: PathBuf,
        #[arg(long, default_value = "https://api.mainnet-beta.solana.com")]
        endpoint: String,
        #[arg(long, default_value_t = 0.4)]
        rps: f64,
        #[arg(long, default_value_t = 6)]
        depth: u32,
        /// Signature pages to walk before declaring an address high-activity.
        ///
        /// Raising it buys a better age estimate for busy funders, which is what
        /// lets the volume-hub rule classify them instead of leaving them in the
        /// budget bucket.
        #[arg(long, default_value_t = 20)]
        page_cap: u32,
    },
    /// Pass two: classifies a committed sample and reports the anonymity ladder.
    ///
    /// Pure and offline. Given the same sample it always produces the same
    /// numbers, which is what makes a published result checkable by someone who
    /// does not trust our endpoint or our run.
    Analyze {
        #[arg(long, default_value = "sample.json")]
        sample: PathBuf,
    },
    /// Compares the loss factor of two populations measured the same way.
    ///
    /// `ρ` is the headline because it is independent of `k` and therefore
    /// comparable across pools of different sizes. This is the command that
    /// exercises that property rather than asserting it.
    ///
    /// The comparison is a bootstrap over the difference, not a subtraction of
    /// two point estimates. Two samples drawn from the same underlying shape
    /// will differ by *something*, and reporting that something as a finding is
    /// the error this command exists to prevent: if the interval contains zero,
    /// it says so and declines to rank them.
    Compare {
        /// The population being examined.
        #[arg(long)]
        sample: PathBuf,
        /// The population it is measured against.
        #[arg(long)]
        against: PathBuf,
        /// A name for each, used only in the output.
        #[arg(long, default_value = "sample")]
        label: String,
        #[arg(long, default_value = "baseline")]
        against_label: String,
    },
    /// Tests whether *being resolvable* is correlated with provenance class.
    ///
    /// This is the assumption every unresolved bracket and every cross-population
    /// comparison quietly rests on. Dropping unresolved members is only harmless
    /// if the members that resolve are a fair draw of the classes present. If
    /// easy-to-trace members are systematically exchange-funded and hard ones are
    /// systematically something else, then the resolved subset is not the
    /// population, and a difference between two such subsets can be pure
    /// selection.
    ///
    /// Given the same frame collected at two budgets, it splits the larger run's
    /// resolved members into those the smaller run also resolved and those only
    /// the larger one reached, and asks whether those two groups have the same
    /// class distribution. **Separation is bad news** — it is evidence that the
    /// unresolved are not missing at random.
    Selection {
        /// The smaller-budget collection.
        #[arg(long)]
        earlier: PathBuf,
        /// The larger-budget collection of the same frame.
        #[arg(long)]
        later: PathBuf,
    },
    /// Runs the whole lifecycle against a live cluster and prints every
    /// signature, so the result is checkable rather than asserted.
    Soak {
        #[arg(long)]
        program: String,
        #[arg(long, default_value = "https://api.devnet.solana.com")]
        url: String,
        /// Payer and settler keypair.
        #[arg(long, default_value = "~/.config/solana/id.json")]
        keypair: String,
        /// Where to write the evidence.
        #[arg(long, default_value = "docs/PROOF.md")]
        out: PathBuf,
    },
    /// Settles a batch of stake delegations to *different* validators, and
    /// measures what that divergence costs in packet space.
    ///
    /// The claim `Soak` makes is that the pool can be a member's authority. This
    /// one asks the harder question: whether a crowd survives its members
    /// wanting different things.
    Crowd {
        #[arg(long)]
        program: String,
        #[arg(long, default_value = "https://api.devnet.solana.com")]
        url: String,
        /// Payer and settler keypair.
        #[arg(long, default_value = "~/.config/solana/id.json")]
        keypair: String,
        /// Where to write the evidence.
        #[arg(long, default_value = "docs/CROWD.md")]
        out: PathBuf,
        /// Re-render the report from the last run's recorded results, without
        /// touching the cluster. For fixing the prose around numbers that were
        /// already measured.
        #[arg(long)]
        render_only: bool,
    },
    /// Proves to a verifier of your choosing that a settled action was yours.
    ///
    /// Nothing on chain changes and nobody else learns anything: a disclosure is
    /// a file handed to one counterparty, not a key anybody holds. It carries
    /// the note's secrets, which authorise nothing once the nullifier is burnt —
    /// and which would hand over the deposit if the note were still unspent,
    /// which is why this refuses to write one for a note that has not settled.
    Disclose {
        #[arg(long)]
        program: String,
        #[arg(long)]
        note: PathBuf,
        /// Where to write the disclosure. Overwrites: unlike a note, it can be
        /// regenerated from the note at any time.
        #[arg(long, default_value = "disclosure.json")]
        out: PathBuf,
        /// Disclose even though doing so leaves the pool's remaining settled
        /// actions below its floor. The cover that costs is the other members',
        /// and they are neither asked nor told.
        #[arg(long)]
        i_accept_the_cost_to_others: bool,
        #[arg(long, default_value = DEFAULT_URL)]
        url: String,
    },
    /// Checks a disclosure against the chain, recomputing every value in it.
    ///
    /// Nothing in the file is taken on trust. The commitment and the nullifier
    /// are rederived from the secrets, the spend record's address is derived
    /// from that nullifier, the accumulator is rebuilt from history and checked
    /// against the pool's own root, and the action is read out of the record.
    /// Every check is reported separately, and any check that cannot be
    /// completed is a failure rather than a silence.
    DiscloseVerify {
        #[arg(long)]
        file: PathBuf,
        #[arg(long, default_value = DEFAULT_URL)]
        url: String,
    },
    /// Reclaims a settlement's lookup table once its cooldown has passed.
    ///
    /// A large settlement publishes a table naming every account it touches.
    /// This returns the rent and removes that list.
    CloseTable {
        #[arg(long)]
        table: String,
        #[arg(long, default_value = "https://api.devnet.solana.com")]
        url: String,
        #[arg(long, default_value = "~/.config/solana/id.json")]
        keypair: String,
    },
    /// Recomputes the verifying key from a seed and reports its digest.
    ///
    /// This is the check a third party runs. It binds the *whole* key — alpha,
    /// beta, gamma, delta and every IC point — not just delta, so a transcript
    /// carrying a key belonging to a different circuit cannot pass. Verifying
    /// delta alone is the common shortcut and it would certify exactly that.
    VerifySetup {
        #[arg(long, default_value = DEV_SETUP_SEED)]
        seed: String,
        /// Expected digest, as printed by `setup`.
        #[arg(long)]
        expect: Option<String>,
    },

    /// Creates a pool for a denomination. Permissionless, and one per size.
    InitPool {
        #[arg(long)]
        program: String,
        /// Lamports each deposit escrows. This also names the pool.
        #[arg(long)]
        denomination: u64,
        /// Notes the pool must hold before it will act.
        #[arg(long, default_value_t = 2)]
        k_floor: u32,
        /// How long a spend waits before it may settle below the floor.
        ///
        /// Fixed at creation and never changeable, like the floor itself. Zero
        /// takes the program's default of one hour.
        #[arg(long, default_value_t = 0)]
        settle_timeout: u32,
        #[arg(long, default_value = DEFAULT_URL)]
        url: String,
        #[arg(long, default_value = DEFAULT_KEYPAIR)]
        keypair: String,
    },

    /// Draws a fresh note and writes its secret to a file.
    ///
    /// The file is the deposit. Nobody can reissue it, which is the same
    /// property that means nobody can freeze it.
    NoteNew {
        /// The pool this note will join, named by its denomination.
        #[arg(long)]
        denomination: u64,
        /// Where to write it. Refuses to overwrite.
        #[arg(long, default_value = "note.json")]
        out: PathBuf,
    },

    /// Escrows a denomination and adds the note to the anonymity set.
    Deposit {
        #[arg(long)]
        program: String,
        #[arg(long)]
        note: PathBuf,
        #[arg(long, default_value = DEFAULT_URL)]
        url: String,
        #[arg(long, default_value = DEFAULT_KEYPAIR)]
        keypair: String,
    },

    /// Rebuilds the accumulator from chain history and checks it against the pool.
    ///
    /// Needs no indexer and no local file. The check is what makes a membership
    /// proof trustworthy: a rebuilt root that matches the chain's is proof the
    /// recovered leaf set is complete and correctly ordered.
    Tree {
        #[arg(long)]
        program: String,
        #[arg(long)]
        denomination: u64,
        #[arg(long, default_value = DEFAULT_URL)]
        url: String,
    },

    /// Proves membership and records an action, signed by a relay.
    ///
    /// The relay signs so the member never does. A member who signs their own
    /// spend has published the link this pool exists to break.
    Spend {
        #[arg(long)]
        program: String,
        #[arg(long)]
        note: PathBuf,
        /// Who receives the payout.
        #[arg(long)]
        to: String,
        /// The relay's keypair. Must not be the member's wallet.
        #[arg(long)]
        relay: String,
        /// Taken out of the denomination, never added to it.
        #[arg(long, default_value_t = 0)]
        relay_fee: u64,
        /// Call this program instead of paying the beneficiary directly.
        #[arg(long)]
        invoke: Option<String>,
        /// Instruction data for --invoke, hex.
        #[arg(long, default_value = "")]
        payload: String,
        /// How many accounts the invoked instruction takes. Bound into the proof.
        #[arg(long, default_value_t = 0)]
        accounts: u8,
        /// Hand the pool's vault to the callee as a signer, so the pool acts as
        /// the authority. This is what a stake delegation needs.
        #[arg(long)]
        pool_signs: bool,
        #[arg(long, default_value = DEFAULT_URL)]
        url: String,
    },

    /// Executes every pending transfer in one transaction. Permissionless.
    Settle {
        #[arg(long)]
        program: String,
        #[arg(long)]
        denomination: u64,
        /// Consent to settling a batch smaller than the pool's floor.
        ///
        /// Without it, a batch below the floor is reported and left alone. The
        /// program refuses one that nobody asked for, and a settlement that
        /// does land below the floor is marked as such on chain — the members
        /// in it got a smaller crowd than the pool advertises, and that should
        /// be somebody's decision rather than a default.
        #[arg(long)]
        allow_below_floor: bool,
        #[arg(long, default_value = DEFAULT_URL)]
        url: String,
        #[arg(long, default_value = DEFAULT_KEYPAIR)]
        keypair: String,
    },
}

fn parse_program(s: &str) -> Result<solana_program::pubkey::Pubkey> {
    s.parse()
        .map_err(|e| anyhow::anyhow!("--program is not a pubkey: {e}"))
}

fn vk_digest(vk: &mirror_circuit::SolanaVerifyingKey) -> String {
    let mut hasher = Sha256::new();
    hasher.update(vk.digest_preimage());
    hex::encode(hasher.finalize())
}

/// Loads a committed sample and returns each seed's class label, where it
/// reached one.
///
/// Keyed by seed rather than flattened, so two collections of the same frame can
/// be aligned member by member.
fn labels_by_seed(path: &std::path::Path) -> Result<std::collections::BTreeMap<String, String>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let sample = mirror_provenance::Sample::from_json(&text)?;
    let (results, _) = mirror_provenance::classify_sample(
        &sample,
        &mirror_provenance::AnchorSet::default(),
        &mirror_provenance::Thresholds::default(),
        sample.manifest.collected_at,
    );
    Ok(results
        .into_iter()
        .filter_map(|(seed, o)| o.label().map(|l| (seed, l.to_string())))
        .collect())
}

/// Loads a committed sample and returns one class label per resolved member.
///
/// Applies the same failure gate `analyze` does. A comparison drawn from a run
/// whose unresolved bucket is substantially our own infrastructure would be
/// comparing endpoints rather than populations, and it would do so invisibly —
/// the difference between two pools and the difference between two collection
/// runs look identical in the output.
fn resolved_labels(path: &std::path::Path) -> Result<(Vec<String>, u64)> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let sample = mirror_provenance::Sample::from_json(&text)?;
    let (results, census) = mirror_provenance::classify_sample(
        &sample,
        &mirror_provenance::AnchorSet::default(),
        &mirror_provenance::Thresholds::default(),
        sample.manifest.collected_at,
    );
    if !census.may_publish() {
        anyhow::bail!(
            "{}: the RPC failure rate is {:.2}%, above the 1% limit. This sample yields no \
             headline on its own and cannot be one side of a comparison.",
            path.display(),
            census.failure_rate() * 100.0
        );
    }
    let labels: Vec<String> = results
        .iter()
        .filter_map(|(_, o)| o.label().map(|s| s.to_string()))
        .collect();
    // Unresolved excluding our own RPC failures, which belong to neither side.
    let unresolved = census.measurable() - census.resolved;
    Ok((labels, unresolved))
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Setup { seed, out } => {
            eprintln!("generating keys from seed {seed:?} (this takes a moment)");
            let keys = mirror_circuit::generate_reproducible(seed.as_bytes())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let svk = keys.solana_vk();
            let source = mirror_circuit::vk_to_rust_source(&svk);
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::write(&out, source).with_context(|| format!("writing {}", out.display()))?;
            println!("wrote {}", out.display());
            println!("public inputs: {}", svk.public_input_count());
            println!("vk sha256:     {}", vk_digest(&svk));
            Ok(())
        }
        Command::CheckEndpoint { endpoint, rps } => {
            let mut client = mirror_provenance::RpcClient::new(&endpoint, rps);
            match client.check_preconditions() {
                Ok(check) => {
                    println!("endpoint             {}", check.endpoint);
                    println!("first available block {}", check.first_available_block);
                    println!(
                        "archival probe slot   {} ok",
                        mirror_provenance::rpc::ARCHIVAL_PROBE_SLOT
                    );
                    println!("\nUSABLE — this endpoint serves history to genesis.");
                    Ok(())
                }
                Err(e) => {
                    eprintln!("REFUSED: {e}");
                    eprintln!(
                        "\nA measurement taken here would report old funding events as \
                         absent and the resulting unresolved bucket would be an artifact."
                    );
                    std::process::exit(2);
                }
            }
        }
        Command::Seeds {
            program,
            n,
            pages,
            scan,
            min_deposit,
            out,
            endpoint,
            rps,
        } => {
            let mut client = mirror_provenance::RpcClient::new(&endpoint, rps);
            let check = client
                .check_preconditions()
                .map_err(|e| anyhow::anyhow!("endpoint refused: {e}"))?;
            eprintln!(
                "endpoint ok (first available block {})",
                check.first_available_block
            );

            let mut seeds: Vec<String> = Vec::new();
            let mut seen = std::collections::BTreeSet::new();
            let mut before: Option<String> = None;
            let mut scanned = 0usize;
            let mut slots: Vec<u64> = Vec::new();

            'paging: for page in 0..pages {
                let batch = client
                    .signatures_for_address(&program, before.as_deref(), 1_000)
                    .map_err(|e| anyhow::anyhow!("listing program signatures: {e}"))?;
                if batch.is_empty() {
                    break;
                }
                before = Some(batch.last().expect("non-empty").signature.clone());
                eprintln!(
                    "page {page}: {} signatures, {} depositors so far",
                    batch.len(),
                    seeds.len()
                );

                // Spread the scan budget evenly over the pages, and stride
                // within each page so the sample spans its time range instead of
                // clustering at its head.
                let scan_this_page = (scan / pages as usize).max(1);
                let stride = (batch.len() / scan_this_page).max(1);
                for info in batch.iter().step_by(stride) {
                    if info.err {
                        continue;
                    }
                    scanned += 1;
                    let tx = match client.transaction(&info.signature) {
                        Ok(t) => t,
                        // A failure here drops one candidate; it never becomes a
                        // claim about the pool.
                        Err(_) => continue,
                    };
                    if let Some(d) = mirror_provenance::depositor_of(&tx, min_deposit) {
                        if seen.insert(d.clone()) {
                            slots.push(tx.slot);
                            seeds.push(d);
                            if seeds.len() >= n {
                                break 'paging;
                            }
                        }
                    }
                }
            }

            anyhow::ensure!(!seeds.is_empty(), "no depositors found for {program}");
            std::fs::write(&out, format!("{}\n", seeds.join("\n")))
                .with_context(|| format!("writing {}", out.display()))?;

            let span = match (slots.iter().min(), slots.iter().max()) {
                (Some(lo), Some(hi)) => hi - lo,
                _ => 0,
            };
            println!(
                "wrote {} ({} distinct depositors)",
                out.display(),
                seeds.len()
            );
            println!(
                "scanned {scanned} transactions, {} rpc calls",
                client.calls_made()
            );
            println!(
                "sample spans {span} slots (~{:.1} hours of chain time)",
                span as f64 * 0.4 / 3600.0
            );
            Ok(())
        }
        Command::Collect {
            seeds,
            out,
            endpoint,
            rps,
            depth,
            page_cap,
        } => {
            let text = std::fs::read_to_string(&seeds)
                .with_context(|| format!("reading {}", seeds.display()))?;
            let seed_list: Vec<String> = text
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect();
            anyhow::ensure!(!seed_list.is_empty(), "no seeds in {}", seeds.display());

            let mut client = mirror_provenance::RpcClient::new(&endpoint, rps);
            let check = client
                .check_preconditions()
                .map_err(|e| anyhow::anyhow!("endpoint refused: {e}"))?;
            eprintln!(
                "endpoint ok (first available block {}), collecting {} seeds",
                check.first_available_block,
                seed_list.len()
            );

            let config = mirror_provenance::CollectionConfig {
                depth_max: depth,
                sig_page_cap: page_cap,
                ..Default::default()
            };
            let thresholds = mirror_provenance::Thresholds::default();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs() as i64;

            let collector = mirror_provenance::Collector::new(&mut client, config);
            let started = std::time::Instant::now();
            let sample =
                collector.collect_with_progress(&seed_list, &check, &thresholds, now, |p| {
                    // Rate is the useful number here: it tells the operator
                    // whether the run is progressing or the endpoint has
                    // started throttling.
                    let elapsed = started.elapsed().as_secs_f64().max(0.001);
                    eprintln!(
                        "  [{:>3}/{}] {}… {} hops, {:?}  ({} calls, {:.1}/s, {:.0}s elapsed){}",
                        p.done,
                        p.total,
                        &p.seed[..p.seed.len().min(8)],
                        p.hops,
                        p.stop,
                        p.rpc_calls,
                        p.rpc_calls as f64 / elapsed,
                        elapsed,
                        p.error
                            .as_deref()
                            .map(|e| format!("\n        -> {}", &e[..e.len().min(160)]))
                            .unwrap_or_default(),
                    );
                });

            std::fs::write(&out, sample.to_json()?)
                .with_context(|| format!("writing {}", out.display()))?;
            println!(
                "wrote {} ({} rpc calls)",
                out.display(),
                sample.manifest.rpc_calls
            );
            if !sample.manifest.excluded_non_wallet.is_empty() {
                println!(
                    "excluded {} seeds that are not wallets (program, PDA, token account, \
                     or closed): a definitional frame criterion, not a difficulty one",
                    sample.manifest.excluded_non_wallet.len()
                );
            }
            println!(
                "ambiguous attribution rate: {:.1}%",
                sample.manifest.ambiguous_attribution_rate * 100.0
            );
            Ok(())
        }
        Command::Analyze { sample } => {
            let text = std::fs::read_to_string(&sample)
                .with_context(|| format!("reading {}", sample.display()))?;
            let sample = mirror_provenance::Sample::from_json(&text)?;
            let thresholds = mirror_provenance::Thresholds::default();
            let anchors = mirror_provenance::AnchorSet::default();

            let (results, census) = mirror_provenance::classify_sample(
                &sample,
                &anchors,
                &thresholds,
                sample.manifest.collected_at,
            );

            println!("endpoint  {}", sample.manifest.endpoint);
            println!("scope     {:?}", sample.manifest.config.scope);
            println!("census    {}", census.summary());
            println!(
                "frame     {} seeds excluded as non-wallets before tracing",
                sample.manifest.excluded_non_wallet.len()
            );
            println!();

            if !census.may_publish() {
                eprintln!(
                    "REFUSING to report a headline: the RPC failure rate is {:.2}%, above the \
                     1% limit. Above that threshold the unresolved bucket is substantially our \
                     own making and any effective-k computed from it would be measuring our \
                     infrastructure rather than the pool.",
                    census.failure_rate() * 100.0
                );
                std::process::exit(3);
            }

            let labels: Vec<String> = results
                .iter()
                .filter_map(|(_, o)| o.label().map(|s| s.to_string()))
                .collect();

            // Members that reached no class, excluding our own failures: those
            // belong to neither reading of the bracket.
            let unresolved = census.measurable() - census.resolved;

            match mirror_provenance::Anonymity::from_labels(&labels) {
                None => {
                    println!("no member resolved to a class; nothing to report");
                }
                Some(a) => {
                    // The bracket is built first and the report refuses without
                    // it. A point estimate alone does not say whether the number
                    // is driven by what was measured or by what was not, and a
                    // reader who copies one line out of this output should not
                    // be able to end up holding a bare effective-k.
                    let mut sizes: std::collections::BTreeMap<&str, u64> =
                        std::collections::BTreeMap::new();
                    for l in &labels {
                        *sizes.entry(l.as_str()).or_insert(0) += 1;
                    }
                    let resolved_sizes: Vec<u64> = sizes.into_values().collect();
                    let Some(bracket) =
                        mirror_provenance::Bracket::new(&resolved_sizes, unresolved)
                    else {
                        eprintln!(
                            "REFUSING to report: the resolved members do not form a partition \
                             this can bracket, so any effective-k printed here would be a point \
                             estimate with nothing to say how much of it is the pool and how \
                             much is the tracer's budget."
                        );
                        std::process::exit(3);
                    };

                    // Sampling error, which the bracket does not cover. These
                    // depositors are a draw from a larger population, and
                    // without an interval over that draw a reader cannot tell a
                    // real difference between two pools from a lucky sample.
                    let sampling = mirror_provenance::loss_factor_interval(
                        &labels,
                        mirror_provenance::bootstrap::DEFAULT_REPLICATES,
                        mirror_provenance::bootstrap::DEFAULT_SEED,
                    );

                    // One renderer, and it is the only thing that can print an
                    // effective-k.
                    println!(
                        "{}",
                        mirror_provenance::Quotation::new(a.clone(), bracket)
                            .with_sampling(sampling.clone())
                    );

                    println!();
                    println!("class-size CCDF (share of members in a class of at most t):");
                    for (t, share) in &a.class_size_ccdf {
                        println!("  t={t:<4} {:.4}", share);
                    }

                    if let Some(i) = &sampling {
                        if !i.contains_point() {
                            println!();
                            println!(
                                "  The resampling range does not contain the point estimate, and \
                                 that is a\n  property of this population rather than an error. \
                                 Resampling leaves about 37% of\n  members unpicked, so \
                                 single-member classes vanish from most replicates; fewer\n  \
                                 classes means lower H(C) and therefore higher rho. The size of \
                                 that gap is a\n  tail diagnostic."
                            );
                        }
                        println!();
                        println!(
                            "  Resampling is the spread of the estimator, not its distance from \
                             the truth.\n  Plug-in entropy is biased low at small n, so rho is \
                             biased HIGH: the real loss\n  factor is plausibly below all of this, \
                             and equally so for any population measured\n  this way."
                        );
                    }

                    if a.good_turing_coverage < 0.8 {
                        println!();
                        println!(
                            "  UNDER-SAMPLED: Good-Turing coverage {:.2} and Chao1 estimates {:.0} \
                             classes against\n  {} observed, so most of the class distribution \
                             was never seen. Effective-k measured at\n  small k understates the \
                             steady-state loss and does not extrapolate upward.",
                            a.good_turing_coverage, a.chao1, a.classes
                        );
                    }
                }
            }
            Ok(())
        }
        Command::Compare {
            sample,
            against,
            label,
            against_label,
        } => {
            let (a, a_unresolved) = resolved_labels(&sample)?;
            let (b, b_unresolved) = resolved_labels(&against)?;

            let reps = mirror_provenance::bootstrap::DEFAULT_REPLICATES;
            let seed = mirror_provenance::bootstrap::DEFAULT_SEED;

            let ia = mirror_provenance::loss_factor_interval(&a, reps, seed)
                .ok_or_else(|| anyhow::anyhow!("{label}: no member resolved to a class"))?;
            let ib = mirror_provenance::loss_factor_interval(&b, reps, seed)
                .ok_or_else(|| anyhow::anyhow!("{against_label}: no member resolved to a class"))?;

            let width = label.len().max(against_label.len()).max(10);
            println!(
                "{:<width$}  members   rho      resampled 2.5-97.5%   bias",
                "population"
            );
            for (name, labels, i) in [(&label, &a, &ia), (&against_label, &b, &ib)] {
                println!(
                    "{name:<width$}  {:>7}   {:.4}   {:.4} .. {:.4}     {:+.4}",
                    labels.len(),
                    i.point,
                    i.lo,
                    i.hi,
                    i.resampling_bias()
                );
            }

            let d = mirror_provenance::difference_interval(&a, &b, reps, seed)
                .ok_or_else(|| anyhow::anyhow!("nothing to compare"))?;
            println!();
            println!(
                "difference ({label} − {against_label}): {:+.4}   95% {:+.4} .. {:+.4}",
                d.point, d.lo, d.hi
            );
            println!();

            // The same informativeness gate `analyze` applies to a single
            // headline, applied to each side. It matters *more* here, not less.
            //
            // Comparing two populations of which one is mostly unresolved is
            // comparing their traceable subsets, and traceability is not
            // independent of provenance class: a wallet funded by an exchange
            // resolves in one hop, and one funded through a chain of fresh
            // intermediaries exhausts the budget. So the unresolved members are
            // plausibly drawn from different classes than the resolved ones, and
            // a difference between the two subsets can be manufactured entirely
            // by that selection.
            let under = |labels: &[String], unresolved: u64| -> bool {
                let total = labels.len() as u64 + unresolved;
                total > 0 && (labels.len() as u64) * 2 < total
            };
            let a_under = under(&a, a_unresolved);
            let b_under = under(&b, b_unresolved);
            if a_under || b_under {
                let who = match (a_under, b_under) {
                    (true, true) => format!("{label} and {against_label} both resolve"),
                    (true, false) => format!("{label} resolves"),
                    _ => format!("{against_label} resolves"),
                };
                println!(
                    "REFUSING to rank these populations: {who} fewer than half its members.\n\n\
                     What is left after the unresolved are dropped is each population's\n\
                     *traceable* subset, and traceability is not independent of provenance\n\
                     class — a wallet funded straight from an exchange resolves in one hop,\n\
                     one funded through fresh intermediaries exhausts the budget. A difference\n\
                     between two such subsets can be produced entirely by that selection, and\n\
                     nothing in the numbers above would show it.\n\n\
                     The figures are printed for completeness, not as a comparison. Closing\n\
                     this needs resolution above half on both sides, which is a bigger\n\
                     traversal budget rather than a different metric."
                );
                return Ok(());
            }

            if d.excludes_zero() {
                let (more, less) = if d.point > 0.0 {
                    (&label, &against_label)
                } else {
                    (&against_label, &label)
                };
                println!(
                    "SEPARATED. The interval on the difference excludes zero, so at these sample\n\
                     sizes {more} is the more provenance-concentrated population of the two —\n\
                     a member's funding class narrows the guess further there than in {less}."
                );
            } else {
                println!(
                    "NOT SEPARATED. The interval on the difference contains zero, so these two\n\
                     populations are indistinguishable at these sample sizes. The point estimates\n\
                     differ, and that difference is not evidence: quoting it as one would be\n\
                     reporting the draw."
                );
            }
            println!();
            println!(
                "Both estimates share the same downward bias in entropy, so both rho values are\n\
                 biased high by roughly the same amount. That is why the comparison survives a\n\
                 bias that neither individual number does."
            );
            Ok(())
        }
        Command::Selection { earlier, later } => {
            let before = labels_by_seed(&earlier)?;
            let after = labels_by_seed(&later)?;

            // Members the larger budget reached, split by whether the smaller
            // one reached them too.
            let mut easy: Vec<String> = Vec::new();
            let mut hard: Vec<String> = Vec::new();
            for (seed, label) in &after {
                if before.contains_key(seed) {
                    easy.push(label.clone());
                } else {
                    hard.push(label.clone());
                }
            }

            println!("earlier  {} resolved", before.len());
            println!(
                "later    {} resolved  ({} of them newly)",
                after.len(),
                hard.len()
            );
            println!();

            if hard.len() < 5 {
                println!(
                    "Only {} members were newly resolved. That is too few to say anything \
                     about\nwhether resolvability selects on class, and this check reports \
                     nothing rather\nthan reporting a number computed from it.",
                    hard.len()
                );
                return Ok(());
            }

            let reps = mirror_provenance::bootstrap::DEFAULT_REPLICATES;
            let seed = mirror_provenance::bootstrap::DEFAULT_SEED;
            let ie = mirror_provenance::loss_factor_interval(&easy, reps, seed)
                .ok_or_else(|| anyhow::anyhow!("no easily-resolved members"))?;
            let ih = mirror_provenance::loss_factor_interval(&hard, reps, seed)
                .ok_or_else(|| anyhow::anyhow!("no newly-resolved members"))?;

            println!("group                    members   rho      resampled 2.5-97.5%");
            println!(
                "resolved at both budgets  {:>7}   {:.4}   {:.4} .. {:.4}",
                easy.len(),
                ie.point,
                ie.lo,
                ie.hi
            );
            println!(
                "only at the larger        {:>7}   {:.4}   {:.4} .. {:.4}",
                hard.len(),
                ih.point,
                ih.lo,
                ih.hi
            );

            let d = mirror_provenance::difference_interval(&easy, &hard, reps, seed)
                .ok_or_else(|| anyhow::anyhow!("nothing to compare"))?;
            println!();
            println!(
                "difference (easy − hard): {:+.4}   95% {:+.4} .. {:+.4}",
                d.point, d.lo, d.hi
            );
            println!();

            if d.excludes_zero() {
                println!(
                    "SELECTION DETECTED. Members that only a larger budget reaches have a\n\
                     measurably different class distribution from those any budget reaches.\n\n\
                     The unresolved are therefore NOT missing at random, and dropping them is\n\
                     not neutral: the resolved subset of a population is biased toward whichever\n\
                     classes happen to be cheap to trace. Every unresolved bracket in this\n\
                     project is still a valid bound, but no comparison between two populations\n\
                     at different resolution rates can be trusted, and more budget does not fix\n\
                     that — it moves the boundary without removing it."
                );
            } else {
                println!(
                    "NO SELECTION DETECTED at this margin. The members that needed a larger\n\
                     budget carry a class distribution indistinguishable from those that did\n\
                     not, so at this margin resolvability is not picking out particular\n\
                     provenance classes.\n\n\
                     This is evidence, not proof. It says the members just beyond the cheaper\n\
                     budget look like the ones inside it; it cannot speak for members beyond\n\
                     the larger budget too, and a heavier tail could still be hiding there."
                );
            }
            Ok(())
        }
        Command::Soak {
            program,
            url,
            keypair,
            out,
        } => soak::run(&program, &url, &keypair, &out),
        Command::Disclose {
            program,
            note,
            out,
            i_accept_the_cost_to_others,
            url,
        } => {
            let chain = chain::Chain::new(&url);
            disclose::create(
                &chain,
                &parse_program(&program)?,
                &note,
                &out,
                i_accept_the_cost_to_others,
            )
        }
        Command::DiscloseVerify { file, url } => {
            let chain = chain::Chain::new(&url);
            disclose::check(&chain, &file)
        }
        Command::CloseTable {
            table,
            url,
            keypair,
        } => {
            let authority = soak::read_keypair(&keypair)?;
            let table: solana_program::pubkey::Pubkey = table
                .parse()
                .map_err(|e| anyhow::anyhow!("bad table address: {e}"))?;
            client::close_table(&chain::Chain::new(&url), &table, &authority)
        }
        Command::Crowd {
            program,
            url,
            keypair,
            out,
            render_only,
        } => {
            if render_only {
                crowd::render_only(&out)
            } else {
                crowd::run(&program, &url, &keypair, &out)
            }
        }

        Command::InitPool {
            program,
            denomination,
            k_floor,
            settle_timeout,
            url,
            keypair,
        } => {
            let chain = chain::Chain::new(&url);
            let payer = soak::read_keypair(&keypair)?;
            client::init_pool(
                &chain,
                &parse_program(&program)?,
                denomination,
                k_floor,
                settle_timeout,
                &payer,
            )
        }

        Command::NoteNew { denomination, out } => client::note_new(denomination, &out),

        Command::Deposit {
            program,
            note,
            url,
            keypair,
        } => {
            let chain = chain::Chain::new(&url);
            let depositor = soak::read_keypair(&keypair)?;
            client::deposit(&chain, &parse_program(&program)?, &note, &depositor)
        }

        Command::Tree {
            program,
            denomination,
            url,
        } => {
            let chain = chain::Chain::new(&url);
            client::tree(&chain, &parse_program(&program)?, denomination).map(|_| ())
        }

        Command::Spend {
            program,
            note,
            to,
            relay,
            relay_fee,
            invoke,
            payload,
            accounts,
            pool_signs,
            url,
        } => {
            let chain = chain::Chain::new(&url);
            let relay = soak::read_keypair(&relay)?;
            let beneficiary: solana_program::pubkey::Pubkey = to
                .parse()
                .map_err(|e| anyhow::anyhow!("--to is not a pubkey: {e}"))?;
            let action = match invoke {
                None => client::Action::Transfer,
                Some(target) => client::Action::Invoke {
                    target: target
                        .parse()
                        .map_err(|e| anyhow::anyhow!("--invoke is not a pubkey: {e}"))?,
                    payload: hex::decode(payload.trim_start_matches("0x"))
                        .context("--payload is not hex")?,
                    accounts,
                    pool_signs,
                },
            };
            client::spend(
                &chain,
                &parse_program(&program)?,
                &note,
                &beneficiary,
                &relay,
                relay_fee,
                action,
            )
        }

        Command::Settle {
            program,
            denomination,
            allow_below_floor,
            url,
            keypair,
        } => {
            let chain = chain::Chain::new(&url);
            let settler = soak::read_keypair(&keypair)?;
            let now = client::now()?;
            client::settle(
                &chain,
                &parse_program(&program)?,
                denomination,
                &settler,
                now,
                allow_below_floor,
            )
        }
        Command::VerifySetup { seed, expect } => {
            let keys = mirror_circuit::generate_reproducible(seed.as_bytes())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let regenerated = keys.solana_vk();
            let digest = vk_digest(&regenerated);
            println!("vk sha256 (regenerated): {digest}");

            // Compare against the key actually compiled into the program, not
            // only against a digest the caller supplies. Printing a hash and
            // exiting zero verifies nothing; the point of a reproducible setup
            // is that a third party can confirm the *deployed* key is the one
            // this circuit and this seed produce.
            let baked = mirror_pool_program::vk::VERIFYING_KEY;
            let mut mismatches: Vec<&str> = Vec::new();
            if regenerated.alpha_g1 != baked.vk_alpha_g1 {
                mismatches.push("alpha_g1");
            }
            if regenerated.beta_g2 != baked.vk_beta_g2 {
                mismatches.push("beta_g2");
            }
            if regenerated.gamma_g2 != baked.vk_gamme_g2 {
                mismatches.push("gamma_g2");
            }
            if regenerated.delta_g2 != baked.vk_delta_g2 {
                mismatches.push("delta_g2");
            }
            if regenerated.ic.len() != baked.vk_ic.len()
                || regenerated.ic.iter().zip(baked.vk_ic).any(|(a, b)| a != b)
            {
                mismatches.push("ic");
            }

            if mismatches.is_empty() {
                println!(
                    "MATCH — every element of the program's verifying key is reproduced \
                     by this seed and this circuit ({} IC points checked)",
                    regenerated.ic.len()
                );
            } else {
                eprintln!(
                    "MISMATCH against the program's baked key: {}",
                    mismatches.join(", ")
                );
                eprintln!(
                    "The deployed program does not verify proofs from this circuit. \
                     A ceremony verifier that checked only delta would have passed this."
                );
                std::process::exit(1);
            }

            match expect {
                Some(want) if want.eq_ignore_ascii_case(&digest) => {
                    println!("and it matches the digest you supplied");
                    Ok(())
                }
                Some(want) => {
                    eprintln!("but the digest you supplied was {want}");
                    std::process::exit(1);
                }
                None => Ok(()),
            }
        }
    }
}
