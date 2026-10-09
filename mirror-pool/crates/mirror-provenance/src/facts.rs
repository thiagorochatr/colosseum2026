//! What collection observed about an address.
//!
//! Pass one fills these in from the chain; pass two classifies over the complete
//! set. The split exists for determinism: two of the five terminal rules depend
//! on the *whole* sample — cluster membership and fan-out are properties of a
//! set, not of an address — so classifying during traversal would make the
//! result depend on visit order. Collect, then classify, and the same sample
//! always yields the same partition.

use crate::edge::FundingEdge;
use serde::{Deserialize, Serialize};

/// A lifetime signature count, which is often only a lower bound.
///
/// Paging stops at a cap. When it is hit, the true count is unknown and larger
/// than what we saw, so the distinction is kept in the type rather than
/// flattened into a number that would later be read as exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SigCount {
    Exact(u64),
    /// Paging hit the cap. The real count is at least this.
    AtLeast(u64),
}

impl SigCount {
    /// The count as a lower bound, which is all either variant guarantees.
    pub fn lower_bound(&self) -> u64 {
        match self {
            SigCount::Exact(n) | SigCount::AtLeast(n) => *n,
        }
    }

    pub fn is_exact(&self) -> bool {
        matches!(self, SigCount::Exact(_))
    }
}

/// Everything pass one recorded about one address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressFacts {
    pub address: String,
    /// Account owner. `None` means the account does not exist on chain, which
    /// is different from an RPC failure and is recorded as such.
    pub owner: Option<String>,
    pub executable: bool,
    pub signatures: SigCount,
    /// Block time of the oldest signature seen, for the age test.
    pub first_seen: Option<i64>,
    /// The oldest incoming value credit: who created this account.
    pub birth_edge: Option<FundingEdge>,
    /// Fee payers observed across this address's transactions, for clustering.
    pub fee_payers: Vec<String>,
    /// Addresses this one was the sole source of a birth edge for. Fan-out.
    pub funded: Vec<String>,
    /// The birth-credit scan ran out of budget before finding a credit.
    ///
    /// Distinguishes "we stopped looking" from "there is nothing there". Only
    /// meaningful when `birth_edge` is `None`, and when it is set the address is
    /// a **budget** outcome rather than evidence about the chain.
    #[serde(default)]
    pub birth_scan_exhausted: bool,
}

impl AddressFacts {
    pub fn new(address: impl Into<String>) -> Self {
        AddressFacts {
            address: address.into(),
            owner: None,
            executable: false,
            signatures: SigCount::Exact(0),
            first_seen: None,
            birth_edge: None,
            fee_payers: Vec::new(),
            funded: Vec::new(),
            birth_scan_exhausted: false,
        }
    }

    /// Age in seconds at `now`, from the oldest signature we saw.
    ///
    /// Clamped at zero. `saturating_sub` on a signed integer saturates at
    /// `i64::MIN`, not at nothing, so a timestamp in the future — clock skew, or
    /// bad data — would otherwise yield a large negative age. Zero is the
    /// fail-safe reading: it means "brand new", which fails the hub age test
    /// rather than passing it by accident.
    pub fn age_seconds(&self, now: i64) -> Option<i64> {
        self.first_seen.map(|t| (now - t).max(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capped_count_is_a_lower_bound_and_says_so() {
        let capped = SigCount::AtLeast(20_000);
        assert_eq!(capped.lower_bound(), 20_000);
        assert!(!capped.is_exact());

        let exact = SigCount::Exact(37);
        assert_eq!(exact.lower_bound(), 37);
        assert!(exact.is_exact());
    }

    #[test]
    fn age_is_measured_from_the_oldest_signature() {
        let mut f = AddressFacts::new("addr");
        assert_eq!(f.age_seconds(1_000), None);
        f.first_seen = Some(400);
        assert_eq!(f.age_seconds(1_000), Some(600));
    }

    #[test]
    fn a_future_timestamp_does_not_produce_a_negative_age() {
        let mut f = AddressFacts::new("addr");
        f.first_seen = Some(2_000);
        assert_eq!(f.age_seconds(1_000), Some(0));
    }
}
