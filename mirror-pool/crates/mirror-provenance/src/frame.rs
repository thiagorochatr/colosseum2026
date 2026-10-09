//! Building a member-weighted frame from a pool's depositors.
//!
//! The frame decides what the measurement is *of*, and getting it wrong is the
//! easiest way to produce a confident number about the wrong population.
//!
//! **Member-weighted, not transaction-weighted.** Each depositor appears once
//! however often they transact. Sampling addresses because they appear in recent
//! blocks is size-biased: an address that transacts a thousand times a day is a
//! thousand times likelier to be drawn than one that transacts daily, so the
//! frame fills with market makers and bots. Our own first run did exactly this
//! and produced nothing; `docs/MEASUREMENT_LOG.md` records it.
//!
//! **Spread across the pool's history, not its last minute.** Pages are walked
//! backwards through the whole signature history and depositors are taken from
//! every page. A published measurement in this space draws its entire sample
//! from a nine-slot window — about four seconds of chain time — and reports
//! Wilson intervals over it as though it were an independent sample of a
//! population.
//!
//! **Nothing is excluded for being hard to trace.** Busy depositors stay in.
//! Dropping them inflates the resolved fraction by construction, and whatever
//! the remaining sample then reports is a property of the exclusion rule rather
//! than a finding about the pool.

use crate::edge::TransactionView;

/// The address that funded a deposit, if this transaction looks like one.
///
/// A deposit debits the payer and credits the pool. We take the fee payer, whose
/// index is zero, and require that they lost at least `min_lamports` beyond the
/// fee — so a withdrawal, in which the payer is a relayer moving nothing of
/// their own, is not mistaken for a deposit.
pub fn depositor_of(tx: &TransactionView, min_lamports: u64) -> Option<String> {
    let deltas = tx.deltas()?;
    let payer_delta = *deltas.first()?;
    if payer_delta >= 0 {
        return None;
    }
    if payer_delta.unsigned_abs() < min_lamports as u128 {
        return None;
    }
    // Something else in the transaction must have gained: a payer who only
    // burned lamports has not deposited anywhere.
    if !deltas.iter().skip(1).any(|d| *d > 0) {
        return None;
    }
    tx.account_keys.first().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(keys: &[&str], pre: &[u64], post: &[u64], fee: u64) -> TransactionView {
        TransactionView {
            signature: "sig".into(),
            slot: 1,
            block_time: Some(1_700_000_000),
            account_keys: keys.iter().map(|s| s.to_string()).collect(),
            pre_balances: pre.to_vec(),
            post_balances: post.to_vec(),
            fee,
        }
    }

    const MIN: u64 = 10_000_000;

    #[test]
    fn a_deposit_names_its_payer() {
        // The payer sends 1 SOL into the pool vault.
        let t = tx(
            &["depositor", "pool_vault"],
            &[2_000_000_000, 5_000_000_000],
            &[999_995_000, 6_000_000_000],
            5_000,
        );
        assert_eq!(depositor_of(&t, MIN).as_deref(), Some("depositor"));
    }

    #[test]
    fn a_withdrawal_is_not_a_deposit() {
        // A relayer pays the fee; the pool pays a recipient. The relayer moved
        // nothing of their own and must not enter the frame as a member.
        let t = tx(
            &["relayer", "pool_vault", "recipient"],
            &[1_000_000_000, 5_000_000_000, 0],
            &[999_995_000, 4_000_000_000, 1_000_000_000],
            5_000,
        );
        assert!(depositor_of(&t, MIN).is_none());
    }

    #[test]
    fn a_payer_who_only_burned_the_fee_is_not_a_depositor() {
        let t = tx(
            &["payer", "other"],
            &[1_000_000_000, 7],
            &[999_995_000, 7],
            5_000,
        );
        assert!(depositor_of(&t, MIN).is_none());
    }

    #[test]
    fn a_transfer_below_the_threshold_is_not_a_deposit() {
        let t = tx(
            &["payer", "sink"],
            &[1_000_000_000, 0],
            &[999_994_000, 1_000],
            5_000,
        );
        assert!(depositor_of(&t, MIN).is_none());
    }

    #[test]
    fn a_malformed_transaction_yields_nothing_rather_than_a_guess() {
        let mut t = tx(&["a", "b"], &[1, 2], &[1, 2], 0);
        t.post_balances.pop();
        assert!(depositor_of(&t, MIN).is_none());
    }
}
