//! Funding-edge extraction from a transaction's balance deltas.
//!
//! We diff `preBalances` against `postBalances` rather than parsing
//! instructions. Parsing `program == "system" && type == "transfer"` misses
//! every program that moves lamports by direct account mutation and emits no
//! transfer instruction at all, plus `createAccount`,
//! `createAccountWithSeed`, `transferWithSeed`, `withdrawNonceAccount`, and
//! `closeAccount` lamport returns. A tracer built on instruction parsing is
//! blind to all of it and reports the resulting gaps as "unresolved".
//!
//! Balance deltas see value movement regardless of how it was caused, which is
//! the property we actually want.
//!
//! ```text
//! Δ_i = postBalances[i] − preBalances[i] + (fee if i == 0)
//! sources = { i : Δ_i < 0 }      sinks = { i : Δ_i > 0 }
//! ```
//!
//! The fee is added back for the payer so that paying for a transaction is not
//! mistaken for sending value.

use serde::{Deserialize, Serialize};

/// A value movement observed in one transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FundingEdge {
    /// The account credited.
    pub sink: String,
    /// Accounts debited. More than one means attribution within the transaction
    /// is ambiguous.
    pub sources: Vec<String>,
    /// Lamports credited to the sink.
    pub value: u64,
    pub signature: String,
    pub slot: u64,
    pub block_time: Option<i64>,
    /// Set when several accounts were debited, so the true source of this
    /// credit is one of a set rather than known. The rate of this is reported.
    pub ambiguous_attribution: bool,
}

/// The parts of a transaction we need. Deliberately a narrow projection: the
/// committed sample stores this rather than whole transactions, which keeps the
/// artifact small enough to publish and makes what we relied on explicit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionView {
    pub signature: String,
    pub slot: u64,
    pub block_time: Option<i64>,
    /// The **fully resolved** key list. Under `jsonParsed`, lookup-table
    /// addresses are folded into `accountKeys` and `preBalances`/`postBalances`
    /// align with it, so no separate `loadedAddresses` handling is needed. This
    /// is verified empirically; the RPC reference text implies otherwise.
    pub account_keys: Vec<String>,
    pub pre_balances: Vec<u64>,
    pub post_balances: Vec<u64>,
    pub fee: u64,
}

impl TransactionView {
    /// Whether the balance arrays align with the resolved key list.
    ///
    /// A misalignment means we would attribute a delta to the wrong account, so
    /// it is checked rather than assumed — this is exactly where a v0
    /// lookup-table transaction would silently poison the graph.
    pub fn is_well_formed(&self) -> bool {
        !self.account_keys.is_empty()
            && self.pre_balances.len() == self.account_keys.len()
            && self.post_balances.len() == self.account_keys.len()
    }

    /// Signed lamport change per account, with the payer's fee added back.
    pub fn deltas(&self) -> Option<Vec<i128>> {
        if !self.is_well_formed() {
            return None;
        }
        Some(
            self.account_keys
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let d = self.post_balances[i] as i128 - self.pre_balances[i] as i128;
                    if i == 0 {
                        d + self.fee as i128
                    } else {
                        d
                    }
                })
                .collect(),
        )
    }

    /// The edge crediting `address`, if this transaction credits it.
    pub fn edge_crediting(&self, address: &str) -> Option<FundingEdge> {
        let deltas = self.deltas()?;
        let index = self.account_keys.iter().position(|k| k == address)?;
        let credited = deltas[index];
        if credited <= 0 {
            return None;
        }

        let sources: Vec<String> = deltas
            .iter()
            .enumerate()
            .filter(|(_, d)| **d < 0)
            .map(|(i, _)| self.account_keys[i].clone())
            .collect();
        if sources.is_empty() {
            return None;
        }

        Some(FundingEdge {
            sink: address.to_string(),
            ambiguous_attribution: sources.len() > 1,
            sources,
            value: credited as u64,
            signature: self.signature.clone(),
            slot: self.slot,
            block_time: self.block_time,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(keys: &[&str], pre: &[u64], post: &[u64], fee: u64) -> TransactionView {
        TransactionView {
            signature: "sig".into(),
            slot: 100,
            block_time: Some(1_700_000_000),
            account_keys: keys.iter().map(|s| s.to_string()).collect(),
            pre_balances: pre.to_vec(),
            post_balances: post.to_vec(),
            fee,
        }
    }

    #[test]
    fn a_simple_transfer_yields_one_source_and_one_sink() {
        // Alice pays the fee and sends 1000 to Bob.
        let t = tx(&["alice", "bob"], &[10_000, 0], &[8_995, 1_000], 5);
        let edge = t.edge_crediting("bob").expect("bob was credited");
        assert_eq!(edge.value, 1_000);
        assert_eq!(edge.sources, vec!["alice"]);
        assert!(!edge.ambiguous_attribution);
    }

    /// The fee must not make the payer look like a sender when it isn't one,
    /// and must not hide a genuine send.
    #[test]
    fn the_payers_fee_is_added_back() {
        // Alice pays a fee and moves nothing. Her delta is exactly zero, so she
        // is neither a source nor a sink.
        let t = tx(&["alice", "bob"], &[10_000, 500], &[9_995, 500], 5);
        assert_eq!(t.deltas().unwrap()[0], 0);
        assert!(t.edge_crediting("bob").is_none(), "bob gained nothing");
    }

    #[test]
    fn a_program_moving_lamports_directly_is_still_seen() {
        // No system-transfer instruction exists here — a program mutated
        // lamports. Instruction parsing would find nothing; the delta is plain.
        let t = tx(
            &["payer", "vault", "recipient"],
            &[1_000, 50_000, 0],
            &[995, 40_000, 10_000],
            5,
        );
        let edge = t.edge_crediting("recipient").unwrap();
        assert_eq!(edge.value, 10_000);
        assert_eq!(edge.sources, vec!["vault"]);
    }

    #[test]
    fn several_debited_accounts_mark_the_attribution_ambiguous() {
        let t = tx(
            &["payer", "second", "recipient"],
            &[10_000, 10_000, 0],
            &[4_995, 5_000, 10_000],
            5,
        );
        let edge = t.edge_crediting("recipient").unwrap();
        assert!(
            edge.ambiguous_attribution,
            "two sources means the true funder is one of a set, not known"
        );
        assert_eq!(edge.sources.len(), 2);
    }

    #[test]
    fn an_account_that_lost_value_is_not_credited() {
        let t = tx(&["alice", "bob"], &[10_000, 5_000], &[10_995, 4_000], 5);
        assert!(t.edge_crediting("bob").is_none());
        assert!(t.edge_crediting("alice").is_some());
    }

    #[test]
    fn an_address_absent_from_the_key_list_yields_nothing() {
        let t = tx(&["alice", "bob"], &[10_000, 0], &[8_995, 1_000], 5);
        assert!(t.edge_crediting("carol").is_none());
    }

    /// Misaligned balance arrays are refused rather than truncated. This is the
    /// shape a mishandled v0 lookup-table transaction takes, and silently
    /// zipping the shorter array would attribute deltas to the wrong accounts.
    #[test]
    fn misaligned_balances_are_refused() {
        let mut t = tx(&["a", "b", "c"], &[1, 2, 3], &[1, 2, 3], 0);
        t.post_balances.pop();
        assert!(!t.is_well_formed());
        assert!(t.deltas().is_none());
        assert!(t.edge_crediting("b").is_none());
    }

    #[test]
    fn a_lookup_table_transaction_is_handled_when_keys_are_fully_resolved() {
        // Twenty resolved keys, eleven from the transaction and nine from a
        // lookup table, with balances aligned to the full list.
        let keys: Vec<String> = (0..20).map(|i| format!("key{i}")).collect();
        let mut pre = vec![1_000u64; 20];
        let mut post = vec![1_000u64; 20];
        pre[0] = 100_000;
        post[0] = 89_995;
        post[17] = 11_000;
        let t = TransactionView {
            signature: "v0sig".into(),
            slot: 1,
            block_time: None,
            account_keys: keys,
            pre_balances: pre,
            post_balances: post,
            fee: 5,
        };
        assert!(t.is_well_formed());
        let edge = t
            .edge_crediting("key17")
            .expect("a lookup-table account was credited");
        assert_eq!(edge.value, 10_000);
        assert_eq!(edge.sources, vec!["key0"]);
    }

    #[test]
    fn zero_value_credits_are_not_edges() {
        let t = tx(&["a", "b"], &[10_000, 500], &[9_995, 500], 5);
        assert!(t.edge_crediting("b").is_none());
    }
}
