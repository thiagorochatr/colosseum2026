//! The measurement this repository publishes, in a form code can quote.
//!
//! Every place that surfaces a pool's *nominal* membership — the CLI's pool
//! status, the settlement summary — is a place a reader can walk away with a `k`
//! that nothing has discounted. The on-chain `k_floor` bounds program-visible
//! membership and nothing else; the effective set is smaller, because an
//! observer can partition members by where their deposit came from. That is not
//! a caveat to keep in a document, so this module puts the measured figure next
//! to the nominal one wherever the nominal one appears.
//!
//! **The numbers below are pinned, not typed.**
//! [`the_published_headline_is_what_the_committed_sample_produces`] recomputes
//! every one of them from the committed sample and fails if they drift. Without
//! that test this file would be a second, unchecked copy of the result — which
//! is the failure mode `docs/MEASUREMENT_LOG.md` already records once.

/// The headline measurement, and where it came from.
///
/// Deliberately carries the pool it describes. This measurement is of *another
/// protocol's* live pool, not of `mirror-pool`, which has no depositors — and a
/// figure quoted without that attached is a figure about to be misread as ours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PublishedHeadline {
    /// The committed sample the figures are recomputed from.
    pub sample: &'static str,
    /// The pool measured, which is not this project's.
    pub pool: &'static str,
    /// Its address, so a reader can go and look rather than take the name.
    pub pool_address: &'static str,
    pub attempted: u64,
    pub resolved: u64,
    /// `ρ = 2^{−H(C)}` over the resolved members.
    pub loss_factor: f64,
    /// The unresolved bracket around it: unresolved as singletons, and merged.
    pub bracket_low: f64,
    pub bracket_high: f64,
}

/// The measurement `README.md` and `docs/MEASUREMENT_LOG.md` publish.
pub const PUBLISHED_HEADLINE: PublishedHeadline = PublishedHeadline {
    sample: "data/sample-privacycash-run6.json",
    pool: "Privacy Cash",
    pool_address: "9fhQBbumKEFuXtMBDw8AaQyAjCorLGJQiS3skWZdQyQD",
    attempted: 83,
    resolved: 54,
    loss_factor: 0.0955,
    bracket_low: 0.0350,
    bracket_high: 0.1136,
};

impl PublishedHeadline {
    /// The line to print beside a nominal `k`.
    ///
    /// Short on purpose: it goes into operational output that a member reads
    /// while doing something else, and its whole job is to stop `k_floor` from
    /// being read as the anonymity set. The argument is in
    /// `docs/PROVENANCE_METHOD.md`; this is the pointer to it.
    pub fn note(&self) -> String {
        // Hand-wrapped to about 78 columns. This lands in a terminal beside
        // other output, and a paragraph that wraps raggedly reads as noise to
        // scroll past — which is the one thing it must not be.
        format!(
            "  That k is program-visible membership only, and the effective set is smaller.\n  \
             Measured against a live pool of comparable shape — {},\n  \
             {} — knowing a member's funding\n  \
             class left ρ = {:.4}, inside an unresolved bracket of {:.4} .. {:.4}. Roughly\n  \
             an order of magnitude of the nominal figure.\n  \
             Method, and what it does not cover: docs/PROVENANCE_METHOD.md.",
            self.pool, self.pool_address, self.loss_factor, self.bracket_low, self.bracket_high
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnchorSet, Anonymity, Bracket, Sample, Thresholds};

    /// The constants above, recomputed from the committed sample.
    ///
    /// This is what makes the module a quotation rather than a claim. A figure
    /// copied by hand into code drifts from the run that produced it silently,
    /// and both copies keep looking right; recomputing means the sample is the
    /// only place the number lives.
    #[test]
    fn the_published_headline_is_what_the_committed_sample_produces() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/sample-privacycash-run6.json"
        );
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("the committed sample must be readable: {e}"));
        let sample = Sample::from_json(&text).expect("the committed sample must parse");

        let (results, census) = crate::classify_sample(
            &sample,
            &AnchorSet::default(),
            &Thresholds::default(),
            sample.manifest.collected_at,
        );
        assert!(
            census.may_publish(),
            "the sample behind the headline must clear the failure gate"
        );

        let labels: Vec<String> = results
            .iter()
            .filter_map(|(_, o)| o.label().map(|s| s.to_string()))
            .collect();
        let point = Anonymity::from_labels(&labels).expect("the sample must resolve members");

        let mut sizes: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
        for l in &labels {
            *sizes.entry(l.as_str()).or_insert(0) += 1;
        }
        let resolved_sizes: Vec<u64> = sizes.into_values().collect();
        let bracket = Bracket::new(&resolved_sizes, census.measurable() - census.resolved)
            .expect("the sample must bracket");
        assert!(
            bracket.is_informative(),
            "the headline must come from a sample that clears the informativeness gate"
        );

        let h = PUBLISHED_HEADLINE;
        assert_eq!(h.sample, "data/sample-privacycash-run6.json");
        assert_eq!(point.nominal_k, h.resolved, "resolved members moved");
        assert_eq!(
            bracket.resolved + bracket.unresolved,
            h.attempted,
            "the attempted count moved"
        );
        // To the four decimals the documents publish, which is the precision
        // the claim is made at.
        for (name, got, pinned) in [
            ("rho", point.loss_factor, h.loss_factor),
            ("bracket low", bracket.lower.loss_factor, h.bracket_low),
            ("bracket high", bracket.upper.loss_factor, h.bracket_high),
        ] {
            assert!(
                (got - pinned).abs() < 5e-5,
                "{name} drifted: the sample produces {got:.6}, this module publishes {pinned:.4}"
            );
        }
    }

    #[test]
    fn the_note_names_the_pool_it_measured_and_never_claims_it_is_ours() {
        let note = PUBLISHED_HEADLINE.note();
        assert!(note.contains("Privacy Cash"), "{note}");
        assert!(note.contains("program-visible"), "{note}");
        assert!(note.contains("PROVENANCE_METHOD.md"), "{note}");
    }
}
