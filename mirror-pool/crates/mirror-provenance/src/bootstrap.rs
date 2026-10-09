//! Sampling error for `ρ`, by bootstrap over members.
//!
//! Every run so far has reported `ρ` as a point estimate with an *unresolved
//! bracket* beside it. Those are two different uncertainties, and only one of
//! them was quantified.
//!
//! * The **bracket** answers: what if the members we could not resolve had
//!   landed in one class, or in classes of their own? It is a bound, not a
//!   distribution, and it is computed exactly.
//! * The **bootstrap here** answers: we drew *these* depositors out of a much
//!   larger population — how much of `ρ` is the draw? Resampling the resolved
//!   members with replacement and recomputing `ρ` each time gives a percentile
//!   interval for that.
//!
//! Both are needed and neither substitutes for the other. Comparing two
//! populations without the second one is eyeballing two numbers and calling the
//! larger one larger, which is precisely the move this project criticises
//! elsewhere.
//!
//! ## What this does not fix
//!
//! **Plug-in entropy is biased downward at small `n`.** Estimating `H(C)` by
//! counting is a maximum-likelihood estimate, and it systematically understates
//! entropy when classes are many and members are few — which is exactly the
//! regime every run here operates in. A bootstrap resamples the same estimator,
//! so its interval is centred on the *biased* value: it measures the spread of
//! the estimate, not its distance from the truth. Since `ρ = 2^{−H(C)}`,
//! understated entropy means **overstated ρ**, so the true loss factor is
//! plausibly smaller than any number reported here.
//!
//! Correcting that needs a different estimator — Miller–Madow, or one of the
//! coverage-adjusted families — and this crate does not implement one. The
//! consequence is stated wherever a `ρ` appears rather than left for a reader to
//! infer, and it applies **in the same direction to every population measured**,
//! which is what keeps a comparison between two of them meaningful even while
//! each is individually biased.

use crate::metrics::Anonymity;
use std::collections::BTreeMap;

/// A deterministic splitmix64.
///
/// Deliberately not an external RNG. `analyze` is a pure, offline, reproducible
/// pass: anyone holding a committed sample must be able to recompute every
/// published figure and get the same bytes, and that includes the interval. A
/// seeded generator implemented here is reproducible across platforms and
/// dependency versions in a way that "whatever `rand` does this year" is not.
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

    /// Uniform on `0..n`, rejecting the biased tail rather than taking a modulus
    /// over it.
    fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0);
        let zone = u64::MAX - (u64::MAX % n);
        loop {
            let v = self.next_u64();
            if v < zone {
                return v % n;
            }
        }
    }
}

/// The spread of the resampled estimator, and the estimate it was resampled
/// from.
///
/// **This is not a confidence interval around `point`, and it is not guaranteed
/// to contain it.** It is the 2.5th-to-97.5th percentile range of `ρ̂` computed
/// over bootstrap replicates. For a statistic that is biased under resampling —
/// and `ρ` badly is, see [`Interval::resampling_bias`] — the whole range can sit
/// to one side of the original estimate. Reading it as "ρ is somewhere in here"
/// is wrong; it says "an estimate computed this way, from a sample like this one,
/// lands in here".
#[derive(Debug, Clone, PartialEq)]
pub struct Interval {
    /// The estimate computed from the sample as drawn.
    pub point: f64,
    pub lo: f64,
    pub hi: f64,
    /// Mean of the bootstrap replicates. Its distance from `point` is the
    /// resampling bias.
    pub mean: f64,
    pub replicates: usize,
}

impl Interval {
    /// Whether the interval excludes zero, for a difference between two
    /// populations.
    ///
    /// Meaningless for a single `ρ`, which is positive by construction.
    pub fn excludes_zero(&self) -> bool {
        self.lo > 0.0 || self.hi < 0.0
    }

    pub fn width(&self) -> f64 {
        self.hi - self.lo
    }

    /// How far resampling moves the estimator, `mean − point`.
    ///
    /// For `ρ` this is systematically **positive** under a heavy tail, and the
    /// mechanism is worth understanding because it is not noise. Resampling `n`
    /// members with replacement leaves roughly `1/e` of them unpicked, so classes
    /// represented by a single member vanish from a replicate about 37% of the
    /// time. Fewer classes means lower `H(C)` means higher `ρ = 2^{−H(C)}`.
    ///
    /// The size of this bias is therefore a **tail diagnostic**: a population
    /// whose classes are mostly singletons shows a large one, and a population
    /// with a few crowded classes shows almost none.
    pub fn resampling_bias(&self) -> f64 {
        self.mean - self.point
    }

    /// Whether the original estimate falls inside the resampled range at all.
    ///
    /// False is not an error. It means the resampling bias exceeds the spread,
    /// which is a real and reportable property of a heavy-tailed population —
    /// not a sign that the interval was computed wrongly.
    pub fn contains_point(&self) -> bool {
        self.lo <= self.point && self.point <= self.hi
    }
}

/// Default replicate count. Enough that the 2.5th and 97.5th percentiles are
/// stable to about three decimals, and cheap: each replicate is O(n).
pub const DEFAULT_REPLICATES: usize = 10_000;

/// The fixed seed every published interval is computed at.
///
/// Published rather than arbitrary, so a reader recomputing the number gets our
/// number and not merely a similar one.
pub const DEFAULT_SEED: u64 = 0x6D69_7272_6F72_0501;

fn class_indices<L: Ord + Clone>(labels: &[L]) -> Vec<usize> {
    let mut index: BTreeMap<L, usize> = BTreeMap::new();
    let mut out = Vec::with_capacity(labels.len());
    for l in labels {
        let next = index.len();
        let id = *index.entry(l.clone()).or_insert(next);
        out.push(id);
    }
    out
}

/// One bootstrap replicate: resample `n` members with replacement and recompute
/// the loss factor.
fn replicate(members: &[usize], classes: usize, rng: &mut SplitMix64) -> Option<f64> {
    let n = members.len();
    let mut counts = vec![0u64; classes];
    for _ in 0..n {
        let pick = members[rng.below(n as u64) as usize];
        counts[pick] += 1;
    }
    // A class that no resampled member landed in is absent from this replicate,
    // not a class of size zero. `Anonymity` rejects a zero, correctly.
    let sizes: Vec<u64> = counts.into_iter().filter(|&c| c > 0).collect();
    Anonymity::from_class_sizes(&sizes).map(|a| a.loss_factor)
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let idx = (q * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// A 95% percentile interval for `ρ` over the draw of members.
///
/// `labels` is one class label per **resolved** member. Unresolved members are
/// not represented: they are the bracket's business, not this one's.
pub fn loss_factor_interval<L: Ord + Clone>(
    labels: &[L],
    replicates: usize,
    seed: u64,
) -> Option<Interval> {
    let point = Anonymity::from_labels(labels)?.loss_factor;
    let members = class_indices(labels);
    let classes = members.iter().copied().max().map(|m| m + 1)?;

    let mut rng = SplitMix64::new(seed);
    let mut draws: Vec<f64> = Vec::with_capacity(replicates);
    for _ in 0..replicates {
        if let Some(r) = replicate(&members, classes, &mut rng) {
            draws.push(r);
        }
    }
    if draws.is_empty() {
        return None;
    }
    draws.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in a loss factor"));

    let mean = draws.iter().sum::<f64>() / draws.len() as f64;
    Some(Interval {
        point,
        lo: percentile(&draws, 0.025),
        hi: percentile(&draws, 0.975),
        mean,
        replicates: draws.len(),
    })
}

/// The resampled spread of `ρ(a) − ρ(b)`.
///
/// Each population is resampled independently within a replicate, which is the
/// right structure here: the two frames are drawn from different populations and
/// share no members.
///
/// If the interval contains zero, the two are **not** distinguishable at this
/// sample size, and reporting one as more concentrated than the other would be
/// reporting noise.
pub fn difference_interval<L: Ord + Clone>(
    a: &[L],
    b: &[L],
    replicates: usize,
    seed: u64,
) -> Option<Interval> {
    let point = Anonymity::from_labels(a)?.loss_factor - Anonymity::from_labels(b)?.loss_factor;

    let ma = class_indices(a);
    let mb = class_indices(b);
    let ca = ma.iter().copied().max().map(|m| m + 1)?;
    let cb = mb.iter().copied().max().map(|m| m + 1)?;

    // One generator, drawn from alternately, so the pairing is reproducible and
    // the two populations cannot accidentally share a stream position.
    let mut rng = SplitMix64::new(seed);
    let mut draws: Vec<f64> = Vec::with_capacity(replicates);
    for _ in 0..replicates {
        let ra = replicate(&ma, ca, &mut rng);
        let rb = replicate(&mb, cb, &mut rng);
        if let (Some(x), Some(y)) = (ra, rb) {
            draws.push(x - y);
        }
    }
    if draws.is_empty() {
        return None;
    }
    draws.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in a loss factor"));

    let mean = draws.iter().sum::<f64>() / draws.len() as f64;
    Some(Interval {
        point,
        lo: percentile(&draws, 0.025),
        hi: percentile(&draws, 0.975),
        mean,
        replicates: draws.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(spec: &[(&str, usize)]) -> Vec<String> {
        let mut out = Vec::new();
        for (name, n) in spec {
            for _ in 0..*n {
                out.push((*name).to_string());
            }
        }
        out
    }

    /// One class means no uncertainty: every resample is the same population,
    /// so the interval must collapse onto the point.
    #[test]
    fn a_single_class_has_no_sampling_spread() {
        let l = labels(&[("cex:binance", 40)]);
        let i = loss_factor_interval(&l, 500, DEFAULT_SEED).unwrap();
        assert_eq!(i.point, 1.0);
        assert_eq!(i.lo, 1.0);
        assert_eq!(i.hi, 1.0);
    }

    /// With crowded classes, resampling barely moves the estimator and the
    /// range does contain it.
    #[test]
    fn a_crowded_population_resamples_around_its_estimate() {
        let l = labels(&[("a", 20), ("b", 10), ("c", 5), ("d", 5)]);
        let i = loss_factor_interval(&l, 2_000, DEFAULT_SEED).unwrap();
        assert!(
            i.contains_point(),
            "point {} outside [{}, {}]",
            i.point,
            i.lo,
            i.hi
        );
        assert!(
            i.resampling_bias().abs() < 0.05,
            "crowded classes should barely shift: bias {}",
            i.resampling_bias()
        );
    }

    /// The case that caught a false assumption of ours, kept as a test so it
    /// cannot be re-assumed.
    ///
    /// An earlier version asserted that the range always contains the point
    /// estimate. It does not, and the reason is not a bug: under a heavy tail,
    /// resampling drops singleton classes about 37% of the time, which lowers
    /// `H(C)` and therefore raises `ρ` in nearly every replicate. The whole
    /// range then sits *above* the original estimate.
    ///
    /// This is why the type is documented as the spread of the resampled
    /// estimator rather than as a confidence interval, and why the bias is
    /// reported next to it instead of being hidden inside it.
    #[test]
    fn a_heavy_tail_shifts_the_whole_range_above_the_estimate() {
        // Thirty members, almost all alone: the regime every real run is in.
        let mut spec: Vec<(String, usize)> = vec![("crowd".to_string(), 6)];
        for i in 0..24 {
            spec.push((format!("solo{i}"), 1));
        }
        let l: Vec<String> = spec
            .iter()
            .flat_map(|(n, c)| std::iter::repeat_n(n.clone(), *c))
            .collect();

        let i = loss_factor_interval(&l, 4_000, DEFAULT_SEED).unwrap();
        assert!(
            i.resampling_bias() > 0.0,
            "a heavy tail must bias rho upward under resampling, got {}",
            i.resampling_bias()
        );
        assert!(
            !i.contains_point(),
            "expected the range to sit off the point under this tail: \
             point {} in [{}, {}]",
            i.point,
            i.lo,
            i.hi
        );
    }

    /// `analyze` is a pure pass and its output is a published number. The same
    /// sample and the same seed must produce the same interval, byte for byte.
    #[test]
    fn the_interval_is_reproducible_from_its_seed() {
        let l = labels(&[("a", 12), ("b", 9), ("c", 4), ("d", 1)]);
        let x = loss_factor_interval(&l, 1_000, DEFAULT_SEED).unwrap();
        let y = loss_factor_interval(&l, 1_000, DEFAULT_SEED).unwrap();
        assert_eq!(x, y);
        let z = loss_factor_interval(&l, 1_000, DEFAULT_SEED ^ 1).unwrap();
        assert_ne!(x.lo, z.lo, "a different seed must actually resample");
    }

    /// Fewer members must mean a wider interval. If this ever inverts, the
    /// resampling is not resampling.
    #[test]
    fn a_smaller_sample_gives_a_wider_interval() {
        let big = labels(&[("a", 40), ("b", 30), ("c", 20), ("d", 10)]);
        let small = labels(&[("a", 4), ("b", 3), ("c", 2), ("d", 1)]);
        let wide = loss_factor_interval(&small, 4_000, DEFAULT_SEED).unwrap();
        let tight = loss_factor_interval(&big, 4_000, DEFAULT_SEED).unwrap();
        assert!(
            wide.width() > tight.width(),
            "small sample width {} not wider than large sample width {}",
            wide.width(),
            tight.width()
        );
    }

    /// Two populations that differ starkly must separate; the interval on the
    /// difference must exclude zero.
    #[test]
    fn a_stark_difference_separates() {
        // One concentrated population and one spread population.
        let concentrated = labels(&[("a", 38), ("b", 1), ("c", 1)]);
        let spread = labels(&[("a", 8), ("b", 8), ("c", 8), ("d", 8), ("e", 8)]);
        let d = difference_interval(&concentrated, &spread, 4_000, DEFAULT_SEED).unwrap();
        assert!(
            d.point > 0.0,
            "concentrated must have the higher loss factor"
        );
        assert!(
            d.excludes_zero(),
            "a stark difference must separate: [{}, {}]",
            d.lo,
            d.hi
        );
    }

    /// The case that matters most, and the one a careless comparison gets
    /// wrong: two samples drawn from the *same* shape must NOT separate.
    ///
    /// Without this the tool would report every pair of populations as
    /// different, which is the failure mode of comparing point estimates.
    #[test]
    fn two_samples_of_the_same_shape_do_not_separate() {
        let a = labels(&[("a", 10), ("b", 6), ("c", 3), ("d", 1)]);
        let b = labels(&[("w", 10), ("x", 6), ("y", 3), ("z", 1)]);
        let d = difference_interval(&a, &b, 4_000, DEFAULT_SEED).unwrap();
        assert!(
            !d.excludes_zero(),
            "identically shaped populations must not separate: [{}, {}]",
            d.lo,
            d.hi
        );
    }

    /// The generator must be uniform enough that it does not itself bias the
    /// interval. A modulus over the full range would fail this at large n.
    #[test]
    fn the_generator_is_uniform_over_its_range() {
        let mut rng = SplitMix64::new(DEFAULT_SEED);
        let buckets = 7u64;
        let draws = 70_000;
        let mut counts = vec![0usize; buckets as usize];
        for _ in 0..draws {
            counts[rng.below(buckets) as usize] += 1;
        }
        let expected = draws as f64 / buckets as f64;
        for (i, &c) in counts.iter().enumerate() {
            let dev = (c as f64 - expected).abs() / expected;
            assert!(dev < 0.05, "bucket {i} deviates {:.3} from uniform", dev);
        }
    }

    #[test]
    fn an_empty_population_yields_nothing() {
        let empty: Vec<String> = Vec::new();
        assert!(loss_factor_interval(&empty, 100, DEFAULT_SEED).is_none());
    }
}
