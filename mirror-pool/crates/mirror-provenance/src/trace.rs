//! Pass one: walk the birth-edge chain and record what was seen.
//!
//! The only networked step. Everything it observes is written to a sample file,
//! and pass two runs over that file with no network at all — so a published
//! number can be recomputed by anyone holding the sample, without RPC access and
//! without trusting that our endpoint behaved the same way on their machine.
//!
//! ## Why the oldest edge
//!
//! Funding is, by definition, among an address's *oldest* transactions.
//! `getSignaturesForAddress` returns newest-first with no forward cursor, so for
//! an address with fewer than one page of lifetime signatures the birth
//! transaction is the **last element of the first page**.
//!
//! Taking the first few entries of that page instead — the most **recent**
//! transactions — is the wrong end of the history for any address with more
//! than a page-fragment of activity. It also fails silently, reporting
//! "unresolved" where the truth is "we looked in the wrong place", and it does
//! so for exactly the active wallets a funding trace most wants to follow.
//!
//! ## Where pass one stops
//!
//! Only on rules that are properties of the address itself: it is a program or a
//! PDA, it has no incoming edge, the edge is too small, or the budget ran out.
//! The set-level rules wait for pass two, which sees the whole sample. Stopping
//! early on an address-local rule is safe because pass two would fire the same
//! rule at the same place; the recorded chain is always a superset of what pass
//! two needs.

use crate::{
    classify::Thresholds,
    facts::{AddressFacts, SigCount},
    outcome::Unresolved,
    rpc::{EndpointCheck, RpcClient, RpcError, SYSTEM_PROGRAM},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Which value flows the run followed.
///
/// Recorded in the manifest because it bounds what the result can mean: a wallet
/// funded in USDC has no SOL funding event to find, and `getSignaturesForAddress`
/// does not index the recipient's wallet for a token transfer into an existing
/// account. A SOL-only run is structurally blind to the dominant exchange
/// withdrawal path, and its unresolved bucket is partly that blindness rather
/// than a property of the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    /// Native SOL only.
    Sol,
    /// SOL plus SPL token flows. Two and a half to three times the call count.
    SolAndSpl,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionConfig {
    pub scope: Scope,
    /// Hops to follow before giving up.
    pub depth_max: u32,
    /// Credits below this are not treated as funding.
    pub min_edge_lamports: u64,
    /// Signature pages to read before declaring the address high-activity.
    ///
    /// Small on purpose. Paging exists to find the birth edge, and for an
    /// address busy enough not to reach it, what we actually need is only enough
    /// history to clear the volume-hub threshold and estimate an age — a few
    /// thousand signatures, not tens of thousands. Raising this buys almost
    /// nothing and costs a great deal: providers meter by compute units, and a
    /// deep page walk over busy funders is what makes an endpoint start
    /// refusing.
    pub sig_page_cap: u32,
    pub page_size: u32,
    /// Transactions to examine, oldest first, while looking for the birth
    /// *credit*.
    ///
    /// The rule is "the oldest credit", and an address's oldest **transaction**
    /// is frequently not one: an account can appear as a passive participant in
    /// someone else's transaction — an ATA creation, a multisig setup, a program
    /// interaction that merely references it — before it ever receives value.
    /// Reading only that one transaction and concluding "no incoming edge"
    /// reports an absence of evidence where the truth is that we looked at a
    /// single transaction out of thousands.
    ///
    /// So the walk continues forward from the oldest until a credit appears.
    /// Exhausting this budget without finding one is a **budget** outcome and is
    /// recorded as such, never as evidence about the address.
    ///
    /// Defaulted on deserialisation so that samples committed before this field
    /// existed still load. A committed artifact is the whole point of the
    /// two-pass split — anyone must be able to recompute a published number from
    /// it years later — and a schema change that silently makes old samples
    /// unreadable would quietly retire the evidence for every earlier claim.
    #[serde(default = "default_birth_scan_cap")]
    pub birth_scan_cap: u32,
}

/// What runs before this field existed effectively did: read exactly one
/// transaction and stop. Recorded honestly rather than backfilled with today's
/// value, so an old sample reports the budget it was actually collected under.
fn default_birth_scan_cap() -> u32 {
    1
}

impl Default for CollectionConfig {
    fn default() -> Self {
        CollectionConfig {
            scope: Scope::Sol,
            depth_max: 6,
            // Below the rent-exempt minimum for an empty account, so account
            // creation itself always counts as funding.
            min_edge_lamports: 500_000,
            sig_page_cap: 20,
            page_size: 1_000,
            // Enough to clear the common cases — an ATA creation, a couple of
            // program interactions — before the account is funded, without
            // turning one address into a hundred transaction fetches.
            birth_scan_cap: 24,
        }
    }
}

/// Why pass one stopped walking a chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChainStop {
    /// An address-local terminal rule fired. Pass two decides which.
    LocalTerminal,
    NoIncomingEdge,
    BelowThreshold,
    DepthExceeded,
    PageCapHit,
    RpcFailure,
    /// The oldest transactions were examined up to `birth_scan_cap` and none of
    /// them credited the address. A budget outcome: the credit may well exist
    /// just beyond the scan.
    BirthScanExhausted,
}

impl ChainStop {
    /// The unresolved reason this stop maps to when no rule fires in pass two.
    pub fn as_unresolved(&self) -> Unresolved {
        match self {
            // A local terminal that pass two does not confirm means the address
            // stopped being classifiable, which is a budget outcome rather than
            // evidence about the chain.
            ChainStop::LocalTerminal | ChainStop::DepthExceeded => Unresolved::DepthExceeded,
            ChainStop::NoIncomingEdge => Unresolved::NoIncomingEdge,
            ChainStop::BelowThreshold => Unresolved::BelowThreshold,
            ChainStop::PageCapHit => Unresolved::PageCapHit,
            ChainStop::RpcFailure => Unresolved::RpcFailure,
            ChainStop::BirthScanExhausted => Unresolved::BirthScanExhausted,
        }
    }
}

/// What a caller learns as each seed completes.
///
/// Collection is bounded by network latency, not by work, so a run over a few
/// hundred seeds takes tens of minutes with nothing to show for it. Reporting
/// per seed is the difference between a tool that looks hung and one that does
/// not.
///
/// A callback rather than printing: a library that writes to stdout takes a
/// decision that belongs to whoever is calling it.
#[derive(Debug, Clone)]
pub struct Progress<'a> {
    pub done: usize,
    pub total: usize,
    pub seed: &'a str,
    pub hops: usize,
    pub stop: ChainStop,
    /// RPC calls made across the whole run so far.
    pub rpc_calls: u64,
    /// When the chain stopped on a failure, what the endpoint actually said.
    ///
    /// Collapsing every failure to "RpcFailure" made the census honest and the
    /// run undiagnosable: a rate limit, a timeout on a large page and a
    /// malformed response are three different problems with three different
    /// fixes, and they looked identical.
    pub error: Option<String>,
}

/// One member's provenance chain, seed first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chain {
    pub seed: String,
    pub visited: Vec<String>,
    pub stop: ChainStop,
}

/// Strips credentials from an endpoint before it is written down.
///
/// The manifest is a committed artifact, and provider URLs carry API keys in the
/// path or the query string. Recording the host tells a reader which provider
/// served the run — which is what they need to judge it — without publishing a
/// key that would then have to be rotated.
pub fn redact_endpoint(url: &str) -> String {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    // Keep scheme and host, drop the path: Alchemy and Helius put the key there.
    match without_query.split_once("://") {
        Some((scheme, rest)) => {
            let host = rest.split('/').next().unwrap_or(rest);
            format!("{scheme}://{host}")
        }
        None => without_query
            .split('/')
            .next()
            .unwrap_or(without_query)
            .to_string(),
    }
}

/// What the manifest records so a reader can judge the run without rerunning it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Host only. Any credential in the path or query is stripped by
    /// [`redact_endpoint`] before this is written.
    pub endpoint: String,
    pub first_available_block: u64,
    pub archival_probe_ok: bool,
    pub collected_at: i64,
    pub config: CollectionConfig,
    pub thresholds_hub_signatures: u64,
    pub thresholds_distributor_fanout: usize,
    pub thresholds_cluster_min_size: usize,
    pub rpc_calls: u64,
    /// Seeds dropped before tracing because they are not wallets: a program, a
    /// PDA, a token account, or an address with no account at all.
    ///
    /// This is a **definitional** frame criterion, not an outcome-correlated
    /// one. A token account is not a person and cannot be a member of an
    /// anonymity set, so including it would measure something other than the
    /// pool. Excluding addresses because they looked *hard to trace* would be a
    /// different thing entirely: that criterion correlates with the outcome, so
    /// it manufactures whichever headline the exclusion implies. The count is
    /// published rather than quietly applied, so the distinction is checkable.
    pub excluded_non_wallet: Vec<String>,
    /// Share of observed credits whose source was one of several debited
    /// accounts, so the funder is a set rather than a single address.
    pub ambiguous_attribution_rate: f64,
}

/// Everything pass one produced. This is the artifact that gets committed, and
/// pass two needs nothing else.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub manifest: Manifest,
    pub chains: Vec<Chain>,
    pub facts: BTreeMap<String, AddressFacts>,
}

impl Sample {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}

/// Walks chains from each seed, recording facts along the way.
pub struct Collector<'a> {
    client: &'a mut RpcClient,
    config: CollectionConfig,
    facts: BTreeMap<String, AddressFacts>,
    /// Addresses whose observation is *complete*, meaning signatures were paged
    /// and the birth edge resolved.
    ///
    /// Distinct from "we know the owner". Frame validation learns the owner of
    /// every seed before tracing, so keying the cache on the owner alone would
    /// make the trace believe every seed was already done and skip the paging
    /// entirely — which yields a run where nothing resolves and nothing errors.
    fully_observed: std::collections::BTreeSet<String>,
    last_error: Option<String>,
    ambiguous: u64,
    edges_seen: u64,
}

impl<'a> Collector<'a> {
    pub fn new(client: &'a mut RpcClient, config: CollectionConfig) -> Self {
        Collector {
            client,
            config,
            facts: BTreeMap::new(),
            fully_observed: std::collections::BTreeSet::new(),
            last_error: None,
            ambiguous: 0,
            edges_seen: 0,
        }
    }

    /// Collects every seed and assembles the sample.
    pub fn collect(
        self,
        seeds: &[String],
        check: &EndpointCheck,
        thresholds: &Thresholds,
        now: i64,
    ) -> Sample {
        self.collect_with_progress(seeds, check, thresholds, now, |_| {})
    }

    /// Collects every seed, reporting each one as it completes.
    pub fn collect_with_progress<F: FnMut(Progress<'_>)>(
        mut self,
        seeds: &[String],
        check: &EndpointCheck,
        thresholds: &Thresholds,
        now: i64,
        mut on_progress: F,
    ) -> Sample {
        let mut chains = Vec::with_capacity(seeds.len());
        let mut excluded_non_wallet = Vec::new();
        for (index, seed) in seeds.iter().enumerate() {
            let chain = match self.is_wallet(seed) {
                Ok(true) => Some(self.trace(seed)),
                Ok(false) => {
                    excluded_non_wallet.push(seed.clone());
                    None
                }
                // An endpoint failure here is not evidence that the seed is not
                // a wallet, so it stays in the frame and fails honestly during
                // the trace.
                Err(_) => Some(self.trace(seed)),
            };
            if let Some(chain) = chain {
                on_progress(Progress {
                    done: index + 1,
                    total: seeds.len(),
                    seed,
                    hops: chain.visited.len(),
                    stop: chain.stop,
                    rpc_calls: self.client.calls_made(),
                    error: self.last_error.take(),
                });
                chains.push(chain);
            }
        }
        Sample {
            manifest: Manifest {
                endpoint: redact_endpoint(&check.endpoint),
                first_available_block: check.first_available_block,
                archival_probe_ok: check.archival_probe_ok,
                collected_at: now,
                config: self.config.clone(),
                thresholds_hub_signatures: thresholds.hub_signatures,
                thresholds_distributor_fanout: thresholds.distributor_fanout,
                thresholds_cluster_min_size: thresholds.cluster_min_size,
                rpc_calls: self.client.calls_made(),
                excluded_non_wallet,
                ambiguous_attribution_rate: if self.edges_seen == 0 {
                    0.0
                } else {
                    self.ambiguous as f64 / self.edges_seen as f64
                },
            },
            chains,
            facts: self.facts,
        }
    }

    /// Whether a seed belongs in the frame at all.
    ///
    /// A member of an anonymity set is a person's wallet: an existing account
    /// owned by the system program. A token account, a PDA, a program, or an
    /// address whose account has been closed is none of those.
    fn is_wallet(&mut self, address: &str) -> Result<bool, RpcError> {
        let owner = self.client.account_owner(address)?;
        let verdict = match &owner {
            None => false,
            Some(o) => o.owner == SYSTEM_PROGRAM && !o.executable,
        };
        // Keep what we learned; the trace would otherwise fetch it again.
        let entry = self
            .facts
            .entry(address.to_string())
            .or_insert_with(|| AddressFacts::new(address));
        entry.executable = owner.as_ref().is_some_and(|o| o.executable);
        entry.owner = owner.map(|o| o.owner);
        Ok(verdict)
    }

    fn trace(&mut self, seed: &str) -> Chain {
        let mut visited = Vec::new();
        let mut current = seed.to_string();

        for hop in 0..=self.config.depth_max {
            visited.push(current.clone());

            let stop = match self.observe(&current) {
                Err(e) => {
                    self.last_error = Some(e.to_string());
                    Some(ChainStop::RpcFailure)
                }
                Ok(local_terminal) if local_terminal => Some(ChainStop::LocalTerminal),
                Ok(_) => None,
            };
            if let Some(stop) = stop {
                return Chain {
                    seed: seed.to_string(),
                    visited,
                    stop,
                };
            }

            if hop == self.config.depth_max {
                return Chain {
                    seed: seed.to_string(),
                    visited,
                    stop: ChainStop::DepthExceeded,
                };
            }

            let facts = &self.facts[&current];
            let Some(edge) = facts.birth_edge.clone() else {
                // Three different reasons there is no edge, and only one of them
                // is a statement about the address. Ordered from ours to theirs:
                // we never reached the oldest page; we reached it but stopped
                // scanning forward before a credit appeared; or we read the
                // whole early history and there genuinely is no credit.
                let stop = if matches!(facts.signatures, SigCount::AtLeast(_)) {
                    ChainStop::PageCapHit
                } else if facts.birth_scan_exhausted {
                    ChainStop::BirthScanExhausted
                } else {
                    ChainStop::NoIncomingEdge
                };
                return Chain {
                    seed: seed.to_string(),
                    visited,
                    stop,
                };
            };
            if edge.value < self.config.min_edge_lamports {
                return Chain {
                    seed: seed.to_string(),
                    visited,
                    stop: ChainStop::BelowThreshold,
                };
            }

            // Follow the first source. With one source this is exact; with
            // several the edge is already flagged ambiguous and the rate is
            // reported, rather than the choice being hidden.
            let next = edge.sources[0].clone();
            self.note_funding(&next, &current);
            current = next;
        }

        Chain {
            seed: seed.to_string(),
            visited,
            stop: ChainStop::DepthExceeded,
        }
    }

    /// Records that `funder` created `child`, for the fan-out rule.
    fn note_funding(&mut self, funder: &str, child: &str) {
        let entry = self
            .facts
            .entry(funder.to_string())
            .or_insert_with(|| AddressFacts::new(funder));
        if !entry.funded.iter().any(|f| f == child) {
            entry.funded.push(child.to_string());
        }
    }

    /// Fills in one address's facts. Returns whether an address-local terminal
    /// rule fired, which is the only reason pass one stops early.
    fn observe(&mut self, address: &str) -> Result<bool, RpcError> {
        if self.fully_observed.contains(address) {
            // Seen completely on another chain. Re-fetching would cost calls and
            // could return a different answer as the chain advances, which would
            // make the sample internally inconsistent.
            let existing = &self.facts[address];
            return Ok(is_local_terminal(existing));
        }

        let mut facts = self
            .facts
            .remove(address)
            .unwrap_or_else(|| AddressFacts::new(address));

        // Frame validation may already have fetched this; do not pay twice.
        if facts.owner.is_none() && !facts.executable {
            let owner = self.client.account_owner(address)?;
            facts.executable = owner.as_ref().is_some_and(|o| o.executable);
            facts.owner = owner.map(|o| o.owner);
        }

        if is_local_terminal(&facts) {
            // A program or PDA terminates here; its funding history is not a
            // person's and costs calls to walk.
            self.facts.insert(address.to_string(), facts);
            self.fully_observed.insert(address.to_string());
            return Ok(true);
        }

        self.page_to_birth(address, &mut facts)?;
        self.facts.insert(address.to_string(), facts);
        self.fully_observed.insert(address.to_string());
        Ok(false)
    }

    /// Pages backwards to the oldest signature and reads the birth edge from it.
    fn page_to_birth(&mut self, address: &str, facts: &mut AddressFacts) -> Result<(), RpcError> {
        let mut before: Option<String> = None;
        let mut seen: u64 = 0;
        let mut oldest: Option<crate::rpc::SignatureInfo> = None;
        let mut last_seen_time: Option<i64> = None;
        // The oldest page, kept whole. The birth credit is looked for by walking
        // forward from its end, so the single oldest signature is not enough.
        let mut oldest_page: Vec<crate::rpc::SignatureInfo> = Vec::new();

        for page in 0..self.config.sig_page_cap {
            let batch = self.client.signatures_for_address(
                address,
                before.as_deref(),
                self.config.page_size,
            )?;
            if batch.is_empty() {
                break;
            }
            seen += batch.len() as u64;
            let last = batch.last().expect("non-empty").clone();
            before = Some(last.signature.clone());
            last_seen_time = last.block_time.or(last_seen_time);
            oldest = Some(last);
            oldest_page = batch.clone();

            if (batch.len() as u32) < self.config.page_size {
                // A short page is the end of the history: this is exact.
                facts.signatures = SigCount::Exact(seen);
                break;
            }
            if page + 1 == self.config.sig_page_cap {
                // The cap is a classification signal with a name, never a
                // silent unresolved: the count is a lower bound and says so.
                facts.signatures = SigCount::AtLeast(seen);
                // Record the age too, from the oldest signature reached. This
                // is the whole point of the volume-hub rule: an address busy
                // enough to exhaust the paging budget is exactly the kind that
                // should be classified rather than dropped.
                //
                // The value is conservative in the right direction. We stopped
                // before the true oldest signature, so the real account is at
                // least this old; an address that already looks thirty days old
                // from a partial view is genuinely at least that.
                facts.first_seen = last_seen_time;
                return Ok(());
            }
        }

        let Some(oldest) = oldest else {
            facts.signatures = SigCount::Exact(0);
            return Ok(());
        };
        if !facts.signatures.is_exact() && seen > 0 {
            facts.signatures = SigCount::Exact(seen);
        }
        facts.first_seen = oldest.block_time;

        // Walk forward from the oldest transaction until one credits the
        // address. The oldest transaction is very often not a credit — an
        // account routinely appears as a passive participant in someone else's
        // transaction before it is ever funded — and stopping there reports "no
        // incoming edge" for an address that plainly has one.
        let scanned = self.scan_for_birth_credit(address, &oldest_page, facts)?;
        facts.birth_scan_exhausted = facts.birth_edge.is_none() && scanned >= self.birth_budget();
        Ok(())
    }

    fn birth_budget(&self) -> usize {
        self.config.birth_scan_cap as usize
    }

    /// Examines up to `birth_scan_cap` transactions, oldest first, and records
    /// the first that credits `address`.
    ///
    /// Returns how many were examined, so the caller can tell "there is no
    /// credit here" from "we stopped looking". Those must never be the same
    /// outcome.
    fn scan_for_birth_credit(
        &mut self,
        address: &str,
        oldest_page: &[crate::rpc::SignatureInfo],
        facts: &mut AddressFacts,
    ) -> Result<usize, RpcError> {
        let budget = self.birth_budget();
        let mut examined = 0usize;

        // `oldest_page` is newest-first, so walking it in reverse is oldest-first.
        for info in oldest_page.iter().rev() {
            if examined >= budget {
                break;
            }
            if info.err {
                // A failed transaction moved no value. It costs nothing to skip
                // and is not worth a fetch.
                continue;
            }
            examined += 1;
            let tx = self.client.transaction(&info.signature)?;

            // Fee-payer clustering wants the payer of the address's earliest
            // transactions, so record it for every one examined rather than
            // only for the one that happens to carry the credit.
            if let Some(payer) = tx.account_keys.first() {
                if !facts.fee_payers.iter().any(|p| p == payer) {
                    facts.fee_payers.push(payer.clone());
                }
            }

            if let Some(edge) = tx.edge_crediting(address) {
                self.edges_seen += 1;
                if edge.ambiguous_attribution {
                    self.ambiguous += 1;
                }
                facts.birth_edge = Some(edge);
                return Ok(examined);
            }
        }
        Ok(examined)
    }
}

/// Address-local terminal test: a program or a PDA.
fn is_local_terminal(facts: &AddressFacts) -> bool {
    facts
        .owner
        .as_ref()
        .is_some_and(|o| o != SYSTEM_PROGRAM || facts.executable)
}

/// Pass two: the first terminal along each chain, over the complete sample.
pub fn classify_sample(
    sample: &Sample,
    anchors: &crate::classify::AnchorSet,
    thresholds: &Thresholds,
    now: i64,
) -> (
    Vec<(String, crate::outcome::Outcome)>,
    crate::outcome::Census,
) {
    use crate::{
        classify::{Classifier, SetStructure},
        outcome::{Census, Outcome},
    };

    let structure = SetStructure::build(&sample.facts, thresholds);
    let classifier = Classifier {
        anchors,
        structure: &structure,
        thresholds,
        now,
    };

    let mut results = Vec::with_capacity(sample.chains.len());
    let mut census = Census::default();

    for chain in &sample.chains {
        let mut outcome = None;
        for (depth, address) in chain.visited.iter().enumerate() {
            // The seed itself is a member, not its own origin.
            if depth == 0 {
                continue;
            }
            if let Some(facts) = sample.facts.get(address) {
                if let Some(found) = classifier.classify(facts) {
                    outcome = Some(found);
                    break;
                }
            }
        }
        let outcome = outcome.unwrap_or(Outcome::Unresolved {
            reason: chain.stop.as_unresolved(),
            depth: chain.visited.len().saturating_sub(1) as u32,
        });
        census.record(&outcome);
        results.push((chain.seed.clone(), outcome));
    }

    (results, census)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{classify::AnchorSet, edge::FundingEdge, outcome::Outcome};

    const NOW: i64 = 1_800_000_000;

    fn wallet(address: &str) -> AddressFacts {
        let mut f = AddressFacts::new(address);
        f.owner = Some(SYSTEM_PROGRAM.to_string());
        f
    }

    fn edge(sink: &str, source: &str, value: u64) -> FundingEdge {
        FundingEdge {
            sink: sink.into(),
            sources: vec![source.into()],
            value,
            signature: "sig".into(),
            slot: 1,
            block_time: Some(NOW - 1_000),
            ambiguous_attribution: false,
        }
    }

    fn sample_with(chains: Vec<Chain>, facts: Vec<AddressFacts>) -> Sample {
        Sample {
            manifest: Manifest {
                endpoint: "test".into(),
                first_available_block: 0,
                archival_probe_ok: true,
                collected_at: NOW,
                config: CollectionConfig::default(),
                thresholds_hub_signatures: 5_000,
                thresholds_distributor_fanout: 20,
                thresholds_cluster_min_size: 3,
                rpc_calls: 0,
                excluded_non_wallet: Vec::new(),
                ambiguous_attribution_rate: 0.0,
            },
            chains,
            facts: facts.into_iter().map(|f| (f.address.clone(), f)).collect(),
        }
    }

    #[test]
    fn a_chain_resolves_at_the_first_terminal_past_the_seed() {
        let mut program = wallet("Vault1");
        program.owner = Some("Whirlpoo1111111111111111111111111111111111".into());
        let sample = sample_with(
            vec![Chain {
                seed: "member".into(),
                visited: vec!["member".into(), "hop1".into(), "Vault1".into()],
                stop: ChainStop::LocalTerminal,
            }],
            vec![wallet("member"), wallet("hop1"), program],
        );
        let (results, census) =
            classify_sample(&sample, &AnchorSet::default(), &Thresholds::default(), NOW);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.label().unwrap(), "program:Vault1");
        assert_eq!(census.resolved, 1);
    }

    /// The seed is the member being measured, so it must not classify itself.
    /// Without this a member who happens to be a busy address would be its own
    /// provenance class, which is meaningless.
    #[test]
    fn a_seed_is_never_its_own_origin() {
        let mut busy_seed = wallet("member");
        busy_seed.signatures = SigCount::AtLeast(50_000);
        busy_seed.first_seen = Some(NOW - 400 * 24 * 3_600);

        let sample = sample_with(
            vec![Chain {
                seed: "member".into(),
                visited: vec!["member".into()],
                stop: ChainStop::NoIncomingEdge,
            }],
            vec![busy_seed],
        );
        let (results, _) =
            classify_sample(&sample, &AnchorSet::default(), &Thresholds::default(), NOW);
        assert!(
            matches!(
                results[0].1,
                Outcome::Unresolved {
                    reason: Unresolved::NoIncomingEdge,
                    ..
                }
            ),
            "the seed classified itself: {:?}",
            results[0].1
        );
    }

    #[test]
    fn every_chain_stop_maps_to_a_named_outcome() {
        // No stop may fall through to a default, and an RPC failure must stay
        // distinguishable from a genuine dead end.
        for (stop, expected) in [
            (ChainStop::NoIncomingEdge, Unresolved::NoIncomingEdge),
            (ChainStop::BelowThreshold, Unresolved::BelowThreshold),
            (ChainStop::DepthExceeded, Unresolved::DepthExceeded),
            (ChainStop::PageCapHit, Unresolved::PageCapHit),
            (ChainStop::RpcFailure, Unresolved::RpcFailure),
            (
                ChainStop::BirthScanExhausted,
                Unresolved::BirthScanExhausted,
            ),
        ] {
            assert_eq!(stop.as_unresolved(), expected, "{stop:?} mapped wrongly");
        }
    }

    /// Running out of birth-scan budget is **our** limit, so it must never be
    /// counted as evidence about the address, and it must never be collapsed
    /// into `NoIncomingEdge`.
    ///
    /// The distinction is the entire point: one says "this address has no
    /// funding credit", which is a claim about the chain, and the other says
    /// "we stopped reading", which is a claim about us.
    #[test]
    fn an_exhausted_birth_scan_is_budget_and_not_evidence() {
        assert!(!Unresolved::BirthScanExhausted.is_evidence());
        assert!(!Unresolved::BirthScanExhausted.is_failure());

        let mut seed = wallet("member");
        seed.birth_scan_exhausted = true;
        let sample = sample_with(
            vec![Chain {
                seed: "member".into(),
                visited: vec!["member".into()],
                stop: ChainStop::BirthScanExhausted,
            }],
            vec![seed],
        );
        let (_, census) =
            classify_sample(&sample, &AnchorSet::default(), &Thresholds::default(), NOW);
        assert_eq!(census.birth_scan_exhausted, 1);
        assert_eq!(
            census.no_incoming_edge, 0,
            "a budget outcome leaked into the evidence bucket"
        );
        assert!(
            census.summary().contains("budget-unresolved 1"),
            "must be reported as budget: {}",
            census.summary()
        );
        assert!(
            census.summary().contains("evidence-unresolved 0"),
            "must not be reported as evidence: {}",
            census.summary()
        );
    }

    #[test]
    fn an_rpc_failure_stays_a_failure_and_is_excluded_from_the_distribution() {
        let sample = sample_with(
            vec![Chain {
                seed: "member".into(),
                visited: vec!["member".into(), "hop1".into()],
                stop: ChainStop::RpcFailure,
            }],
            vec![wallet("member"), wallet("hop1")],
        );
        let (_, census) =
            classify_sample(&sample, &AnchorSet::default(), &Thresholds::default(), NOW);
        assert_eq!(census.rpc_failure, 1);
        assert_eq!(census.measurable(), 0);
        assert!(
            !census.may_publish(),
            "a run that is all failure must not publish"
        );
    }

    /// Two members funded through the same exchange land in the same class even
    /// though their chains differ in length. That is the whole mechanism.
    #[test]
    fn members_sharing_an_origin_share_a_class() {
        let sample = sample_with(
            vec![
                Chain {
                    seed: "alice".into(),
                    visited: vec!["alice".into(), "ExchangeHot".into()],
                    stop: ChainStop::LocalTerminal,
                },
                Chain {
                    seed: "bob".into(),
                    visited: vec!["bob".into(), "hop".into(), "ExchangeHot".into()],
                    stop: ChainStop::LocalTerminal,
                },
            ],
            vec![
                wallet("alice"),
                wallet("bob"),
                wallet("hop"),
                wallet("ExchangeHot"),
            ],
        );
        let anchors = AnchorSet::from_pairs([("ExchangeHot", "someexchange")]);
        let (results, _) = classify_sample(&sample, &anchors, &Thresholds::default(), NOW);
        assert_eq!(results[0].1.label(), results[1].1.label());
        assert_eq!(results[0].1.label().unwrap(), "entity:someexchange");
    }

    #[test]
    fn a_sample_round_trips_through_json() {
        let mut funded = wallet("funder");
        funded.birth_edge = Some(edge("funder", "upstream", 1_000_000));
        funded.funded = vec!["a".into(), "b".into()];
        let sample = sample_with(
            vec![Chain {
                seed: "member".into(),
                visited: vec!["member".into(), "funder".into()],
                stop: ChainStop::DepthExceeded,
            }],
            vec![wallet("member"), funded],
        );
        let json = sample.to_json().unwrap();
        let back = Sample::from_json(&json).unwrap();
        assert_eq!(
            sample, back,
            "the committed artifact must survive a round trip"
        );
    }

    /// The property the whole two-pass design exists for: analysis of a
    /// committed sample is a pure function, so anyone can recompute the headline
    /// without RPC access.
    #[test]
    fn classifying_the_same_sample_twice_gives_the_same_answer() {
        let mut hub = wallet("Hub");
        hub.signatures = SigCount::AtLeast(50_000);
        hub.first_seen = Some(NOW - 400 * 24 * 3_600);
        let sample = sample_with(
            vec![Chain {
                seed: "member".into(),
                visited: vec!["member".into(), "Hub".into()],
                stop: ChainStop::LocalTerminal,
            }],
            vec![wallet("member"), hub],
        );
        let a = classify_sample(&sample, &AnchorSet::default(), &Thresholds::default(), NOW);
        let b = classify_sample(&sample, &AnchorSet::default(), &Thresholds::default(), NOW);
        assert_eq!(a.0, b.0);
        assert_eq!(a.1, b.1);
        assert_eq!(a.0[0].1.label().unwrap(), "busy-unlabelled:Hub");
    }

    /// An address busy enough to exhaust the paging budget is exactly the kind
    /// the volume-hub rule exists for, so it must arrive at pass two carrying an
    /// age. An earlier version returned from paging before recording one, and
    /// every page-capped funder fell through to unresolved — which on a real
    /// sample was thirty chains out of thirty-seven.
    #[test]
    fn a_page_capped_funder_is_classified_rather_than_dropped() {
        let mut hub = wallet("BusyFunder");
        hub.signatures = SigCount::AtLeast(20_000);
        hub.first_seen = Some(NOW - 400 * 24 * 3_600);

        let sample = sample_with(
            vec![Chain {
                seed: "member".into(),
                visited: vec!["member".into(), "BusyFunder".into()],
                stop: ChainStop::PageCapHit,
            }],
            vec![wallet("member"), hub],
        );
        let (results, census) =
            classify_sample(&sample, &AnchorSet::default(), &Thresholds::default(), NOW);
        assert_eq!(
            results[0].1.label().unwrap(),
            "busy-unlabelled:BusyFunder",
            "a page-capped funder with an age must classify"
        );
        assert_eq!(census.resolved, 1);
        assert_eq!(census.page_cap_hit, 0);
    }

    /// Without an age it cannot classify, and that is the correct outcome — but
    /// it must land in the budget bucket rather than look like evidence.
    #[test]
    fn a_page_capped_funder_without_an_age_stays_a_budget_outcome() {
        let mut hub = wallet("BusyFunder");
        hub.signatures = SigCount::AtLeast(20_000);
        hub.first_seen = None;

        let sample = sample_with(
            vec![Chain {
                seed: "member".into(),
                visited: vec!["member".into(), "BusyFunder".into()],
                stop: ChainStop::PageCapHit,
            }],
            vec![wallet("member"), hub],
        );
        let (results, census) =
            classify_sample(&sample, &AnchorSet::default(), &Thresholds::default(), NOW);
        assert!(matches!(
            results[0].1,
            Outcome::Unresolved {
                reason: Unresolved::PageCapHit,
                ..
            }
        ));
        assert_eq!(census.page_cap_hit, 1);
        assert!(!Unresolved::PageCapHit.is_evidence());
    }

    /// The manifest is committed, so a provider key must never reach it.
    #[test]
    fn a_credential_never_reaches_the_manifest() {
        for (url, expected) in [
            (
                "https://solana-mainnet.g.alchemy.com/v2/alch_SECRETKEY123",
                "https://solana-mainnet.g.alchemy.com",
            ),
            (
                "https://mainnet.helius-rpc.com/?api-key=deadbeef-cafe",
                "https://mainnet.helius-rpc.com",
            ),
            (
                "https://api.mainnet-beta.solana.com",
                "https://api.mainnet-beta.solana.com",
            ),
        ] {
            let got = redact_endpoint(url);
            assert_eq!(got, expected);
            assert!(!got.contains("SECRETKEY"), "key survived redaction: {got}");
            assert!(!got.contains("deadbeef"), "key survived redaction: {got}");
        }
    }

    #[test]
    fn the_default_edge_threshold_admits_account_creation() {
        // Rent exemption for an empty account is about 890,880 lamports, so a
        // threshold above it would discard the very event that creates a wallet.
        let c = CollectionConfig::default();
        assert!(
            c.min_edge_lamports < 890_880,
            "the threshold would reject account creation itself"
        );
    }
}
