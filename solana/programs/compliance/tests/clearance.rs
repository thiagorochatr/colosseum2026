//! Curators and clearances, in a real SVM, against the built program.
//!
//! The pool here is a stand-in account with mirror-pool's layout holding the
//! vectors' pool root. tests/mirror_pool.rs runs the same flow against the
//! real mirror-pool program.

use compliance_interface::{clearance_address, curator_address, mirror_pool, tag, Clearance};
use compliance_program::verify::ProofRequest;
use litesvm::LiteSVM;
use solana_account::Account;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_signer::Signer;
use solana_transaction::Transaction;

fn program_bytes() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/sbf/deploy/compliance_program.so"
    );
    std::fs::read(path)
        .unwrap_or_else(|e| panic!("could not read {path}: {e}\n\nRun `make build-sbf` first."))
}

fn arr<const N: usize>(hex_str: &str) -> [u8; N] {
    hex::decode(hex_str).unwrap().try_into().unwrap()
}

fn fixtures() -> Vec<ProofRequest> {
    let v: serde_json::Value = serde_json::from_str(include_str!("fixtures/proofs.json")).unwrap();
    v["proofs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let ins: Vec<[u8; 32]> = p["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| arr(x.as_str().unwrap()))
                .collect();
            ProofRequest {
                a: arr(p["a"].as_str().unwrap()),
                b: arr(p["b"].as_str().unwrap()),
                c: arr(p["c"].as_str().unwrap()),
                inputs: ins.try_into().unwrap(),
            }
        })
        .collect()
}

const SYSTEM: Pubkey = solana_system_interface::program::ID;

struct Env {
    svm: LiteSVM,
    program: Pubkey,
    authority: Keypair,
    payer: Keypair,
    pool: Pubkey,
}

impl Env {
    fn new() -> Self {
        let mut svm = LiteSVM::new();
        let program = Pubkey::new_unique();
        svm.add_program(program, &program_bytes()).unwrap();
        let authority = Keypair::new();
        let payer = Keypair::new();
        svm.airdrop(&authority.pubkey(), 10_000_000_000).unwrap();
        svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();
        let pool = Pubkey::new_unique();
        let mut env = Env {
            svm,
            program,
            authority,
            payer,
            pool,
        };
        env.set_pool_roots(&[fixtures()[0].inputs[0]]);
        env
    }

    /// A stand-in pool: mirror-pool's layout, with `roots` in its history.
    fn set_pool_roots(&mut self, roots: &[[u8; 32]]) {
        let mut data = vec![0u8; mirror_pool::LEN];
        data[mirror_pool::VERSION_AT] = mirror_pool::VERSION;
        data[mirror_pool::DEPTH_AT] = mirror_pool::DEPTH;
        for (i, r) in roots.iter().enumerate() {
            let at = mirror_pool::ROOTS_AT + 32 * (i + 5);
            data[at..at + 32].copy_from_slice(r);
        }
        let account = Account {
            lamports: 1_000_000_000,
            data,
            owner: Pubkey::new_unique(),
            executable: false,
            rent_epoch: 0,
        };
        self.svm.set_account(self.pool, account).unwrap();
    }

    fn send(&mut self, ix: Instruction, signers: &[&Keypair]) -> Result<u64, String> {
        let msg = Message::new(&[ix], Some(&signers[0].pubkey()));
        self.svm.expire_blockhash();
        let tx = Transaction::new(signers, msg, self.svm.latest_blockhash());
        self.svm
            .send_transaction(tx)
            .map(|m| m.compute_units_consumed)
            .map_err(|e| format!("{:?}", e.err))
    }

    fn curator(&self) -> Pubkey {
        curator_address(&self.program, &self.authority.pubkey()).0
    }

    fn init_curator(&mut self) -> Result<u64, String> {
        let ix = Instruction::new_with_bytes(
            self.program,
            &[&[tag::INIT_CURATOR][..], &[9u8; 32]].concat(),
            vec![
                AccountMeta::new(self.authority.pubkey(), true),
                AccountMeta::new(self.curator(), false),
                AccountMeta::new_readonly(SYSTEM, false),
            ],
        );
        let a = self.authority.insecure_clone();
        self.send(ix, &[&a])
    }

    fn curator_ix(&self, t: u8, data: &[u8], signer: &Pubkey) -> Instruction {
        Instruction::new_with_bytes(
            self.program,
            &[&[t][..], data].concat(),
            vec![
                AccountMeta::new_readonly(*signer, true),
                AccountMeta::new(self.curator(), false),
            ],
        )
    }

    fn publish(&mut self, root: [u8; 32]) -> Result<u64, String> {
        let ix = self.curator_ix(tag::PUBLISH_ROOT, &root, &self.authority.pubkey());
        let a = self.authority.insecure_clone();
        self.send(ix, &[&a])
    }

    fn revoke(&mut self) -> Result<u64, String> {
        let ix = self.curator_ix(tag::REVOKE, &[], &self.authority.pubkey());
        let a = self.authority.insecure_clone();
        self.send(ix, &[&a])
    }

    fn clearance(&self, req: &ProofRequest) -> Pubkey {
        clearance_address(&self.program, &self.pool, &req.inputs[2]).0
    }

    fn verify_clearance_at(
        &mut self,
        req: &ProofRequest,
        clearance: Pubkey,
    ) -> Result<u64, String> {
        let ix = Instruction::new_with_bytes(
            self.program,
            &[&[tag::VERIFY_CLEARANCE][..], &req.encode()].concat(),
            vec![
                AccountMeta::new(self.payer.pubkey(), true),
                AccountMeta::new_readonly(self.curator(), false),
                AccountMeta::new_readonly(self.pool, false),
                AccountMeta::new(clearance, false),
                AccountMeta::new_readonly(SYSTEM, false),
            ],
        );
        let p = self.payer.insecure_clone();
        self.send(ix, &[&p])
    }

    fn verify_clearance(&mut self, req: &ProofRequest) -> Result<u64, String> {
        let c = self.clearance(req);
        self.verify_clearance_at(req, c)
    }
}

fn code(n: u32) -> String {
    format!("Custom({n})")
}

fn ready() -> (Env, ProofRequest) {
    let mut env = Env::new();
    let req = fixtures().remove(0);
    env.init_curator().unwrap();
    env.publish(req.inputs[1]).unwrap();
    (env, req)
}

#[test]
fn the_interface_layout_matches_mirror_pool() {
    assert_eq!(mirror_pool::LEN, mirror_pool_program::POOL_LEN);
    assert_eq!(mirror_pool::VERSION, mirror_pool_program::POOL_VERSION);
    assert_eq!(mirror_pool::ROOT_HISTORY, mirror_pool_program::ROOT_HISTORY);
    assert_eq!(
        mirror_pool::ROOTS_AT,
        mirror_pool::LEN - 32 * mirror_pool::ROOT_HISTORY
    );
}

#[test]
fn a_valid_proof_against_a_published_root_writes_a_clearance() {
    let (mut env, req) = ready();
    let cu = env.verify_clearance(&req).unwrap();
    println!("VerifyClearance: {cu} compute units");
    assert!(cu < 200_000, "used {cu}");

    let account = env.svm.get_account(&env.clearance(&req)).unwrap();
    assert_eq!(account.owner, env.program);
    let c = Clearance::read(&account.data).expect("a version-1 clearance");
    assert_eq!(c.pool, env.pool);
    assert_eq!(c.curator_authority, env.authority.pubkey());
    assert_eq!(c.nullifier, req.inputs[2]);
    assert_eq!(c.binding, req.inputs[3]);
    assert_eq!(c.clean_root, req.inputs[1]);
    assert_eq!(c.epoch, 0);
    assert_eq!(
        c.slot,
        env.svm.get_sysvar::<solana_program::clock::Clock>().slot
    );
}

#[test]
fn a_clearance_cannot_be_written_twice_in_one_slot() {
    let (mut env, req) = ready();
    env.verify_clearance(&req).unwrap();
    assert!(env.verify_clearance(&req).unwrap_err().contains(&code(10)));
}

#[test]
fn a_clearance_from_an_earlier_slot_is_replaced() {
    // Whoever saw the proof could write the clearance alone, in an earlier
    // slot than the spend. If that froze the account, the note could never
    // be spent: the spend needs a clearance from its own slot.
    let (mut env, req) = ready();
    env.verify_clearance(&req).unwrap();
    let slot = env.svm.get_sysvar::<solana_program::clock::Clock>().slot;
    env.svm.warp_to_slot(slot + 1);
    env.svm.expire_blockhash();
    env.verify_clearance(&req).unwrap();
    let c = Clearance::read(&env.svm.get_account(&env.clearance(&req)).unwrap().data).unwrap();
    assert_eq!(c.slot, slot + 1);
}

#[test]
fn a_root_the_curator_never_published_is_refused() {
    let mut env = Env::new();
    let req = fixtures().remove(0);
    env.init_curator().unwrap();
    env.publish([5u8; 32]).unwrap();
    assert!(env.verify_clearance(&req).unwrap_err().contains(&code(9)));
}

#[test]
fn revoking_invalidates_every_published_root_until_one_is_published_again() {
    let (mut env, req) = ready();
    env.revoke().unwrap();
    assert!(env.verify_clearance(&req).unwrap_err().contains(&code(9)));
    env.publish(req.inputs[1]).unwrap();
    env.verify_clearance(&req).unwrap();
    let c = Clearance::read(&env.svm.get_account(&env.clearance(&req)).unwrap().data).unwrap();
    assert_eq!(c.epoch, 1);
}

#[test]
fn a_pool_without_that_root_is_refused() {
    let (mut env, req) = ready();
    env.set_pool_roots(&[[3u8; 32]]);
    assert!(env.verify_clearance(&req).unwrap_err().contains(&code(8)));
}

#[test]
fn a_proof_with_a_changed_binding_is_refused() {
    let (mut env, mut req) = ready();
    req.inputs[3][31] ^= 1;
    assert!(env.verify_clearance(&req).unwrap_err().contains(&code(3)));
}

#[test]
fn the_clearance_must_live_at_its_derived_address() {
    let (mut env, req) = ready();
    let wrong = Pubkey::new_unique();
    assert!(env
        .verify_clearance_at(&req, wrong)
        .unwrap_err()
        .contains(&code(5)));
}

#[test]
fn lamports_sent_ahead_to_the_clearance_address_do_not_block_it() {
    let (mut env, req) = ready();
    let c = env.clearance(&req);
    env.svm.airdrop(&c, 1_000_000).unwrap();
    env.verify_clearance(&req).unwrap();
}

#[test]
fn only_the_authority_can_publish_or_revoke() {
    let (mut env, req) = ready();
    let stranger = Keypair::new();
    env.svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();
    let ix = env.curator_ix(tag::PUBLISH_ROOT, &req.inputs[1], &stranger.pubkey());
    assert!(env.send(ix, &[&stranger]).unwrap_err().contains(&code(7)));
    let ix = env.curator_ix(tag::REVOKE, &[], &stranger.pubkey());
    assert!(env.send(ix, &[&stranger]).unwrap_err().contains(&code(7)));
}

#[test]
fn a_curator_is_created_once() {
    let mut env = Env::new();
    env.init_curator().unwrap();
    assert!(env.init_curator().unwrap_err().contains(&code(6)));
}

#[test]
fn a_zero_or_non_canonical_root_cannot_be_published() {
    let mut env = Env::new();
    env.init_curator().unwrap();
    assert!(env.publish([0u8; 32]).unwrap_err().contains(&code(2)));
    assert!(env.publish([0xffu8; 32]).unwrap_err().contains(&code(2)));
}

#[test]
fn the_ring_keeps_the_last_32_roots() {
    let (mut env, req) = ready();
    for i in 0..31u8 {
        env.publish([i + 1; 32]).unwrap();
    }
    // 32 roots published since the real one: it is the oldest, still held.
    env.verify_clearance(&req).unwrap();
    let mut env2 = Env::new();
    env2.init_curator().unwrap();
    env2.publish(req.inputs[1]).unwrap();
    for i in 0..32u8 {
        env2.publish([i + 1; 32]).unwrap();
    }
    // One more pushes it out.
    assert!(env2.verify_clearance(&req).unwrap_err().contains(&code(9)));
}

#[test]
fn the_authority_can_replace_the_policy_hash_and_nobody_else_can() {
    let (mut env, _) = ready();
    let at = compliance_interface::curator::at::POLICY_HASH;
    let read = |env: &Env| env.svm.get_account(&env.curator()).unwrap().data[at..at + 32].to_vec();
    assert_eq!(read(&env), vec![9u8; 32], "the hash set at creation");

    let ix = env.curator_ix(tag::UPDATE_POLICY, &[4u8; 32], &env.authority.pubkey());
    let a = env.authority.insecure_clone();
    env.send(ix, &[&a]).unwrap();
    assert_eq!(read(&env), vec![4u8; 32]);

    let stranger = Keypair::new();
    env.svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();
    let ix = env.curator_ix(tag::UPDATE_POLICY, &[5u8; 32], &stranger.pubkey());
    assert!(env.send(ix, &[&stranger]).unwrap_err().contains(&code(7)));
    let ix = env.curator_ix(tag::UPDATE_POLICY, &[5u8; 31], &env.authority.pubkey());
    assert!(env.send(ix, &[&a]).unwrap_err().contains(&code(1)));
    assert_eq!(read(&env), vec![4u8; 32], "neither attempt changed it");
}

#[test]
fn updating_the_policy_leaves_roots_and_epoch_alone() {
    let (mut env, req) = ready();
    let ix = env.curator_ix(tag::UPDATE_POLICY, &[4u8; 32], &env.authority.pubkey());
    let a = env.authority.insecure_clone();
    env.send(ix, &[&a]).unwrap();
    env.verify_clearance(&req).unwrap();
}
