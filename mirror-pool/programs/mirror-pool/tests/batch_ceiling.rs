//! How many spends actually fit in one settlement, measured against the compiled
//! program.
//!
//! `settle_epoch` takes a count and reads three accounts per spend, and every
//! sentence in this repository that says "batch" is quietly assuming that count
//! can be large. Two unrelated things bound it: the 1232-byte packet a
//! transaction has to fit inside, and the compute budget the instruction runs
//! under. Only one of them binds, and which one it is changes what a settler
//! should do about it — ask for more compute, or send more transactions.
//!
//! So the number is taken rather than argued. Nothing below is extrapolated from
//! a smaller batch: the ceiling is found by settling real spends against the real
//! `.so` in litesvm, and both limits are reported at the answer.
//!
//! **This measures a *legacy* transaction, and that scoping is the whole
//! meaning of the number.** A legacy message names every account by its full 32
//! bytes, which is what makes a member cost ~99 bytes and what puts the wall at
//! ten. A v0 message may instead resolve accounts through an address lookup
//! table published on chain, at one byte per account — the packet then stops
//! binding entirely, and something else takes over. `crates/mirror-cli/src/
//! lookup.rs` is that path, and `mirror settle` takes it automatically for any
//! batch this test would refuse. Ten is the floor a settler gets with no setup
//! at all, not the most the program can do.
//!
//! Requires `make build-sbf` first, like the end-to-end suite. This is a
//! measurement of the deployed artefact, not of the host crate, so a missing
//! artefact says so rather than quietly measuring nothing.

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
const SELECTOR: u64 = mirror_pool_program::processor::SELECTOR_TRANSFER;

/// The 1280-byte IPv6 minimum MTU, less a 40-byte IPv6 header and an 8-byte
/// fragment header. Restated here because no crate this test already depends on
/// re-exports it; `solana_packet::PACKET_DATA_SIZE` is the original, and the
/// derivation is written out so a reader can check the number rather than trust
/// it.
const PACKET_DATA_SIZE: usize = 1280 - 40 - 8;

/// What a transaction carrying a single non-budget instruction is given unless it
/// asks for more. Asking costs a second instruction, and a second instruction
/// costs bytes this measurement has none to spare — which is the whole reason the
/// two limits have to be reported together rather than one at a time.
const DEFAULT_COMPUTE_BUDGET: u64 = 200_000;

// ---------------------------------------------------------------------------
// The answers, pinned
//
// Produced by the test at the bottom of this file and written back here, so that
// a change to the account shape of `settle_epoch` — a fourth account per spend,
// a signer added to the batch — fails a test instead of silently halving what a
// settler can carry.
// ---------------------------------------------------------------------------

/// The largest batch that both fits in a packet and executes.
const MAX_SPENDS_PER_SETTLEMENT: usize = 10;
/// The wire size of that settlement. There is no room for an eleventh spend and
/// not much room for anything else either.
///
/// It was 1228 until `settle_epoch` gained the below-floor flag, which costs one
/// byte of instruction data. The *ceiling* did not move — a spend costs three
/// account keys, ninety-six bytes, so one byte was never going to buy or lose a
/// member — and that separation is why both numbers are pinned rather than one:
/// this figure moving alone means the encoding changed, and the other moving
/// means the crowd did.
const BYTES_AT_MAX: usize = 1229;

fn program_bytes() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/sbf/deploy/mirror_pool_program.so"
    );
    std::fs::read(path).unwrap_or_else(|e| {
        panic!("could not read {path}: {e}\n\nRun `make build-sbf` first.");
    })
}

struct Env {
    svm: LiteSVM,
    program_id: Pubkey,
    payer: Keypair,
    pool: Pubkey,
    vault: Pubkey,
}

fn setup() -> Env {
    let mut svm = LiteSVM::new();
    let program_id = Pubkey::new_unique();
    svm.add_program(program_id, &program_bytes())
        .expect("loading the built program");

    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 100_000_000_000).unwrap();

    let (pool, _) = pool_address(&program_id, DENOMINATION);
    let (vault, _) = vault_address(&program_id, &pool);

    Env {
        svm,
        program_id,
        payer,
        pool,
        vault,
    }
}

impl Env {
    fn send(&mut self, ix: Instruction, signer: &Keypair) -> Result<(), String> {
        let msg = Message::new(&[ix], Some(&signer.pubkey()));
        let tx = Transaction::new(&[signer], msg, self.svm.latest_blockhash());
        self.svm
            .send_transaction(tx)
            .map(|_| ())
            .map_err(|e| format!("{:?}", e.err))
    }

    fn init_pool(&mut self) {
        let ix = Instruction::new_with_bytes(
            self.program_id,
            &MirrorIx::InitPool {
                denomination: DENOMINATION,
                entry_fee: ENTRY_FEE,
                k_floor: K_FLOOR,
                settle_timeout_seconds: 0,
            }
            .pack(),
            vec![
                AccountMeta::new(self.payer.pubkey(), true),
                AccountMeta::new(self.pool, false),
                AccountMeta::new(self.vault, false),
                AccountMeta::new_readonly(solana_system_interface::program::ID, false),
            ],
        );
        let payer = self.payer.insecure_clone();
        self.send(ix, &payer).expect("init_pool");
    }

    fn deposit(&mut self, commitment: Field, depositor: &Keypair) -> Result<(), String> {
        let ix = Instruction::new_with_bytes(
            self.program_id,
            &MirrorIx::Deposit {
                commitment: commitment.to_bytes(),
            }
            .pack(),
            vec![
                AccountMeta::new(depositor.pubkey(), true),
                AccountMeta::new(self.pool, false),
                AccountMeta::new(self.vault, false),
                AccountMeta::new_readonly(solana_system_interface::program::ID, false),
            ],
        );
        self.send(ix, depositor)
    }

    fn spend_count(&self) -> u64 {
        let mut data = self.svm.get_account(&self.pool).expect("pool exists").data;
        Pool::load(&mut data).unwrap().spend_count()
    }
}

/// One spend as settlement sees it: the record, the beneficiary the proof bound,
/// and the relay that submitted it. Three accounts, which is the unit this whole
/// measurement is denominated in.
type Party = (Pubkey, Pubkey, Pubkey);

fn spend_ix(
    env: &Env,
    proof: &SolanaProof,
    nullifier: [u8; 32],
    beneficiary: &Pubkey,
    relay: &Pubkey,
) -> Instruction {
    let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);
    Instruction::new_with_bytes(
        env.program_id,
        &MirrorIx::SubmitSpend {
            proof_a: proof.proof_a,
            proof_b: proof.proof_b,
            proof_c: proof.proof_c,
            root: proof.public_inputs[0],
            nullifier,
            selector: SELECTOR,
            target_program: [0u8; 32],
            beneficiary: beneficiary.to_bytes(),
            relay_fee: RELAY_FEE,
            action_accounts: 0,
            payload: Vec::new(),
        }
        .pack(),
        vec![
            AccountMeta::new(*relay, true),
            AccountMeta::new(env.pool, false),
            AccountMeta::new(spend_pda, false),
            AccountMeta::new_readonly(solana_system_interface::program::ID, false),
        ],
    )
}

fn settle_ix(env: &Env, batch: &[Party], settler: &Pubkey) -> Instruction {
    let mut metas = vec![
        AccountMeta::new(*settler, true),
        AccountMeta::new(env.pool, false),
        AccountMeta::new(env.vault, false),
    ];
    for (spend, beneficiary, relay) in batch {
        metas.push(AccountMeta::new(*spend, false));
        metas.push(AccountMeta::new(*beneficiary, false));
        metas.push(AccountMeta::new(*relay, false));
    }
    Instruction::new_with_bytes(
        env.program_id,
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

fn settle_tx(env: &Env, batch: &[Party], settler: &Keypair) -> Transaction {
    let ix = settle_ix(env, batch, &settler.pubkey());
    let msg = Message::new(&[ix], Some(&settler.pubkey()));
    Transaction::new(&[settler], msg, env.svm.latest_blockhash())
}

/// What this transaction weighs on the wire.
///
/// A legacy transaction is a compact array of 64-byte signatures followed by the
/// serialized message, and `Message::serialize` is the encoding a validator
/// receives, not an approximation of it. The one-byte length prefix is asserted
/// rather than assumed: a second signer would move the answer by 65 bytes without
/// changing anything visible at the call site.
fn wire_len(tx: &Transaction) -> usize {
    assert_eq!(
        tx.signatures.len(),
        1,
        "the framing here holds for a single signature"
    );
    1 + 64 * tx.signatures.len() + tx.message.serialize().len()
}

/// The largest batch whose transaction still fits in a packet, found by
/// serializing rather than by arithmetic.
///
/// Placeholder keys are legitimate here and only here: transaction size depends
/// on how many distinct accounts the message names, not on which. The real batch
/// is checked against this bracket below, so an error in that reasoning fails the
/// test rather than biasing it.
fn largest_batch_that_fits(env: &Env, settler: &Keypair) -> usize {
    let mut fits = 0;
    for n in 1..=64 {
        let placeholders: Vec<Party> = (0..n)
            .map(|_| {
                (
                    Pubkey::new_unique(),
                    Pubkey::new_unique(),
                    Pubkey::new_unique(),
                )
            })
            .collect();
        if wire_len(&settle_tx(env, &placeholders, settler)) <= PACKET_DATA_SIZE {
            fits = n;
        }
    }
    fits
}

/// A member's proof and the two addresses it was bound to.
///
/// The binding covers the selector, the target, the beneficiary, the relay, the
/// relay fee and the payload — but *not* the pool. So one round of proving can be
/// replayed into a second, identical pool, which is what lets this file run two
/// independent settlement experiments for the price of one.
///
/// That the relay is bound is why it is carried here rather than minted at
/// submission time: both runs must present the same relay, because a second
/// relay would need a second proof.
struct Ticket {
    proof: SolanaProof,
    beneficiary: Pubkey,
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

fn prove_tickets(keys: &Keys, tree: &MerkleTree, notes: &[Note]) -> Vec<Ticket> {
    use ark_std::rand::SeedableRng;
    notes
        .iter()
        .enumerate()
        .map(|(index, note)| {
            let beneficiary = Pubkey::new_unique();
            // Minted before the binding, not after: the relay is part of the
            // preimage now, so the proof has to know which key will carry it.
            let relay = Keypair::new();
            let merkle_proof = tree.proof(index as u64).unwrap();
            let binding = mirror_core::action_binding(
                SELECTOR,
                &[0u8; 32],
                &beneficiary.to_bytes(),
                &relay.pubkey().to_bytes(),
                RELAY_FEE,
                0,
                &[],
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
                beneficiary,
                relay,
            }
        })
        .collect()
}

/// A fresh pool holding `notes`, with every ticket submitted and pending.
fn pending_pool(notes: &[Note], tickets: &[Ticket]) -> (Env, Vec<Party>) {
    let mut env = setup();
    env.init_pool();

    for (i, note) in notes.iter().enumerate() {
        let depositor = Keypair::new();
        env.svm
            .airdrop(&depositor.pubkey(), DENOMINATION + 10_000_000)
            .unwrap();
        env.deposit(note.commitment().unwrap(), &depositor)
            .unwrap_or_else(|e| panic!("deposit {i} failed: {e}"));
    }

    let mut pending = Vec::new();
    for (i, ticket) in tickets.iter().enumerate() {
        env.svm
            .airdrop(&ticket.relay.pubkey(), 10_000_000_000)
            .unwrap();
        let nullifier = ticket.proof.public_inputs[1];
        let ix = spend_ix(
            &env,
            &ticket.proof,
            nullifier,
            &ticket.beneficiary,
            &ticket.relay.pubkey(),
        );
        let relay = ticket.relay.insecure_clone();
        env.send(ix, &relay)
            .unwrap_or_else(|e| panic!("submit {i} failed: {e}"));
        let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);
        pending.push((spend_pda, ticket.beneficiary, ticket.relay.pubkey()));
    }
    (env, pending)
}

/// The measurement.
///
/// Bracketing, because a Groth16 proof takes about four seconds and sweeping
/// upward from one spend would spend a minute rediscovering that small batches
/// are small. Transaction size is a pure function of the account count, so the
/// region worth proving for is found first by serializing placeholder batches —
/// free, and cross-checked against the real batch inside the sweep. Only then are
/// proofs made, one more than the bracket admits, and the real sweep runs
/// *downward* from the bracket: a settlement that is rejected leaves its records
/// pending, so one pool can be asked many times, but a settlement that succeeds
/// consumes them, so the first success has to be the answer.
///
/// Every spend here brings three accounts no other spend shares. That is the
/// worst case for the encoding and it is the case a settler has to plan against;
/// a batch whose members happened to share a relay would name fewer distinct keys
/// and is a different measurement.
#[test]
fn ten_spends_fit_in_one_settlement_and_the_packet_is_what_stops_the_eleventh() {
    let probe = setup();
    let prober = Keypair::new();
    let bracket = largest_batch_that_fits(&probe, &prober);
    println!("the encoding admits {bracket} spends; proving one more than that");

    let keys = generate_reproducible(SEED).expect("setup");
    let (tree, notes) = host_tree(bracket + 1);
    let tickets = prove_tickets(&keys, &tree, &notes);

    let (mut env, pending) = pending_pool(&notes, &tickets);
    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();

    let mut ceiling = 0usize;
    let mut bytes_at_ceiling = 0usize;
    let mut cu_at_ceiling = 0u64;
    for n in (1..=bracket).rev() {
        let tx = settle_tx(&env, &pending[..n], &settler);
        let bytes = wire_len(&tx);
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
            Err(e) => println!("{n} spends fit in {bytes} bytes but failed: {:?}", e.err),
        }
    }
    assert!(ceiling > 0, "no batch settled at all");
    assert_eq!(
        env.spend_count(),
        ceiling as u64,
        "the settlement that succeeded did not settle the whole batch"
    );
    println!(
        "settled {ceiling} spends in one transaction: {bytes_at_ceiling} bytes \
         ({} to spare), {cu_at_ceiling} CU of {DEFAULT_COMPUTE_BUDGET}",
        PACKET_DATA_SIZE - bytes_at_ceiling
    );

    // Why one more fails, demonstrated rather than asserted — and it needs two
    // judges, because neither can answer alone. litesvm never sees a packet, so
    // it will happily execute a batch no validator would accept; the wire format
    // never sees a compute meter. So the same batch of `ceiling + 1` is measured
    // on the wire and replayed into a second, identical pool where the SVM gets
    // to rule on it. If the SVM settles it while the bytes are over the limit,
    // the packet is what binds and compute is not close.
    let over = ceiling + 1;
    let (mut second, again) = pending_pool(&notes, &tickets);
    let other = Keypair::new();
    second.svm.airdrop(&other.pubkey(), 10_000_000_000).unwrap();
    let tx = settle_tx(&second, &again[..over], &other);
    let bytes_over = wire_len(&tx);
    let verdict = second.svm.send_transaction(tx);

    match (&verdict, bytes_over > PACKET_DATA_SIZE) {
        (Ok(meta), true) => println!(
            "{over} spends: rejected by the wire at {bytes_over} bytes, \
             {} over the {PACKET_DATA_SIZE}-byte limit — while the SVM settled the \
             same batch in {} CU, so compute is not the constraint",
            bytes_over - PACKET_DATA_SIZE,
            meta.compute_units_consumed
        ),
        (Err(e), _) => println!(
            "{over} spends: rejected by the runtime at {bytes_over} bytes: {:?}",
            e.err
        ),
        (Ok(_), false) => panic!(
            "a batch of {over} both fits in {bytes_over} bytes and executes, so the \
             sweep above stopped one short of the real ceiling"
        ),
    }

    assert!(
        bytes_over > PACKET_DATA_SIZE,
        "{over} spends fit in {bytes_over} bytes, so the packet is not what stops them"
    );
    // Both halves of the claim are load-bearing. If the SVM ever refuses this
    // batch, the packet stops being the only thing in the way and the name of
    // this test is no longer true, whatever the ceiling turns out to be.
    let Ok(over_meta) = &verdict else {
        panic!(
            "the SVM refused {over} spends, so the packet limit is not what binds \
             and this test reports the wrong constraint"
        )
    };
    assert!(
        over_meta.compute_units_consumed < DEFAULT_COMPUTE_BUDGET,
        "settling {over} spends cost {} CU, at or over the default budget, so \
         compute now binds alongside the packet",
        over_meta.compute_units_consumed
    );

    assert_eq!(
        ceiling, MAX_SPENDS_PER_SETTLEMENT,
        "the settlement ceiling moved: {ceiling} spends per transaction, not \
         {MAX_SPENDS_PER_SETTLEMENT}"
    );
    assert_eq!(
        bytes_at_ceiling, BYTES_AT_MAX,
        "the wire size of a full settlement moved: {bytes_at_ceiling} bytes, not \
         {BYTES_AT_MAX}"
    );
    assert!(
        cu_at_ceiling < DEFAULT_COMPUTE_BUDGET,
        "a full settlement used {cu_at_ceiling} CU, over the default budget"
    );
}
