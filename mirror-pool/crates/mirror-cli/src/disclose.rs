//! Voluntary disclosure: proving to one chosen verifier that a settled action
//! was yours.
//!
//! Everything about *what happened* is already public. A spend record PDA is
//! derived from the nullifier and holds, in cleartext, the beneficiary, the
//! selector, the target program, the relay fee, the payload and whether it
//! settled. The pool hides exactly one thing: **which member asked for it.**
//!
//! So a disclosure is not an audit trail, an escrow, or a key anyone else holds.
//! It is a member handing a file to a counterparty of their own choosing — an
//! exchange, a tax authority, a court, a friend — that lets that counterparty
//! recompute the link for themselves. There is no on-chain component, no
//! auditor key, and no path by which the protocol can be made to produce one.
//! That is deliberate: a compel path that exists is a compel path that can be
//! used, and a pool with one is a pool whose members are anonymous only until
//! somebody with standing asks.
//!
//! The verifier is not asked to trust the file. Every field in it is either
//! recomputed from the secrets or read off the cluster; a disclosure whose
//! stated nullifier disagrees with `H1(k)` fails rather than being believed.
//! See [`verify`].
//!
//! ## Why this is safe after settlement and catastrophic before
//!
//! `(k, r)` is the whole authority over a note. Before the note is spent, the
//! secrets *are* the money: anyone holding them can produce the membership
//! proof and direct the payout wherever they like. Disclosing an unspent note
//! is therefore not a disclosure at all — it is handing over the deposit, and
//! the recipient can spend it before the discloser finishes explaining what the
//! file was for.
//!
//! Once the nullifier is burnt and the record is settled, `(k, r)` authorises
//! nothing. The replay guard is the *existence* of the spend record, so a second
//! spend of the same note fails at account creation and never reaches the
//! verifier. What is left in the secrets is only the ability to demonstrate the
//! link — which is the thing being disclosed on purpose.
//!
//! [`build`] refuses to write a disclosure for a note that is not settled. That
//! is not a policy check that a determined member can be talked out of; it is
//! the one mistake in this module that costs somebody their deposit.

use crate::chain::Chain;
use crate::client;
use crate::history;
use crate::note::StoredNote;
use anyhow::{anyhow, Context, Result};
use mirror_core::{Field, MerkleTree, Note};
use mirror_pool_program::{
    pda::{pool_address, spend_address},
    spend::{Spend, STATUS_SETTLED},
};
use serde::{Deserialize, Serialize};
use solana_program::pubkey::Pubkey;
use std::path::Path;

/// The flag that constructs a [`DisclosureOverride`], named in one place so the
/// refusal message can tell a reader exactly what to type.
pub const ACKNOWLEDGE_FLAG: &str = "--i-accept-the-cost-to-others";

// ---------------------------------------------------------------------------
// The file

/// A member's claim that one settled action was theirs, in the form a verifier
/// can check.
///
/// Plain JSON with base58 pubkeys and hex byte strings, for the same reason the
/// note file is: it is meant to be read by the person receiving it, not only
/// parsed by this binary. A verifier who does not trust this tool should be able
/// to see what is being claimed and check it with their own code.
///
/// It carries what a verifier needs and nothing that helps a third party. Every
/// field here is either already public on chain or is a secret that settlement
/// has already spent.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Disclosure {
    /// The pool program, base58.
    pub program: String,
    /// The pool account, base58. Derived from the program and the denomination,
    /// and [`verify`] rederives it rather than trusting this field.
    pub pool: String,
    pub denomination: u64,
    /// Where the note sits in the accumulator, counting from the pool's first
    /// deposit.
    pub leaf_index: u64,
    /// `H3(k, r, denom_tag)`, 32 bytes big-endian hex.
    pub commitment: String,
    /// Nullifier preimage, 32 bytes big-endian hex.
    ///
    /// This is the secret that authorised the spend, and it is published here
    /// **only because the spend already happened**. Its nullifier is burnt, the
    /// spend record exists, and the record's existence is what refuses a second
    /// spend — so the secret no longer moves any money. The same two lines in a
    /// file written before settlement would hand the reader the deposit.
    pub k: String,
    /// Blinding factor, 32 bytes big-endian hex. Safe for exactly the same
    /// reason `k` is, and worthless without it.
    pub r: String,
    /// `H1(k)`, 32 bytes big-endian hex. Already public — it was an argument to
    /// the spend instruction — and repeated here so a reader can see the link
    /// being claimed without running anything.
    pub nullifier: String,
    /// The spend record account, base58.
    pub spend_record: String,
    /// What the member is claiming they asked the pool to do.
    pub action: ClaimedAction,
}

/// The action a disclosure claims, field for field as the record holds it.
///
/// Restated here rather than left implicit in "go read the record" because the
/// claim is the point: a member is asserting *this* payout was theirs, and a
/// verifier compares the assertion against the chain rather than reading the
/// chain and being told what it means.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ClaimedAction {
    /// Who received the outcome, base58.
    pub beneficiary: String,
    /// Which kind of action the pool executed.
    pub selector: u64,
    /// The program the pool invoked, base58. All zeroes for a plain transfer.
    pub target_program: String,
    /// Taken out of the denomination, never added to it.
    pub relay_fee: u64,
    /// Instruction data for the invocation, hex. Empty for a plain transfer.
    pub payload: String,
}

impl Disclosure {
    pub fn read(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading the disclosure at {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Writes the disclosure, overwriting if one is already there.
    ///
    /// Unlike a note file, this is regenerable from the note at any time, so
    /// refusing to overwrite would protect nothing and would strand a member who
    /// ran the command twice.
    pub fn write(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text)
            .with_context(|| format!("writing the disclosure to {}", path.display()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// What the chain says

/// The spend record as the chain holds it, reduced to the fields a disclosure
/// claims.
#[derive(Clone, Debug)]
pub struct SpendRecordView {
    /// `STATUS_SETTLED`, not merely present. A pending record means the action
    /// has not executed, and a member who discloses one is disclosing a note
    /// whose secrets still authorise a payout.
    pub settled: bool,
    pub pool: [u8; 32],
    pub beneficiary: [u8; 32],
    pub selector: u64,
    pub target_program: [u8; 32],
    pub relay_fee: u64,
    pub payload: Vec<u8>,
}

/// Everything [`verify`] needs from the cluster, gathered in one place.
///
/// Split out from the RPC so the verification logic is a pure function of a
/// value a test can construct. That is not a testing convenience: it is what
/// makes the tamper matrix below exhaustive, because every one of these fields
/// is something an attacker controls if they control the endpoint, and each one
/// must be shown to break exactly one check.
#[derive(Clone, Debug)]
pub struct ChainView {
    /// The denomination the pool account reports, which is not necessarily the
    /// one the disclosure claims.
    pub denomination: u64,
    pub k_floor: u32,
    /// The pool's current accumulator root.
    pub root: [u8; 32],
    /// Spends the pool counts as settled. Incremented at settlement, not at
    /// submission, so it is the size of the set a disclosure shrinks.
    pub settled_spends: u64,
    /// Commitments in insertion order, recovered from deposit instructions.
    pub leaves: Vec<[u8; 32]>,
    /// `None` when no account exists at the disclosed address.
    pub record: Option<SpendRecordView>,
}

/// Reads the cluster into a [`ChainView`].
///
/// Deliberately thin, and deliberately the only part of this module that talks
/// to a network. Anything it cannot establish is an error rather than a
/// [`ChainView`] with a plausible-looking hole in it: a rebuilt accumulator that
/// is missing leaves would fail the root check and read to a member like the
/// discloser's fault rather than the endpoint's.
pub fn observe(
    chain: &Chain,
    program: &Pubkey,
    denomination: u64,
    record: &Pubkey,
    verbose: bool,
) -> Result<ChainView> {
    let state = client::read_pool(chain, program, denomination)?;
    let history = history::scan(chain, program, &state.pool, verbose)?;
    if history.commitments.len() as u64 != state.deposits {
        return Err(anyhow!(
            "recovered {} leaves but the pool has inserted {}. The history is \
             incomplete — most likely the endpoint has pruned it — and a \
             disclosure cannot be checked against a partial accumulator. \
             `mirror check-endpoint` tests for exactly that.",
            history.commitments.len(),
            state.deposits
        ));
    }
    let record = match chain.account_data(record)? {
        None => None,
        Some(mut data) => {
            let spend = Spend::load(&mut data)
                .map_err(|e| anyhow!("{record} exists but is not a spend record: {e:?}"))?;
            Some(SpendRecordView {
                settled: spend.status() == STATUS_SETTLED,
                pool: spend.pool(),
                beneficiary: spend.beneficiary(),
                selector: spend.selector(),
                target_program: spend.target_program(),
                relay_fee: spend.relay_fee(),
                payload: spend.payload().to_vec(),
            })
        }
    };
    Ok(ChainView {
        denomination: state.denomination,
        k_floor: state.k_floor,
        root: state.root,
        settled_spends: state.spends,
        leaves: history.commitments,
        record,
    })
}

// ---------------------------------------------------------------------------
// The co-participant gate

/// What disclosing costs the members who did not.
///
/// A pool's privacy is joint property. Every settled action is a candidate for
/// every member, and a member who names one of them as theirs removes it from
/// everyone else's cover: the remaining actions are now shared among a smaller
/// set. The discloser pays nothing for this — they have already chosen — and the
/// people who pay are not in the room.
#[derive(Clone, Copy, Debug)]
pub struct CoParticipantCost {
    pub settled_spends: u64,
    pub k_floor: u32,
    /// Settled actions still unattributed once this one is named.
    pub remaining: u64,
}

impl CoParticipantCost {
    pub fn new(settled_spends: u64, k_floor: u32) -> Self {
        CoParticipantCost {
            settled_spends,
            k_floor,
            // Saturating rather than checked: a pool with no settled spend has
            // nothing to disclose, and [`build`] refuses it earlier for the
            // stronger reason that the note is unspent.
            remaining: settled_spends.saturating_sub(1),
        }
    }

    /// Whether what is left after this disclosure is still a crowd by the
    /// pool's own definition.
    pub fn leaves_a_crowd(&self) -> bool {
        self.remaining >= self.k_floor as u64
    }
}

/// Deliberate consent to disclose below the pool's floor.
///
/// The gate is **advisory and cannot be otherwise**. The secrets are the
/// member's; they can publish them in a text message, and no code here or on
/// chain can stop that. What this type buys is that the cost is visible at the
/// moment it is paid, to the person paying it on someone else's behalf, rather
/// than discovered later by the members who lost cover.
///
/// It is a constructed type instead of a `bool` parameter for the same reason
/// the staker and the withdrawer are separate arguments elsewhere in this crate:
/// a bare `true` is one keystroke and one misread signature away from being
/// passed by a caller who never considered the question. Building this requires
/// having computed the cost first.
#[derive(Clone, Copy, Debug)]
pub struct DisclosureOverride {
    acknowledged: CoParticipantCost,
}

impl DisclosureOverride {
    /// Consents to the cost described, which the caller must have computed.
    pub fn acknowledging(cost: CoParticipantCost) -> Self {
        DisclosureOverride { acknowledged: cost }
    }

    /// The cost that was consented to, for the record this prints.
    pub fn cost(&self) -> CoParticipantCost {
        self.acknowledged
    }
}

// ---------------------------------------------------------------------------
// Building one

/// Assembles a disclosure for a note this pool has already settled.
///
/// Every field is derived rather than accepted: the leaf index is found by
/// searching the recovered leaf set for the recomputed commitment, the record
/// address is the PDA of the recomputed nullifier, and the action is copied out
/// of the record the chain holds rather than out of anything the member typed.
/// A disclosure this tool would refuse to verify is never written.
pub fn build(
    program: &Pubkey,
    pool: &Pubkey,
    stored: &StoredNote,
    view: &ChainView,
    consent: Option<&DisclosureOverride>,
) -> Result<Disclosure> {
    let note = stored.note()?;
    if view.denomination != stored.denomination {
        return Err(anyhow!(
            "this note is for denomination {} and the pool at {pool} holds {}",
            stored.denomination,
            view.denomination
        ));
    }
    let commitment = note.commitment().map_err(|e| anyhow!("{e:?}"))?.to_bytes();
    let leaf_index = view
        .leaves
        .iter()
        .position(|leaf| leaf == &commitment)
        .ok_or_else(|| {
            anyhow!(
                "this note is not a leaf of the pool at {pool}. Either it was \
                 never deposited, or it belongs to a different denomination."
            )
        })? as u64;
    let nullifier = note.nullifier().map_err(|e| anyhow!("{e:?}"))?.to_bytes();
    let (record_address, _) = spend_address(program, pool, &nullifier);

    // The refusal that protects the discloser rather than their co-members. An
    // unspent note's secrets still authorise a payout, so a "disclosure" of one
    // is a transfer of the deposit to whoever reads the file.
    let record = view.record.as_ref().ok_or_else(|| {
        anyhow!(
            "there is no spend record at {record_address}, so this note has not \
             been spent.\n\n\
             Its secrets still authorise a payout: anyone holding them can prove \
             membership and direct the money wherever they like. Disclosing an \
             unspent note does not prove what you did with it — it gives it away.\n\n\
             Spend the note first. Once it has settled, k and r authorise nothing \
             and this file proves only what it says."
        )
    })?;
    if !record.settled {
        return Err(anyhow!(
            "the spend record at {record_address} exists but has not settled.\n\n\
             The action has not executed yet, so there is nothing to prove was \
             yours, and the note's secrets are still the only thing standing \
             between the deposit and whoever holds them. Wait for settlement — \
             `mirror settle` runs it, and so does anyone else."
        ));
    }
    if record.pool != pool.to_bytes() {
        return Err(anyhow!(
            "the record at {record_address} belongs to pool {}, not {pool}. \
             A disclosure naming this pool would not verify.",
            Pubkey::new_from_array(record.pool)
        ));
    }

    let cost = CoParticipantCost::new(view.settled_spends, view.k_floor);
    if !cost.leaves_a_crowd() && consent.is_none() {
        return Err(anyhow!(
            "REFUSING: this pool has settled {} spend(s) and its floor is {}. \
             Naming one of them as yours leaves {} unattributed, which is below \
             the floor.\n\n\
             The people that costs are not you. Every settled action is a \
             candidate for every member; removing one narrows the guess for all \
             the rest, and they did not agree to it and will not be told.\n\n\
             If you have weighed that and still want to, pass {ACKNOWLEDGE_FLAG}. \
             Nothing here can stop you disclosing out-of-band anyway — the secrets \
             are yours. This exists so the cost is visible at the moment it is \
             paid.",
            cost.settled_spends,
            cost.k_floor,
            cost.remaining
        ));
    }

    Ok(Disclosure {
        program: program.to_string(),
        pool: pool.to_string(),
        denomination: stored.denomination,
        leaf_index,
        commitment: hex::encode(commitment),
        k: hex::encode(note.k.to_bytes()),
        r: hex::encode(note.r.to_bytes()),
        nullifier: hex::encode(nullifier),
        spend_record: record_address.to_string(),
        action: ClaimedAction {
            beneficiary: Pubkey::new_from_array(record.beneficiary).to_string(),
            selector: record.selector,
            target_program: Pubkey::new_from_array(record.target_program).to_string(),
            relay_fee: record.relay_fee,
            payload: hex::encode(&record.payload),
        },
    })
}

// ---------------------------------------------------------------------------
// Checking one

/// One independently reported step of a verification.
///
/// Separate variants rather than one boolean because "this disclosure is false"
/// is not a useful thing to hand somebody. A verifier needs to know whether the
/// secrets are wrong, the leaf set is incomplete, or the record says something
/// else — those are different conversations with different people.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Check {
    /// `H3(k, r, denom_tag)` equals the stated commitment.
    SecretsRecomputeToTheCommitment,
    /// The stated pool is the PDA for the stated denomination, and the account
    /// there reports that denomination.
    ThePoolMatchesTheDenomination,
    /// The recomputed commitment sits at the stated leaf index.
    TheCommitmentIsTheStatedLeaf,
    /// The accumulator rebuilt from history has the pool's on-chain root.
    TheRebuiltRootMatchesTheChain,
    /// `H1(k)` equals the stated nullifier.
    SecretsRecomputeToTheNullifier,
    /// The stated record address is the PDA of the *recomputed* nullifier.
    TheRecordAddressIsTheNullifiersPda,
    /// An account exists at that address.
    TheRecordExistsOnChain,
    /// It settled, rather than merely being submitted.
    TheRecordIsSettled,
    /// Its `pool` field is this pool.
    TheRecordBelongsToThisPool,
    /// Its beneficiary, selector, target, fee and payload are what is claimed.
    TheRecordMatchesTheClaimedAction,
}

impl Check {
    /// Every check, in the order [`verify`] reports them.
    pub const ALL: [Check; 10] = [
        Check::SecretsRecomputeToTheCommitment,
        Check::ThePoolMatchesTheDenomination,
        Check::TheCommitmentIsTheStatedLeaf,
        Check::TheRebuiltRootMatchesTheChain,
        Check::SecretsRecomputeToTheNullifier,
        Check::TheRecordAddressIsTheNullifiersPda,
        Check::TheRecordExistsOnChain,
        Check::TheRecordIsSettled,
        Check::TheRecordBelongsToThisPool,
        Check::TheRecordMatchesTheClaimedAction,
    ];

    /// A line a verifier can read without knowing this codebase.
    pub fn name(&self) -> &'static str {
        match self {
            Check::SecretsRecomputeToTheCommitment => "the secrets recompute to the commitment",
            Check::ThePoolMatchesTheDenomination => "the pool matches the denomination",
            Check::TheCommitmentIsTheStatedLeaf => "the commitment is the stated leaf",
            Check::TheRebuiltRootMatchesTheChain => "the rebuilt root matches the chain",
            Check::SecretsRecomputeToTheNullifier => "the secrets recompute to the nullifier",
            Check::TheRecordAddressIsTheNullifiersPda => {
                "the record address is the nullifier's PDA"
            }
            Check::TheRecordExistsOnChain => "the spend record exists on chain",
            Check::TheRecordIsSettled => "the spend record is settled",
            Check::TheRecordBelongsToThisPool => "the spend record belongs to this pool",
            Check::TheRecordMatchesTheClaimedAction => "the record matches the claimed action",
        }
    }
}

/// One check's verdict, with what was compared.
#[derive(Debug)]
pub struct Verdict {
    pub check: Check,
    pub passed: bool,
    /// What the check saw, phrased so a failure tells the reader what to do
    /// about it rather than only that something is wrong.
    pub detail: String,
}

/// Every check and its verdict.
#[derive(Debug, Default)]
pub struct Verification {
    verdicts: Vec<Verdict>,
}

impl Verification {
    fn record(&mut self, check: Check, outcome: Result<String, String>) {
        let (passed, detail) = match outcome {
            Ok(detail) => (true, detail),
            Err(detail) => (false, detail),
        };
        self.verdicts.push(Verdict {
            check,
            passed,
            detail,
        });
    }

    pub fn verdicts(&self) -> &[Verdict] {
        &self.verdicts
    }

    pub fn failures(&self) -> impl Iterator<Item = &Verdict> {
        self.verdicts.iter().filter(|v| !v.passed)
    }

    /// True only when every check ran and passed. There is no partial credit:
    /// a disclosure that half-verifies proves nothing.
    pub fn passed(&self) -> bool {
        self.verdicts.len() == Check::ALL.len() && self.verdicts.iter().all(|v| v.passed)
    }
}

/// The secrets, turned back into the two values the chain published.
struct Recomputed {
    commitment: [u8; 32],
    nullifier: [u8; 32],
}

/// Every field of the disclosure that has to be parsed before it can be used.
///
/// Held as `Result`s rather than unwrapped up front so that a field this tool
/// cannot read fails the checks that depend on it instead of aborting the whole
/// verification. A verifier handed a corrupt file should still learn which parts
/// of the claim survive.
struct Parsed {
    program: Result<Pubkey, String>,
    pool: Result<Pubkey, String>,
    commitment: Result<[u8; 32], String>,
    nullifier: Result<[u8; 32], String>,
    record_address: Result<Pubkey, String>,
    beneficiary: Result<Pubkey, String>,
    target_program: Result<Pubkey, String>,
    payload: Result<Vec<u8>, String>,
    secrets: Result<Recomputed, String>,
}

fn parse_pubkey(s: &str, what: &str) -> Result<Pubkey, String> {
    s.parse()
        .map_err(|e| format!("the {what} is not a base58 pubkey: {e}"))
}

fn parse_hex32(s: &str, what: &str) -> Result<[u8; 32], String> {
    let raw = hex::decode(s).map_err(|e| format!("the {what} is not hex: {e}"))?;
    raw.try_into()
        .map_err(|_| format!("the {what} is not 32 bytes"))
}

fn parse_field(s: &str, what: &str) -> Result<Field, String> {
    let bytes = parse_hex32(s, what)?;
    Field::from_bytes(bytes).map_err(|e| format!("{what} is not a canonical field element: {e:?}"))
}

/// Recomputes the commitment and the nullifier from the disclosed secrets.
///
/// This is the recompute-first rule in one function: the two values the rest of
/// the verification compares against come from `(k, r)` and the denomination,
/// never from the fields of the file that claim them.
fn recompute(disclosure: &Disclosure) -> Result<Recomputed, String> {
    let k = parse_field(&disclosure.k, "k")?;
    let r = parse_field(&disclosure.r, "r")?;
    let note = Note::new(k, r, Field::from_u64(disclosure.denomination));
    Ok(Recomputed {
        commitment: note
            .commitment()
            .map_err(|e| format!("hashing the commitment: {e:?}"))?
            .to_bytes(),
        nullifier: note
            .nullifier()
            .map_err(|e| format!("hashing the nullifier: {e:?}"))?
            .to_bytes(),
    })
}

/// Rebuilds the accumulator from a recovered leaf set.
///
/// Order matters and is the whole reason this is worth checking: the same
/// leaves inserted in a different order give a different root, so a root that
/// matches the chain's is evidence the leaf set is both complete and correctly
/// ordered — which is what makes "leaf 7 is the commitment" mean anything.
fn rebuild_root(leaves: &[[u8; 32]]) -> Result<[u8; 32], String> {
    let mut tree = MerkleTree::new().map_err(|e| format!("{e:?}"))?;
    for (i, leaf) in leaves.iter().enumerate() {
        let field = Field::from_bytes(*leaf)
            .map_err(|_| format!("recovered leaf {i} is not a canonical field element"))?;
        tree.insert(field).map_err(|e| format!("{e:?}"))?;
    }
    tree.root()
        .map(|root| root.to_bytes())
        .map_err(|e| format!("{e:?}"))
}

fn secrets_recompute_to_the_commitment(parsed: &Parsed) -> Result<String, String> {
    let secrets = parsed.secrets.as_ref().map_err(Clone::clone)?;
    let stated = parsed.commitment.as_ref().map_err(Clone::clone)?;
    if secrets.commitment != *stated {
        return Err(format!(
            "the secrets commit to {}, but the disclosure states {}",
            hex::encode(secrets.commitment),
            hex::encode(stated)
        ));
    }
    Ok(format!(
        "H3(k, r, denom_tag) = {}",
        hex::encode(secrets.commitment)
    ))
}

/// The pool is a PDA of the program and the denomination, so a disclosure that
/// names all three cannot be internally consistent by accident — and the account
/// at that address must itself report the denomination the commitment was bound
/// to, or the note being disclosed belongs to a different tree.
fn the_pool_matches_the_denomination(
    disclosure: &Disclosure,
    view: &ChainView,
    parsed: &Parsed,
) -> Result<String, String> {
    let program = parsed.program.as_ref().map_err(Clone::clone)?;
    let pool = parsed.pool.as_ref().map_err(Clone::clone)?;
    let (derived, _) = pool_address(program, disclosure.denomination);
    if derived != *pool {
        return Err(format!(
            "denomination {} under program {program} is pool {derived}, not {pool}",
            disclosure.denomination
        ));
    }
    if view.denomination != disclosure.denomination {
        return Err(format!(
            "the pool at {pool} holds denomination {}, but the disclosure claims {}",
            view.denomination, disclosure.denomination
        ));
    }
    Ok(format!(
        "{pool} is the pool for {} lamports",
        view.denomination
    ))
}

/// Checked against the *recomputed* commitment. A file whose commitment field
/// were trusted here would let a discloser point at somebody else's leaf.
fn the_commitment_is_the_stated_leaf(
    disclosure: &Disclosure,
    view: &ChainView,
    parsed: &Parsed,
) -> Result<String, String> {
    let secrets = parsed.secrets.as_ref().map_err(Clone::clone)?;
    let at = view
        .leaves
        .get(disclosure.leaf_index as usize)
        .ok_or_else(|| {
            format!(
                "the disclosure claims leaf {}, but the pool's history holds only {} leaves",
                disclosure.leaf_index,
                view.leaves.len()
            )
        })?;
    if *at != secrets.commitment {
        let elsewhere = view
            .leaves
            .iter()
            .position(|leaf| leaf == &secrets.commitment);
        return Err(match elsewhere {
            Some(i) => format!(
                "the secrets commit to leaf {i}, not to leaf {}",
                disclosure.leaf_index
            ),
            None => format!(
                "the commitment these secrets produce is not in the pool's leaf \
                 set at all ({} leaves recovered)",
                view.leaves.len()
            ),
        });
    }
    Ok(format!(
        "leaf {} of {} is {}",
        disclosure.leaf_index,
        view.leaves.len(),
        hex::encode(secrets.commitment)
    ))
}

/// The check that makes the leaf index mean something.
///
/// Without it, a verifier is trusting whoever served the history. A leaf set
/// with an insertion missing, or reordered, produces a tree in which some other
/// member's note occupies the disclosed index — and every other check here would
/// still pass. Matching the on-chain root is the only available evidence that
/// the recovered set is the set the program accumulated.
fn the_rebuilt_root_matches_the_chain(view: &ChainView) -> Result<String, String> {
    let rebuilt = rebuild_root(&view.leaves)?;
    if rebuilt != view.root {
        return Err(format!(
            "the accumulator rebuilt from {} recovered leaves has root {}, but \
             the pool holds {}. The leaf set or its order is wrong, so the leaf \
             index above names nothing.",
            view.leaves.len(),
            hex::encode(rebuilt),
            hex::encode(view.root)
        ));
    }
    Ok(format!(
        "{} leaves rebuild to {}",
        view.leaves.len(),
        hex::encode(rebuilt)
    ))
}

fn secrets_recompute_to_the_nullifier(parsed: &Parsed) -> Result<String, String> {
    let secrets = parsed.secrets.as_ref().map_err(Clone::clone)?;
    let stated = parsed.nullifier.as_ref().map_err(Clone::clone)?;
    if secrets.nullifier != *stated {
        return Err(format!(
            "H1(k) is {}, but the disclosure states {}",
            hex::encode(secrets.nullifier),
            hex::encode(stated)
        ));
    }
    Ok(format!("H1(k) = {}", hex::encode(secrets.nullifier)))
}

/// Derived from the secrets, not read out of the file.
///
/// This is what binds the record to the note. A disclosure that could name any
/// record address would be a member pointing at whichever settled action suited
/// them; the address has to fall out of `H1(k)` and the pool.
fn the_record_address_is_the_nullifiers_pda(parsed: &Parsed) -> Result<String, String> {
    let secrets = parsed.secrets.as_ref().map_err(Clone::clone)?;
    let program = parsed.program.as_ref().map_err(Clone::clone)?;
    let pool = parsed.pool.as_ref().map_err(Clone::clone)?;
    let stated = parsed.record_address.as_ref().map_err(Clone::clone)?;
    let (derived, _) = spend_address(program, pool, &secrets.nullifier);
    if derived != *stated {
        return Err(format!(
            "H1(k) in this pool is record {derived}, but the disclosure names {stated}"
        ));
    }
    Ok(format!("{derived} is the record for H1(k)"))
}

fn the_record_exists_on_chain(view: &ChainView, parsed: &Parsed) -> Result<String, String> {
    let stated = parsed.record_address.as_ref().map_err(Clone::clone)?;
    match &view.record {
        None => Err(format!(
            "no account exists at {stated}. Either the spend was never submitted, \
             or this endpoint cannot see it."
        )),
        Some(_) => Ok(format!("{stated} holds a spend record")),
    }
}

/// Settled, not merely submitted.
///
/// A pending record is a burnt nullifier and an action that has not run. The
/// member has not yet done the thing they are claiming to have done, and the
/// action may still fail at settlement.
fn the_record_is_settled(view: &ChainView) -> Result<String, String> {
    let record = view
        .record
        .as_ref()
        .ok_or_else(|| "there is no record to read a status from".to_string())?;
    if !record.settled {
        return Err(
            "the record is pending: the action was authorised but has not executed, \
             so there is nothing yet to have been done"
                .to_string(),
        );
    }
    Ok("settled".to_string())
}

fn the_record_belongs_to_this_pool(parsed: &Parsed, view: &ChainView) -> Result<String, String> {
    let pool = parsed.pool.as_ref().map_err(Clone::clone)?;
    let record = view
        .record
        .as_ref()
        .ok_or_else(|| "there is no record to read a pool from".to_string())?;
    if record.pool != pool.to_bytes() {
        return Err(format!(
            "the record names pool {}, not {pool}",
            Pubkey::new_from_array(record.pool)
        ));
    }
    Ok(format!("the record names {pool}"))
}

/// Every field the action binding covered, compared one at a time.
///
/// Reported field by field rather than as a single equality because the fields
/// mean different things to a verifier: a wrong beneficiary is a different claim
/// entirely, and a wrong relay fee is an arithmetic disagreement about the same
/// one.
fn the_record_matches_the_claimed_action(
    disclosure: &Disclosure,
    view: &ChainView,
    parsed: &Parsed,
) -> Result<String, String> {
    let record = view
        .record
        .as_ref()
        .ok_or_else(|| "there is no record to compare the action against".to_string())?;
    let beneficiary = parsed.beneficiary.as_ref().map_err(Clone::clone)?;
    let target = parsed.target_program.as_ref().map_err(Clone::clone)?;
    let payload = parsed.payload.as_ref().map_err(Clone::clone)?;

    let mut wrong: Vec<String> = Vec::new();
    if record.beneficiary != beneficiary.to_bytes() {
        wrong.push(format!(
            "beneficiary: the record paid {}, the disclosure claims {beneficiary}",
            Pubkey::new_from_array(record.beneficiary)
        ));
    }
    if record.selector != disclosure.action.selector {
        wrong.push(format!(
            "selector: the record holds {}, the disclosure claims {}",
            record.selector, disclosure.action.selector
        ));
    }
    if record.target_program != target.to_bytes() {
        wrong.push(format!(
            "target program: the record invoked {}, the disclosure claims {target}",
            Pubkey::new_from_array(record.target_program)
        ));
    }
    if record.relay_fee != disclosure.action.relay_fee {
        wrong.push(format!(
            "relay fee: the record holds {}, the disclosure claims {}",
            record.relay_fee, disclosure.action.relay_fee
        ));
    }
    if record.payload != *payload {
        wrong.push(format!(
            "payload: the record holds {}, the disclosure claims {}",
            hex::encode(&record.payload),
            hex::encode(payload)
        ));
    }
    if !wrong.is_empty() {
        return Err(wrong.join("; "));
    }
    Ok(format!(
        "selector {} paid {beneficiary}, fee {}, {} payload byte(s)",
        record.selector,
        record.relay_fee,
        record.payload.len()
    ))
}

/// Re-derives the whole claim and reports every check separately.
///
/// Infallible by design. A malformed disclosure produces failed checks rather
/// than an error, because "this tool could not read the file" and "this tool
/// read the file and it was false" must never be distinguishable to a caller
/// that only looks at a return code — both mean the claim is not established.
///
/// Fail-closed throughout: a check whose inputs are missing fails, and a check
/// whose prerequisite failed fails on its own terms rather than being skipped.
/// There is no path through this function that reports a pass for something it
/// did not compare.
pub fn verify(disclosure: &Disclosure, view: &ChainView) -> Verification {
    let parsed = Parsed {
        program: parse_pubkey(&disclosure.program, "program"),
        pool: parse_pubkey(&disclosure.pool, "pool"),
        commitment: parse_hex32(&disclosure.commitment, "commitment"),
        nullifier: parse_hex32(&disclosure.nullifier, "nullifier"),
        record_address: parse_pubkey(&disclosure.spend_record, "spend record address"),
        beneficiary: parse_pubkey(&disclosure.action.beneficiary, "beneficiary"),
        target_program: parse_pubkey(&disclosure.action.target_program, "target program"),
        payload: hex::decode(&disclosure.action.payload)
            .map_err(|e| format!("the payload is not hex: {e}")),
        secrets: recompute(disclosure),
    };

    let mut result = Verification::default();
    result.record(
        Check::SecretsRecomputeToTheCommitment,
        secrets_recompute_to_the_commitment(&parsed),
    );
    result.record(
        Check::ThePoolMatchesTheDenomination,
        the_pool_matches_the_denomination(disclosure, view, &parsed),
    );
    result.record(
        Check::TheCommitmentIsTheStatedLeaf,
        the_commitment_is_the_stated_leaf(disclosure, view, &parsed),
    );
    result.record(
        Check::TheRebuiltRootMatchesTheChain,
        the_rebuilt_root_matches_the_chain(view),
    );
    result.record(
        Check::SecretsRecomputeToTheNullifier,
        secrets_recompute_to_the_nullifier(&parsed),
    );
    result.record(
        Check::TheRecordAddressIsTheNullifiersPda,
        the_record_address_is_the_nullifiers_pda(&parsed),
    );
    result.record(
        Check::TheRecordExistsOnChain,
        the_record_exists_on_chain(view, &parsed),
    );
    result.record(Check::TheRecordIsSettled, the_record_is_settled(view));
    result.record(
        Check::TheRecordBelongsToThisPool,
        the_record_belongs_to_this_pool(&parsed, view),
    );
    result.record(
        Check::TheRecordMatchesTheClaimedAction,
        the_record_matches_the_claimed_action(disclosure, view, &parsed),
    );
    result
}

/// Prints the checks as a table, failures included.
///
/// Every check is printed whether it passed or not. A report that listed only
/// the failures would leave a verifier unable to tell a claim that was checked
/// thoroughly from one that was barely checked at all.
pub fn report(disclosure: &Disclosure, result: &Verification) {
    println!(
        "disclosure for leaf {} of {}",
        disclosure.leaf_index, disclosure.pool
    );
    println!("  nullifier {}", disclosure.nullifier);
    println!("  record    {}", disclosure.spend_record);
    println!();
    for verdict in result.verdicts() {
        println!(
            "  {:<42} {}",
            verdict.check.name(),
            if verdict.passed { "PASS" } else { "FAIL" }
        );
        println!("      {}", verdict.detail);
    }
}

// ---------------------------------------------------------------------------
// The two commands

/// Writes a disclosure for a settled note.
pub fn create(
    chain: &Chain,
    program: &Pubkey,
    note_path: &Path,
    out: &Path,
    acknowledge_cost: bool,
) -> Result<()> {
    let stored = StoredNote::read(note_path)?;
    let note = stored.note()?;
    let (pool, _) = pool_address(program, stored.denomination);
    let nullifier = note.nullifier().map_err(|e| anyhow!("{e:?}"))?.to_bytes();
    let (record, _) = spend_address(program, &pool, &nullifier);

    println!("rebuilding the pool from chain history:");
    let view = observe(chain, program, stored.denomination, &record, true)?;

    // The flag arrives as a bool because a command-line flag is one. It becomes
    // a constructed override here, at the edge, so that nothing inside this
    // module can be handed a consent it never asked for.
    let cost = CoParticipantCost::new(view.settled_spends, view.k_floor);
    let consent = if acknowledge_cost {
        Some(DisclosureOverride::acknowledging(cost))
    } else {
        None
    };
    let disclosure = build(program, &pool, &stored, &view, consent.as_ref())?;

    // Checked before it is written, against the same view. A disclosure that
    // would not verify is a file a member would hand over and be disbelieved
    // for, and they would have no way to tell why.
    let result = verify(&disclosure, &view);
    if !result.passed() {
        report(&disclosure, &result);
        return Err(anyhow!(
            "refusing to write a disclosure that does not verify against the \
             chain it was built from"
        ));
    }

    disclosure.write(out)?;
    println!();
    println!("wrote {}", out.display());
    println!("  leaf      {}", disclosure.leaf_index);
    println!("  nullifier {}", disclosure.nullifier);
    println!("  record    {}", disclosure.spend_record);
    if let Some(consent) = consent {
        let cost = consent.cost();
        println!();
        println!(
            "You disclosed below the floor: {} settled spend(s), floor {}, {} left",
            cost.settled_spends, cost.k_floor, cost.remaining
        );
        println!("unattributed. That cost was paid by the other members of this pool.");
    }
    println!();
    println!("This file contains k and r. They authorise nothing now — the nullifier is");
    println!("burnt and the record has settled — but they identify you to whoever holds");
    println!("it, permanently and with no way to take it back. Send it to one verifier,");
    println!("over a channel you would send a passport scan over, and to nobody else.");
    println!();
    println!("The verifier checks it with: mirror disclose-verify --file <path>");
    Ok(())
}

/// Checks a disclosure against the cluster and prints every verdict.
pub fn check(chain: &Chain, path: &Path) -> Result<()> {
    let disclosure = Disclosure::read(path)?;
    // Needed before the RPC can be pointed anywhere. A file too malformed to
    // name a program cannot be checked at all, which is a failure to establish
    // the claim rather than a claim that failed.
    let program: Pubkey = disclosure
        .program
        .parse()
        .map_err(|e| anyhow!("the program in {} is not a pubkey: {e}", path.display()))?;
    let record: Pubkey = disclosure.spend_record.parse().map_err(|e| {
        anyhow!(
            "the spend record address in {} is not a pubkey: {e}",
            path.display()
        )
    })?;

    println!("rebuilding the pool from chain history:");
    let view = observe(chain, &program, disclosure.denomination, &record, true)?;
    println!();
    let result = verify(&disclosure, &view);
    report(&disclosure, &result);
    println!();

    if result.passed() {
        println!(
            "VERIFIED. Every value in this file was recomputed from the secrets or \
             read off the\ncluster. The holder of this note is whoever asked the pool \
             for the action above."
        );
        return Ok(());
    }
    Err(anyhow!(
        "NOT VERIFIED: {} of {} checks failed. This file does not establish that its \
         author authorised that action — it establishes nothing at all.",
        result.failures().count(),
        Check::ALL.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DENOM: u64 = 100_000_000;
    /// Where the disclosed note sits in the fixture's leaf set.
    const LEAF: u64 = 3;
    const FEE: u64 = 5_000;
    const SELECTOR: u64 = 2;

    fn program() -> Pubkey {
        Pubkey::new_from_array([7u8; 32])
    }

    fn beneficiary() -> Pubkey {
        Pubkey::new_from_array([11u8; 32])
    }

    fn target() -> Pubkey {
        Pubkey::new_from_array([13u8; 32])
    }

    fn note() -> Note {
        Note::new(
            Field::from_u64(1_234_567),
            Field::from_u64(7_654_321),
            Field::from_u64(DENOM),
        )
    }

    fn root_of(leaves: &[[u8; 32]]) -> [u8; 32] {
        rebuild_root(leaves).unwrap()
    }

    fn leaves_with(commitment: [u8; 32]) -> Vec<[u8; 32]> {
        let mut leaves: Vec<[u8; 32]> = (1..=6u64)
            .map(|i| Field::from_u64(i * 977 + 13).to_bytes())
            .collect();
        leaves[LEAF as usize] = commitment;
        leaves
    }

    fn record_for(pool: &Pubkey) -> SpendRecordView {
        SpendRecordView {
            settled: true,
            pool: pool.to_bytes(),
            beneficiary: beneficiary().to_bytes(),
            selector: SELECTOR,
            target_program: target().to_bytes(),
            relay_fee: FEE,
            payload: b"delegate".to_vec(),
        }
    }

    /// A pool that has settled comfortably above its floor, holding the note.
    fn honest() -> (Disclosure, ChainView) {
        let program = program();
        let (pool, _) = pool_address(&program, DENOM);
        let stored = StoredNote::from_note(&note(), DENOM).unwrap();
        let leaves = leaves_with(stored.commitment_bytes().unwrap());
        let view = ChainView {
            denomination: DENOM,
            k_floor: 2,
            root: root_of(&leaves),
            settled_spends: 5,
            leaves,
            record: Some(record_for(&pool)),
        };
        let disclosure = build(&program, &pool, &stored, &view, None).unwrap();
        (disclosure, view)
    }

    fn verify_with(mutate: impl FnOnce(&mut Disclosure, &mut ChainView)) -> Verification {
        let (mut disclosure, mut view) = honest();
        mutate(&mut disclosure, &mut view);
        verify(&disclosure, &view)
    }

    fn failed(result: &Verification) -> Vec<Check> {
        result.failures().map(|v| v.check).collect()
    }

    fn detail(result: &Verification, check: Check) -> String {
        result
            .verdicts()
            .iter()
            .find(|v| v.check == check)
            .expect("every check is reported")
            .detail
            .clone()
    }

    // -- the honest cases ---------------------------------------------------

    #[test]
    fn an_honest_disclosure_verifies() {
        let (disclosure, view) = honest();
        let result = verify(&disclosure, &view);
        assert!(
            result.passed(),
            "an honest disclosure failed: {:?}",
            failed(&result)
        );
    }

    /// The claim is only as good as its coverage, so the report must show every
    /// check exactly once and in a stable order — a verifier comparing two runs
    /// should be comparing the same list.
    #[test]
    fn every_check_is_reported_exactly_once_and_in_order() {
        let (disclosure, view) = honest();
        let result = verify(&disclosure, &view);
        let reported: Vec<Check> = result.verdicts().iter().map(|v| v.check).collect();
        assert_eq!(reported, Check::ALL.to_vec());
    }

    #[test]
    fn a_disclosure_round_trips_through_its_file_form() {
        let (disclosure, view) = honest();
        let text = serde_json::to_string(&disclosure).unwrap();
        let read: Disclosure = serde_json::from_str(&text).unwrap();
        assert!(verify(&read, &view).passed());
    }

    /// The action is copied out of the record rather than out of anything the
    /// member typed, so what the disclosure claims is what the chain holds.
    #[test]
    fn the_claimed_action_is_taken_from_the_record() {
        let (disclosure, _) = honest();
        assert_eq!(disclosure.action.beneficiary, beneficiary().to_string());
        assert_eq!(disclosure.action.selector, SELECTOR);
        assert_eq!(disclosure.action.target_program, target().to_string());
        assert_eq!(disclosure.action.relay_fee, FEE);
        assert_eq!(disclosure.action.payload, hex::encode(b"delegate"));
    }

    // -- the tamper matrix --------------------------------------------------

    /// Wrong `k`: everything derived from the nullifier preimage moves at once.
    /// The fan-out is the point — `k` is what ties the leaf, the nullifier and
    /// the record address together, so a substituted one cannot satisfy any of
    /// them.
    #[test]
    fn a_disclosure_with_the_wrong_k_fails_every_check_that_derives_from_it() {
        let result = verify_with(|d, _| d.k = hex::encode(Field::from_u64(42).to_bytes()));
        assert_eq!(
            failed(&result),
            vec![
                Check::SecretsRecomputeToTheCommitment,
                Check::TheCommitmentIsTheStatedLeaf,
                Check::SecretsRecomputeToTheNullifier,
                Check::TheRecordAddressIsTheNullifiersPda,
            ]
        );
    }

    /// Wrong `r`: the commitment moves and the nullifier does not, because `r`
    /// blinds the leaf and never enters `H1`.
    #[test]
    fn a_disclosure_with_the_wrong_r_fails_the_commitment_but_not_the_nullifier() {
        let result = verify_with(|d, _| d.r = hex::encode(Field::from_u64(99).to_bytes()));
        assert_eq!(
            failed(&result),
            vec![
                Check::SecretsRecomputeToTheCommitment,
                Check::TheCommitmentIsTheStatedLeaf,
            ]
        );
    }

    #[test]
    fn a_non_canonical_secret_is_refused_rather_than_reduced() {
        let result = verify_with(|d, _| d.k = hex::encode([0xffu8; 32]));
        assert!(detail(&result, Check::SecretsRecomputeToTheCommitment).contains("canonical"));
    }

    /// A leaf set that is internally consistent — its root matches the chain —
    /// but does not contain this note. This is the shape a member pointing at
    /// somebody else's pool would produce.
    #[test]
    fn a_commitment_absent_from_the_recovered_leaf_set_is_refused() {
        let result = verify_with(|_, view| {
            view.leaves[LEAF as usize] = Field::from_u64(31_337).to_bytes();
            view.root = root_of(&view.leaves);
        });
        assert_eq!(failed(&result), vec![Check::TheCommitmentIsTheStatedLeaf]);
        assert!(detail(&result, Check::TheCommitmentIsTheStatedLeaf).contains("not in the pool"));
    }

    /// The right note, the wrong index. Nothing else in the file changes, and
    /// only the check that compares the two notices.
    #[test]
    fn the_right_commitment_at_the_wrong_leaf_index_is_refused() {
        let result = verify_with(|d, _| d.leaf_index = 1);
        assert_eq!(failed(&result), vec![Check::TheCommitmentIsTheStatedLeaf]);
        assert!(detail(&result, Check::TheCommitmentIsTheStatedLeaf)
            .contains(&format!("commit to leaf {LEAF}")));
    }

    #[test]
    fn a_leaf_index_beyond_the_recovered_history_is_refused() {
        let result = verify_with(|d, _| d.leaf_index = 4_000);
        assert_eq!(failed(&result), vec![Check::TheCommitmentIsTheStatedLeaf]);
        assert!(detail(&result, Check::TheCommitmentIsTheStatedLeaf).contains("only 6 leaves"));
    }

    /// An incomplete or reordered history. Every other check still passes,
    /// which is exactly why this one has to exist: without it a verifier would
    /// be trusting whoever served the leaves.
    #[test]
    fn a_rebuilt_root_that_disagrees_with_the_chain_is_refused() {
        let result = verify_with(|_, view| view.root = [0xAB; 32]);
        assert_eq!(failed(&result), vec![Check::TheRebuiltRootMatchesTheChain]);
    }

    /// The file's own nullifier field is never believed. It is compared against
    /// `H1(k)`, and the record address is derived from `H1(k)` regardless — so
    /// this fails the equality check and nothing else.
    #[test]
    fn a_nullifier_field_inconsistent_with_the_secrets_is_refused() {
        let result = verify_with(|d, _| d.nullifier = hex::encode([0x01u8; 32]));
        assert_eq!(failed(&result), vec![Check::SecretsRecomputeToTheNullifier]);
    }

    #[test]
    fn a_spend_record_address_that_is_not_the_derived_pda_is_refused() {
        let result =
            verify_with(|d, _| d.spend_record = Pubkey::new_from_array([9u8; 32]).to_string());
        assert_eq!(
            failed(&result),
            vec![Check::TheRecordAddressIsTheNullifiersPda]
        );
    }

    /// Fail-closed: the three checks that need a record fail on their own terms
    /// rather than being skipped, because a skipped check reads as a passed one
    /// to anybody scanning the table.
    #[test]
    fn an_absent_spend_record_fails_every_check_that_needs_one() {
        let result = verify_with(|_, view| view.record = None);
        assert_eq!(
            failed(&result),
            vec![
                Check::TheRecordExistsOnChain,
                Check::TheRecordIsSettled,
                Check::TheRecordBelongsToThisPool,
                Check::TheRecordMatchesTheClaimedAction,
            ]
        );
    }

    #[test]
    fn a_record_that_is_still_pending_is_refused() {
        let result = verify_with(|_, view| {
            view.record.as_mut().unwrap().settled = false;
        });
        assert_eq!(failed(&result), vec![Check::TheRecordIsSettled]);
        assert!(detail(&result, Check::TheRecordIsSettled).contains("pending"));
    }

    /// A record from another denomination's pool. The address is pool-scoped, so
    /// reaching this state means the record itself was tampered with, and the
    /// check that reads its `pool` field is the one that catches it.
    #[test]
    fn a_record_belonging_to_a_different_pool_is_refused() {
        let result = verify_with(|_, view| {
            view.record.as_mut().unwrap().pool = [0x55; 32];
        });
        assert_eq!(failed(&result), vec![Check::TheRecordBelongsToThisPool]);
    }

    #[test]
    fn a_substituted_beneficiary_is_refused() {
        let result = verify_with(|d, _| {
            d.action.beneficiary = Pubkey::new_from_array([0x21; 32]).to_string();
        });
        assert_eq!(
            failed(&result),
            vec![Check::TheRecordMatchesTheClaimedAction]
        );
        assert!(detail(&result, Check::TheRecordMatchesTheClaimedAction).contains("beneficiary"));
    }

    #[test]
    fn a_substituted_selector_is_refused() {
        let result = verify_with(|d, _| d.action.selector = SELECTOR + 1);
        assert!(detail(&result, Check::TheRecordMatchesTheClaimedAction).contains("selector"));
        assert_eq!(
            failed(&result),
            vec![Check::TheRecordMatchesTheClaimedAction]
        );
    }

    #[test]
    fn a_substituted_target_program_is_refused() {
        let result = verify_with(|d, _| {
            d.action.target_program = Pubkey::new_from_array([0x31; 32]).to_string();
        });
        assert!(detail(&result, Check::TheRecordMatchesTheClaimedAction).contains("target program"));
        assert_eq!(
            failed(&result),
            vec![Check::TheRecordMatchesTheClaimedAction]
        );
    }

    #[test]
    fn a_substituted_relay_fee_is_refused() {
        let result = verify_with(|d, _| d.action.relay_fee = FEE + 1);
        assert!(detail(&result, Check::TheRecordMatchesTheClaimedAction).contains("relay fee"));
        assert_eq!(
            failed(&result),
            vec![Check::TheRecordMatchesTheClaimedAction]
        );
    }

    /// The payload is the action's parameters. A disclosure that understated it
    /// would describe a different instruction than the one the pool executed.
    #[test]
    fn a_substituted_payload_is_refused() {
        let result = verify_with(|d, _| d.action.payload = hex::encode(b"withdraw"));
        assert!(detail(&result, Check::TheRecordMatchesTheClaimedAction).contains("payload"));
        assert_eq!(
            failed(&result),
            vec![Check::TheRecordMatchesTheClaimedAction]
        );
    }

    /// The pool the disclosure names holds a different size than it claims, so
    /// the commitment it recomputes belongs to another tree entirely.
    #[test]
    fn a_denomination_the_pool_does_not_hold_is_refused() {
        let result = verify_with(|_, view| view.denomination = DENOM + 1);
        assert_eq!(failed(&result), vec![Check::ThePoolMatchesTheDenomination]);
    }

    /// The pool address is a PDA of the program and the denomination, so a
    /// disclosure naming some other account is refused without an RPC call.
    #[test]
    fn a_pool_that_is_not_the_pda_for_the_denomination_is_refused() {
        let result = verify_with(|d, _| d.pool = Pubkey::new_from_array([0x41; 32]).to_string());
        // The record's own pool field no longer matches either, which is the
        // record correctly disagreeing with a substituted claim.
        assert_eq!(
            failed(&result),
            vec![
                Check::ThePoolMatchesTheDenomination,
                Check::TheRecordAddressIsTheNullifiersPda,
                Check::TheRecordBelongsToThisPool,
            ]
        );
    }

    /// A file this tool cannot parse must not verify. It is the case where a
    /// bare `is_err()` would be indistinguishable from a passing claim.
    #[test]
    fn a_malformed_field_fails_closed_rather_than_erroring() {
        let result = verify_with(|d, _| d.commitment = "not hex".to_string());
        assert!(!result.passed());
        assert!(detail(&result, Check::SecretsRecomputeToTheCommitment).contains("not hex"));
    }

    // -- building -----------------------------------------------------------

    /// The refusal that protects the discloser. An unspent note's secrets are
    /// the deposit, so writing them into a file to hand somebody is not a
    /// disclosure — it is a transfer.
    #[test]
    fn a_disclosure_for_an_unspent_note_is_refused() {
        let program = program();
        let (pool, _) = pool_address(&program, DENOM);
        let stored = StoredNote::from_note(&note(), DENOM).unwrap();
        let leaves = leaves_with(stored.commitment_bytes().unwrap());
        let view = ChainView {
            denomination: DENOM,
            k_floor: 2,
            root: root_of(&leaves),
            settled_spends: 5,
            leaves,
            record: None,
        };
        let err = build(&program, &pool, &stored, &view, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("has not been spent"), "{err}");
    }

    #[test]
    fn a_disclosure_for_a_pending_spend_is_refused() {
        let program = program();
        let (pool, _) = pool_address(&program, DENOM);
        let stored = StoredNote::from_note(&note(), DENOM).unwrap();
        let leaves = leaves_with(stored.commitment_bytes().unwrap());
        let mut record = record_for(&pool);
        record.settled = false;
        let view = ChainView {
            denomination: DENOM,
            k_floor: 2,
            root: root_of(&leaves),
            settled_spends: 5,
            leaves,
            record: Some(record),
        };
        let err = build(&program, &pool, &stored, &view, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("has not settled"), "{err}");
    }

    #[test]
    fn a_disclosure_for_a_note_the_pool_never_held_is_refused() {
        let program = program();
        let (pool, _) = pool_address(&program, DENOM);
        let stored = StoredNote::from_note(&note(), DENOM).unwrap();
        let leaves: Vec<[u8; 32]> = (1..=6u64).map(|i| Field::from_u64(i).to_bytes()).collect();
        let view = ChainView {
            denomination: DENOM,
            k_floor: 2,
            root: root_of(&leaves),
            settled_spends: 5,
            leaves,
            record: Some(record_for(&pool)),
        };
        let err = build(&program, &pool, &stored, &view, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a leaf"), "{err}");
    }

    // -- the co-participant gate --------------------------------------------

    #[test]
    fn the_gate_counts_what_is_left_for_everybody_else() {
        let cost = CoParticipantCost::new(5, 2);
        assert_eq!(cost.remaining, 4);
        assert!(cost.leaves_a_crowd());
        // Exactly at the floor is still a crowd; one below is not.
        assert!(CoParticipantCost::new(3, 2).leaves_a_crowd());
        assert!(!CoParticipantCost::new(2, 2).leaves_a_crowd());
    }

    fn view_with_settled(settled: u64, k_floor: u32) -> (Pubkey, Pubkey, StoredNote, ChainView) {
        let program = program();
        let (pool, _) = pool_address(&program, DENOM);
        let stored = StoredNote::from_note(&note(), DENOM).unwrap();
        let leaves = leaves_with(stored.commitment_bytes().unwrap());
        let view = ChainView {
            denomination: DENOM,
            k_floor,
            root: root_of(&leaves),
            settled_spends: settled,
            leaves,
            record: Some(record_for(&pool)),
        };
        (program, pool, stored, view)
    }

    /// Disclosing is only free to the person doing it. Below the floor the
    /// default is refusal, and the message names the people it would cost.
    #[test]
    fn the_gate_refuses_a_disclosure_that_leaves_the_others_below_the_floor() {
        let (program, pool, stored, view) = view_with_settled(2, 2);
        let err = build(&program, &pool, &stored, &view, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("REFUSING"), "{err}");
        assert!(err.contains(ACKNOWLEDGE_FLAG), "{err}");
    }

    #[test]
    fn the_gate_permits_a_disclosure_that_leaves_a_crowd() {
        let (program, pool, stored, view) = view_with_settled(3, 2);
        assert!(build(&program, &pool, &stored, &view, None).is_ok());
    }

    /// The override is a value the caller has to construct out of the cost it is
    /// overriding, so it cannot be produced without the number being computed.
    #[test]
    fn an_explicit_override_permits_a_disclosure_below_the_floor() {
        let (program, pool, stored, view) = view_with_settled(2, 2);
        let consent = DisclosureOverride::acknowledging(CoParticipantCost::new(
            view.settled_spends,
            view.k_floor,
        ));
        let disclosure = build(&program, &pool, &stored, &view, Some(&consent)).unwrap();
        assert!(verify(&disclosure, &view).passed());
        assert!(!consent.cost().leaves_a_crowd());
    }
}
