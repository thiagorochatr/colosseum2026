//! Funding-provenance measurement over real on-chain data.
//!
//! An anonymity set on a public ledger can be partitioned by where each member's
//! capital came from, and once it is, what survives is the size of the actor's
//! class rather than the advertised `k`. This crate measures that.
//!
//! It is a measurement crate. It makes no privacy claims by itself, and every
//! number it produces names the data it came from. Method, adversary model and
//! the limits of what the numbers prove are in `docs/PROVENANCE_METHOD.md`.
#![forbid(unsafe_code)]

pub mod bootstrap;
pub mod classify;
pub mod edge;
pub mod facts;
pub mod frame;
pub mod metrics;
pub mod outcome;
pub mod published;
pub mod rpc;
pub mod trace;

pub use bootstrap::{difference_interval, loss_factor_interval, Interval};
pub use classify::{AnchorSet, Classifier, SetStructure, Thresholds};
pub use edge::{FundingEdge, TransactionView};
pub use facts::{AddressFacts, SigCount};
pub use frame::depositor_of;
pub use metrics::{Anonymity, Bracket, Quotation};
pub use outcome::{Census, Outcome, TerminalRule, Unresolved};
pub use published::{PublishedHeadline, PUBLISHED_HEADLINE};
pub use rpc::{RpcClient, RpcError};
pub use trace::{classify_sample, Chain, CollectionConfig, Collector, Sample, Scope};
