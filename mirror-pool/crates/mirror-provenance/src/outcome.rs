//! How a trace ends, and the census that keeps infrastructure limits from
//! reading as evidence.
//!
//! The governing principle: **an RPC failure is not a finding.** A rate-limited
//! call that returns nothing must never be recorded as "this address has no
//! funder", because that is indistinguishable in the output from a genuine
//! terminal — and it inflates exactly the bucket a privacy measurement most
//! wants to be large.
//!
//! A published tracer in this space uses `.unwrap_or(0)` for signature counts
//! and `.unwrap_or_default()` for signature lists. A throttled call there makes
//! an address look like it has zero history, so it is not detected as a hub and
//! is admitted to the sample, and a throttled trace yields no funder, so the
//! member is classified unresolved. Its large unresolved bucket is what
//! systematic rate limiting would manufacture, and its headline does not say so.

use serde::{Deserialize, Serialize};

/// Which terminal rule classified an address. Recorded so every rule can be
/// ablated in the sensitivity table rather than taken on faith.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalRule {
    /// R1: the account is a program or PDA — not owned by the system program,
    /// or executable. Definitional, one call, and what separates "funded by a
    /// Raydium vault" from "funded by a person".
    ProgramOrPda,
    /// R2: a hit in the curated, MIT-licensed anchor set.
    CuratedAnchor,
    /// R3: a member of a fee-payer or sweep cluster of at least three.
    StructuralCluster,
    /// R4: high lifetime activity and at least thirty days old.
    ///
    /// This is **not** an attributable origin and must never be described as
    /// one. It means "busy address we could not name".
    VolumeHub,
    /// R5: has funded many distinct previously-unseen addresses.
    Distributor,
}

/// Why a trace failed to reach a class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unresolved {
    /// The address has no qualifying incoming value edge. Genuine evidence.
    NoIncomingEdge,
    /// The incoming edge was below the minimum value threshold. Evidence.
    BelowThreshold,
    /// Hit the depth limit. A budget limit, not evidence.
    DepthExceeded,
    /// Hit the signature page cap without terminating. Budget, not evidence.
    PageCapHit,
    /// An RPC error, 429 exhaustion, or timeout. **Not evidence.**
    RpcFailure,
    /// SOL-only scope, and the address holds token accounts with inflow, so the
    /// funding event is structurally invisible. A scope limit, not evidence.
    SplBlindSpot,
    /// The oldest transactions were read up to the birth-scan cap and none of
    /// them credited the address. **Budget, not evidence** — the credit is
    /// plausibly the next transaction along.
    ///
    /// Kept separate from `NoIncomingEdge` deliberately. Collapsing the two
    /// would let "we stopped looking" be reported as "there is nothing there",
    /// which is the exact substitution this taxonomy exists to prevent.
    BirthScanExhausted,
}

impl Unresolved {
    /// Whether this outcome tells us something about the address, as opposed to
    /// something about our own budget or infrastructure.
    pub fn is_evidence(&self) -> bool {
        matches!(
            self,
            Unresolved::NoIncomingEdge | Unresolved::BelowThreshold
        )
    }

    /// Whether this outcome is a failure of ours rather than a property of the
    /// chain. These members leave the class distribution entirely.
    pub fn is_failure(&self) -> bool {
        matches!(self, Unresolved::RpcFailure)
    }
}

/// The end state of tracing one member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// Classified. The label is what the partition groups on.
    Terminal { rule: TerminalRule, label: String },
    /// Not classified, with the reason kept.
    Unresolved { reason: Unresolved, depth: u32 },
}

impl Outcome {
    pub fn label(&self) -> Option<&str> {
        match self {
            Outcome::Terminal { label, .. } => Some(label),
            Outcome::Unresolved { .. } => None,
        }
    }
}

/// A count of every terminal state in a run. There is no "other" bucket,
/// because an "other" bucket is where an unaccounted failure would hide.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Census {
    pub resolved: u64,
    pub no_incoming_edge: u64,
    pub below_threshold: u64,
    pub depth_exceeded: u64,
    pub page_cap_hit: u64,
    pub rpc_failure: u64,
    pub spl_blind_spot: u64,
    /// Budget, not evidence: the birth-credit scan ran out before finding one.
    #[serde(default)]
    pub birth_scan_exhausted: u64,
}

/// Above this rate of RPC failures a run refuses to publish a headline.
///
/// One percent. Beyond it, the unresolved bucket is substantially our own
/// making, and any effective-k computed from it would be measuring our
/// infrastructure rather than the pool.
pub const MAX_FAILURE_RATE: f64 = 0.01;

impl Census {
    pub fn record(&mut self, outcome: &Outcome) {
        match outcome {
            Outcome::Terminal { .. } => self.resolved += 1,
            Outcome::Unresolved { reason, .. } => match reason {
                Unresolved::NoIncomingEdge => self.no_incoming_edge += 1,
                Unresolved::BelowThreshold => self.below_threshold += 1,
                Unresolved::DepthExceeded => self.depth_exceeded += 1,
                Unresolved::PageCapHit => self.page_cap_hit += 1,
                Unresolved::RpcFailure => self.rpc_failure += 1,
                Unresolved::SplBlindSpot => self.spl_blind_spot += 1,
                Unresolved::BirthScanExhausted => self.birth_scan_exhausted += 1,
            },
        }
    }

    /// Every member the run attempted, including failures.
    pub fn attempted(&self) -> u64 {
        self.resolved
            + self.no_incoming_edge
            + self.below_threshold
            + self.depth_exceeded
            + self.page_cap_hit
            + self.rpc_failure
            + self.spl_blind_spot
            + self.birth_scan_exhausted
    }

    /// Members whose outcome says something about the chain: everything except
    /// our own RPC failures.
    pub fn measurable(&self) -> u64 {
        self.attempted() - self.rpc_failure
    }

    pub fn failure_rate(&self) -> f64 {
        let attempted = self.attempted();
        if attempted == 0 {
            return 0.0;
        }
        self.rpc_failure as f64 / attempted as f64
    }

    /// Whether a headline may be published from this run.
    ///
    /// Deliberately a hard gate rather than a warning. A warning printed above a
    /// number gets dropped when the number is quoted.
    pub fn may_publish(&self) -> bool {
        self.attempted() > 0 && self.failure_rate() <= MAX_FAILURE_RATE
    }

    /// The line that must accompany any reported figure.
    pub fn summary(&self) -> String {
        format!(
            "attempted {} | resolved {} | evidence-unresolved {} | budget-unresolved {} \
             | scope-unresolved {} | rpc failures {} ({:.2}%)",
            self.attempted(),
            self.resolved,
            self.no_incoming_edge + self.below_threshold,
            self.depth_exceeded + self.page_cap_hit + self.birth_scan_exhausted,
            self.spl_blind_spot,
            self.rpc_failure,
            self.failure_rate() * 100.0
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal(label: &str) -> Outcome {
        Outcome::Terminal {
            rule: TerminalRule::CuratedAnchor,
            label: label.to_string(),
        }
    }

    fn unresolved(reason: Unresolved) -> Outcome {
        Outcome::Unresolved { reason, depth: 3 }
    }

    #[test]
    fn an_rpc_failure_is_never_evidence() {
        assert!(!Unresolved::RpcFailure.is_evidence());
        assert!(Unresolved::RpcFailure.is_failure());
        // The two that genuinely say something about the address.
        assert!(Unresolved::NoIncomingEdge.is_evidence());
        assert!(Unresolved::BelowThreshold.is_evidence());
        // Budget and scope limits are neither.
        for r in [
            Unresolved::DepthExceeded,
            Unresolved::PageCapHit,
            Unresolved::SplBlindSpot,
        ] {
            assert!(!r.is_evidence(), "{r:?} must not count as evidence");
            assert!(!r.is_failure(), "{r:?} is not an infrastructure failure");
        }
    }

    #[test]
    fn the_census_accounts_for_every_member() {
        let mut c = Census::default();
        c.record(&terminal("entity:binance"));
        c.record(&terminal("entity:binance"));
        c.record(&unresolved(Unresolved::NoIncomingEdge));
        c.record(&unresolved(Unresolved::RpcFailure));
        c.record(&unresolved(Unresolved::DepthExceeded));
        c.record(&unresolved(Unresolved::SplBlindSpot));
        assert_eq!(c.attempted(), 6);
        assert_eq!(
            c.measurable(),
            5,
            "failures leave the distribution entirely"
        );
        assert_eq!(c.resolved, 2);
    }

    #[test]
    fn a_run_with_too_many_failures_refuses_to_publish() {
        let mut c = Census::default();
        for _ in 0..99 {
            c.record(&terminal("entity:x"));
        }
        c.record(&unresolved(Unresolved::RpcFailure));
        // 1 in 100 is exactly at the limit and still publishable.
        assert!((c.failure_rate() - 0.01).abs() < 1e-12);
        assert!(c.may_publish());

        c.record(&unresolved(Unresolved::RpcFailure));
        assert!(
            !c.may_publish(),
            "two failures in 101 is above the limit and must block the headline"
        );
    }

    #[test]
    fn an_empty_run_cannot_publish() {
        assert!(!Census::default().may_publish());
    }

    #[test]
    fn the_summary_names_every_bucket_separately() {
        let mut c = Census::default();
        c.record(&terminal("a"));
        c.record(&unresolved(Unresolved::RpcFailure));
        let s = c.summary();
        for expected in ["attempted", "resolved", "rpc failures"] {
            assert!(s.contains(expected), "summary missing {expected}: {s}");
        }
        // The failure count must be visible as its own number, not folded into
        // the unresolved total.
        assert!(s.contains("rpc failures 1"), "{s}");
    }

    #[test]
    fn a_volume_hub_is_not_an_attributable_origin() {
        // Encoded as a test because this is where a provenance tracer most
        // easily launders a budget limit into a finding: if a volume hub counts
        // as an origin, "reaches an attributable origin" degenerates into "the
        // address hit the RPC page cap".
        let hub = Outcome::Terminal {
            rule: TerminalRule::VolumeHub,
            label: "busy-unlabelled:SomeAddress".to_string(),
        };
        assert!(
            hub.label().unwrap().starts_with("busy-unlabelled:"),
            "a volume hub must be labelled as unnamed, never as an entity"
        );
    }
}
