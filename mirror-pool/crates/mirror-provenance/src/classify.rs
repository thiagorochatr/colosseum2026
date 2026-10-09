//! The five terminal rules, applied over a complete fact set.
//!
//! Order is R1, R2, R3, R5, R4 — and R4 is deliberately last rather than fourth
//! as the methodology table lists it. R4 is the only rule that does not name
//! anything: it produces `busy-unlabelled`, which the methodology is explicit is
//! **not an attributable origin**. So any rule that can name an entity must get
//! the chance first, and R4 is the residual for "busy, and we could not say
//! what". Evaluating it before R5 would relabel a demonstrable distributor as an
//! anonymous hub, discarding information we already paid for.
//!
//! The rule that fired is carried in the outcome, so each can be ablated in a
//! sensitivity table rather than trusted as a block.

use crate::{
    facts::AddressFacts,
    outcome::{Outcome, TerminalRule},
    rpc::SYSTEM_PROGRAM,
};
use std::collections::{BTreeMap, BTreeSet};

/// Thresholds, all configurable and all reported in the run manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thresholds {
    /// Lifetime signatures at or above which an address is a volume hub.
    ///
    /// Deliberately far below the paging cap, and the two must never be the
    /// same constant. They sound like the same question — "how many signatures
    /// do we care about" — but setting the hub threshold equal to the RPC page
    /// limit makes "is a hub" mean "hit the page cap", which admits every DEX
    /// program and bot while excluding a genuine exchange withdrawal address
    /// with 800 transactions.
    pub hub_signatures: u64,
    /// Minimum account age for the hub rule, in seconds. A young address with
    /// many transactions is a bot or an airdrop, not an origin.
    pub hub_min_age_seconds: i64,
    /// Distinct addresses an account must have created to be a distributor.
    pub distributor_fanout: usize,
    /// Members a fee-payer cluster needs before it counts as structural.
    pub cluster_min_size: usize,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            hub_signatures: 5_000,
            hub_min_age_seconds: 30 * 24 * 3_600,
            distributor_fanout: 20,
            cluster_min_size: 3,
        }
    }
}

/// The curated anchor set: addresses whose operator is known by name.
#[derive(Debug, Clone, Default)]
pub struct AnchorSet {
    entries: BTreeMap<String, String>,
}

impl AnchorSet {
    pub fn from_pairs<K: Into<String>, V: Into<String>>(
        pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        AnchorSet {
            entries: pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    pub fn name_of(&self, address: &str) -> Option<&str> {
        self.entries.get(address).map(|s| s.as_str())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Set-level structure derived once from the complete fact set.
///
/// Built in one pass over everything collected, so membership never depends on
/// the order addresses were visited in.
#[derive(Debug, Clone, Default)]
pub struct SetStructure {
    /// address -> canonical cluster id, for clusters at or above the threshold.
    cluster_of: BTreeMap<String, String>,
}

impl SetStructure {
    /// Groups addresses by shared fee payer and keeps groups large enough to be
    /// structural.
    ///
    /// The cluster id is the lexicographically smallest member, so it does not
    /// depend on iteration order or on which member was seen first.
    pub fn build(facts: &BTreeMap<String, AddressFacts>, thresholds: &Thresholds) -> Self {
        let mut by_payer: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for f in facts.values() {
            for payer in &f.fee_payers {
                by_payer
                    .entry(payer.as_str())
                    .or_default()
                    .insert(f.address.as_str());
            }
        }

        let mut cluster_of = BTreeMap::new();
        for members in by_payer.values() {
            if members.len() < thresholds.cluster_min_size {
                continue;
            }
            let canonical = members
                .iter()
                .min()
                .expect("non-empty by construction")
                .to_string();
            for m in members {
                // A member already in a cluster keeps the smaller id, so
                // overlapping payer groups resolve deterministically.
                cluster_of
                    .entry(m.to_string())
                    .and_modify(|existing: &mut String| {
                        if canonical < *existing {
                            *existing = canonical.clone();
                        }
                    })
                    .or_insert_with(|| canonical.clone());
            }
        }
        SetStructure { cluster_of }
    }

    pub fn cluster_of(&self, address: &str) -> Option<&str> {
        self.cluster_of.get(address).map(|s| s.as_str())
    }
}

/// Applies the terminal rules to one address.
pub struct Classifier<'a> {
    pub anchors: &'a AnchorSet,
    pub structure: &'a SetStructure,
    pub thresholds: &'a Thresholds,
    /// Wall time the classification is relative to, for the age test. Passed in
    /// rather than read from the clock so a replay reproduces exactly.
    pub now: i64,
}

impl Classifier<'_> {
    /// Whether `facts` terminates a trace, and under which rule.
    pub fn classify(&self, facts: &AddressFacts) -> Option<Outcome> {
        // R1 — a program or PDA. Definitional and cheap, and the only thing that
        // separates "funded by a protocol vault" from "funded by a person".
        // Without it, one exchange hot wallet and one AMM vault land in the same
        // class, and the class distribution the headline is computed over is wrong
        // in a direction nothing downstream can detect.
        if let Some(owner) = &facts.owner {
            if owner != SYSTEM_PROGRAM || facts.executable {
                return Some(Outcome::Terminal {
                    rule: TerminalRule::ProgramOrPda,
                    label: format!("program:{}", facts.address),
                });
            }
        }

        // R2 — a named operator.
        if let Some(name) = self.anchors.name_of(&facts.address) {
            return Some(Outcome::Terminal {
                rule: TerminalRule::CuratedAnchor,
                label: format!("entity:{name}"),
            });
        }

        // R3 — a fee-payer cluster large enough to be structural.
        if let Some(id) = self.structure.cluster_of(&facts.address) {
            return Some(Outcome::Terminal {
                rule: TerminalRule::StructuralCluster,
                label: format!("cluster:{id}"),
            });
        }

        // R5 — created many distinct addresses. Evaluated before R4 because it
        // names a role and R4 does not.
        let distinct_funded: BTreeSet<&str> = facts.funded.iter().map(|s| s.as_str()).collect();
        if distinct_funded.len() >= self.thresholds.distributor_fanout {
            return Some(Outcome::Terminal {
                rule: TerminalRule::Distributor,
                label: format!("distributor:{}", facts.address),
            });
        }

        // R4 — busy and old, but unnamed. The label says so, and it must never
        // be described as an attributable origin.
        let old_enough = facts
            .age_seconds(self.now)
            .is_some_and(|age| age >= self.thresholds.hub_min_age_seconds);
        if facts.signatures.lower_bound() >= self.thresholds.hub_signatures && old_enough {
            return Some(Outcome::Terminal {
                rule: TerminalRule::VolumeHub,
                label: format!("busy-unlabelled:{}", facts.address),
            });
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::SigCount;

    const NOW: i64 = 1_800_000_000;
    const OLD: i64 = NOW - 400 * 24 * 3_600;
    const YOUNG: i64 = NOW - 3 * 24 * 3_600;

    fn wallet(address: &str) -> AddressFacts {
        let mut f = AddressFacts::new(address);
        f.owner = Some(SYSTEM_PROGRAM.to_string());
        f
    }

    struct Env {
        anchors: AnchorSet,
        structure: SetStructure,
        thresholds: Thresholds,
    }

    impl Env {
        fn new() -> Self {
            Env {
                anchors: AnchorSet::from_pairs([("BinanceHot1", "binance")]),
                structure: SetStructure::default(),
                thresholds: Thresholds::default(),
            }
        }
        fn classify(&self, f: &AddressFacts) -> Option<Outcome> {
            Classifier {
                anchors: &self.anchors,
                structure: &self.structure,
                thresholds: &self.thresholds,
                now: NOW,
            }
            .classify(f)
        }
    }

    #[test]
    fn r1_catches_a_program() {
        let env = Env::new();
        let mut f = wallet("SomeProgram");
        f.owner = Some("BPFLoaderUpgradeab1e11111111111111111111111".into());
        f.executable = true;
        let out = env.classify(&f).unwrap();
        assert!(matches!(
            out,
            Outcome::Terminal {
                rule: TerminalRule::ProgramOrPda,
                ..
            }
        ));
        assert_eq!(out.label().unwrap(), "program:SomeProgram");
    }

    #[test]
    fn r1_catches_a_pda_that_is_not_executable() {
        // A PDA owned by a program is not a person's wallet even though it runs
        // no code. This is what distinguishes "funded by a vault" from "funded
        // by someone".
        let env = Env::new();
        let mut f = wallet("SomeVault");
        f.owner = Some("Whirlpoo1111111111111111111111111111111111".into());
        f.executable = false;
        assert!(matches!(
            env.classify(&f).unwrap(),
            Outcome::Terminal {
                rule: TerminalRule::ProgramOrPda,
                ..
            }
        ));
    }

    #[test]
    fn a_plain_wallet_is_not_terminal_on_r1() {
        let env = Env::new();
        assert!(env.classify(&wallet("JustAWallet")).is_none());
    }

    #[test]
    fn an_account_that_does_not_exist_is_not_called_a_program() {
        // owner is None: the account is absent, which we do not know how to
        // classify. Treating absence as "not system-owned" would label every
        // closed account a program.
        let env = Env::new();
        let mut f = AddressFacts::new("Gone");
        f.owner = None;
        assert!(env.classify(&f).is_none());
    }

    #[test]
    fn r2_names_a_curated_anchor() {
        let env = Env::new();
        let out = env.classify(&wallet("BinanceHot1")).unwrap();
        assert_eq!(out.label().unwrap(), "entity:binance");
    }

    #[test]
    fn r3_groups_addresses_that_share_a_fee_payer() {
        let mut facts = BTreeMap::new();
        for a in ["zebra", "alpha", "mango"] {
            let mut f = wallet(a);
            f.fee_payers = vec!["Sweeper1".into()];
            facts.insert(a.to_string(), f);
        }
        let thresholds = Thresholds::default();
        let structure = SetStructure::build(&facts, &thresholds);

        // The canonical id is the smallest member, so it does not depend on the
        // order addresses were collected in.
        assert_eq!(structure.cluster_of("zebra"), Some("alpha"));
        assert_eq!(structure.cluster_of("mango"), Some("alpha"));

        let env = Env {
            anchors: AnchorSet::default(),
            structure,
            thresholds,
        };
        let out = env.classify(&facts["zebra"]).unwrap();
        assert_eq!(out.label().unwrap(), "cluster:alpha");
    }

    #[test]
    fn a_fee_payer_group_below_the_threshold_is_not_a_cluster() {
        let mut facts = BTreeMap::new();
        for a in ["one", "two"] {
            let mut f = wallet(a);
            f.fee_payers = vec!["Payer1".into()];
            facts.insert(a.to_string(), f);
        }
        let structure = SetStructure::build(&facts, &Thresholds::default());
        assert!(
            structure.cluster_of("one").is_none(),
            "two is below the floor of three"
        );
    }

    #[test]
    fn r5_catches_a_distributor_and_counts_distinct_addresses_only() {
        let env = Env::new();
        let mut f = wallet("Fanout1");
        // Twenty entries but only five distinct: not a distributor.
        f.funded = (0..20).map(|i| format!("child{}", i % 5)).collect();
        assert!(env.classify(&f).is_none(), "repeats must not count");

        f.funded = (0..20).map(|i| format!("child{i}")).collect();
        let out = env.classify(&f).unwrap();
        assert!(matches!(
            out,
            Outcome::Terminal {
                rule: TerminalRule::Distributor,
                ..
            }
        ));
    }

    #[test]
    fn r4_needs_both_volume_and_age() {
        let env = Env::new();
        let mut f = wallet("Busy1");
        f.signatures = SigCount::AtLeast(20_000);

        f.first_seen = Some(YOUNG);
        assert!(
            env.classify(&f).is_none(),
            "a young address with many transactions is a bot, not an origin"
        );

        f.first_seen = Some(OLD);
        let out = env.classify(&f).unwrap();
        assert!(matches!(
            out,
            Outcome::Terminal {
                rule: TerminalRule::VolumeHub,
                ..
            }
        ));
        assert_eq!(out.label().unwrap(), "busy-unlabelled:Busy1");
    }

    /// The distinction that is easiest to collapse and most costly to lose: a
    /// busy address is not an attributable origin, and its label must never
    /// read like one.
    #[test]
    fn a_volume_hub_is_labelled_as_unnamed() {
        let env = Env::new();
        let mut f = wallet("Busy2");
        f.signatures = SigCount::Exact(9_000);
        f.first_seen = Some(OLD);
        let label = env.classify(&f).unwrap().label().unwrap().to_string();
        assert!(label.starts_with("busy-unlabelled:"));
        assert!(!label.starts_with("entity:"));
    }

    /// The hub threshold must be reachable well inside the paging budget, or
    /// "is a hub" degenerates into "we stopped looking".
    #[test]
    fn the_hub_threshold_is_decoupled_from_the_page_cap() {
        let t = Thresholds::default();
        assert!(
            t.hub_signatures < 20_000,
            "the threshold must be reachable before paging stops, not equal to it"
        );
        assert!(
            t.hub_signatures > 1_000,
            "and above a single page, or ordinary wallets qualify"
        );
    }

    #[test]
    fn a_named_entity_beats_an_anonymous_hub() {
        // Both rules fire. The one that names something must win, or we discard
        // information we already paid an RPC call for.
        let env = Env::new();
        let mut f = wallet("BinanceHot1");
        f.signatures = SigCount::AtLeast(50_000);
        f.first_seen = Some(OLD);
        assert_eq!(env.classify(&f).unwrap().label().unwrap(), "entity:binance");
    }

    #[test]
    fn a_distributor_beats_an_anonymous_hub() {
        let env = Env::new();
        let mut f = wallet("Distrib1");
        f.signatures = SigCount::AtLeast(50_000);
        f.first_seen = Some(OLD);
        f.funded = (0..40).map(|i| format!("child{i}")).collect();
        assert!(matches!(
            env.classify(&f).unwrap(),
            Outcome::Terminal {
                rule: TerminalRule::Distributor,
                ..
            }
        ));
    }

    #[test]
    fn rule_precedence_is_total_and_stable() {
        // Every rule fires at once; R1 must win, and the result must not depend
        // on anything but the rule order.
        let mut facts = BTreeMap::new();
        for a in ["aaa", "bbb", "ccc"] {
            let mut f = wallet(a);
            f.fee_payers = vec!["Payer1".into()];
            facts.insert(a.to_string(), f);
        }
        let thresholds = Thresholds::default();
        let structure = SetStructure::build(&facts, &thresholds);

        let mut everything = wallet("aaa");
        everything.owner = Some("SomeProgram11111111111111111111111111111111".into());
        everything.fee_payers = vec!["Payer1".into()];
        everything.funded = (0..50).map(|i| format!("c{i}")).collect();
        everything.signatures = SigCount::AtLeast(99_999);
        everything.first_seen = Some(OLD);

        let env = Env {
            anchors: AnchorSet::from_pairs([("aaa", "someexchange")]),
            structure,
            thresholds,
        };
        assert!(matches!(
            env.classify(&everything).unwrap(),
            Outcome::Terminal {
                rule: TerminalRule::ProgramOrPda,
                ..
            }
        ));
    }

    #[test]
    fn classification_is_independent_of_collection_order() {
        // The same addresses inserted in two different orders must produce the
        // same clusters and therefore the same labels. This is the property the
        // two-pass split exists to guarantee.
        let build = |order: &[&str]| {
            let mut facts = BTreeMap::new();
            for a in order {
                let mut f = wallet(a);
                f.fee_payers = vec!["P".into()];
                facts.insert(a.to_string(), f);
            }
            SetStructure::build(&facts, &Thresholds::default())
        };
        let a = build(&["delta", "alpha", "charlie"]);
        let b = build(&["charlie", "delta", "alpha"]);
        for addr in ["alpha", "charlie", "delta"] {
            assert_eq!(a.cluster_of(addr), b.cluster_of(addr));
        }
    }
}
