//! What a curation policy costs a pool's anonymity set, measured on real data.
//!
//! On Solana a curator publishes a clean set: the pool's deposits minus those
//! whose depositor its policy blocks. A withdrawal proves membership in the clean
//! set, so the clean set is the anonymity set, and the curator's policy decides
//! how large it is and which funding classes it contains.
//!
//! This binary takes the committed mainnet samples behind mirror-pool's
//! funding-provenance measurement, treats the measured depositors as a pool,
//! applies a curation policy, and recomputes the method's metrics over what is
//! left, as an observer who knows the clean set would. Everything is computed by
//! `mirror-provenance` itself (classification, `Anonymity`, `Bracket`,
//! `loss_factor_interval`); the only arithmetic added here is the policy, the
//! per-policy resampling, and two ratios against the uncurated baseline.
//!
//! Pure and offline: no network, fixed seeds, byte-identical output on rerun.
#![forbid(unsafe_code)]

use mirror_provenance::{
    bootstrap::{DEFAULT_REPLICATES, DEFAULT_SEED},
    classify_sample, loss_factor_interval, AnchorSet, Anonymity, Bracket, Interval, Outcome,
    Quotation, Sample, TerminalRule, Thresholds, PUBLISHED_HEADLINE,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Exclusion draws averaged for a random blocklist's point estimate.
const EXCLUSION_DRAWS: usize = 10_000;

// ---------------------------------------------------------------------------
// Resampling
// ---------------------------------------------------------------------------

/// The same splitmix64 as `mirror_provenance::bootstrap`, which keeps its own
/// private. Reimplemented here so the random-blocklist rows and the paired
/// retention interval resample exactly as the crate does;
/// `the_resampler_is_the_crates` checks it reproduces the crate's published
/// interval bit for bit.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        let zone = u64::MAX - (u64::MAX % n);
        loop {
            let v = self.next_u64();
            if v < zone {
                return v % n;
            }
        }
    }

    /// True with probability `p`, from the top 53 bits.
    fn chance(&mut self, p: f64) -> bool {
        ((self.next_u64() >> 11) as f64 / (1u64 << 53) as f64) < p
    }
}

/// The crate's percentile rule, so intervals are read off the same way.
fn percentile(sorted: &[f64], q: f64) -> f64 {
    let idx = (q * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn interval_of(point: f64, mut draws: Vec<f64>) -> Option<Interval> {
    if draws.is_empty() {
        return None;
    }
    draws.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    let mean = draws.iter().sum::<f64>() / draws.len() as f64;
    Some(Interval {
        point,
        lo: percentile(&draws, 0.025),
        hi: percentile(&draws, 0.975),
        mean,
        replicates: draws.len(),
    })
}

/// `ρ` of a multiset of class ids; `None` when it is empty.
fn rho_of(classes: impl Iterator<Item = usize>, n_classes: usize) -> Option<f64> {
    let mut counts = vec![0u64; n_classes];
    for c in classes {
        counts[c] += 1;
    }
    let sizes: Vec<u64> = counts.into_iter().filter(|&c| c > 0).collect();
    Anonymity::from_class_sizes(&sizes).map(|a| a.loss_factor)
}

// ---------------------------------------------------------------------------
// The population
// ---------------------------------------------------------------------------

/// One measured depositor. `class` is `None` when the trace reached no class.
#[derive(Debug, Clone)]
struct Member {
    class: Option<usize>,
}

struct Population {
    path: PathBuf,
    census_line: String,
    /// Every measurable member: RPC failures are dropped, as `analyze` drops them.
    members: Vec<Member>,
    /// Per class id: its label and the rule that produced it.
    classes: Vec<(String, TerminalRule)>,
}

impl Population {
    fn load(path: &Path) -> Self {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        let sample = Sample::from_json(&text).expect("the committed sample parses");
        // Exactly the classification `mirror analyze` runs.
        let (results, census) = classify_sample(
            &sample,
            &AnchorSet::default(),
            &Thresholds::default(),
            sample.manifest.collected_at,
        );
        assert!(
            census.may_publish(),
            "{}: above the RPC failure limit, no headline",
            path.display()
        );

        let mut index: BTreeMap<String, usize> = BTreeMap::new();
        let mut classes = Vec::new();
        let mut members = Vec::new();
        for (_, outcome) in &results {
            match outcome {
                Outcome::Terminal { rule, label } => {
                    let next = classes.len();
                    let id = *index.entry(label.clone()).or_insert_with(|| {
                        classes.push((label.clone(), rule.clone()));
                        next
                    });
                    members.push(Member { class: Some(id) });
                }
                Outcome::Unresolved { reason, .. } if reason.is_failure() => {}
                Outcome::Unresolved { .. } => members.push(Member { class: None }),
            }
        }
        Population {
            path: path.to_path_buf(),
            census_line: census.summary(),
            members,
            classes,
        }
    }

    fn rule_of(&self, m: &Member) -> Option<&TerminalRule> {
        m.class.map(|c| &self.classes[c].1)
    }

    fn label(&self, c: usize) -> &str {
        &self.classes[c].0
    }

    fn class_sizes(&self) -> Vec<u64> {
        let mut n = vec![0u64; self.classes.len()];
        for c in self.members.iter().filter_map(|m| m.class) {
            n[c] += 1;
        }
        n
    }

    fn resolved(&self) -> usize {
        self.members.iter().filter(|m| m.class.is_some()).count()
    }
}

// ---------------------------------------------------------------------------
// Policies
// ---------------------------------------------------------------------------

/// A curator's keep-or-exclude decision for one member.
type KeepFn = Box<dyn Fn(&Population, &Member) -> bool>;

/// What a curator keeps. A deterministic rule is a function of what the
/// curator can see about a member; a random blocklist ignores it entirely.
enum Rule {
    Keep(KeepFn),
    Random(f64),
}

struct Policy {
    id: String,
    name: String,
    rule: Rule,
}

fn policies(pop: &Population) -> Vec<Policy> {
    let sizes = pop.class_sizes();
    // Largest class, ties to the lowest id; the id order is the sample's own.
    let largest = (0..sizes.len())
        .max_by_key(|&c| (sizes[c], std::cmp::Reverse(c)))
        .expect("at least one class");
    let largest_rule = match pop.classes[largest].1 {
        TerminalRule::ProgramOrPda => "R1 program",
        TerminalRule::CuratedAnchor => "R2 anchor",
        TerminalRule::StructuralCluster => "R3 cluster",
        TerminalRule::VolumeHub => "R4 busy-unlabelled",
        TerminalRule::Distributor => "R5 distributor",
    };

    let mut out = vec![Policy {
        id: "P0".into(),
        name: "no curation".into(),
        rule: Rule::Keep(Box::new(|_, _| true)),
    }];
    for p in [0.01, 0.05, 0.10, 0.25] {
        out.push(Policy {
            id: "P1".into(),
            name: format!("random blocklist, {:.0}%", p * 100.0),
            rule: Rule::Random(p),
        });
    }
    out.push(Policy {
        id: "P2a".into(),
        name: "block untraced funding".into(),
        rule: Rule::Keep(Box::new(|_, m| m.class.is_some())),
    });
    out.push(Policy {
        id: "P2b".into(),
        name: "block unattributed (untraced + R4)".into(),
        rule: Rule::Keep(Box::new(
            |pop, m| matches!(pop.rule_of(m), Some(r) if *r != TerminalRule::VolumeHub),
        )),
    });
    out.push(Policy {
        id: "P2c".into(),
        name: format!(
            "block one source: largest class ({largest_rule}, {})",
            sizes[largest]
        ),
        rule: Rule::Keep(Box::new(move |_, m| m.class != Some(largest))),
    });
    out.push(Policy {
        id: "P3a".into(),
        name: "allow only R3 fee-payer clusters".into(),
        rule: Rule::Keep(Box::new(|pop, m| {
            matches!(pop.rule_of(m), Some(TerminalRule::StructuralCluster))
        })),
    });
    out.push(Policy {
        id: "P3b".into(),
        name: "allow only R3 clusters + R5 distributors".into(),
        rule: Rule::Keep(Box::new(|pop, m| {
            matches!(
                pop.rule_of(m),
                Some(TerminalRule::StructuralCluster | TerminalRule::Distributor)
            )
        })),
    });
    out
}

// ---------------------------------------------------------------------------
// One clean set, measured
// ---------------------------------------------------------------------------

/// Every figure for one clean set. Averaged field by field over exclusion draws
/// for a random blocklist.
#[derive(Debug, Clone, Default)]
struct Eval {
    kept: f64,
    resolved_kept: f64,
    classes: f64,
    /// `ρ` over the resolved members kept: the crate's point estimate.
    rho: f64,
    /// The unresolved bracket on `ρ`, over the whole clean set.
    rho_lo: f64,
    rho_hi: f64,
    /// `2^{H(X|C)}` over the resolved kept, as `analyze` prints it.
    effk_resolved: f64,
    /// `ρ · K'`: the method's reading of `ρ` as the surviving share of nominal
    /// k, applied to the whole clean set. Assumes the unresolved are spread
    /// like the resolved.
    effk_scaled: f64,
    /// The bracket on effective-k over the whole clean set.
    effk_lo: f64,
    effk_hi: f64,
    /// `ρ'K' / ρ₀K`, the share of the uncurated pool's effective-k kept.
    retention: f64,
    /// Same, with Miller–Madow-corrected entropy on both sides. Sensitivity
    /// only: the crate does not implement it, and §2.6a explains why plug-in is
    /// what is published.
    retention_mm: f64,
    /// Geometric mean, over admitted resolved members, of the share of their
    /// own class-mates still in the set — reading the unresolved as apart from
    /// every class (both bracket readings).
    cover_apart: f64,
    /// Same, reading the unresolved as spread like the resolved (MAR).
    cover_mar: f64,
    /// Whether the clean set clears the crate's informativeness gate.
    informative: f64,
}

/// Miller–Madow `ρ`: plug-in entropy plus `(m−1)/(2n ln 2)` bits.
fn rho_mm(a: &Anonymity) -> f64 {
    let h = a.leakage_shannon_bits
        + (a.classes as f64 - 1.0) / (2.0 * a.nominal_k as f64 * std::f64::consts::LN_2);
    (-h).exp2()
}

struct Baseline {
    rho: f64,
    rho_mm: f64,
    m: f64,
}

fn evaluate(pop: &Population, keep: &[bool], base: &Baseline) -> Option<Eval> {
    let full = pop.class_sizes();
    let m_total = pop.members.len() as f64;
    let r_total = pop.resolved() as f64;

    let mut kept_sizes = vec![0u64; pop.classes.len()];
    let mut kept = 0u64;
    let mut unresolved_kept = 0u64;
    for (m, &k) in pop.members.iter().zip(keep) {
        if !k {
            continue;
        }
        kept += 1;
        match m.class {
            Some(c) => kept_sizes[c] += 1,
            None => unresolved_kept += 1,
        }
    }
    let resolved_sizes: Vec<u64> = kept_sizes.iter().copied().filter(|&n| n > 0).collect();
    let point = Anonymity::from_class_sizes(&resolved_sizes)?;
    let bracket = Bracket::new(&resolved_sizes, unresolved_kept)?;

    // Per admitted member, the share of its class-mates still admitted.
    let mut log_apart = 0.0;
    let mut log_mar = 0.0;
    for (c, &n_kept) in kept_sizes.iter().enumerate() {
        if n_kept == 0 {
            continue;
        }
        let share = n_kept as f64 / full[c] as f64;
        let mar = (r_total * share + unresolved_kept as f64) / m_total;
        log_apart += n_kept as f64 * share.log2();
        log_mar += n_kept as f64 * mar.log2();
    }
    let rk = point.nominal_k as f64;

    Some(Eval {
        kept: kept as f64,
        resolved_kept: rk,
        classes: point.classes as f64,
        rho: point.loss_factor,
        rho_lo: bracket.lower.loss_factor,
        rho_hi: bracket.upper.loss_factor,
        effk_resolved: point.eff_k_shannon,
        effk_scaled: point.loss_factor * kept as f64,
        effk_lo: bracket.lower.eff_k_shannon,
        effk_hi: bracket.upper.eff_k_shannon,
        retention: point.loss_factor * kept as f64 / (base.rho * base.m),
        retention_mm: rho_mm(&point) * kept as f64 / (base.rho_mm * base.m),
        cover_apart: (log_apart / rk).exp2(),
        cover_mar: (log_mar / rk).exp2(),
        informative: if bracket.is_informative() { 1.0 } else { 0.0 },
    })
}

fn mean_eval(evals: &[Eval]) -> Eval {
    let n = evals.len() as f64;
    let mut s = Eval::default();
    for e in evals {
        s.kept += e.kept / n;
        s.resolved_kept += e.resolved_kept / n;
        s.classes += e.classes / n;
        s.rho += e.rho / n;
        s.rho_lo += e.rho_lo / n;
        s.rho_hi += e.rho_hi / n;
        s.effk_resolved += e.effk_resolved / n;
        s.effk_scaled += e.effk_scaled / n;
        s.effk_lo += e.effk_lo / n;
        s.effk_hi += e.effk_hi / n;
        s.retention += e.retention / n;
        s.retention_mm += e.retention_mm / n;
        s.cover_apart += e.cover_apart / n;
        s.cover_mar += e.cover_mar / n;
        s.informative += e.informative / n;
    }
    s
}

/// Distinct, published stream per (policy, purpose), so no two rows share draws.
fn seed_for(row: usize, purpose: u64) -> u64 {
    DEFAULT_SEED
        ^ (row as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ purpose.wrapping_mul(0xD1B5_4A32_D192_ED03)
}

struct Row {
    id: String,
    name: String,
    point: Eval,
    /// Spread of `ρ'` over the draw of depositors (and, for a random blocklist,
    /// over which of them the list hits).
    rho_interval: Option<Interval>,
    /// Paired spread of the retention ratio.
    retention_interval: Option<Interval>,
}

fn measure(pop: &Population, row: usize, policy: &Policy, base: &Baseline) -> Row {
    let n = pop.members.len();
    let n_classes = pop.classes.len();

    // Point estimate on the sample as drawn.
    let point = match &policy.rule {
        Rule::Keep(f) => {
            let keep: Vec<bool> = pop.members.iter().map(|m| f(pop, m)).collect();
            evaluate(pop, &keep, base).expect("the policy keeps a resolved member")
        }
        Rule::Random(p) => {
            let mut rng = SplitMix64::new(seed_for(row, 1));
            let evals: Vec<Eval> = (0..EXCLUSION_DRAWS)
                .filter_map(|_| {
                    let keep: Vec<bool> = (0..n).map(|_| !rng.chance(*p)).collect();
                    evaluate(pop, &keep, base)
                })
                .collect();
            mean_eval(&evals)
        }
    };

    // Sampling spread of ρ'. A deterministic clean set is a fixed subset, so it
    // goes through the crate's own estimator, at the crate's own seed. A random
    // blocklist adds which members it hits: each replicate draws the list, then
    // resamples the kept resolved members as the crate does.
    let rho_interval = match &policy.rule {
        Rule::Keep(f) => {
            let labels: Vec<String> = pop
                .members
                .iter()
                .filter(|m| f(pop, m))
                .filter_map(|m| m.class.map(|c| pop.label(c).to_string()))
                .collect();
            loss_factor_interval(&labels, DEFAULT_REPLICATES, DEFAULT_SEED)
        }
        Rule::Random(p) => {
            let mut rng = SplitMix64::new(seed_for(row, 2));
            let mut draws = Vec::with_capacity(DEFAULT_REPLICATES);
            for _ in 0..DEFAULT_REPLICATES {
                let kept: Vec<usize> = pop
                    .members
                    .iter()
                    .filter(|_| !rng.chance(*p))
                    .filter_map(|m| m.class)
                    .collect();
                if kept.is_empty() {
                    continue;
                }
                let k = kept.len() as u64;
                let resampled: Vec<usize> = (0..k).map(|_| kept[rng.below(k) as usize]).collect();
                if let Some(r) = rho_of(resampled.into_iter(), n_classes) {
                    draws.push(r);
                }
            }
            interval_of(point.rho, draws)
        }
    };

    // Paired spread of the retention ratio: resample all members, measure the
    // uncurated and the curated set on the same replicate, take the ratio.
    let retention_interval = {
        let mut rng = SplitMix64::new(seed_for(row, 3));
        let mut draws = Vec::with_capacity(DEFAULT_REPLICATES);
        for _ in 0..DEFAULT_REPLICATES {
            let sample: Vec<&Member> = (0..n)
                .map(|_| &pop.members[rng.below(n as u64) as usize])
                .collect();
            let Some(rho0) = rho_of(sample.iter().filter_map(|m| m.class), n_classes) else {
                continue;
            };
            let keep: Vec<bool> = match &policy.rule {
                Rule::Keep(f) => sample.iter().map(|m| f(pop, m)).collect(),
                Rule::Random(p) => sample.iter().map(|_| !rng.chance(*p)).collect(),
            };
            let kept = keep.iter().filter(|&&k| k).count() as f64;
            let Some(rho1) = rho_of(
                sample
                    .iter()
                    .zip(&keep)
                    .filter(|(_, &k)| k)
                    .filter_map(|(m, _)| m.class),
                n_classes,
            ) else {
                continue;
            };
            draws.push(rho1 * kept / (rho0 * n as f64));
        }
        interval_of(point.retention, draws)
    };

    Row {
        id: policy.id.clone(),
        name: policy.name.clone(),
        point,
        rho_interval,
        retention_interval,
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

fn iv(i: &Option<Interval>) -> String {
    match i {
        Some(i) => format!("{:.4} .. {:.4}", i.lo, i.hi),
        None => "n/a".into(),
    }
}

fn kept_cell(e: &Eval, total: usize, random: bool) -> String {
    let share = e.kept / total as f64 * 100.0;
    if random {
        format!("{:.1} ({share:.0}%)", e.kept)
    } else {
        format!("{:.0} ({share:.0}%)", e.kept)
    }
}

fn run(path: &Path) {
    let pop = Population::load(path);
    let n = pop.members.len();
    let name = pop
        .path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    println!("## {name}");
    println!();
    println!("census    {}", pop.census_line);
    println!(
        "members   {n} measurable, {} resolved into {} classes, {} unresolved",
        pop.resolved(),
        pop.classes.len(),
        n - pop.resolved()
    );
    println!();

    // P0 through the crate's own renderer, the one `mirror analyze` prints.
    let labels: Vec<String> = pop
        .members
        .iter()
        .filter_map(|m| m.class.map(|c| pop.label(c).to_string()))
        .collect();
    let a0 = Anonymity::from_labels(&labels).expect("resolved members");
    let b0 = Bracket::new(
        &{
            let mut s = pop.class_sizes();
            s.retain(|&x| x > 0);
            s
        },
        (n - pop.resolved()) as u64,
    )
    .expect("bracket");
    let i0 = loss_factor_interval(&labels, DEFAULT_REPLICATES, DEFAULT_SEED);
    println!("P0, through mirror-provenance's Quotation (what `mirror analyze` prints):");
    println!("```");
    println!(
        "{}",
        Quotation::new(a0.clone(), b0.clone()).with_sampling(i0.clone())
    );
    println!("```");

    if name == "sample-privacycash-run6.json" {
        let h = PUBLISHED_HEADLINE;
        let ok = a0.nominal_k == h.resolved
            && n as u64 == h.attempted
            && (a0.loss_factor - h.loss_factor).abs() < 5e-5
            && (b0.lower.loss_factor - h.bracket_low).abs() < 5e-5
            && (b0.upper.loss_factor - h.bracket_high).abs() < 5e-5;
        assert!(ok, "P0 does not reproduce PUBLISHED_HEADLINE");
        println!(
            "P0 check: matches mirror_provenance::PUBLISHED_HEADLINE (resolved {}, attempted {}, \
             rho {:.4}, bracket {:.4} .. {:.4})",
            h.resolved, h.attempted, h.loss_factor, h.bracket_low, h.bracket_high
        );
        println!();
    }

    let base = Baseline {
        rho: a0.loss_factor,
        rho_mm: rho_mm(&a0),
        m: n as f64,
    };
    let rows: Vec<Row> = policies(&pop)
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut r = measure(&pop, i, p, &base);
            if p.id == "P0" {
                // The ratio of a set to itself; its "interval" is resampling the
                // same members on both sides of a division and is 1 exactly.
                r.retention_interval = None;
            }
            r
        })
        .collect();
    let random = |r: &Row| r.id == "P1";

    println!(
        "Table 1. Size, concentration, and effective-k of the clean set. rho' is over the \
         resolved members kept;"
    );
    println!(
        "its interval is the 2.5-97.5% spread under resampling ({DEFAULT_REPLICATES} replicates). \
         Effective-k is rho' x K'."
    );
    println!("Retention is that effective-k over P0's, with a paired resampled interval.");
    println!();
    println!(
        "| policy | kept K' | resolved kept, classes | rho' | rho' resampled 95% | effective-k \
         | retention | retention 95% | cover kept by admitted members |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    for r in &rows {
        let e = &r.point;
        println!(
            "| {} {} | {} | {:.1}, {:.1} | {:.4} | {} | {:.2} | {:.3} | {} | {:.3} .. {:.3} |",
            r.id,
            r.name,
            kept_cell(e, n, random(r)),
            e.resolved_kept,
            e.classes,
            e.rho,
            iv(&r.rho_interval),
            e.effk_scaled,
            e.retention,
            match &r.retention_interval {
                Some(i) => format!("{:.3} .. {:.3}", i.lo, i.hi),
                None => "1 by definition".into(),
            },
            e.cover_mar,
            e.cover_apart,
        );
    }
    println!();
    println!(
        "Table 2. The unresolved bracket for each clean set (unresolved kept read as singletons \
         .. as one class),"
    );
    println!(
        "the crate's effective-k over the resolved kept, and retention with Miller-Madow \
         entropy (sensitivity)."
    );
    println!();
    println!(
        "| policy | unresolved kept | rho' bracket | effective-k bracket over K' | effective-k \
         over resolved | retention, Miller-Madow | informative |"
    );
    println!("|---|---|---|---|---|---|---|");
    for r in &rows {
        let e = &r.point;
        let informative = if random(r) {
            format!("{:.0}% of draws", e.informative * 100.0)
        } else if e.informative > 0.5 {
            "yes".into()
        } else {
            "NO".into()
        };
        println!(
            "| {} {} | {:.1} | {:.4} .. {:.4} | {:.2} .. {:.2} | {:.2} | {:.3} | {} |",
            r.id,
            r.name,
            e.kept - e.resolved_kept,
            e.rho_lo,
            e.rho_hi,
            e.effk_lo,
            e.effk_hi,
            e.effk_resolved,
            e.retention_mm,
            informative,
        );
    }
    println!();
    println!(
        "P1 rows are means over {EXCLUSION_DRAWS} blocklist draws (each member blocked \
         independently with probability p);"
    );
    println!("their rho' interval also spans which members the list hits.");
    println!(
        "Cover kept by admitted members: MAR reading .. unresolved-apart reading. For a \
         policy that removes whole classes"
    );
    println!(
        "the second value is 1 by construction (the curator and the observer use the same \
         labels): an identity, not a finding."
    );
    println!();
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mirror-pool/data");
    let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let paths = if args.is_empty() {
        vec![
            root.join("sample-privacycash-run6.json"),
            root.join("sample-marinade-run8.json"),
        ]
    } else {
        args
    };
    println!("# Curation cost, measured with mirror-provenance");
    println!();
    println!(
        "seed {DEFAULT_SEED:#018x} (mirror-provenance's published seed), {DEFAULT_REPLICATES} \
         replicates, {EXCLUSION_DRAWS} blocklist draws"
    );
    println!();
    for p in &paths {
        run(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run6() -> Population {
        Population::load(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../mirror-pool/data/sample-privacycash-run6.json"),
        )
    }

    /// The local resampler, driven the way the crate drives its own, must give
    /// the crate's interval exactly. Otherwise the P1 and retention intervals
    /// would not be "the crate's bootstrap" but a lookalike.
    #[test]
    fn the_resampler_is_the_crates() {
        let pop = run6();
        let labels: Vec<String> = pop
            .members
            .iter()
            .filter_map(|m| m.class.map(|c| pop.label(c).to_string()))
            .collect();
        let crate_iv = loss_factor_interval(&labels, DEFAULT_REPLICATES, DEFAULT_SEED).unwrap();

        // Class ids by first appearance, as the crate assigns them.
        let mut first: BTreeMap<&str, usize> = BTreeMap::new();
        let ids: Vec<usize> = labels
            .iter()
            .map(|l| {
                let next = first.len();
                *first.entry(l.as_str()).or_insert(next)
            })
            .collect();
        let mut rng = SplitMix64::new(DEFAULT_SEED);
        let k = ids.len() as u64;
        let draws: Vec<f64> = (0..DEFAULT_REPLICATES)
            .filter_map(|_| {
                let r: Vec<usize> = (0..k).map(|_| ids[rng.below(k) as usize]).collect();
                rho_of(r.into_iter(), first.len())
            })
            .collect();
        let local = interval_of(crate_iv.point, draws).unwrap();
        assert_eq!(local, crate_iv);
    }

    /// P0 is the published headline.
    #[test]
    fn p0_is_the_published_headline() {
        let pop = run6();
        let h = PUBLISHED_HEADLINE;
        assert_eq!(pop.members.len() as u64, h.attempted);
        assert_eq!(pop.resolved() as u64, h.resolved);
        let base = Baseline {
            rho: 0.0,
            rho_mm: 1.0,
            m: 1.0,
        };
        let e = evaluate(&pop, &vec![true; pop.members.len()], &base).unwrap();
        assert!((e.rho - h.loss_factor).abs() < 5e-5);
        assert!((e.rho_lo - h.bracket_low).abs() < 5e-5);
        assert!((e.rho_hi - h.bracket_high).abs() < 5e-5);
        assert!((e.cover_apart - 1.0).abs() < 1e-12);
        assert!((e.cover_mar - 1.0).abs() < 1e-12);
    }

    /// Removing whole classes leaves every admitted member's class intact, so
    /// cover-apart is 1. This is the identity the documentation warns about,
    /// pinned so the table cannot drift from it silently.
    #[test]
    fn removing_whole_classes_costs_admitted_members_nothing_by_construction() {
        let pop = run6();
        let base = Baseline {
            rho: 0.1,
            rho_mm: 0.1,
            m: 83.0,
        };
        let keep: Vec<bool> = pop
            .members
            .iter()
            .map(|m| m.class.is_some_and(|c| c % 2 == 0))
            .collect();
        let e = evaluate(&pop, &keep, &base).unwrap();
        assert!((e.cover_apart - 1.0).abs() < 1e-12);
    }

    /// The table is a pure function of the committed sample.
    #[test]
    fn rows_are_reproducible() {
        let pop = run6();
        let base = Baseline {
            rho: 0.0955,
            rho_mm: 0.1,
            m: 83.0,
        };
        let ps = policies(&pop);
        let a = measure(&pop, 1, &ps[1], &base);
        let b = measure(&pop, 1, &ps[1], &base);
        assert_eq!(a.rho_interval, b.rho_interval);
        assert_eq!(a.retention_interval, b.retention_interval);
        assert_eq!(a.point.rho, b.point.rho);
    }
}
