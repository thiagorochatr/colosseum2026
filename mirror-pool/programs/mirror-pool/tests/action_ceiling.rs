//! What an action costs the crowd, measured against the compiled program.
//!
//! `batch_ceiling.rs` establishes that ten spends fit in one settlement and that
//! the 1232-byte packet, not compute, is what stops the eleventh. That number is
//! for plain transfers, where a spend brings exactly three accounts nobody else
//! shares: its record, its beneficiary and its relay. A spend whose action is a
//! CPI brings more — the callee's own program account, plus one slot per account
//! the action needs — so the ceiling for an action batch is lower, and until this
//! file nobody had taken it.
//!
//! The ceiling is not one number, because it is not a property of the action's
//! *shape* alone. Transaction size is driven by how many **distinct** keys the
//! message names, and two members delegating to the same validator name one vote
//! account between them while two members delegating to different validators name
//! two. So a batch whose members agree carries more members than a batch whose
//! members diverge: divergence in the content of an action is paid for in
//! anonymity-set size. That is the result this file is named for, and the two
//! stake batches below are settled for real rather than argued.
//!
//! Real means real. litesvm carries the Core BPF stake program, so a member's
//! `DelegateStake` here is executed by the actual stake program against a vote
//! account the actual vote program initialised, with the pool's vault signing as
//! the stake authority through `invoke_signed`. The batches settle, the stake
//! accounts end up delegated, and the compute the settlement consumed is
//! reported next to the bytes — because a size-only claim cannot say whether
//! compute also binds.
//!
//! Not every row is executed, and the table says which are. The two
//! stake-delegation rows — the result — are settled at their ceiling. The generic
//! curve around them (a call with k action accounts, shared between the members
//! or one each) is a size measurement and nothing more: it is taken from the
//! program's real instruction encoding rather than from arithmetic, but no batch
//! of that shape was run, so it is labelled `size only` and carries no compute
//! figure. The transfer row is the control: `batch_ceiling.rs` already settles it
//! against the same `.so`, so this file recomputing 10 spends / 1229 bytes is a
//! check on the encoder rather than a new claim. If that row ever moves, the
//! model of the wire here is wrong and every other number it prints is suspect.
//!
//! Requires `make build-sbf` first, like the rest of the on-chain suite.

use litesvm::LiteSVM;
use mirror_circuit::{generate_reproducible, prove, Keys, SolanaProof, Witness};
use mirror_core::{Field, MerkleTree, Note};
use mirror_pool_program::{
    instruction::Instruction as MirrorIx,
    pda::{pool_address, spend_address, vault_address},
    Pool,
};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_signer::Signer;
use solana_transaction::Transaction;

const SEED: &[u8] = b"mirror-pool-reproducible-dev-setup-v1";
const DENOMINATION: u64 = 100_000_000; // 0.1 SOL
const ENTRY_FEE: u64 = 0;
const K_FLOOR: u32 = 4;
const RELAY_FEE: u64 = 1_000_000;
const INVOKE_SIGNED: u64 = mirror_pool_program::processor::SELECTOR_INVOKE_SIGNED;

/// The 1280-byte IPv6 minimum MTU, less a 40-byte IPv6 header and an 8-byte
/// fragment header. Restated here for the same reason `batch_ceiling.rs` restates
/// it: no crate this test already depends on re-exports
/// `solana_packet::PACKET_DATA_SIZE`, and a derivation a reader can check is
/// worth more than a constant they have to trust.
const PACKET_DATA_SIZE: usize = 1280 - 40 - 8;

/// What a transaction carrying a single non-budget instruction gets unless it
/// asks for more. Asking costs a second instruction, and at these batch sizes
/// there are no bytes to spare for one.
const DEFAULT_COMPUTE_BUDGET: u64 = 200_000;

// The addresses of the programs and sysvars a stake delegation names. Written as
// strings because these are cluster constants rather than anything this
// repository derives, and because `solana-program` 4.0 no longer re-exports the
// stake and vote ids at all.
const STAKE_PROGRAM: &str = "Stake11111111111111111111111111111111111111";
const VOTE_PROGRAM: &str = "Vote111111111111111111111111111111111111111";
/// Unused by the current stake program and still required in the account list at
/// the position `DelegateStake` has always put it. It costs a slot either way,
/// which is the only reason this measurement cares.
const STAKE_CONFIG: &str = "StakeConfig11111111111111111111111111111111";
const CLOCK_SYSVAR: &str = "SysvarC1ock11111111111111111111111111111111";
const STAKE_HISTORY_SYSVAR: &str = "SysvarStakeHistory1111111111111111111111111";
const RENT_SYSVAR: &str = "SysvarRent111111111111111111111111111111111";

/// `VoteStateV3::size_of()`, and `VoteStateV4` deliberately matches it.
const VOTE_STATE_LEN: usize = 3762;
/// `StakeStateV2::size_of()`.
const STAKE_STATE_LEN: usize = 200;

/// `StakeInstruction::DelegateStake`, bincode-encoded: a unit variant at index
/// two of the enum, which is a four-byte little-endian discriminant and nothing
/// else. This is the entire payload a member commits to when they delegate — the
/// validator is chosen by an *account*, not by the instruction data, which is
/// exactly why divergent validators cost keys rather than bytes.
const DELEGATE_STAKE: [u8; 4] = [2, 0, 0, 0];

/// What each stake account holds before the pool delegates it.
///
/// It has to be pre-funded, and the reason is the selector rather than the test
/// rig: `SELECTOR_INVOKE_SIGNED` pays the beneficiary *after* the call so that
/// the vault's lamports are untouched at the moment it signs, so the amount the
/// stake program sees is whatever the account already held. The member's payout
/// lands on top of the delegation afterwards. One SOL clears the stake program's
/// minimum delegation with room to spare.
const PRE_STAKED: u64 = 1_000_000_000;

// ---------------------------------------------------------------------------
// The answers, pinned
//
// Produced by the tests below and written back here. A fourth account per spend,
// or a stake delegation that grows a slot, must fail a test rather than quietly
// shrink the crowd a single settlement can carry.
// ---------------------------------------------------------------------------

/// The plain-transfer ceiling, recomputed here from the encoder and cross-checked
/// against the executed measurement in `batch_ceiling.rs`.
const TRANSFER_CEILING: usize = 10;
const TRANSFER_BYTES: usize = 1229;

/// A batch of members all delegating to the **same** validator. Per spend the
/// distinct keys are the record, the stake account and the relay; the vote
/// account, the two sysvars, the config account, the stake program and the pool's
/// vault are named once for the whole batch.
const SAME_VALIDATOR_CEILING: usize = 7;
const SAME_VALIDATOR_BYTES: usize = 1141;

/// The same delegation where every member picked their own validator. One more
/// distinct key per spend, and the crowd loses a member.
const DIFFERENT_VALIDATOR_CEILING: usize = 6;
const DIFFERENT_VALIDATOR_BYTES: usize = 1195;

fn key(s: &str) -> Pubkey {
    s.parse().expect("a valid base58 address")
}

fn program_bytes() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/sbf/deploy/mirror_pool_program.so"
    );
    std::fs::read(path).unwrap_or_else(|e| {
        panic!("could not read {path}: {e}\n\nRun `make build-sbf` first.");
    })
}

// ---------------------------------------------------------------------------
// The wire
// ---------------------------------------------------------------------------

/// The three accounts every settlement names before the batch begins.
///
/// Split out from the SVM because transaction size is a pure function of the
/// message, so the sweeps below cost nothing to run and the real settlements can
/// be checked against them.
#[derive(Clone, Copy)]
struct Wire {
    program_id: Pubkey,
    pool: Pubkey,
    vault: Pubkey,
}

impl Wire {
    fn new(program_id: Pubkey) -> Self {
        let (pool, _) = pool_address(&program_id, DENOMINATION);
        let (vault, _) = vault_address(&program_id, &pool);
        Wire {
            program_id,
            pool,
            vault,
        }
    }
}

/// One spend in the order `settle_epoch` walks it: the record, the beneficiary
/// the proof bound, the relay that submitted it, then — for a CPI selector — the
/// callee's own program account followed by exactly the accounts the action
/// declared. The target precedes the action accounts because the handler reads it
/// first; getting that order wrong would measure a transaction the program cannot
/// execute.
struct Spend {
    record: Pubkey,
    beneficiary: Pubkey,
    relay: Pubkey,
    target: Option<Pubkey>,
    action: Vec<AccountMeta>,
}

fn settle_ix(wire: &Wire, batch: &[Spend], settler: &Pubkey) -> Instruction {
    let mut metas = vec![
        AccountMeta::new(*settler, true),
        AccountMeta::new(wire.pool, false),
        AccountMeta::new(wire.vault, false),
    ];
    for spend in batch {
        metas.push(AccountMeta::new(spend.record, false));
        metas.push(AccountMeta::new(spend.beneficiary, false));
        metas.push(AccountMeta::new(spend.relay, false));
        if let Some(target) = spend.target {
            metas.push(AccountMeta::new_readonly(target, false));
        }
        metas.extend(spend.action.iter().cloned());
    }
    Instruction::new_with_bytes(
        wire.program_id,
        &MirrorIx::SettleEpoch {
            count: batch.len() as u8,
            // This file measures the wire and the lock limit, and the sweep
            // starts below the pool's floor. Asking for the under-floor path
            // keeps the crowd rule from being what stops a batch here, so the
            // ceiling reported is the one the transaction imposes.
            allow_below_floor: true,
        }
        .pack(),
        metas,
    )
}

fn settle_tx(svm: &LiteSVM, wire: &Wire, batch: &[Spend], settler: &Keypair) -> Transaction {
    let ix = settle_ix(wire, batch, &settler.pubkey());
    let msg = Message::new(&[ix], Some(&settler.pubkey()));
    Transaction::new(&[settler], msg, svm.latest_blockhash())
}

/// What this transaction weighs on the wire.
///
/// A legacy transaction is a compact array of 64-byte signatures followed by the
/// serialized message, and `Message::serialize` is what a validator receives
/// rather than an approximation of it. The single-byte length prefix is asserted
/// rather than assumed: a second signer would move every number in this file by
/// 65 bytes without changing anything visible at the call site.
fn wire_len(tx: &Transaction) -> usize {
    assert_eq!(
        tx.signatures.len(),
        1,
        "the framing here holds for a single signature"
    );
    1 + 64 * tx.signatures.len() + tx.message.serialize().len()
}

// ---------------------------------------------------------------------------
// Shapes
// ---------------------------------------------------------------------------

/// Where the key in an action's account slot comes from, which is the only thing
/// about that slot the encoding cares about.
///
/// A slot always costs one byte in the instruction's account-index list. Whether
/// it also costs 32 bytes of key depends entirely on whether the batch has named
/// that account already — which is why `Shared`, `Vault` and `Beneficiary` are
/// distinguished from `Unique` at all.
#[derive(Clone, Copy)]
enum Slot {
    /// A key belonging to this spend alone: a second member's identical action
    /// would name a different one. Every member's own vote account, under
    /// divergent delegation.
    Unique,
    /// A key the whole batch shares. The index picks *which* shared key, so two
    /// shared slots are two accounts, not one.
    Shared(usize),
    /// The pool's vault, which the settlement has already named as its third
    /// account. Free in keys, one byte in indices.
    Vault,
    /// This spend's own beneficiary, already named. A stake delegation puts the
    /// stake account here, and it is the same account the payout goes to.
    Beneficiary,
}

/// A settlement shape: what each spend in the batch brings.
struct Shape {
    name: String,
    /// Whether the selector makes a call, and so whether each spend also names
    /// the callee's program account. Shared across a batch whose members all
    /// invoke the same program, which is the interesting case.
    calls: bool,
    action: Vec<Slot>,
}

impl Shape {
    fn transfer() -> Self {
        Shape {
            name: "transfer".into(),
            calls: false,
            action: Vec::new(),
        }
    }

    fn cpi(name: &str, action: Vec<Slot>) -> Self {
        Shape {
            name: name.into(),
            calls: true,
            action,
        }
    }

    /// The stake delegation both experiments below settle for real, in the order
    /// `DelegateStake` requires: stake account, vote account, clock, stake
    /// history, the unused config account, and the stake authority — which for a
    /// member who must never appear on chain can only be the pool's vault.
    fn delegation(name: &str, vote: Slot) -> Self {
        Shape::cpi(
            name,
            vec![
                Slot::Beneficiary,
                vote,
                Slot::Shared(1),
                Slot::Shared(2),
                Slot::Shared(3),
                Slot::Vault,
            ],
        )
    }

    /// How many keys a spend of this shape adds that no other spend shares: the
    /// record, the beneficiary and the relay, plus its unique action accounts.
    /// This is the number the ceiling actually turns on.
    fn distinct_keys_per_spend(&self) -> usize {
        3 + self
            .action
            .iter()
            .filter(|s| matches!(s, Slot::Unique))
            .count()
    }

    fn slots_per_spend(&self) -> usize {
        3 + usize::from(self.calls) + self.action.len()
    }
}

/// A batch of `n` spends of this shape, filled with placeholder keys.
///
/// Placeholders are legitimate here for the reason `batch_ceiling.rs` sets out:
/// transaction size depends on how many distinct accounts a message names, not on
/// which. What placeholders cannot check is that the shape is the one the program
/// walks — so the real settlements below assert their own wire size against the
/// prediction this makes, and a mistake in the model fails a test instead of
/// quietly biasing the answer.
fn placeholder_batch(wire: &Wire, shape: &Shape, n: usize) -> Vec<Spend> {
    let target = Pubkey::new_unique();
    let shared: Vec<Pubkey> = (0..shape.action.len().max(1))
        .map(|_| Pubkey::new_unique())
        .collect();
    (0..n)
        .map(|_| {
            let beneficiary = Pubkey::new_unique();
            let action = shape
                .action
                .iter()
                .map(|slot| match slot {
                    Slot::Unique => AccountMeta::new_readonly(Pubkey::new_unique(), false),
                    Slot::Shared(i) => AccountMeta::new_readonly(shared[*i], false),
                    Slot::Vault => AccountMeta::new_readonly(wire.vault, false),
                    Slot::Beneficiary => AccountMeta::new(beneficiary, false),
                })
                .collect();
            Spend {
                record: Pubkey::new_unique(),
                beneficiary,
                relay: Pubkey::new_unique(),
                target: shape.calls.then_some(target),
                action,
            }
        })
        .collect()
}

/// The largest batch of this shape that still fits in a packet, and its size,
/// found by serializing rather than by arithmetic.
///
/// The sweep stops at the first batch that does not fit, which is sound because
/// size is monotonic in the batch length — every extra spend adds slots and can
/// only add keys. It is also necessary: a message can name at most 256 accounts,
/// and a shape with four unique action accounts reaches that ceiling long before
/// the loop would otherwise end, which is a panic rather than a measurement.
fn ceiling_of(svm: &LiteSVM, wire: &Wire, settler: &Keypair, shape: &Shape) -> (usize, usize) {
    let mut answer = (0, 0);
    for n in 1..=64 {
        let batch = placeholder_batch(wire, shape, n);
        let bytes = wire_len(&settle_tx(svm, wire, &batch, settler));
        if bytes > PACKET_DATA_SIZE {
            break;
        }
        answer = (n, bytes);
    }
    answer
}

// ---------------------------------------------------------------------------
// The pool
// ---------------------------------------------------------------------------

struct Env {
    svm: LiteSVM,
    wire: Wire,
    payer: Keypair,
}

fn setup() -> Env {
    let mut svm = LiteSVM::new();
    let program_id = Pubkey::new_unique();
    svm.add_program(program_id, &program_bytes())
        .expect("loading the built program");

    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 1_000_000_000_000).unwrap();

    Env {
        svm,
        wire: Wire::new(program_id),
        payer,
    }
}

impl Env {
    fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> Result<u64, String> {
        let msg = Message::new(ixs, Some(&signers[0].pubkey()));
        let tx = Transaction::new(signers, msg, self.svm.latest_blockhash());
        self.svm
            .send_transaction(tx)
            .map(|m| m.compute_units_consumed)
            .map_err(|e| format!("{:?} | logs: {:#?}", e.err, e.meta.logs))
    }

    fn init_pool(&mut self) {
        let ix = Instruction::new_with_bytes(
            self.wire.program_id,
            &MirrorIx::InitPool {
                denomination: DENOMINATION,
                entry_fee: ENTRY_FEE,
                k_floor: K_FLOOR,
                settle_timeout_seconds: 0,
            }
            .pack(),
            vec![
                AccountMeta::new(self.payer.pubkey(), true),
                AccountMeta::new(self.wire.pool, false),
                AccountMeta::new(self.wire.vault, false),
                AccountMeta::new_readonly(solana_system_interface::program::ID, false),
            ],
        );
        let payer = self.payer.insecure_clone();
        self.send(&[ix], &[&payer]).expect("init_pool");
    }

    fn deposit(&mut self, commitment: Field) {
        let depositor = Keypair::new();
        self.svm
            .airdrop(&depositor.pubkey(), DENOMINATION + 10_000_000)
            .unwrap();
        let ix = Instruction::new_with_bytes(
            self.wire.program_id,
            &MirrorIx::Deposit {
                commitment: commitment.to_bytes(),
            }
            .pack(),
            vec![
                AccountMeta::new(depositor.pubkey(), true),
                AccountMeta::new(self.wire.pool, false),
                AccountMeta::new(self.wire.vault, false),
                AccountMeta::new_readonly(solana_system_interface::program::ID, false),
            ],
        );
        self.send(&[ix], &[&depositor]).expect("deposit");
    }

    fn spend_count(&self) -> u64 {
        let mut data = self
            .svm
            .get_account(&self.wire.pool)
            .expect("pool exists")
            .data;
        Pool::load(&mut data).unwrap().spend_count()
    }

    /// A vote account made by the real vote program, so the stake program is
    /// handed genuine state rather than bytes this test guessed at.
    fn make_vote_account(&mut self) -> Pubkey {
        let vote = Keypair::new();
        let node = Keypair::new();
        let lamports = self.svm.minimum_balance_for_rent_exemption(VOTE_STATE_LEN);
        let create = solana_system_interface::instruction::create_account(
            &self.payer.pubkey(),
            &vote.pubkey(),
            lamports,
            VOTE_STATE_LEN as u64,
            &key(VOTE_PROGRAM),
        );
        // `VoteInstruction::InitializeAccount(VoteInit)`: a four-byte variant
        // index of zero, then the node identity, the authorized voter, the
        // authorized withdrawer and a one-byte commission. Hand-encoded because
        // the vote interface crate is not a dependency of this program and the
        // encoding is four fields long.
        let mut data = Vec::with_capacity(101);
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&node.pubkey().to_bytes());
        data.extend_from_slice(&node.pubkey().to_bytes());
        data.extend_from_slice(&node.pubkey().to_bytes());
        data.push(0);
        let init = Instruction::new_with_bytes(
            key(VOTE_PROGRAM),
            &data,
            vec![
                AccountMeta::new(vote.pubkey(), false),
                AccountMeta::new_readonly(key(RENT_SYSVAR), false),
                AccountMeta::new_readonly(key(CLOCK_SYSVAR), false),
                AccountMeta::new_readonly(node.pubkey(), true),
            ],
        );
        let payer = self.payer.insecure_clone();
        self.send(&[create, init], &[&payer, &vote, &node])
            .expect("initialising a vote account");
        vote.pubkey()
    }

    /// A stake account whose authority is the pool's vault, created and
    /// initialised by the real stake program.
    ///
    /// The keypair is supplied by the caller because the stake account is the
    /// spend's beneficiary and therefore inside the member's proof: the same
    /// address has to exist in every pool a given proof is replayed into.
    fn make_stake_account(&mut self, stake: &Keypair) {
        let lamports = self.svm.minimum_balance_for_rent_exemption(STAKE_STATE_LEN) + PRE_STAKED;
        let create = solana_system_interface::instruction::create_account(
            &self.payer.pubkey(),
            &stake.pubkey(),
            lamports,
            STAKE_STATE_LEN as u64,
            &key(STAKE_PROGRAM),
        );
        // `StakeInstruction::Initialize(Authorized, Lockup)`: variant index zero,
        // then staker and withdrawer, then a lockup of no timestamp, no epoch and
        // no custodian. Both authorities are the vault, because the member never
        // appears on chain and nobody else may move this stake.
        let mut data = Vec::with_capacity(116);
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&self.wire.vault.to_bytes());
        data.extend_from_slice(&self.wire.vault.to_bytes());
        data.extend_from_slice(&0i64.to_le_bytes());
        data.extend_from_slice(&0u64.to_le_bytes());
        data.extend_from_slice(&[0u8; 32]);
        let init = Instruction::new_with_bytes(
            key(STAKE_PROGRAM),
            &data,
            vec![
                AccountMeta::new(stake.pubkey(), false),
                AccountMeta::new_readonly(key(RENT_SYSVAR), false),
            ],
        );
        let payer = self.payer.insecure_clone();
        self.send(&[create, init], &[&payer, stake])
            .expect("initialising a stake account");
    }

    /// True once the stake program has written a delegation into the account.
    ///
    /// `StakeStateV2` is a bincode enum and variant two is `Stake`, so the first
    /// four bytes are the whole test. Reading the discriminant rather than
    /// trusting the transaction's success means a settlement that returned Ok
    /// without the CPI actually landing cannot pass.
    fn is_delegated(&self, stake: &Pubkey) -> bool {
        self.svm
            .get_account(stake)
            .map(|a| a.data[..4] == [2, 0, 0, 0])
            .unwrap_or(false)
    }
}

/// A member's proof and the stake account it was bound to.
///
/// The binding covers the selector, the target program, the beneficiary, the
/// relay fee, the declared account count and the payload — and nothing else. The
/// vote account is a settler-supplied slot, so the *same* proof serves both
/// experiments below: agreeing on a validator and diverging over one are the same
/// spend as far as the member's commitment is concerned, which is precisely why
/// the cost of diverging is measured in keys rather than in proofs.
struct Ticket {
    proof: SolanaProof,
    stake: Keypair,
    relay: Keypair,
}

fn host_tree(count: usize) -> (MerkleTree, Vec<Note>) {
    let denom_tag = Field::from_u64(DENOMINATION);
    let mut tree = MerkleTree::new().unwrap();
    let mut notes = Vec::new();
    for i in 1..=count as u64 {
        let note = Note::new(
            Field::from_u64(i * 1_000_003),
            Field::from_u64(i * 7_919),
            denom_tag,
        );
        tree.insert(note.commitment().unwrap()).unwrap();
        notes.push(note);
    }
    (tree, notes)
}

fn prove_delegations(keys: &Keys, tree: &MerkleTree, notes: &[Note]) -> Vec<Ticket> {
    use ark_std::rand::SeedableRng;
    notes
        .iter()
        .enumerate()
        .map(|(index, note)| {
            let stake = Keypair::new();
            // Minted before the binding, not after: the relay is part of the
            // preimage now, so the proof has to know which key will carry it.
            let relay = Keypair::new();
            let merkle_proof = tree.proof(index as u64).unwrap();
            let binding = mirror_core::action_binding(
                INVOKE_SIGNED,
                &key(STAKE_PROGRAM).to_bytes(),
                &stake.pubkey().to_bytes(),
                &relay.pubkey().to_bytes(),
                RELAY_FEE,
                6,
                &DELEGATE_STAKE,
            );
            let witness = Witness {
                note: *note,
                merkle_proof: &merkle_proof,
                root: tree.root().unwrap(),
                action_binding: binding,
            };
            let mut rng = ark_std::rand::rngs::StdRng::from_seed([index as u8; 32]);
            Ticket {
                proof: prove(keys, &witness, &mut rng).expect("proving"),
                stake,
                relay,
            }
        })
        .collect()
}

/// One spend of the delegation shape, ready for settlement.
fn delegation_spend(wire: &Wire, ticket: &Ticket, vote: &Pubkey) -> Spend {
    let nullifier = ticket.proof.public_inputs[1];
    let (record, _) = spend_address(&wire.program_id, &wire.pool, &nullifier);
    Spend {
        record,
        beneficiary: ticket.stake.pubkey(),
        relay: ticket.relay.pubkey(),
        target: Some(key(STAKE_PROGRAM)),
        action: vec![
            AccountMeta::new(ticket.stake.pubkey(), false),
            AccountMeta::new_readonly(*vote, false),
            AccountMeta::new_readonly(key(CLOCK_SYSVAR), false),
            AccountMeta::new_readonly(key(STAKE_HISTORY_SYSVAR), false),
            AccountMeta::new_readonly(key(STAKE_CONFIG), false),
            // Not marked a signer here, and it cannot be: a PDA has no key. The
            // signature comes from `invoke_signed` inside the program, from seeds
            // only the program holds.
            AccountMeta::new_readonly(wire.vault, false),
        ],
    }
}

/// A fresh pool holding `notes`, with every delegation submitted and pending, and
/// with the stake accounts the proofs named already initialised.
///
/// `distinct_votes` is the entire difference between the two experiments: with it
/// false the crowd agrees on one validator, with it true every member brings their
/// own. Everything else — the notes, the proofs, the payload, the stake accounts —
/// is identical, which is what makes the two ceilings comparable at all.
fn pending_pool(notes: &[Note], tickets: &[Ticket], distinct_votes: bool) -> (Env, Vec<Spend>) {
    let mut env = setup();
    env.init_pool();
    for note in notes {
        env.deposit(note.commitment().unwrap());
    }

    let shared_vote = (!distinct_votes).then(|| env.make_vote_account());
    let mut spends = Vec::new();
    for (i, ticket) in tickets.iter().enumerate() {
        env.make_stake_account(&ticket.stake);
        env.svm
            .airdrop(&ticket.relay.pubkey(), 10_000_000_000)
            .unwrap();

        let nullifier = ticket.proof.public_inputs[1];
        let (record, _) = spend_address(&env.wire.program_id, &env.wire.pool, &nullifier);
        let ix = Instruction::new_with_bytes(
            env.wire.program_id,
            &MirrorIx::SubmitSpend {
                proof_a: ticket.proof.proof_a,
                proof_b: ticket.proof.proof_b,
                proof_c: ticket.proof.proof_c,
                root: ticket.proof.public_inputs[0],
                nullifier,
                selector: INVOKE_SIGNED,
                target_program: key(STAKE_PROGRAM).to_bytes(),
                beneficiary: ticket.stake.pubkey().to_bytes(),
                relay_fee: RELAY_FEE,
                action_accounts: 6,
                payload: DELEGATE_STAKE.to_vec(),
            }
            .pack(),
            vec![
                AccountMeta::new(ticket.relay.pubkey(), true),
                AccountMeta::new(env.wire.pool, false),
                AccountMeta::new(record, false),
                AccountMeta::new_readonly(solana_system_interface::program::ID, false),
            ],
        );
        let relay = ticket.relay.insecure_clone();
        env.send(&[ix], &[&relay])
            .unwrap_or_else(|e| panic!("submit {i} failed: {e}"));

        let vote = match shared_vote {
            Some(vote) => vote,
            None => env.make_vote_account(),
        };
        spends.push(delegation_spend(&env.wire, ticket, &vote));
    }
    (env, spends)
}

// ---------------------------------------------------------------------------
// The measurements
// ---------------------------------------------------------------------------

/// The curve, as a table, before anything is settled.
///
/// Nothing in *this* test executes: it is the encoding alone, which is free, and
/// that is what makes it worth printing across the whole range rather than at the
/// two points the executed test can afford. Every row says where its number comes
/// from, because "size only" and "settled" are not the same claim and a table
/// that blurs them is the kind of thing this repository does not publish.
///
/// The transfer row is the control. It has to reproduce the ceiling
/// `batch_ceiling.rs` reached by settling real spends; if it does not, this
/// file's model of the wire is wrong and nothing else it prints can be trusted.
#[test]
fn every_account_an_action_names_costs_the_batch_a_member() {
    let svm = LiteSVM::new();
    let wire = Wire::new(Pubkey::new_unique());
    let settler = Keypair::new();

    let row = |shape: &Shape, taken: &str| {
        let (ceiling, bytes) = ceiling_of(&svm, &wire, &settler, shape);
        assert!(ceiling > 0, "{} does not fit at all", shape.name);
        println!(
            "  {:<28} {:>5} {:>5} {:>8} {:>6}   {taken}",
            shape.name,
            shape.slots_per_spend(),
            shape.distinct_keys_per_spend(),
            ceiling,
            bytes
        );
        (ceiling, bytes)
    };

    println!("\n  shape                        slots  keys  ceiling  bytes   taken by");
    let transfer = row(&Shape::transfer(), "size; settled in batch_ceiling.rs");
    let bare_call = row(&Shape::cpi("call, no accounts", Vec::new()), "size only");

    // The two curves the table exists to separate: the same action account
    // *count* costing shared keys, and costing a key per member. Only the second
    // curve is a statement about anonymity — the first is what a batch pays for
    // making a call at all.
    let mut shared_curve = vec![(0usize, bare_call)];
    let mut unique_curve = Vec::new();
    for k in [1usize, 2, 4, 6, 8] {
        let shared = row(
            &Shape::cpi(
                &format!("call, {k} shared"),
                (0..k).map(Slot::Shared).collect(),
            ),
            "size only",
        );
        let unique = row(
            &Shape::cpi(
                &format!("call, {k} per member"),
                (0..k).map(|_| Slot::Unique).collect(),
            ),
            "size only",
        );
        assert!(
            unique.0 < shared.0,
            "with {k} action accounts, a batch whose members name their own \
             accounts carried {} members and a batch that shares them carried \
             {}: naming a distinct account has stopped costing anything, which \
             the encoding cannot do",
            unique.0,
            shared.0
        );
        shared_curve.push((k, shared));
        unique_curve.push((k, unique));
    }

    let same = row(
        &Shape::delegation("delegate, one validator", Slot::Shared(0)),
        "size; settled below",
    );
    let diverging = row(
        &Shape::delegation("delegate, a validator each", Slot::Unique),
        "size; settled below",
    );
    println!();

    assert_eq!(
        transfer,
        (TRANSFER_CEILING, TRANSFER_BYTES),
        "the transfer row no longer reproduces the executed measurement in \
         batch_ceiling.rs, so this file's encoding of a settlement is wrong and \
         every other number it prints is unsafe"
    );
    assert_eq!(
        same,
        (SAME_VALIDATOR_CEILING, SAME_VALIDATOR_BYTES),
        "the ceiling for a batch delegating to one validator moved"
    );
    assert_eq!(
        diverging,
        (DIFFERENT_VALIDATOR_CEILING, DIFFERENT_VALIDATOR_BYTES),
        "the ceiling for a batch delegating to a validator each moved"
    );

    // Both curves have to fall. An action that needs more accounts can never
    // carry more members than one that needs fewer, whoever the accounts belong
    // to; if that ever stops holding, the model of the wire above is wrong.
    for curve in [&shared_curve, &unique_curve] {
        for pair in curve.windows(2) {
            let [(smaller, (more, _)), (larger, (fewer, _))] = pair else {
                unreachable!("windows(2) yields pairs")
            };
            assert!(
                fewer <= more,
                "{larger} action accounts carried {fewer} members and {smaller} \
                 carried {more}, so a larger action bought a larger crowd"
            );
        }
    }
}

/// The result, settled rather than serialized.
///
/// Two crowds of members ask the pool to delegate stake on their behalf. In the
/// first every member picked the same validator; in the second every member
/// picked their own. Nothing else differs — not the selector, not the payload,
/// not the proofs, which are literally the same proofs replayed into a second
/// pool, because the vote account is a settlement slot rather than something the
/// member committed to.
///
/// The agreeing crowd fits one more member. That is the whole finding: an
/// anonymity set is bounded by how much its members' actions have in common, and
/// on Solana the bound is mechanical rather than statistical — a batch that names
/// more distinct accounts does not fit in a packet, whatever the members wanted.
///
/// Both ceilings are executed against the real `.so` and the real stake program,
/// and both are checked against the byte count the placeholder sweep predicted,
/// so a mistake in the model above fails here rather than passing quietly. The
/// batch one larger than each ceiling is then settled in a fresh pool to show
/// what actually stops it: the SVM takes it, so the packet is the constraint and
/// compute is not close.
#[test]
fn a_crowd_that_agrees_on_its_validator_carries_one_more_member() {
    let probe_svm = LiteSVM::new();
    let probe_wire = Wire::new(Pubkey::new_unique());
    let prober = Keypair::new();
    let same_shape = Shape::delegation("same", Slot::Shared(0));
    let diverging_shape = Shape::delegation("diverging", Slot::Unique);
    let (same_bracket, _) = ceiling_of(&probe_svm, &probe_wire, &prober, &same_shape);
    let (diverging_bracket, _) = ceiling_of(&probe_svm, &probe_wire, &prober, &diverging_shape);
    println!(
        "the encoding admits {same_bracket} delegations to one validator and \
         {diverging_bracket} to a validator each; proving one more than the larger"
    );

    let keys = generate_reproducible(SEED).expect("setup");
    let (tree, notes) = host_tree(same_bracket.max(diverging_bracket) + 1);
    let tickets = prove_delegations(&keys, &tree, &notes);

    let same = settle_at_ceiling(&notes, &tickets, false, "one validator");
    let diverging = settle_at_ceiling(&notes, &tickets, true, "a validator each");

    assert_eq!(
        (same.0, same.1),
        (SAME_VALIDATOR_CEILING, SAME_VALIDATOR_BYTES),
        "the executed ceiling for a batch delegating to one validator moved"
    );
    assert_eq!(
        (diverging.0, diverging.1),
        (DIFFERENT_VALIDATOR_CEILING, DIFFERENT_VALIDATOR_BYTES),
        "the executed ceiling for a batch delegating to a validator each moved"
    );
    assert!(
        same.0 > diverging.0,
        "agreeing on a validator no longer buys the crowd a member: {} against {}",
        same.0,
        diverging.0
    );
    println!(
        "\n  a crowd that agrees on its validator settles {} members; a crowd that \
         diverges settles {}. The divergent batch names {} distinct keys per spend \
         instead of {}, and the extra key is the whole difference.\n",
        same.0,
        diverging.0,
        diverging_shape.distinct_keys_per_spend(),
        same_shape.distinct_keys_per_spend()
    );
}

/// Settles the largest batch that fits, and then demonstrates what stops the next
/// one. Returns the ceiling and its wire size.
///
/// The shape is derived from `distinct_votes` rather than passed alongside it, so
/// the batch this settles and the batch the placeholder sweep models cannot drift
/// apart. Every real batch is weighed against its placeholder twin at the same
/// length: that is what turns the free sweep from an assumption into a checked
/// prediction, and it is the only thing standing between this file and a table of
/// numbers about a transaction the program could not execute.
///
/// The sweep runs *downward* from the bracket for the same reason
/// `batch_ceiling.rs` does: a rejected settlement leaves its records pending, so
/// one pool can be asked many times, but an accepted one consumes them, so the
/// first acceptance has to be the answer.
fn settle_at_ceiling(
    notes: &[Note],
    tickets: &[Ticket],
    distinct_votes: bool,
    label: &str,
) -> (usize, usize) {
    let shape = Shape::delegation(
        label,
        if distinct_votes {
            Slot::Unique
        } else {
            Slot::Shared(0)
        },
    );
    let (mut env, pending) = pending_pool(notes, tickets, distinct_votes);
    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();
    let (bracket, _) = ceiling_of(&env.svm, &env.wire, &settler, &shape);

    let mut ceiling = 0usize;
    let mut bytes_at_ceiling = 0usize;
    let mut cu_at_ceiling = 0u64;
    for n in (1..=bracket).rev() {
        let tx = settle_tx(&env.svm, &env.wire, &pending[..n], &settler);
        let bytes = wire_len(&tx);
        let predicted = wire_len(&settle_tx(
            &env.svm,
            &env.wire,
            &placeholder_batch(&env.wire, &shape, n),
            &settler,
        ));
        assert_eq!(
            bytes, predicted,
            "the placeholder model of {label} weighs a batch of {n} differently \
             from the real one, so the table this file prints describes a \
             settlement that does not exist"
        );
        assert!(
            bytes <= PACKET_DATA_SIZE,
            "the placeholder bracket disagreed with a real batch of {n}: {bytes} bytes"
        );
        match env.svm.send_transaction(tx) {
            Ok(meta) => {
                ceiling = n;
                bytes_at_ceiling = bytes;
                cu_at_ceiling = meta.compute_units_consumed;
                break;
            }
            Err(e) => println!(
                "{n} delegations fit in {bytes} bytes but failed: {:?}",
                e.err
            ),
        }
    }
    assert!(ceiling > 0, "no batch of delegations settled at all");
    assert_eq!(
        env.spend_count(),
        ceiling as u64,
        "the settlement that succeeded did not settle the whole batch"
    );
    // The transaction succeeding is not evidence that the delegations happened;
    // the stake accounts saying so is.
    for spend in &pending[..ceiling] {
        assert!(
            env.is_delegated(&spend.beneficiary),
            "settlement returned Ok but {} was never delegated",
            spend.beneficiary
        );
    }
    println!(
        "settled {ceiling} real stake delegations ({label}) in one transaction: \
         {bytes_at_ceiling} bytes ({} to spare), {cu_at_ceiling} CU of \
         {DEFAULT_COMPUTE_BUDGET}",
        PACKET_DATA_SIZE - bytes_at_ceiling
    );
    assert!(
        cu_at_ceiling < DEFAULT_COMPUTE_BUDGET,
        "a full settlement used {cu_at_ceiling} CU, over the default budget"
    );

    // Why one more fails, and it takes two judges. litesvm never sees a packet,
    // so it will happily execute a batch no validator would accept; the wire
    // format never sees a compute meter. The same batch of `ceiling + 1` is
    // measured on the wire and replayed into a second, identical pool where the
    // SVM rules on it.
    let over = ceiling + 1;
    let (mut second, again) = pending_pool(notes, tickets, distinct_votes);
    let other = Keypair::new();
    second.svm.airdrop(&other.pubkey(), 10_000_000_000).unwrap();
    let tx = settle_tx(&second.svm, &second.wire, &again[..over], &other);
    let bytes_over = wire_len(&tx);
    assert_eq!(
        bytes_over,
        wire_len(&settle_tx(
            &second.svm,
            &second.wire,
            &placeholder_batch(&second.wire, &shape, over),
            &other,
        )),
        "the placeholder model and the real batch disagree at {over} spends"
    );
    let verdict = second.svm.send_transaction(tx);

    assert!(
        bytes_over > PACKET_DATA_SIZE,
        "{over} delegations fit in {bytes_over} bytes, so the packet is not what \
         stops them and the sweep above stopped one short"
    );
    let Ok(over_meta) = &verdict else {
        panic!(
            "the SVM refused {over} delegations, so the packet limit is not the \
             only thing in the way and this ceiling reports the wrong constraint"
        )
    };
    assert!(
        over_meta.compute_units_consumed < DEFAULT_COMPUTE_BUDGET,
        "settling {over} delegations cost {} CU, at or over the default budget, so \
         compute now binds alongside the packet",
        over_meta.compute_units_consumed
    );
    println!(
        "{over} delegations ({label}): rejected by the wire at {bytes_over} bytes, \
         {} over the {PACKET_DATA_SIZE}-byte limit — while the SVM settled the same \
         batch in {} CU, so compute is not the constraint",
        bytes_over - PACKET_DATA_SIZE,
        over_meta.compute_units_consumed
    );

    (ceiling, bytes_at_ceiling)
}
