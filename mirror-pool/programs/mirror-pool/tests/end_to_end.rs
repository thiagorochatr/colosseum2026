//! The whole protocol against the real compiled program.
//!
//! Every other test in this repository calls functions directly. This one loads
//! the `.so` that `make build-sbf` produces into a real SVM, sends real
//! transactions, and verifies a real Groth16 proof through the actual syscall.
//! It is the only test that can catch a divergence between what the host
//! believes and what the chain does.
//!
//! Requires `make build-sbf` first. If the artifact is missing the test says so
//! rather than silently passing, because a privacy test that quietly does
//! nothing is worse than no test.

use litesvm::LiteSVM;
use mirror_circuit::{generate_reproducible, prove, Keys, Witness};
use mirror_core::{Field, MerkleTree, Note};
#[allow(unused_imports)]
use mirror_pool_program::{
    instruction::Instruction as MirrorIx,
    pda::{pool_address, spend_address, vault_address},
    spend::{spend_len, Spend, SPEND_BASE_LEN, STATUS_PENDING, STATUS_SETTLED},
    Pool, POOL_LEN,
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

/// The real SPL Memo program, fetched from mainnet, used as a CPI target so the
/// action path is exercised against a program that exists rather than a stub.
const MEMO_PROGRAM: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

fn memo_bytes() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/spl_memo.so");
    std::fs::read(path).expect("spl_memo.so fixture")
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
    /// Sends a transaction, treating runtime deduplication as a test bug.
    ///
    /// A resubmitted byte-identical transaction is rejected as AlreadyProcessed
    /// *before the program runs*, so any negative test that reaches that state is
    /// asserting nothing about this program. Two tests in this file passed that
    /// way before this guard existed. Vary the fee payer to make a genuine
    /// retry.
    fn send(&mut self, ix: Instruction, signer: &Keypair) -> Result<(), String> {
        let msg = Message::new(&[ix], Some(&signer.pubkey()));
        let tx = Transaction::new(&[signer], msg, self.svm.latest_blockhash());
        self.svm.send_transaction(tx).map(|_| ()).map_err(|e| {
            let rendered = format!(
                "{:?} | logs: {:?}",
                e.err,
                e.meta.logs.iter().rev().take(4).collect::<Vec<_>>()
            );
            assert!(
                !rendered.contains("AlreadyProcessed"),
                "the runtime deduplicated this transaction, so the program never \
                 ran and the test proves nothing. Vary the fee payer: {rendered}"
            );
            rendered
        })
    }

    /// Sends and hands back the metadata, for the tests whose subject is what
    /// the program *said* rather than only whether it succeeded.
    ///
    /// A log line is program output like any other: asserting it from the
    /// transaction's own metadata is what makes "settlement leaves a mark" a
    /// checked claim rather than a sentence in a document.
    fn send_meta(
        &mut self,
        ix: Instruction,
        signer: &Keypair,
    ) -> Result<litesvm::types::TransactionMetadata, String> {
        let msg = Message::new(&[ix], Some(&signer.pubkey()));
        let tx = Transaction::new(&[signer], msg, self.svm.latest_blockhash());
        self.svm
            .send_transaction(tx)
            .map_err(|e| format!("{:?} | logs: {:#?}", e.err, e.meta.logs))
    }

    fn send_expect_cu(&mut self, ix: Instruction, signer: &Keypair) -> u64 {
        let msg = Message::new(&[ix], Some(&signer.pubkey()));
        let tx = Transaction::new(&[signer], msg, self.svm.latest_blockhash());
        let meta = self.svm.send_transaction(tx).unwrap_or_else(|e| {
            panic!("transaction failed: {:?}\nlogs: {:#?}", e.err, e.meta.logs)
        });
        meta.compute_units_consumed
    }

    /// Creates the pool. Zero takes the program's default timeout, which is
    /// what every pool created before the field existed holds and what the rest
    /// of this suite exercises.
    fn init_pool_with_timeout(&mut self, settle_timeout_seconds: u32) {
        let ix = Instruction::new_with_bytes(
            self.program_id,
            &MirrorIx::InitPool {
                denomination: DENOMINATION,
                entry_fee: ENTRY_FEE,
                k_floor: K_FLOOR,
                settle_timeout_seconds,
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

    fn pool_state(&self) -> Vec<u8> {
        self.svm.get_account(&self.pool).expect("pool exists").data
    }

    fn vault_lamports(&self) -> u64 {
        self.svm
            .get_account(&self.vault)
            .map(|a| a.lamports)
            .unwrap_or(0)
    }
}

/// Builds a pool with `count` deposits and returns the host-side tree plus the
/// notes, so a proof can be produced for any of them.
fn seeded_pool(count: u64) -> (Env, MerkleTree, Vec<Note>, Keys) {
    seeded_pool_with_timeout(count, 0)
}

fn seeded_pool_with_timeout(count: u64, timeout: u32) -> (Env, MerkleTree, Vec<Note>, Keys) {
    let mut env = setup();
    env.init_pool_with_timeout(timeout);

    let denom_tag = Field::from_u64(DENOMINATION);
    let mut tree = MerkleTree::new().unwrap();
    let mut notes = Vec::new();

    for i in 1..=count {
        let note = Note::new(
            Field::from_u64(i * 1_000_003),
            Field::from_u64(i * 7_919),
            denom_tag,
        );
        let depositor = Keypair::new();
        env.svm
            .airdrop(&depositor.pubkey(), DENOMINATION + 10_000_000)
            .unwrap();
        env.deposit(note.commitment().unwrap(), &depositor)
            .unwrap_or_else(|e| panic!("deposit {i} failed: {e}"));
        tree.insert(note.commitment().unwrap()).unwrap();
        notes.push(note);
    }

    let keys = generate_reproducible(SEED).expect("setup");
    (env, tree, notes, keys)
}

fn spend_ix(
    env: &Env,
    proof: &mirror_circuit::SolanaProof,
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

/// Produces a proof for note `index`, bound to `beneficiary`.
/// A proof and a `submit_spend` for note `index` at an arbitrary relay fee.
///
/// The fee is bound into the proof, so a differing fee is a differing statement
/// rather than a field a test can edit afterwards — which is why this exists
/// separately from the fixed-fee helpers.
#[allow(clippy::too_many_arguments)]
fn spend_ix_at_fee(
    env: &Env,
    keys: &Keys,
    tree: &MerkleTree,
    notes: &[Note],
    index: usize,
    beneficiary: &Pubkey,
    relay: &Pubkey,
    relay_fee: u64,
) -> ([u8; 32], Instruction) {
    use ark_std::rand::SeedableRng;
    let merkle_proof = tree.proof(index as u64).unwrap();
    let binding = mirror_core::action_binding(
        SELECTOR,
        &[0u8; 32],
        &beneficiary.to_bytes(),
        &relay.to_bytes(),
        relay_fee,
        0,
        &[],
    );
    let witness = Witness {
        note: notes[index],
        merkle_proof: &merkle_proof,
        root: tree.root().unwrap(),
        action_binding: binding,
    };
    let mut rng = ark_std::rand::rngs::StdRng::from_seed([index as u8; 32]);
    let proof = prove(keys, &witness, &mut rng).expect("proving");
    let nullifier = proof.public_inputs[1];
    let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);
    let ix = Instruction::new_with_bytes(
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
            relay_fee,
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
    );
    (nullifier, ix)
}

fn proof_for(
    keys: &Keys,
    tree: &MerkleTree,
    notes: &[Note],
    index: usize,
    beneficiary: &Pubkey,
    relay: &Pubkey,
) -> mirror_circuit::SolanaProof {
    use ark_std::rand::SeedableRng;
    let merkle_proof = tree.proof(index as u64).unwrap();
    let binding = mirror_core::action_binding(
        SELECTOR,
        &[0u8; 32],
        &beneficiary.to_bytes(),
        &relay.to_bytes(),
        RELAY_FEE,
        0,
        &[],
    );
    let witness = Witness {
        note: notes[index],
        merkle_proof: &merkle_proof,
        root: tree.root().unwrap(),
        action_binding: binding,
    };
    let mut rng = ark_std::rand::rngs::StdRng::from_seed([index as u8; 32]);
    prove(keys, &witness, &mut rng).expect("proving")
}

#[test]
fn a_pool_initialises_with_an_empty_accumulator() {
    let mut env = setup();
    env.init_pool_with_timeout(0);

    let mut data = env.pool_state();
    assert_eq!(data.len(), POOL_LEN);
    let pool = Pool::load(&mut data).unwrap();
    assert_eq!(pool.denomination(), DENOMINATION);
    assert_eq!(pool.k_floor(), K_FLOOR);
    assert_eq!(pool.deposit_count(), 0);
    assert_eq!(
        pool.current_root().unwrap(),
        mirror_core::Frontier::new().unwrap().root(),
        "the deployed program and the host must start from one accumulator"
    );
}

#[test]
fn deposits_escrow_exactly_the_denomination_and_advance_the_root() {
    let (env, tree, _notes, _keys) = seeded_pool(4);

    let mut data = env.pool_state();
    let pool = Pool::load(&mut data).unwrap();
    assert_eq!(pool.deposit_count(), 4);
    assert_eq!(
        pool.current_root().unwrap(),
        tree.root().unwrap(),
        "the on-chain root diverged from the host tree"
    );

    // Escrow, plus the vault's own rent exemption.
    assert!(
        env.vault_lamports() >= 4 * DENOMINATION,
        "vault holds {} for 4 notes of {DENOMINATION}",
        env.vault_lamports()
    );
    assert_eq!(pool.required_vault_lamports().unwrap(), 4 * DENOMINATION);
}

/// The decisive test: a proof built by the host prover, verified by the real
/// Groth16 syscall inside the deployed program.
#[test]
fn a_real_proof_verifies_on_chain_and_records_the_spend() {
    let (mut env, tree, notes, keys) = seeded_pool(5);
    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();

    let proof = proof_for(&keys, &tree, &notes, 2, &beneficiary, &relay.pubkey());
    let nullifier = proof.public_inputs[1];

    let ix = spend_ix(&env, &proof, nullifier, &beneficiary, &relay.pubkey());
    let cu = env.send_expect_cu(ix, &relay);
    println!("submit_spend consumed {cu} compute units");
    // The headline cost of the protocol, and the reason the two phases exist.
    // Bounded rather than pinned: the exact figure moves by a few thousand CU
    // between runs with the bump search on a freshly generated nullifier.
    assert!(
        cu < 110_000,
        "submit_spend used {cu} CU, above what it has ever measured and close \
         to the 200,000 default per-instruction budget"
    );

    let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);
    let mut data = env
        .svm
        .get_account(&spend_pda)
        .expect("spend recorded")
        .data;
    assert_eq!(
        data.len(),
        spend_len(0),
        "a transfer action carries no payload"
    );
    let record = Spend::load(&mut data).unwrap();
    assert_eq!(record.status(), STATUS_PENDING);
    assert_eq!(record.selector(), SELECTOR);
    assert_eq!(record.relay_fee(), RELAY_FEE);
    assert_eq!(record.beneficiary(), beneficiary.to_bytes());
    assert_eq!(
        record.relay(),
        relay.pubkey().to_bytes(),
        "the relay that submitted must be the one paid at settlement"
    );

    // No member key appears anywhere in this path. The spend was submitted by a
    // relay and the record names only the relay and the beneficiary, neither of
    // which is the depositor: every deposit in this pool came from its own fresh
    // keypair inside `seeded_pool`, and none of those signed anything here.
    assert_eq!(record.relay(), relay.pubkey().to_bytes());
    assert_ne!(record.relay(), record.beneficiary());
}

#[test]
fn the_same_note_cannot_be_spent_twice() {
    let (mut env, tree, notes, keys) = seeded_pool(5);
    let beneficiary = Pubkey::new_unique();
    let first_relay = Keypair::new();
    let second_relay = Keypair::new();
    env.svm
        .airdrop(&first_relay.pubkey(), 10_000_000_000)
        .unwrap();
    env.svm
        .airdrop(&second_relay.pubkey(), 10_000_000_000)
        .unwrap();

    let proof = proof_for(&keys, &tree, &notes, 1, &beneficiary, &first_relay.pubkey());
    let nullifier = proof.public_inputs[1];

    let ix = spend_ix(&env, &proof, nullifier, &beneficiary, &first_relay.pubkey());
    env.send(ix, &first_relay).expect("first spend");

    // The replay goes through a *different* relay, so the transaction is not
    // byte-identical and the runtime's duplicate-signature check cannot be what
    // rejects it. An earlier version of this test reused the first relay and
    // passed for the wrong reason: it was rejected as AlreadyProcessed before
    // the program ran at all.
    //
    // Since the relay joined the action binding, this proof would *also* fail
    // the pairing under the second relay — which is exactly why the assertion
    // below is on `Custom(15)` and not merely on "rejected". The nullifier guard
    // is checked before the binding is recomputed, so a spent note costs a
    // replayer ~11k CU instead of the ~95k a pairing would. Getting
    // ProofVerificationFailed here would mean that ordering had silently
    // inverted, and the cheap guard had stopped being the first thing to run.
    let replay = spend_ix(
        &env,
        &proof,
        nullifier,
        &beneficiary,
        &second_relay.pubkey(),
    );
    let err = env
        .send(replay, &second_relay)
        .expect_err("a replayed proof was accepted");
    println!("replay rejected: {err}");
    assert!(
        !err.contains("AlreadyProcessed"),
        "the runtime deduplicated the transaction; the nullifier guard was never reached: {err}"
    );
    assert!(
        err.contains("Custom(15)"),
        "expected NullifierAlreadySpent (15), got: {err}"
    );
}

#[test]
fn a_relay_cannot_redirect_the_payout() {
    let (mut env, tree, notes, keys) = seeded_pool(5);
    let honest_beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();

    let proof = proof_for(
        &keys,
        &tree,
        &notes,
        0,
        &honest_beneficiary,
        &relay.pubkey(),
    );
    let nullifier = proof.public_inputs[1];

    // The relay swaps in its own address after the member proved.
    let thief = Pubkey::new_unique();
    let ix = spend_ix(&env, &proof, nullifier, &thief, &relay.pubkey());
    let err = env
        .send(ix, &relay)
        .expect_err("a redirected payout was accepted");
    println!("redirect rejected: {err}");
    assert!(
        err.contains("Custom(18)"),
        "expected ProofVerificationFailed: {err}"
    );
}

#[test]
fn a_relay_cannot_inflate_its_own_fee() {
    let (mut env, tree, notes, keys) = seeded_pool(5);
    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();

    let proof = proof_for(&keys, &tree, &notes, 3, &beneficiary, &relay.pubkey());
    let nullifier = proof.public_inputs[1];
    let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);

    let ix = Instruction::new_with_bytes(
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
            relay_fee: RELAY_FEE * 50, // proved for RELAY_FEE
            action_accounts: 0,
            payload: Vec::new(),
        }
        .pack(),
        vec![
            AccountMeta::new(relay.pubkey(), true),
            AccountMeta::new(env.pool, false),
            AccountMeta::new(spend_pda, false),
            AccountMeta::new_readonly(solana_system_interface::program::ID, false),
        ],
    );
    let err = env
        .send(ix, &relay)
        .expect_err("an inflated relay fee was accepted");
    println!("fee inflation rejected: {err}");
    assert!(
        err.contains("Custom(18)"),
        "expected ProofVerificationFailed: {err}"
    );
}

#[test]
fn a_proof_against_an_unknown_root_is_rejected() {
    let (mut env, tree, notes, keys) = seeded_pool(5);
    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();

    let mut proof = proof_for(&keys, &tree, &notes, 4, &beneficiary, &relay.pubkey());
    let nullifier = proof.public_inputs[1];
    // A root the pool has never held. Flip a low bit so the value stays a
    // canonical scalar and the rejection comes from the history check.
    proof.public_inputs[0][31] ^= 1;

    let ix = spend_ix(&env, &proof, nullifier, &beneficiary, &relay.pubkey());
    let err = env
        .send(ix, &relay)
        .expect_err("an unknown root was accepted");
    println!("unknown root rejected: {err}");
    assert!(err.contains("Custom(17)"), "expected UnknownRoot: {err}");
}

#[test]
fn a_pool_below_its_anonymity_floor_refuses_to_act() {
    // Three deposits against a floor of four: acting here would give the member
    // an anonymity set smaller than the pool promises.
    let (mut env, tree, notes, keys) = seeded_pool(3);
    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();

    let proof = proof_for(&keys, &tree, &notes, 0, &beneficiary, &relay.pubkey());
    let nullifier = proof.public_inputs[1];
    let ix = spend_ix(&env, &proof, nullifier, &beneficiary, &relay.pubkey());
    let err = env
        .send(ix, &relay)
        .expect_err("a spend below the anonymity floor was accepted");
    println!("below-floor rejected: {err}");
    assert!(
        err.contains("Custom(19)"),
        "expected BelowAnonymityFloor: {err}"
    );
}

#[test]
fn a_forged_proof_is_rejected() {
    let (mut env, tree, notes, keys) = seeded_pool(5);
    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();

    let mut proof = proof_for(&keys, &tree, &notes, 2, &beneficiary, &relay.pubkey());
    let nullifier = proof.public_inputs[1];
    // Corrupt the proof itself.
    proof.proof_a[63] ^= 1;

    let ix = spend_ix(&env, &proof, nullifier, &beneficiary, &relay.pubkey());
    let err = env
        .send(ix, &relay)
        .expect_err("a corrupted proof was accepted");
    println!("forged proof rejected: {err}");
    assert!(
        err.contains("Custom(18)"),
        "expected ProofVerificationFailed: {err}"
    );
}

// ---------------------------------------------------------------------------
// Settlement
// ---------------------------------------------------------------------------

/// Submits `n` spends and returns their (spend PDA, beneficiary, relay) triples.
fn submit_batch(
    env: &mut Env,
    tree: &MerkleTree,
    notes: &[Note],
    keys: &Keys,
    n: usize,
) -> Vec<(Pubkey, Pubkey, Keypair)> {
    let mut out = Vec::new();
    for i in 0..n {
        let beneficiary = Pubkey::new_unique();
        let relay = Keypair::new();
        env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();
        let proof = proof_for(keys, tree, notes, i, &beneficiary, &relay.pubkey());
        let nullifier = proof.public_inputs[1];
        let ix = spend_ix(env, &proof, nullifier, &beneficiary, &relay.pubkey());
        env.send(ix, &relay)
            .unwrap_or_else(|e| panic!("submit {i} failed: {e}"));
        let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);
        out.push((spend_pda, beneficiary, relay));
    }
    out
}

/// Settlement that asks for the crowd, which is what an ordinary settler sends.
///
/// The below-floor flag is deliberately *not* a default anywhere in this suite:
/// a test that settles under the floor has to say so, exactly as a settler does.
/// That is what keeps the two paths from being confused for one another here.
fn settle_ix(env: &Env, batch: &[(Pubkey, Pubkey, Keypair)], settler: &Pubkey) -> Instruction {
    settle_ix_with_targets(env, batch, settler, &[], false)
}

/// Settlement that consents to a batch below the pool's floor.
fn settle_ix_below_floor(
    env: &Env,
    batch: &[(Pubkey, Pubkey, Keypair)],
    settler: &Pubkey,
) -> Instruction {
    settle_ix_full(env, batch, settler, &[], &[], true)
}

/// Settlement where some spends invoke a program. `targets[i]`, when present,
/// is the program account for `batch[i]`, which a CPI requires to be in the
/// caller's account list.
fn settle_ix_with_targets(
    env: &Env,
    batch: &[(Pubkey, Pubkey, Keypair)],
    settler: &Pubkey,
    targets: &[Option<Pubkey>],
    allow_below_floor: bool,
) -> Instruction {
    settle_ix_full(env, batch, settler, targets, &[], allow_below_floor)
}

/// Settlement carrying, per spend, an optional target program and that action's
/// own account list — the shape a real CPI needs.
fn settle_ix_full(
    env: &Env,
    batch: &[(Pubkey, Pubkey, Keypair)],
    settler: &Pubkey,
    targets: &[Option<Pubkey>],
    action_accounts: &[Vec<AccountMeta>],
    allow_below_floor: bool,
) -> Instruction {
    let mut metas = vec![
        AccountMeta::new(*settler, true),
        AccountMeta::new(env.pool, false),
        AccountMeta::new(env.vault, false),
    ];
    for (i, (spend, beneficiary, relay)) in batch.iter().enumerate() {
        metas.push(AccountMeta::new(*spend, false));
        metas.push(AccountMeta::new(*beneficiary, false));
        metas.push(AccountMeta::new(relay.pubkey(), false));
        if let Some(Some(target)) = targets.get(i) {
            metas.push(AccountMeta::new_readonly(*target, false));
        }
        if let Some(accounts) = action_accounts.get(i) {
            metas.extend(accounts.iter().cloned());
        }
    }
    Instruction::new_with_bytes(
        env.program_id,
        &MirrorIx::SettleEpoch {
            count: batch.len() as u8,
            allow_below_floor,
        }
        .pack(),
        metas,
    )
}

#[test]
fn a_full_crowd_settles_and_pays_every_beneficiary() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let batch = submit_batch(&mut env, &tree, &notes, &keys, K_FLOOR as usize);

    let before: Vec<u64> = batch
        .iter()
        .map(|(_, b, _)| env.svm.get_account(b).map(|a| a.lamports).unwrap_or(0))
        .collect();
    let vault_before = env.vault_lamports();

    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();
    let ix = settle_ix(&env, &batch, &settler.pubkey());
    let cu = env.send_expect_cu(ix, &settler);
    println!(
        "settle_epoch of {} spends consumed {cu} compute units",
        batch.len()
    );

    let payout = DENOMINATION - RELAY_FEE;
    for (i, (spend, beneficiary, _relay)) in batch.iter().enumerate() {
        let after = env.svm.get_account(beneficiary).unwrap().lamports;
        assert_eq!(
            after - before[i],
            payout,
            "beneficiary {i} received the wrong amount"
        );
        let mut data = env.svm.get_account(spend).unwrap().data;
        let record = Spend::load(&mut data).unwrap();
        assert_eq!(record.status(), STATUS_SETTLED);
    }

    // The vault paid out exactly the batch, nothing more.
    assert_eq!(
        vault_before - env.vault_lamports(),
        batch.len() as u64 * DENOMINATION
    );

    // And the invariant still holds against the notes that remain unspent.
    let mut data = env.pool_state();
    let pool = Pool::load(&mut data).unwrap();
    assert_eq!(pool.spend_count(), batch.len() as u64);
    assert_eq!(pool.outstanding_notes().unwrap(), 6 - batch.len() as u64);
    assert!(
        env.vault_lamports() >= pool.required_vault_lamports().unwrap(),
        "vault {} cannot cover {} outstanding notes",
        env.vault_lamports(),
        pool.outstanding_notes().unwrap()
    );
}

/// The double-spend an attacker reaches for first: present one pending spend
/// twice inside a single batch and be paid twice for one note.
#[test]
fn the_same_spend_cannot_be_settled_twice_within_one_batch() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let batch = submit_batch(&mut env, &tree, &notes, &keys, K_FLOOR as usize);

    // Replace the last entry with a duplicate of the first.
    let mut doubled: Vec<(Pubkey, Pubkey, Keypair)> = batch
        .iter()
        .map(|(s, b, r)| (*s, *b, r.insecure_clone()))
        .collect();
    let last = doubled.len() - 1;
    doubled[last] = (batch[0].0, batch[0].1, batch[0].2.insecure_clone());

    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();
    let ix = settle_ix(&env, &doubled, &settler.pubkey());
    let err = env
        .send(ix, &settler)
        .expect_err("a spend was settled twice in one batch");
    println!("in-batch double settle rejected: {err}");
    assert!(
        err.contains("Custom(16)"),
        "expected AlreadySettled (16), got: {err}"
    );
}

#[test]
fn a_settled_spend_cannot_be_settled_again_later() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let batch = submit_batch(&mut env, &tree, &notes, &keys, K_FLOOR as usize);

    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();
    let ix = settle_ix(&env, &batch, &settler.pubkey());
    env.send(ix, &settler).expect("first settlement");

    // A different settler, so the transaction is not byte-identical and the
    // rejection has to come from the program.
    let other = Keypair::new();
    env.svm.airdrop(&other.pubkey(), 10_000_000_000).unwrap();
    let again = settle_ix(&env, &batch, &other.pubkey());
    let err = env
        .send(again, &other)
        .expect_err("a settled batch was replayed");
    println!("settlement replay rejected: {err}");
    assert!(err.contains("Custom(16)"), "expected AlreadySettled: {err}");
}

#[test]
fn a_batch_below_the_crowd_size_must_wait() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    // One short of the floor, and freshly submitted.
    let batch = submit_batch(&mut env, &tree, &notes, &keys, K_FLOOR as usize - 1);

    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();
    // Asking for the under-floor path explicitly, so that what this test
    // measures is the clock and not the consent — the two refusals carry
    // different codes and a test that accepted either would prove neither.
    let ix = settle_ix_below_floor(&env, &batch, &settler.pubkey());
    let err = env
        .send(ix, &settler)
        .expect_err("a batch below the crowd size settled immediately");
    println!("small batch rejected: {err}");
    assert!(
        err.contains("Custom(21)"),
        "expected CrowdTooSmall (21), got: {err}"
    );
}

/// Consent, and the fact that it is not a bypass.
///
/// The flag and the timeout are independent gates and this pins both: an
/// under-floor batch without the flag is refused whatever the clock says, and
/// the flag on its own does not buy a settlement the clock has not earned.
#[test]
fn an_under_floor_batch_is_refused_unless_the_settler_asked_for_it() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let batch = submit_batch(&mut env, &tree, &notes, &keys, K_FLOOR as usize - 1);

    // A settler per attempt. Byte-identical transactions are deduplicated by
    // the runtime before the program runs, and a negative case that never
    // reached the program asserts nothing about it.
    let early = Keypair::new();
    let late = Keypair::new();
    let consenting = Keypair::new();
    for k in [&early, &late, &consenting] {
        env.svm.airdrop(&k.pubkey(), 10_000_000_000).unwrap();
    }

    // No flag, before the timeout: refused for want of consent, and the code
    // says which of the two gates closed.
    let err = env
        .send(settle_ix(&env, &batch, &early.pubkey()), &early)
        .expect_err("an under-floor batch settled without the flag");
    println!("under-floor batch without consent rejected: {err}");
    assert!(
        err.contains("Custom(26)"),
        "expected BelowFloorNotPermitted (26), got: {err}"
    );

    // No flag, *after* the timeout: still refused. The clock does not imply
    // consent, which is the whole point of asking for it.
    let mut clock = env.svm.get_sysvar::<solana_program::clock::Clock>();
    clock.unix_timestamp += mirror_pool_program::processor::SETTLE_TIMEOUT_SECONDS + 1;
    env.svm.set_sysvar(&clock);
    let err = env
        .send(settle_ix(&env, &batch, &late.pubkey()), &late)
        .expect_err("the timeout was read as consent");
    assert!(
        err.contains("Custom(26)"),
        "expected BelowFloorNotPermitted (26) after the timeout, got: {err}"
    );

    // With the flag, now that the clock has run: settles.
    env.send(
        settle_ix_below_floor(&env, &batch, &consenting.pubkey()),
        &consenting,
    )
    .expect("an under-floor batch with consent and an elapsed timeout");
}

/// A crowd-sized batch ignores the flag entirely.
///
/// Worth pinning because the alternative reading — that the flag is a mode the
/// settler puts the instruction into — would make an ordinary settlement behave
/// differently depending on a byte that should not matter to it.
#[test]
fn the_below_floor_flag_changes_nothing_for_a_batch_that_meets_the_floor() {
    for asked in [false, true] {
        let (mut env, tree, notes, keys) = seeded_pool(6);
        let batch = submit_batch(&mut env, &tree, &notes, &keys, K_FLOOR as usize);
        let settler = Keypair::new();
        env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();

        let ix = settle_ix_full(&env, &batch, &settler.pubkey(), &[], &[], asked);
        let meta = env.send_meta(ix, &settler).unwrap_or_else(|e| {
            panic!("a full crowd was refused with allow_below_floor={asked}: {e}")
        });

        // And no mark: this batch was the crowd it claimed to be.
        assert!(
            !meta.logs.iter().any(|l| l.contains("below the pool floor")),
            "a full crowd logged the under-floor marker with allow_below_floor={asked}: {:#?}",
            meta.logs
        );
    }
}

/// A pool settles on its own clock, not the program's.
///
/// The default is an hour; this pool asked for a minute. Both halves matter: the
/// spend must not settle before its own timeout, and it must settle after it
/// rather than waiting out an hour it never agreed to. A program that read the
/// constant instead of the account would pass the first half and fail the
/// second, which is the failure this pins.
#[test]
fn a_pool_settles_on_the_timeout_it_chose_rather_than_the_default() {
    const CHOSEN: u32 = mirror_pool_program::state::MIN_SETTLE_TIMEOUT_SECONDS; // 60s
    let (mut env, tree, notes, keys) = seeded_pool_with_timeout(6, CHOSEN);
    let batch = submit_batch(&mut env, &tree, &notes, &keys, 1);

    let early = Keypair::new();
    let late = Keypair::new();
    for k in [&early, &late] {
        env.svm.airdrop(&k.pubkey(), 10_000_000_000).unwrap();
    }

    // A second before its own timeout: refused.
    let mut clock = env.svm.get_sysvar::<solana_program::clock::Clock>();
    clock.unix_timestamp += i64::from(CHOSEN) - 1;
    env.svm.set_sysvar(&clock);
    let err = env
        .send(settle_ix_below_floor(&env, &batch, &early.pubkey()), &early)
        .expect_err("settled before the pool's own timeout");
    assert!(
        err.contains("Custom(21)"),
        "expected CrowdTooSmall (21) before the chosen timeout, got: {err}"
    );

    // A second after it, and still far short of the program's default hour.
    clock.unix_timestamp += 2;
    env.svm.set_sysvar(&clock);
    let meta = env
        .send_meta(settle_ix_below_floor(&env, &batch, &late.pubkey()), &late)
        .expect("a lone spend after the pool's own timeout");

    // And the mark names the pool's figure, not the default.
    let mark = meta
        .logs
        .iter()
        .find(|l| l.contains("below the pool floor"))
        .unwrap_or_else(|| panic!("no under-floor mark: {:#?}", meta.logs));
    assert!(
        mark.contains(&format!("on the {CHOSEN}s timeout")),
        "the mark quotes a timeout this pool did not choose: {mark}"
    );
}

/// The mark R1 asks for: a settlement that lands below the floor says so on
/// chain, so a crowd settlement and a solo timeout settlement are not the same
/// transaction shape read two ways.
#[test]
fn a_settlement_below_the_floor_leaves_a_mark_naming_the_floor_it_missed() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let batch = submit_batch(&mut env, &tree, &notes, &keys, 1);

    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();
    let mut clock = env.svm.get_sysvar::<solana_program::clock::Clock>();
    clock.unix_timestamp += mirror_pool_program::processor::SETTLE_TIMEOUT_SECONDS + 1;
    env.svm.set_sysvar(&clock);

    let meta = env
        .send_meta(
            settle_ix_below_floor(&env, &batch, &settler.pubkey()),
            &settler,
        )
        .expect("a lone spend after the timeout");

    let mark = meta
        .logs
        .iter()
        .find(|l| l.contains("below the pool floor"))
        .unwrap_or_else(|| panic!("no under-floor mark in the logs: {:#?}", meta.logs));
    println!("{mark}");
    // Both numbers, because either alone is unreadable: a count with no floor
    // does not say it fell short, and a floor with no count does not say by how
    // much.
    assert!(
        mark.contains("settled 1 spend(s)") && mark.contains("floor of 4"),
        "the mark does not carry both the count and the floor: {mark}"
    );
}

/// The escape valve. A quiet pool must not hold a member's funds forever, so
/// once the timeout has passed a lone spend settles on its own.
#[test]
fn a_lone_spend_settles_once_the_timeout_has_passed() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let batch = submit_batch(&mut env, &tree, &notes, &keys, 1);

    let early = Keypair::new();
    let late = Keypair::new();
    env.svm.airdrop(&early.pubkey(), 10_000_000_000).unwrap();
    env.svm.airdrop(&late.pubkey(), 10_000_000_000).unwrap();

    // Before the timeout: refused.
    let ix = settle_ix_below_floor(&env, &batch, &early.pubkey());
    assert!(env.send(ix, &early).is_err(), "settled too early");

    // Advance the clock past the timeout.
    let mut clock = env.svm.get_sysvar::<solana_program::clock::Clock>();
    clock.unix_timestamp += mirror_pool_program::processor::SETTLE_TIMEOUT_SECONDS + 1;
    env.svm.set_sysvar(&clock);

    let before = env
        .svm
        .get_account(&batch[0].1)
        .map(|a| a.lamports)
        .unwrap_or(0);
    let ix = settle_ix_below_floor(&env, &batch, &late.pubkey());
    env.send(ix, &late).expect("lone spend after timeout");
    let after = env.svm.get_account(&batch[0].1).unwrap().lamports;
    assert_eq!(after - before, DENOMINATION - RELAY_FEE);
}

#[test]
fn settlement_refuses_a_beneficiary_the_proof_did_not_bind() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let batch = submit_batch(&mut env, &tree, &notes, &keys, K_FLOOR as usize);

    // Swap one beneficiary for an address the member never proved.
    let mut tampered: Vec<(Pubkey, Pubkey, Keypair)> = batch
        .iter()
        .map(|(s, b, r)| (*s, *b, r.insecure_clone()))
        .collect();
    tampered[1].1 = Pubkey::new_unique();

    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();
    let ix = settle_ix(&env, &tampered, &settler.pubkey());
    let err = env
        .send(ix, &settler)
        .expect_err("settlement paid an unbound beneficiary");
    println!("unbound beneficiary rejected: {err}");
    assert!(err.contains("Custom(2)"), "expected InvalidPda: {err}");
}

/// The permissionless exit, demonstrated rather than added.
///
/// No `self_spend` instruction exists, because none is needed: a member acts as
/// their own relay with a zero fee, and settlement is already permissionless, so
/// they can settle their own batch once the timeout passes. Nothing in the
/// protocol can hold their escrow — no relay has to cooperate and no operator
/// has to be alive.
///
/// The cost is exactly the one you would expect: their own wallet signs, so this
/// path gives up the anonymity the relay path provides. It is an escape hatch,
/// not a mode of operation, and this test pins that it works rather than leaving
/// it as an assertion in a README.
#[test]
fn a_member_can_always_exit_without_any_relay() {
    let (mut env, tree, notes, keys) = seeded_pool(6);

    // The member is their own relay and their own beneficiary, fee zero.
    let member = Keypair::new();
    env.svm.airdrop(&member.pubkey(), 10_000_000_000).unwrap();
    let beneficiary = member.pubkey();

    let merkle_proof = tree.proof(0).unwrap();
    let binding = mirror_core::action_binding(
        SELECTOR,
        &[0u8; 32],
        &beneficiary.to_bytes(),
        &member.pubkey().to_bytes(),
        0,
        0,
        &[],
    );
    let witness = Witness {
        note: notes[0],
        merkle_proof: &merkle_proof,
        root: tree.root().unwrap(),
        action_binding: binding,
    };
    let mut rng = {
        use ark_std::rand::SeedableRng;
        ark_std::rand::rngs::StdRng::from_seed([42u8; 32])
    };
    let proof = prove(&keys, &witness, &mut rng).expect("proving");
    let nullifier = proof.public_inputs[1];
    let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);

    let ix = Instruction::new_with_bytes(
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
            relay_fee: 0,
            action_accounts: 0,
            payload: Vec::new(),
        }
        .pack(),
        vec![
            AccountMeta::new(member.pubkey(), true),
            AccountMeta::new(env.pool, false),
            AccountMeta::new(spend_pda, false),
            AccountMeta::new_readonly(solana_system_interface::program::ID, false),
        ],
    );
    env.send(ix, &member)
        .expect("member submitted their own spend");

    // Alone, so the crowd rule sends them to the timeout.
    let mut clock = env.svm.get_sysvar::<solana_program::clock::Clock>();
    clock.unix_timestamp += mirror_pool_program::processor::SETTLE_TIMEOUT_SECONDS + 1;
    env.svm.set_sysvar(&clock);

    let before = env.svm.get_account(&member.pubkey()).unwrap().lamports;
    let batch = vec![(spend_pda, beneficiary, member.insecure_clone())];
    let settle = settle_ix_below_floor(&env, &batch, &member.pubkey());
    env.send(settle, &member)
        .expect("member settled their own spend");

    let after = env.svm.get_account(&member.pubkey()).unwrap().lamports;
    // The full denomination, minus whatever the transaction itself cost.
    assert!(
        after > before,
        "the member ended up worse off: {before} -> {after}"
    );
    assert!(
        after - before >= DENOMINATION - 100_000,
        "expected roughly the full denomination back, got {}",
        after - before
    );
}

// ---------------------------------------------------------------------------
// Behavioural actions
//
// The brief asks for "Tornado Cash for behavioural patterns and withdrawals —
// not for funds". A pool that only moves lamports answers the wrong question:
// what should be deniable is that *you* staked, swapped or voted, not merely
// where your money went.
//
// These exercise the pool invoking a real third-party program on a member's
// behalf. The target is the actual SPL Memo program, fetched from mainnet, so
// the CPI path runs against something that exists.
// ---------------------------------------------------------------------------

const INVOKE: u64 = mirror_pool_program::processor::SELECTOR_INVOKE;
const INVOKE_SIGNED: u64 = mirror_pool_program::processor::SELECTOR_INVOKE_SIGNED;

/// A spend that asks the pool to invoke `target` with `payload`.
fn action_ix(
    env: &Env,
    proof: &mirror_circuit::SolanaProof,
    nullifier: [u8; 32],
    target: &Pubkey,
    beneficiary: &Pubkey,
    relay: &Pubkey,
    payload: &[u8],
) -> Instruction {
    action_ix_n(
        env,
        proof,
        nullifier,
        target,
        beneficiary,
        relay,
        0,
        payload,
    )
}

#[allow(clippy::too_many_arguments)]
fn action_ix_n(
    env: &Env,
    proof: &mirror_circuit::SolanaProof,
    nullifier: [u8; 32],
    target: &Pubkey,
    beneficiary: &Pubkey,
    relay: &Pubkey,
    action_accounts: u8,
    payload: &[u8],
) -> Instruction {
    action_ix_sel(
        env,
        proof,
        nullifier,
        target,
        beneficiary,
        relay,
        action_accounts,
        payload,
        INVOKE,
    )
}

#[allow(clippy::too_many_arguments)]
fn action_ix_sel(
    env: &Env,
    proof: &mirror_circuit::SolanaProof,
    nullifier: [u8; 32],
    target: &Pubkey,
    beneficiary: &Pubkey,
    relay: &Pubkey,
    action_accounts: u8,
    payload: &[u8],
    selector: u64,
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
            selector,
            target_program: target.to_bytes(),
            beneficiary: beneficiary.to_bytes(),
            relay_fee: RELAY_FEE,
            action_accounts,
            payload: payload.to_vec(),
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

#[allow(clippy::too_many_arguments)]
fn action_proof(
    keys: &Keys,
    tree: &MerkleTree,
    notes: &[Note],
    index: usize,
    target: &Pubkey,
    beneficiary: &Pubkey,
    relay: &Pubkey,
    payload: &[u8],
) -> mirror_circuit::SolanaProof {
    action_proof_n(
        keys,
        tree,
        notes,
        index,
        target,
        beneficiary,
        relay,
        0,
        payload,
    )
}

#[allow(clippy::too_many_arguments)]
fn action_proof_n(
    keys: &Keys,
    tree: &MerkleTree,
    notes: &[Note],
    index: usize,
    target: &Pubkey,
    beneficiary: &Pubkey,
    relay: &Pubkey,
    action_accounts: u8,
    payload: &[u8],
) -> mirror_circuit::SolanaProof {
    action_proof_sel(
        keys,
        tree,
        notes,
        index,
        target,
        beneficiary,
        relay,
        action_accounts,
        payload,
        INVOKE,
    )
}

#[allow(clippy::too_many_arguments)]
fn action_proof_sel(
    keys: &Keys,
    tree: &MerkleTree,
    notes: &[Note],
    index: usize,
    target: &Pubkey,
    beneficiary: &Pubkey,
    relay: &Pubkey,
    action_accounts: u8,
    payload: &[u8],
    selector: u64,
) -> mirror_circuit::SolanaProof {
    use ark_std::rand::SeedableRng;
    let merkle_proof = tree.proof(index as u64).unwrap();
    let binding = mirror_core::action_binding(
        selector,
        &target.to_bytes(),
        &beneficiary.to_bytes(),
        &relay.to_bytes(),
        RELAY_FEE,
        action_accounts,
        payload,
    );
    let witness = Witness {
        note: notes[index],
        merkle_proof: &merkle_proof,
        root: tree.root().unwrap(),
        action_binding: binding,
    };
    let mut rng = ark_std::rand::rngs::StdRng::from_seed([200 + index as u8; 32]);
    prove(keys, &witness, &mut rng).expect("proving")
}

/// The thesis, end to end: a crowd of members each perform the *same shape* of
/// protocol action, and the pool invokes the target program for every one of
/// them in a single transaction — one timestamp, one signer on the transaction,
/// nothing distinguishing which member asked for which.
///
/// An observer sees four memos land at one timestamp, signed by one pool, and
/// has nothing in the transaction that distinguishes which member asked for
/// which.
#[test]
fn a_crowd_of_members_perform_a_real_protocol_action_together() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let memo: Pubkey = MEMO_PROGRAM.parse().unwrap();
    env.svm
        .add_program(memo, &memo_bytes())
        .expect("loading the real SPL Memo program");

    let mut batch = Vec::new();
    for i in 0..K_FLOOR as usize {
        let beneficiary = Pubkey::new_unique();
        let relay = Keypair::new();
        env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();
        // Every member sends the identical payload, which is what makes the
        // crowd a crowd: the actions are indistinguishable by content.
        let payload = b"mirror-pool".as_slice();
        let proof = action_proof(
            &keys,
            &tree,
            &notes,
            i,
            &memo,
            &beneficiary,
            &relay.pubkey(),
            payload,
        );
        let nullifier = proof.public_inputs[1];
        let ix = action_ix(
            &env,
            &proof,
            nullifier,
            &memo,
            &beneficiary,
            &relay.pubkey(),
            payload,
        );
        env.send(ix, &relay)
            .unwrap_or_else(|e| panic!("submitting action {i}: {e}"));
        let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);
        batch.push((spend_pda, beneficiary, relay));
    }

    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();
    let targets: Vec<Option<Pubkey>> = batch.iter().map(|_| Some(memo)).collect();
    let ix = settle_ix_with_targets(&env, &batch, &settler.pubkey(), &targets, false);
    let cu = env.send_expect_cu(ix, &settler);
    println!(
        "settled {} real CPI actions in one transaction, {cu} CU",
        batch.len()
    );
    // A bound, not a fixed figure. Settlement's cost moves by a few thousand CU
    // between runs because the accounts are freshly generated each time and
    // `find_program_address` searches a different number of bumps to derive each
    // record's address. Pinning the exact number would produce a test that fails
    // on nothing, so what is asserted is the property the design depends on:
    // four CPI actions and their payouts fit in one instruction's default
    // budget, with room to spare.
    assert!(
        cu < 60_000,
        "four settled CPI actions cost {cu} CU, well above what this batch has \
         ever measured — settlement got materially more expensive"
    );

    for (spend, beneficiary, _) in &batch {
        let mut data = env.svm.get_account(spend).unwrap().data;
        let record = Spend::load(&mut data).unwrap();
        assert_eq!(record.status(), STATUS_SETTLED);
        assert_eq!(record.target_program(), memo.to_bytes());
        assert_eq!(record.payload(), b"mirror-pool");
        // The action was funded, so the target acted on real value.
        assert!(env.svm.get_account(beneficiary).unwrap().lamports > 0);
    }
}

/// The action path with a non-empty account list, which nothing exercised
/// before: every action test passed zero accounts, so the loop that reads them
/// and the metas built from them were dead code.
///
/// SPL Memo requires every account handed to it to have signed, so passing one
/// makes the CPI's account list load-bearing rather than decorative — if the
/// program dropped an account, mislabelled its signer flag, or miscounted, Memo
/// rejects.
#[test]
fn an_action_runs_with_a_non_empty_account_list() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let memo: Pubkey = MEMO_PROGRAM.parse().unwrap();
    env.svm.add_program(memo, &memo_bytes()).unwrap();

    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();
    let payload = b"signed by the pool".as_slice();

    // One action account, declared in the proof and therefore bound.
    let proof = action_proof_n(
        &keys,
        &tree,
        &notes,
        0,
        &memo,
        &beneficiary,
        &relay.pubkey(),
        1,
        payload,
    );
    let nullifier = proof.public_inputs[1];
    let ix = action_ix_n(
        &env,
        &proof,
        nullifier,
        &memo,
        &beneficiary,
        &relay.pubkey(),
        1,
        payload,
    );
    env.send(ix, &relay).expect("submitting the action");

    let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);
    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();

    let mut clock = env.svm.get_sysvar::<solana_program::clock::Clock>();
    clock.unix_timestamp += mirror_pool_program::processor::SETTLE_TIMEOUT_SECONDS + 1;
    env.svm.set_sysvar(&clock);

    // The settler itself is the action's account. It signs the transaction, so
    // Memo's requirement that every account has signed is genuinely satisfied
    // through the program's metas rather than by Memo having nothing to check.
    let batch = vec![(spend_pda, beneficiary, relay.insecure_clone())];
    let ix = settle_ix_full(
        &env,
        &batch,
        &settler.pubkey(),
        &[Some(memo)],
        &[vec![AccountMeta::new_readonly(settler.pubkey(), true)]],
        true,
    );
    let cu = env.send_expect_cu(ix, &settler);
    println!("settled a signed CPI action, {cu} CU");

    let mut data = env.svm.get_account(&spend_pda).unwrap().data;
    let record = Spend::load(&mut data).unwrap();
    assert_eq!(record.status(), STATUS_SETTLED);
    assert_eq!(record.action_accounts(), 1);
    assert_eq!(record.payload(), payload);
}

/// A settler must not be able to hand the action a different account list than
/// the member declared, because the count is now inside the binding.
#[test]
fn a_settler_cannot_change_how_many_accounts_an_action_gets() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let memo: Pubkey = MEMO_PROGRAM.parse().unwrap();
    env.svm.add_program(memo, &memo_bytes()).unwrap();

    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();
    let payload = b"one account".as_slice();

    let proof = action_proof_n(
        &keys,
        &tree,
        &notes,
        1,
        &memo,
        &beneficiary,
        &relay.pubkey(),
        1,
        payload,
    );
    let nullifier = proof.public_inputs[1];
    let ix = action_ix_n(
        &env,
        &proof,
        nullifier,
        &memo,
        &beneficiary,
        &relay.pubkey(),
        1,
        payload,
    );
    env.send(ix, &relay).expect("submitting");

    let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);
    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();
    let mut clock = env.svm.get_sysvar::<solana_program::clock::Clock>();
    clock.unix_timestamp += mirror_pool_program::processor::SETTLE_TIMEOUT_SECONDS + 1;
    env.svm.set_sysvar(&clock);

    // Supply no action accounts where the record declares one. The program reads
    // one account past this spend's own, so it consumes something that is not
    // there and fails rather than invoking with a truncated list.
    let batch = vec![(spend_pda, beneficiary, relay.insecure_clone())];
    let ix = settle_ix_full(
        &env,
        &batch,
        &settler.pubkey(),
        &[Some(memo)],
        &[vec![]],
        true,
    );
    let err = env
        .send(ix, &settler)
        .expect_err("settlement invoked with fewer accounts than declared");
    println!("truncated action account list rejected: {err}");
    // The one negative case without a code of ours: the runtime runs out of
    // accounts before the program can rule on it. Asserted explicitly so the
    // exception stays visible rather than looking like an oversight.
    assert!(
        err.contains("NotEnoughAccountKeys"),
        "expected the runtime to refuse before the program: {err}"
    );
}

/// The pool's signature is the member's decision and a settler cannot help
/// themselves to it.
///
/// Under plain `INVOKE` the payout leaves the vault *before* the call, so the
/// vault cannot cross the CPI boundary — the runtime would reject the whole
/// instruction as unbalanced. The program refuses it by name instead, which
/// turns an opaque runtime failure into a stated rule. A member who wants the
/// pool to sign proves `INVOKE_SIGNED`, and the selector is inside the action
/// binding, so this is not something settlement can decide.
#[test]
fn a_settler_cannot_place_the_vault_in_an_action_account_list() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let memo: Pubkey = MEMO_PROGRAM.parse().unwrap();
    env.svm.add_program(memo, &memo_bytes()).unwrap();

    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();
    let payload = b"vault please".as_slice();

    let proof = action_proof_n(
        &keys,
        &tree,
        &notes,
        2,
        &memo,
        &beneficiary,
        &relay.pubkey(),
        1,
        payload,
    );
    let nullifier = proof.public_inputs[1];
    let ix = action_ix_n(
        &env,
        &proof,
        nullifier,
        &memo,
        &beneficiary,
        &relay.pubkey(),
        1,
        payload,
    );
    env.send(ix, &relay).expect("submitting");

    let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);
    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();
    let mut clock = env.svm.get_sysvar::<solana_program::clock::Clock>();
    clock.unix_timestamp += mirror_pool_program::processor::SETTLE_TIMEOUT_SECONDS + 1;
    env.svm.set_sysvar(&clock);

    let batch = vec![(spend_pda, beneficiary, relay.insecure_clone())];
    let ix = settle_ix_full(
        &env,
        &batch,
        &settler.pubkey(),
        &[Some(memo)],
        &[vec![AccountMeta::new(env.vault, false)]],
        true,
    );
    let err = env
        .send(ix, &settler)
        .expect_err("the vault was accepted as an action account");
    println!("vault as action account rejected: {err}");
    assert!(
        err.contains("Custom(1)"),
        "expected MalformedInstruction: {err}"
    );
}

/// The pool acting as a delegated **authority**, not merely as a source of funds.
///
/// This is the capability a stake delegation or a governance vote needs and a
/// plain transfer does not: somebody has to sign as the authority, and for a
/// member who must never appear on chain that somebody can only be the pool.
///
/// SPL Memo is the witness, and it is a good one because it is a real third-party
/// program that refuses any account handed to it that has not signed, and because
/// it *names* its signers in its logs. So a memo whose only account is the pool's
/// vault cannot pass by accident: either Memo rejects the instruction, or it
/// prints the vault's own pubkey. The assertion below reads that log.
///
/// Note what the settler supplies: the vault marked **not** a signer, because a
/// PDA has no key and cannot sign a transaction. The signature comes from
/// `invoke_signed` inside the program, from seeds only this program holds.
#[test]
fn the_pool_signs_an_action_as_its_own_authority() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let memo: Pubkey = MEMO_PROGRAM.parse().unwrap();
    env.svm.add_program(memo, &memo_bytes()).unwrap();

    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();
    let payload = b"the pool signed this".as_slice();

    let proof = action_proof_sel(
        &keys,
        &tree,
        &notes,
        0,
        &memo,
        &beneficiary,
        &relay.pubkey(),
        1,
        payload,
        INVOKE_SIGNED,
    );
    let nullifier = proof.public_inputs[1];
    let ix = action_ix_sel(
        &env,
        &proof,
        nullifier,
        &memo,
        &beneficiary,
        &relay.pubkey(),
        1,
        payload,
        INVOKE_SIGNED,
    );
    env.send(ix, &relay).expect("submitting the signed action");

    let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);
    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();
    let mut clock = env.svm.get_sysvar::<solana_program::clock::Clock>();
    clock.unix_timestamp += mirror_pool_program::processor::SETTLE_TIMEOUT_SECONDS + 1;
    env.svm.set_sysvar(&clock);

    let vault_before = env.vault_lamports();
    let batch = vec![(spend_pda, beneficiary, relay.insecure_clone())];
    let ix = settle_ix_full(
        &env,
        &batch,
        &settler.pubkey(),
        &[Some(memo)],
        &[vec![AccountMeta::new(env.vault, false)]],
        true,
    );
    let msg = Message::new(&[ix], Some(&settler.pubkey()));
    let tx = Transaction::new(&[&settler], msg, env.svm.latest_blockhash());
    let meta = env
        .svm
        .send_transaction(tx)
        .unwrap_or_else(|e| panic!("settling a signed action: {:?}\n{:#?}", e.err, e.meta.logs));

    let vault = env.vault.to_string();
    assert!(
        meta.logs
            .iter()
            .any(|l| l.contains("Signed by") && l.contains(&vault)),
        "SPL Memo did not report the pool's vault as a signer, so the pool did \
         not actually sign: {:#?}",
        meta.logs
    );
    println!(
        "the pool signed a real CPI as authority, {} CU",
        meta.compute_units_consumed
    );
    assert!(
        meta.compute_units_consumed < 50_000,
        "a pool-signed action cost {} CU, well above what it has ever measured",
        meta.compute_units_consumed
    );

    // The member is still paid in full, and the payout still leaves the vault —
    // the reordering changes when the callee sees the value, not who is owed it.
    assert_eq!(
        env.svm.get_account(&beneficiary).unwrap().lamports,
        DENOMINATION - RELAY_FEE
    );
    assert_eq!(vault_before - env.vault_lamports(), DENOMINATION);

    let mut data = env.svm.get_account(&spend_pda).unwrap().data;
    assert_eq!(Spend::load(&mut data).unwrap().status(), STATUS_SETTLED);
}

/// A mixed batch: plain transfers and a call the pool signs, settled together.
///
/// This is the case a live cluster found and a single-spend test could not. The
/// runtime's objection is not to a signed call following a payout *for that
/// member* — it is to a signed call following any lamport this program moved in
/// the same instruction, which in a batch means the three members settled before
/// it. Settlement therefore runs every signed call first and pays everybody
/// afterwards.
///
/// The shape matters beyond the bug. A crowd is mixed by definition, and a
/// signed action that could only settle by itself would have to wait out the
/// timeout instead of joining a batch — losing the shared timestamp that makes
/// the crowd worth standing in.
#[test]
fn a_signed_action_settles_inside_a_batch_of_plain_transfers() {
    let (mut env, tree, notes, keys) = seeded_pool(8);
    let memo: Pubkey = MEMO_PROGRAM.parse().unwrap();
    env.svm.add_program(memo, &memo_bytes()).unwrap();

    // Three ordinary members first, so the vault has already moved by the time
    // the signed call runs.
    let mut batch = submit_batch(&mut env, &tree, &notes, &keys, 3);
    let mut targets: Vec<Option<Pubkey>> = vec![None, None, None];
    let mut action_accounts: Vec<Vec<AccountMeta>> = vec![Vec::new(), Vec::new(), Vec::new()];

    let signer_beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();
    let payload = b"one of four".as_slice();
    let proof = action_proof_sel(
        &keys,
        &tree,
        &notes,
        3,
        &memo,
        &signer_beneficiary,
        &relay.pubkey(),
        1,
        payload,
        INVOKE_SIGNED,
    );
    let nullifier = proof.public_inputs[1];
    let ix = action_ix_sel(
        &env,
        &proof,
        nullifier,
        &memo,
        &signer_beneficiary,
        &relay.pubkey(),
        1,
        payload,
        INVOKE_SIGNED,
    );
    env.send(ix, &relay).expect("submitting the signed action");
    let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);
    batch.push((spend_pda, signer_beneficiary, relay.insecure_clone()));
    targets.push(Some(memo));
    action_accounts.push(vec![AccountMeta::new(env.vault, false)]);

    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();
    let vault_before = env.vault_lamports();
    let ix = settle_ix_full(
        &env,
        &batch,
        &settler.pubkey(),
        &targets,
        &action_accounts,
        false,
    );
    let msg = Message::new(&[ix], Some(&settler.pubkey()));
    let tx = Transaction::new(&[&settler], msg, env.svm.latest_blockhash());
    let meta = env
        .svm
        .send_transaction(tx)
        .unwrap_or_else(|e| panic!("mixed batch: {:?}\n{:#?}", e.err, e.meta.logs));

    let vault = env.vault.to_string();
    assert!(
        meta.logs
            .iter()
            .any(|l| l.contains("Signed by") && l.contains(&vault)),
        "the pool did not sign inside a mixed batch: {:#?}",
        meta.logs
    );
    println!(
        "mixed batch of 4 settled, one of them pool-signed, {} CU",
        meta.compute_units_consumed
    );

    // Everyone in the batch is paid, whatever their action was.
    for (_, beneficiary, _) in &batch {
        assert_eq!(
            env.svm.get_account(beneficiary).unwrap().lamports,
            DENOMINATION - RELAY_FEE,
            "a member of the mixed batch went unpaid"
        );
    }
    assert_eq!(vault_before - env.vault_lamports(), 4 * DENOMINATION);
}

/// The attack the signing selector invites: point the pool's own signature at
/// the System Program and tell it to move the escrow somewhere else.
///
/// Every ingredient is legitimate. The proof is genuine, the member owns the
/// note, the selector is one the protocol offers, and the payload is a
/// well-formed `SystemInstruction::Transfer` draining the vault to an address
/// the member chose. Nothing in this program inspects the payload — it cannot,
/// the payload is opaque by design — so what refuses this is the ownership rule
/// underneath: the System Program may only debit accounts it owns, and the vault
/// is owned by the pool.
///
/// That is the reason the pool's signature is safe to hand out. It is worth a
/// test rather than a sentence, because the sentence is the kind that stays true
/// only until somebody gives the vault a second owner or a data layout.
#[test]
fn the_pools_signature_cannot_be_turned_against_its_own_vault() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let system = solana_system_interface::program::ID;

    let attacker = Pubkey::new_unique();
    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();

    // SystemInstruction::Transfer { lamports }: discriminant 2 as u32, then the
    // amount. Drain everything the vault is holding for the other members.
    let mut payload = 2u32.to_le_bytes().to_vec();
    payload.extend_from_slice(&(DENOMINATION * 5).to_le_bytes());

    let proof = action_proof_sel(
        &keys,
        &tree,
        &notes,
        1,
        &system,
        &beneficiary,
        &relay.pubkey(),
        2,
        &payload,
        INVOKE_SIGNED,
    );
    let nullifier = proof.public_inputs[1];
    let ix = action_ix_sel(
        &env,
        &proof,
        nullifier,
        &system,
        &beneficiary,
        &relay.pubkey(),
        2,
        &payload,
        INVOKE_SIGNED,
    );
    env.send(ix, &relay).expect("submitting the drain attempt");

    let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);
    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();
    let mut clock = env.svm.get_sysvar::<solana_program::clock::Clock>();
    clock.unix_timestamp += mirror_pool_program::processor::SETTLE_TIMEOUT_SECONDS + 1;
    env.svm.set_sysvar(&clock);

    let vault_before = env.vault_lamports();
    let batch = vec![(spend_pda, beneficiary, relay.insecure_clone())];
    let ix = settle_ix_full(
        &env,
        &batch,
        &settler.pubkey(),
        &[Some(system)],
        &[vec![
            AccountMeta::new(env.vault, false),
            AccountMeta::new(attacker, false),
        ]],
        true,
    );
    let err = env
        .send(ix, &settler)
        .expect_err("the pool signed away its own escrow");
    println!("vault drain via the pool's own signature rejected: {err}");

    // The whole transaction reverted, so nothing moved and the note is unspent.
    assert_eq!(env.vault_lamports(), vault_before);
    assert_eq!(env.svm.get_account(&attacker).map(|a| a.lamports), None);
    let mut data = env.svm.get_account(&spend_pda).unwrap().data;
    assert_eq!(
        Spend::load(&mut data).unwrap().status(),
        mirror_pool_program::spend::STATUS_PENDING,
        "the spend was consumed by an attack that failed"
    );
}

#[test]
fn a_relay_cannot_swap_the_target_program() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let memo: Pubkey = MEMO_PROGRAM.parse().unwrap();
    env.svm.add_program(memo, &memo_bytes()).unwrap();

    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();
    let payload = b"hello".as_slice();
    let proof = action_proof(
        &keys,
        &tree,
        &notes,
        0,
        &memo,
        &beneficiary,
        &relay.pubkey(),
        payload,
    );
    let nullifier = proof.public_inputs[1];

    // Proved for the memo program; submitted for something else.
    let impostor = Pubkey::new_unique();
    let ix = action_ix(
        &env,
        &proof,
        nullifier,
        &impostor,
        &beneficiary,
        &relay.pubkey(),
        payload,
    );
    let err = env
        .send(ix, &relay)
        .expect_err("a swapped target program was accepted");
    println!("target swap rejected: {err}");
    assert!(err.contains("Custom(18)"), "expected proof failure: {err}");
}

#[test]
fn a_relay_cannot_alter_the_action_payload() {
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let memo: Pubkey = MEMO_PROGRAM.parse().unwrap();
    env.svm.add_program(memo, &memo_bytes()).unwrap();

    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();
    let proof = action_proof(
        &keys,
        &tree,
        &notes,
        1,
        &memo,
        &beneficiary,
        &relay.pubkey(),
        b"stake 0.1",
    );
    let nullifier = proof.public_inputs[1];

    let ix = action_ix(
        &env,
        &proof,
        nullifier,
        &memo,
        &beneficiary,
        &relay.pubkey(),
        b"stake 9.9", // proved for a different amount
    );
    let err = env
        .send(ix, &relay)
        .expect_err("a tampered action payload was accepted");
    println!("payload tamper rejected: {err}");
    assert!(err.contains("Custom(18)"), "expected proof failure: {err}");
}

#[test]
fn an_action_cannot_re_enter_the_pool() {
    // A payload that invokes mirror-pool itself would re-enter settlement in the
    // middle of a lamport-moving loop. Refused explicitly rather than left to
    // careful reading of the loop.
    let (mut env, tree, notes, keys) = seeded_pool(6);
    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();

    let self_target = env.program_id;
    let payload = b"reenter".as_slice();
    let proof = action_proof(
        &keys,
        &tree,
        &notes,
        2,
        &self_target,
        &beneficiary,
        &relay.pubkey(),
        payload,
    );
    let nullifier = proof.public_inputs[1];
    let ix = action_ix(
        &env,
        &proof,
        nullifier,
        &self_target,
        &beneficiary,
        &relay.pubkey(),
        payload,
    );
    env.send(ix, &relay).expect("submitting is allowed");

    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();
    let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);

    // Alone, so wait out the timeout.
    let mut clock = env.svm.get_sysvar::<solana_program::clock::Clock>();
    clock.unix_timestamp += mirror_pool_program::processor::SETTLE_TIMEOUT_SECONDS + 1;
    env.svm.set_sysvar(&clock);

    let batch = vec![(spend_pda, beneficiary, relay.insecure_clone())];
    // Alone after the timeout, so the under-floor path has to be asked for.
    let ix = settle_ix_with_targets(&env, &batch, &settler.pubkey(), &[Some(self_target)], true);
    let err = env.send(ix, &settler).expect_err("the pool invoked itself");
    println!("self-invocation rejected: {err}");
    assert!(
        err.contains("Custom(23)"),
        "expected SelfInvocationRefused: {err}"
    );
}

/// The griefing vector an adversarial review found: `action_accounts` was
/// relay-supplied, written verbatim into the record, and outside the binding.
///
/// A relay handed a valid transfer proof could submit it with a count that
/// settlement can never satisfy. The nullifier burns, the record cannot be
/// amended, there is no refund instruction, and the note is destroyed for free.
/// The relay forfeits only its fee.
#[test]
fn a_relay_cannot_declare_an_account_count_the_member_did_not_authorise() {
    let (mut env, tree, notes, keys) = seeded_pool(5);
    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();

    // Proved for a plain transfer, which takes no action accounts.
    let proof = proof_for(&keys, &tree, &notes, 0, &beneficiary, &relay.pubkey());
    let nullifier = proof.public_inputs[1];
    let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);

    let ix = Instruction::new_with_bytes(
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
            action_accounts: 1, // the member authorised zero
            payload: Vec::new(),
        }
        .pack(),
        vec![
            AccountMeta::new(relay.pubkey(), true),
            AccountMeta::new(env.pool, false),
            AccountMeta::new(spend_pda, false),
            AccountMeta::new_readonly(solana_system_interface::program::ID, false),
        ],
    );
    let err = env
        .send(ix, &relay)
        .expect_err("a relay inflated the account count and burnt the note");
    println!("inflated action_accounts rejected: {err}");
    assert!(
        err.contains("Custom(1)"),
        "expected MalformedInstruction: {err}"
    );

    // The nullifier must still be spendable: the griefing attempt cost the
    // member nothing.
    assert!(
        env.svm.get_account(&spend_pda).is_none(),
        "the spend record was created, so the note is now unspendable"
    );
    let ok = spend_ix(&env, &proof, nullifier, &beneficiary, &relay.pubkey());
    env.send(ok, &relay)
        .expect("the note must remain spendable after a failed griefing attempt");
}

#[test]
fn an_unknown_selector_is_refused_before_the_nullifier_burns() {
    let (mut env, tree, notes, keys) = seeded_pool(5);
    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();

    let merkle_proof = tree.proof(1).unwrap();
    let binding = mirror_core::action_binding(
        99,
        &[0u8; 32],
        &beneficiary.to_bytes(),
        &relay.pubkey().to_bytes(),
        RELAY_FEE,
        0,
        &[],
    );
    let witness = Witness {
        note: notes[1],
        merkle_proof: &merkle_proof,
        root: tree.root().unwrap(),
        action_binding: binding,
    };
    let mut rng = {
        use ark_std::rand::SeedableRng;
        ark_std::rand::rngs::StdRng::from_seed([77u8; 32])
    };
    let proof = prove(&keys, &witness, &mut rng).expect("proving");
    let nullifier = proof.public_inputs[1];
    let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);

    let ix = Instruction::new_with_bytes(
        env.program_id,
        &MirrorIx::SubmitSpend {
            proof_a: proof.proof_a,
            proof_b: proof.proof_b,
            proof_c: proof.proof_c,
            root: proof.public_inputs[0],
            nullifier,
            selector: 99,
            target_program: [0u8; 32],
            beneficiary: beneficiary.to_bytes(),
            relay_fee: RELAY_FEE,
            action_accounts: 0,
            payload: Vec::new(),
        }
        .pack(),
        vec![
            AccountMeta::new(relay.pubkey(), true),
            AccountMeta::new(env.pool, false),
            AccountMeta::new(spend_pda, false),
            AccountMeta::new_readonly(solana_system_interface::program::ID, false),
        ],
    );
    let err = env
        .send(ix, &relay)
        .expect_err("an unknown selector was stored");
    assert!(
        err.contains("Custom(22)"),
        "expected UnknownSelector: {err}"
    );
    assert!(
        env.svm.get_account(&spend_pda).is_none(),
        "the nullifier burnt for a selector settlement can never execute"
    );
}

/// Squatting a PDA must not brick anything.
///
/// Every address this program creates is derived from public data — a spend PDA
/// from the nullifier, which is visible in the `submit_spend` instruction
/// itself. `create_account` fails when the destination already holds lamports,
/// so a bystander who front-runs the transaction with the rent-exempt minimum
/// would have made the note permanently unspendable for about 0.00089 SOL. The
/// relay the proof was handed to is the obvious candidate.
///
/// The three-step create is immune: a squatter can send lamports but cannot
/// allocate or assign without the PDA's seeds, so their deposit only reduces
/// what the legitimate payer owes.
#[test]
fn squatting_a_spend_pda_does_not_brick_the_note() {
    let (mut env, tree, notes, keys) = seeded_pool(5);
    let beneficiary = Pubkey::new_unique();
    let relay = Keypair::new();
    env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();

    let proof = proof_for(&keys, &tree, &notes, 0, &beneficiary, &relay.pubkey());
    let nullifier = proof.public_inputs[1];
    let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);

    // The attacker reads the nullifier and funds the PDA first.
    let squatter = Keypair::new();
    env.svm.airdrop(&squatter.pubkey(), 10_000_000_000).unwrap();
    let squat = Instruction::new_with_bytes(
        solana_system_interface::program::ID,
        &solana_system_interface::instruction::transfer(&squatter.pubkey(), &spend_pda, 890_880)
            .data,
        vec![
            AccountMeta::new(squatter.pubkey(), true),
            AccountMeta::new(spend_pda, false),
        ],
    );
    env.send(squat, &squatter).expect("squatting the pda");
    assert!(
        env.svm.get_account(&spend_pda).unwrap().lamports > 0,
        "precondition: the pda is funded by a stranger"
    );

    // The spend must still work.
    let ix = spend_ix(&env, &proof, nullifier, &beneficiary, &relay.pubkey());
    env.send(ix, &relay)
        .expect("a squatted pda made the note unspendable");

    let mut data = env.svm.get_account(&spend_pda).unwrap().data;
    let record = Spend::load(&mut data).unwrap();
    assert_eq!(record.status(), STATUS_PENDING);
    assert_eq!(record.beneficiary(), beneficiary.to_bytes());
}

#[test]
fn squatting_a_pool_pda_does_not_prevent_the_pool() {
    let mut env = setup();
    let squatter = Keypair::new();
    env.svm.airdrop(&squatter.pubkey(), 10_000_000_000).unwrap();

    // Deny the canonical pool for this denomination, for the price of rent.
    for target in [env.pool, env.vault] {
        let squat = Instruction::new_with_bytes(
            solana_system_interface::program::ID,
            &solana_system_interface::instruction::transfer(&squatter.pubkey(), &target, 890_880)
                .data,
            vec![
                AccountMeta::new(squatter.pubkey(), true),
                AccountMeta::new(target, false),
            ],
        );
        env.send(squat, &squatter).expect("squatting");
    }

    env.init_pool_with_timeout(0);
    let mut data = env.pool_state();
    let pool = Pool::load(&mut data).unwrap();
    assert_eq!(pool.denomination(), DENOMINATION);
    assert_eq!(pool.k_floor(), K_FLOOR);
}

/// A batch whose members paid different relay fees settles into visibly
/// different payouts, and that is an anonymity set an observer can partition by
/// reading balances — no proof broken, no secret learned.
///
/// The crowd rule, the single settling signature and the shared timestamp all
/// exist to prevent exactly that partition, so a mixed-fee batch is refused
/// rather than documented. It is a property of the batch and not of any record,
/// which is why the check lives in settlement: a member may agree any fee with
/// their relay, and settles with the members who agreed the same one.
#[test]
fn a_batch_whose_members_paid_different_fees_is_refused() {
    let (mut env, tree, notes, keys) = seeded_pool(5);
    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();

    // Four members — the pool's floor — identical in every respect except what
    // the last one's relay charged.
    let fees = [RELAY_FEE, RELAY_FEE, RELAY_FEE, RELAY_FEE + 1];
    let mut batch = Vec::new();
    for (i, fee) in fees.iter().enumerate() {
        let beneficiary = Pubkey::new_unique();
        let relay = Keypair::new();
        env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();
        let (nullifier, ix) = spend_ix_at_fee(
            &env,
            &keys,
            &tree,
            &notes,
            i,
            &beneficiary,
            &relay.pubkey(),
            *fee,
        );
        env.send(ix, &relay).expect("submitting at an agreed fee");
        let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);
        batch.push((spend_pda, beneficiary, relay));
    }

    let err = env
        .send(settle_ix(&env, &batch, &settler.pubkey()), &settler)
        .expect_err("a mixed-fee batch must not settle");
    assert!(
        err.contains("Custom(25)"),
        "expected FeeNotUniform, got {err}"
    );

    // Refused, not partially applied: neither member was paid and neither record
    // was consumed, so both can still settle with somebody who paid what they did.
    for (spend, beneficiary, _) in &batch {
        let mut data = env.svm.get_account(spend).unwrap().data;
        assert_eq!(Spend::load(&mut data).unwrap().status(), STATUS_PENDING);
        // The beneficiary was never funded, so the account does not exist at
        // all — which is the strongest form of "nothing was paid".
        assert_eq!(
            env.svm
                .get_account(beneficiary)
                .map(|a| a.lamports)
                .unwrap_or(0),
            0
        );
    }
}

/// The same two members, once their fees agree, settle normally — so the check
/// above rejects the mixture and not the members.
#[test]
fn the_same_members_settle_once_their_fees_agree() {
    let (mut env, tree, notes, keys) = seeded_pool(5);
    let settler = Keypair::new();
    env.svm.airdrop(&settler.pubkey(), 10_000_000_000).unwrap();

    let odd_fee = RELAY_FEE + 1;
    let mut batch = Vec::new();
    for i in 0..4 {
        let beneficiary = Pubkey::new_unique();
        let relay = Keypair::new();
        env.svm.airdrop(&relay.pubkey(), 10_000_000_000).unwrap();
        let (nullifier, ix) = spend_ix_at_fee(
            &env,
            &keys,
            &tree,
            &notes,
            i,
            &beneficiary,
            &relay.pubkey(),
            odd_fee,
        );
        env.send(ix, &relay).expect("submitting");
        let (spend_pda, _) = spend_address(&env.program_id, &env.pool, &nullifier);
        batch.push((spend_pda, beneficiary, relay));
    }

    env.send(settle_ix(&env, &batch, &settler.pubkey()), &settler)
        .expect("a uniform batch settles whatever the fee is");

    // Equal payouts is the property, and it is the thing to assert.
    let paid: Vec<u64> = batch
        .iter()
        .map(|(_, b, _)| env.svm.get_account(b).unwrap().lamports)
        .collect();
    assert!(
        paid.iter().all(|p| *p == DENOMINATION - odd_fee),
        "members were paid different amounts: {paid:?}"
    );
}
