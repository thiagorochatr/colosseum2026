//! The compliant pool end to end in a real SVM: mirror-pool's own pool,
//! deposit and settlement, the compliance program's curator and clearance,
//! and SubmitClearedSpend between them, with real proofs from the ceremony's
//! final key made through the SDK.

use compliance_interface::{clearance_address, curator_address, tag, Clearance};
use compliance_program::verify::ProofRequest;
use compliant_pool_program::{cleared::ClearedSpend, TAG_SUBMIT_CLEARED_SPEND};
use litesvm::LiteSVM;
use mirror_core::{action_binding, commitment, nullifier, poseidon2, Field, MerkleTree};
use mirror_pool_program::{
    instruction::Instruction as MirrorIx,
    pda::{pool_address, spend_address, vault_address},
    state::Pool,
};
use solana_account::Account;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::process::Command;

const SYSTEM: Pubkey = solana_system_interface::program::ID;
const DENOMINATION: u64 = 1_000_000_000;
const FEE: u64 = 5_000_000;
const COMPLIANCE_ID: Pubkey = Pubkey::from_str_const(env!("MIRROR_COMPLIANCE_PROGRAM_ID"));

fn so(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../../target/sbf/deploy/{name}.so",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("could not read {path}: {e}\n\nRun `make build-sbf` first."))
}

fn keypair(name: &str) -> Keypair {
    let path = format!(
        "{}/../../tests/keys/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes: Vec<u8> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    Keypair::try_from(bytes.as_slice()).unwrap()
}

fn f(v: u64) -> Field {
    Field::from_u64(v)
}

#[derive(Clone, Copy)]
struct Note {
    k: Field,
    r: Field,
}

impl Note {
    fn new(i: u64) -> Self {
        Note {
            k: poseidon2(f(0xC0FFEE), f(i)).unwrap(),
            r: poseidon2(f(0xBEEF), f(i)).unwrap(),
        }
    }
    fn leaf(&self) -> Field {
        commitment(self.k, self.r, f(DENOMINATION)).unwrap()
    }
    fn nullifier(&self) -> [u8; 32] {
        nullifier(self.k).unwrap().to_bytes()
    }
}

fn dec(x: &Field) -> String {
    format!("0x{}", hex::encode(x.to_bytes()))
}

struct Env {
    svm: LiteSVM,
    program: Pubkey,
    curator: Keypair,
    pool: Pubkey,
    vault: Pubkey,
    leaves: Vec<Field>,
}

impl Env {
    fn new() -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program(COMPLIANCE_ID, &so("compliance_program"))
            .unwrap();
        let program = Pubkey::new_unique();
        svm.add_program(program, &so("compliant_pool_program"))
            .unwrap();
        let curator = keypair("curator");
        svm.airdrop(&curator.pubkey(), 10_000_000_000).unwrap();
        let pool = pool_address(&program, DENOMINATION).0;
        let vault = vault_address(&program, &pool).0;
        let mut env = Env {
            svm,
            program,
            curator,
            pool,
            vault,
            leaves: vec![],
        };
        let payer = env.funded();
        let ix = Instruction::new_with_bytes(
            program,
            &MirrorIx::InitPool {
                denomination: DENOMINATION,
                entry_fee: 0,
                k_floor: 2,
                settle_timeout_seconds: 0,
            }
            .pack(),
            vec![
                AccountMeta::new(payer.pubkey(), true),
                AccountMeta::new(pool, false),
                AccountMeta::new(vault, false),
                AccountMeta::new_readonly(SYSTEM, false),
            ],
        );
        env.send(&[ix], &[&payer]).unwrap();
        env
    }

    fn funded(&mut self) -> Keypair {
        let k = Keypair::new();
        self.svm.airdrop(&k.pubkey(), 10_000_000_000).unwrap();
        k
    }

    fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> Result<u64, String> {
        let msg = Message::new(ixs, Some(&signers[0].pubkey()));
        self.svm.expire_blockhash();
        let tx = Transaction::new(signers, msg, self.svm.latest_blockhash());
        self.svm
            .send_transaction(tx)
            .map(|m| m.compute_units_consumed)
            .map_err(|e| format!("{:?}", e.err))
    }

    fn deposit(&mut self, note: &Note) {
        let user = self.funded();
        let ix = Instruction::new_with_bytes(
            self.program,
            &MirrorIx::Deposit {
                commitment: note.leaf().to_bytes(),
            }
            .pack(),
            vec![
                AccountMeta::new(user.pubkey(), true),
                AccountMeta::new(self.pool, false),
                AccountMeta::new(self.vault, false),
                AccountMeta::new_readonly(SYSTEM, false),
            ],
        );
        self.send(&[ix], &[&user]).unwrap();
        self.leaves.push(note.leaf());
    }

    fn curator_pda(&self, authority: &Pubkey) -> Pubkey {
        curator_address(&COMPLIANCE_ID, authority).0
    }

    fn init_curator(&mut self, authority: &Keypair) {
        let ix = Instruction::new_with_bytes(
            COMPLIANCE_ID,
            &[&[tag::INIT_CURATOR][..], &[1u8; 32]].concat(),
            vec![
                AccountMeta::new(authority.pubkey(), true),
                AccountMeta::new(self.curator_pda(&authority.pubkey()), false),
                AccountMeta::new_readonly(SYSTEM, false),
            ],
        );
        self.send(&[ix], &[authority]).unwrap();
    }

    /// The curator publishes the clean root of the pool with `excluded` masked.
    fn publish(&mut self, authority: &Keypair, excluded: &[usize]) {
        let mut clean = MerkleTree::new().unwrap();
        for (i, l) in self.leaves.iter().enumerate() {
            clean
                .insert(if excluded.contains(&i) {
                    Field::ZERO
                } else {
                    *l
                })
                .unwrap();
        }
        let ix = Instruction::new_with_bytes(
            COMPLIANCE_ID,
            &[&[tag::PUBLISH_ROOT][..], &clean.root().unwrap().to_bytes()].concat(),
            vec![
                AccountMeta::new_readonly(authority.pubkey(), true),
                AccountMeta::new(self.curator_pda(&authority.pubkey()), false),
            ],
        );
        self.send(&[ix], &[authority]).unwrap();
    }

    fn pool_knows(&self, root: &[u8; 32]) -> bool {
        let mut data = self.svm.get_account(&self.pool).unwrap().data;
        Pool::load(&mut data)
            .unwrap()
            .knows_root(Field::from_bytes(*root).unwrap())
    }

    fn prove(
        &self,
        note: &Note,
        index: usize,
        excluded: &[usize],
        binding: Field,
    ) -> Option<ProofRequest> {
        let script = format!("{}/tests/prove.mjs", env!("CARGO_MANIFEST_DIR"));
        let ex = if excluded.is_empty() {
            "-".into()
        } else {
            excluded
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        let leaves = self.leaves.iter().map(dec).collect::<Vec<_>>().join(",");
        let out = Command::new("node")
            .args([
                &script,
                &dec(&note.k),
                &dec(&note.r),
                &DENOMINATION.to_string(),
                &index.to_string(),
                &dec(&binding),
                &ex,
                &leaves,
            ])
            .output()
            .expect("node");
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("prover output");
        if !v["ok"].as_bool().unwrap() {
            return None;
        }
        let arr = |s: &serde_json::Value| hex::decode(s.as_str().unwrap()).unwrap();
        let inputs: Vec<[u8; 32]> = v["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| arr(x).try_into().unwrap())
            .collect();
        Some(ProofRequest {
            a: arr(&v["a"]).try_into().unwrap(),
            b: arr(&v["b"]).try_into().unwrap(),
            c: arr(&v["c"]).try_into().unwrap(),
            inputs: inputs.try_into().unwrap(),
        })
    }

    fn verify_clearance_ix(
        &self,
        relay: &Pubkey,
        curator: &Pubkey,
        pool: &Pubkey,
        req: &ProofRequest,
    ) -> Instruction {
        Instruction::new_with_bytes(
            COMPLIANCE_ID,
            &[&[tag::VERIFY_CLEARANCE][..], &req.encode()].concat(),
            vec![
                AccountMeta::new(*relay, true),
                AccountMeta::new_readonly(self.curator_pda(curator), false),
                AccountMeta::new_readonly(*pool, false),
                AccountMeta::new(
                    clearance_address(&COMPLIANCE_ID, pool, curator, &req.inputs[2]).0,
                    false,
                ),
                AccountMeta::new_readonly(SYSTEM, false),
            ],
        )
    }

    fn cleared_spend_ix(
        &self,
        relay: &Pubkey,
        note: &Note,
        beneficiary: &Pubkey,
        clearance: Pubkey,
    ) -> Instruction {
        let req = ClearedSpend {
            nullifier: note.nullifier(),
            selector: 0,
            target_program: [0u8; 32],
            beneficiary: beneficiary.to_bytes(),
            relay_fee: FEE,
            action_accounts: 0,
            payload: vec![],
        };
        Instruction::new_with_bytes(
            self.program,
            &[&[TAG_SUBMIT_CLEARED_SPEND][..], &req.encode()].concat(),
            vec![
                AccountMeta::new(*relay, true),
                AccountMeta::new(self.pool, false),
                AccountMeta::new(
                    spend_address(&self.program, &self.pool, &note.nullifier()).0,
                    false,
                ),
                AccountMeta::new_readonly(SYSTEM, false),
                AccountMeta::new_readonly(clearance, false),
            ],
        )
    }

    fn binding_for(&self, relay: &Pubkey, beneficiary: &Pubkey) -> Field {
        action_binding(
            0,
            &[0u8; 32],
            &beneficiary.to_bytes(),
            &relay.to_bytes(),
            FEE,
            0,
            &[],
        )
    }

    fn clearance_of(&self, note: &Note) -> Pubkey {
        clearance_address(
            &COMPLIANCE_ID,
            &self.pool,
            &self.curator.pubkey(),
            &note.nullifier(),
        )
        .0
    }

    /// Proves `note`, then verifies and spends in ONE transaction.
    fn clear_and_spend(
        &mut self,
        relay: &Keypair,
        note: &Note,
        index: usize,
        excluded: &[usize],
        beneficiary: &Pubkey,
    ) -> Result<u64, String> {
        let req = self
            .prove(
                note,
                index,
                excluded,
                self.binding_for(&relay.pubkey(), beneficiary),
            )
            .expect("provable");
        assert!(
            self.pool_knows(&req.inputs[0]),
            "the SDK's pool root is one the pool holds"
        );
        let curator = self.curator.pubkey();
        let ixs = [
            self.verify_clearance_ix(&relay.pubkey(), &curator, &self.pool.clone(), &req),
            self.cleared_spend_ix(&relay.pubkey(), note, beneficiary, self.clearance_of(note)),
        ];
        self.send(&ixs, &[relay])
    }

    fn settle(&mut self, spends: &[(&Note, Pubkey, Pubkey)]) -> Result<u64, String> {
        let settler = self.funded();
        let mut metas = vec![
            AccountMeta::new(settler.pubkey(), true),
            AccountMeta::new(self.pool, false),
            AccountMeta::new(self.vault, false),
        ];
        for (note, beneficiary, relay) in spends {
            metas.push(AccountMeta::new(
                spend_address(&self.program, &self.pool, &note.nullifier()).0,
                false,
            ));
            metas.push(AccountMeta::new(*beneficiary, false));
            metas.push(AccountMeta::new(*relay, false));
        }
        let ix = Instruction::new_with_bytes(
            self.program,
            &MirrorIx::SettleEpoch {
                count: spends.len() as u8,
                allow_below_floor: false,
            }
            .pack(),
            metas,
        );
        self.send(&[ix], &[&settler])
    }
}

fn code(n: u32) -> String {
    format!("Custom({n})")
}

/// Three deposits, Bob's excluded, curator ready.
fn scenario() -> (Env, [Note; 3]) {
    let mut env = Env::new();
    let notes = [Note::new(0), Note::new(1), Note::new(2)];
    for n in &notes {
        env.deposit(n);
    }
    let curator = env.curator.insecure_clone();
    env.init_curator(&curator);
    env.publish(&curator, &[1]);
    (env, notes)
}

#[test]
fn clean_notes_clear_spend_and_settle_through_mirror_pool() {
    let (mut env, notes) = scenario();
    let relay = env.funded();
    let alice_to = Pubkey::new_unique();
    let carol_to = Pubkey::new_unique();
    let cu = env
        .clear_and_spend(&relay, &notes[0], 0, &[1], &alice_to)
        .unwrap();
    println!("VerifyClearance + SubmitClearedSpend, one transaction: {cu} CU");
    env.clear_and_spend(&relay, &notes[2], 2, &[1], &carol_to)
        .unwrap();

    let c = Clearance::read(
        &env.svm
            .get_account(&env.clearance_of(&notes[0]))
            .unwrap()
            .data,
    )
    .unwrap();
    assert_eq!(c.pool, env.pool);
    assert_eq!(c.curator_authority, env.curator.pubkey());

    let relay_before = env.svm.get_balance(&relay.pubkey()).unwrap();
    env.settle(&[
        (&notes[0], alice_to, relay.pubkey()),
        (&notes[2], carol_to, relay.pubkey()),
    ])
    .unwrap();
    assert_eq!(env.svm.get_balance(&alice_to).unwrap(), DENOMINATION - FEE);
    assert_eq!(env.svm.get_balance(&carol_to).unwrap(), DENOMINATION - FEE);
    assert_eq!(
        env.svm.get_balance(&relay.pubkey()).unwrap(),
        relay_before + 2 * FEE
    );
}

#[test]
fn an_excluded_note_cannot_be_proved() {
    let (env, notes) = scenario();
    let relay = Pubkey::new_unique();
    let b = env.binding_for(&relay, &Pubkey::new_unique());
    assert!(env.prove(&notes[1], 1, &[1], b).is_none());
}

#[test]
fn the_plain_spend_is_refused() {
    let (mut env, _) = scenario();
    let relay = env.funded();
    let mut data = vec![2u8];
    data.extend(vec![0u8; 403]);
    let ix = Instruction::new_with_bytes(
        env.program,
        &data,
        vec![AccountMeta::new(relay.pubkey(), true)],
    );
    assert!(env.send(&[ix], &[&relay]).unwrap_err().contains(&code(100)));
}

#[test]
fn a_spend_without_a_clearance_is_refused() {
    let (mut env, notes) = scenario();
    let relay = env.funded();
    let ix = env.cleared_spend_ix(
        &relay.pubkey(),
        &notes[0],
        &Pubkey::new_unique(),
        env.clearance_of(&notes[0]),
    );
    assert!(env.send(&[ix], &[&relay]).unwrap_err().contains(&code(101)));
}

#[test]
fn a_clearance_from_an_earlier_slot_is_stale() {
    let (mut env, notes) = scenario();
    let relay = env.funded();
    let to = Pubkey::new_unique();
    let req = env
        .prove(&notes[0], 0, &[1], env.binding_for(&relay.pubkey(), &to))
        .unwrap();
    let curator = env.curator.pubkey();
    let pool = env.pool;
    let ix = env.verify_clearance_ix(&relay.pubkey(), &curator, &pool, &req);
    env.send(&[ix], &[&relay]).unwrap();
    let slot = env.svm.get_sysvar::<solana_program::clock::Clock>().slot;
    env.svm.warp_to_slot(slot + 1);
    let ix = env.cleared_spend_ix(&relay.pubkey(), &notes[0], &to, env.clearance_of(&notes[0]));
    assert!(env.send(&[ix], &[&relay]).unwrap_err().contains(&code(105)));
}

#[test]
fn a_clearance_written_alone_in_an_earlier_slot_does_not_freeze_the_note() {
    // Someone who saw the proof sends VerifyClearance alone. In a later slot,
    // the real relay sends both instructions together, and the spend lands.
    let (mut env, notes) = scenario();
    let relay = env.funded();
    let squatter = env.funded();
    let to = Pubkey::new_unique();
    let req = env
        .prove(&notes[0], 0, &[1], env.binding_for(&relay.pubkey(), &to))
        .unwrap();
    let curator = env.curator.pubkey();
    let pool = env.pool;
    let ix = env.verify_clearance_ix(&squatter.pubkey(), &curator, &pool, &req);
    env.send(&[ix], &[&squatter]).unwrap();
    let slot = env.svm.get_sysvar::<solana_program::clock::Clock>().slot;
    env.svm.warp_to_slot(slot + 1);
    env.svm.expire_blockhash();
    let verify = env.verify_clearance_ix(&relay.pubkey(), &curator, &pool, &req);
    let spend = env.cleared_spend_ix(&relay.pubkey(), &notes[0], &to, env.clearance_of(&notes[0]));
    env.send(&[verify, spend], &[&relay]).unwrap();
}

#[test]
fn a_clearance_written_alone_in_the_same_slot_does_not_block_the_spend() {
    // The same race inside one slot: someone lands VerifyClearance just before
    // the relay. The relay's own VerifyClearance rewrites it and the spend lands.
    let (mut env, notes) = scenario();
    let relay = env.funded();
    let squatter = env.funded();
    let to = Pubkey::new_unique();
    let req = env
        .prove(&notes[0], 0, &[1], env.binding_for(&relay.pubkey(), &to))
        .unwrap();
    let curator = env.curator.pubkey();
    let pool = env.pool;
    let ix = env.verify_clearance_ix(&squatter.pubkey(), &curator, &pool, &req);
    env.send(&[ix], &[&squatter]).unwrap();
    let verify = env.verify_clearance_ix(&relay.pubkey(), &curator, &pool, &req);
    let spend = env.cleared_spend_ix(&relay.pubkey(), &notes[0], &to, env.clearance_of(&notes[0]));
    env.send(&[verify, spend], &[&relay]).unwrap();
}

#[test]
fn a_clearance_for_another_beneficiary_does_not_match() {
    let (mut env, notes) = scenario();
    let relay = env.funded();
    let to = Pubkey::new_unique();
    let req = env
        .prove(&notes[0], 0, &[1], env.binding_for(&relay.pubkey(), &to))
        .unwrap();
    let curator = env.curator.pubkey();
    let pool = env.pool;
    let ixs = [
        env.verify_clearance_ix(&relay.pubkey(), &curator, &pool, &req),
        env.cleared_spend_ix(
            &relay.pubkey(),
            &notes[0],
            &Pubkey::new_unique(),
            env.clearance_of(&notes[0]),
        ),
    ];
    assert!(env.send(&ixs, &[&relay]).unwrap_err().contains(&code(106)));
}

#[test]
fn a_clearance_from_another_relay_does_not_match() {
    // The binding names the relay, so a bystander cannot lift a clearance.
    let (mut env, notes) = scenario();
    let relay = env.funded();
    let thief = env.funded();
    let to = Pubkey::new_unique();
    let req = env
        .prove(&notes[0], 0, &[1], env.binding_for(&relay.pubkey(), &to))
        .unwrap();
    let curator = env.curator.pubkey();
    let pool = env.pool;
    let ixs = [
        env.verify_clearance_ix(&thief.pubkey(), &curator, &pool, &req),
        env.cleared_spend_ix(&thief.pubkey(), &notes[0], &to, env.clearance_of(&notes[0])),
    ];
    assert!(env.send(&ixs, &[&thief]).unwrap_err().contains(&code(106)));
}

#[test]
fn a_clearance_under_another_curator_is_refused() {
    let (mut env, notes) = scenario();
    let other = env.funded();
    env.init_curator(&other);
    env.publish(&other, &[]);
    let relay = env.funded();
    let to = Pubkey::new_unique();
    let req = env
        .prove(&notes[0], 0, &[], env.binding_for(&relay.pubkey(), &to))
        .unwrap();
    let pool = env.pool;
    // The other curator's clearance lives at its own address, so it cannot
    // overwrite the one naming this pool's curator; handed to the pool, it is refused.
    let theirs = clearance_address(&COMPLIANCE_ID, &pool, &other.pubkey(), &req.inputs[2]).0;
    assert_ne!(theirs, env.clearance_of(&notes[0]));
    let ixs = [
        env.verify_clearance_ix(&relay.pubkey(), &other.pubkey(), &pool, &req),
        env.cleared_spend_ix(&relay.pubkey(), &notes[0], &to, theirs),
    ];
    assert!(env.send(&ixs, &[&relay]).unwrap_err().contains(&code(104)));
}

#[test]
fn a_pool_of_another_denomination_cannot_be_created() {
    let mut env = Env::new();
    let payer = env.funded();
    let other = DENOMINATION + 1;
    let pool = pool_address(&env.program, other).0;
    let vault = vault_address(&env.program, &pool).0;
    let ix = Instruction::new_with_bytes(
        env.program,
        &MirrorIx::InitPool {
            denomination: other,
            entry_fee: 0,
            k_floor: 2,
            settle_timeout_seconds: 0,
        }
        .pack(),
        vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(pool, false),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(SYSTEM, false),
        ],
    );
    assert!(env.send(&[ix], &[&payer]).unwrap_err().contains(&code(107)));
}

#[test]
fn another_pool_cannot_spend_against_this_pools_clean_set() {
    // The attack one pool per deployment closes: a second pool, holding a
    // commitment identical to a clean one here at the same index, proves its
    // pool root together with THIS pool's clean root. The compliance program
    // writes the clearance, since the curator holds that clean root, but the
    // compliant pool pays only the pool it serves.
    let (mut env, notes) = scenario();
    let real = env.svm.get_account(&env.pool).unwrap();
    let other = pool_address(&env.program, DENOMINATION + 1).0;
    env.svm.set_account(other, real).unwrap();
    let relay = env.funded();
    let to = Pubkey::new_unique();
    let req = env
        .prove(&notes[0], 0, &[1], env.binding_for(&relay.pubkey(), &to))
        .unwrap();
    let curator = env.curator.pubkey();
    let clearance = clearance_address(&COMPLIANCE_ID, &other, &curator, &req.inputs[2]).0;
    let verify = env.verify_clearance_ix(&relay.pubkey(), &curator, &other, &req);
    env.send(&[verify], &[&relay]).unwrap();
    let mut spend = env.cleared_spend_ix(&relay.pubkey(), &notes[0], &to, clearance);
    spend.accounts[1].pubkey = other;
    spend.accounts[2].pubkey = spend_address(&env.program, &other, &notes[0].nullifier()).0;
    assert!(env
        .send(&[spend], &[&relay])
        .unwrap_err()
        .contains(&code(107)));
}

#[test]
fn a_clearance_naming_a_fake_pool_is_refused() {
    // The compliance program does not check the pool's owner. This is why it
    // does not need to: the clearance names the fake account, not this pool.
    let (mut env, notes) = scenario();
    let real = env.svm.get_account(&env.pool).unwrap();
    let fake = Pubkey::new_unique();
    env.svm
        .set_account(
            fake,
            Account {
                owner: Pubkey::new_unique(),
                ..real
            },
        )
        .unwrap();
    let relay = env.funded();
    let to = Pubkey::new_unique();
    let req = env
        .prove(&notes[0], 0, &[1], env.binding_for(&relay.pubkey(), &to))
        .unwrap();
    let curator = env.curator.pubkey();
    let fake_clearance = clearance_address(&COMPLIANCE_ID, &fake, &curator, &req.inputs[2]).0;
    let ixs = [
        env.verify_clearance_ix(&relay.pubkey(), &curator, &fake, &req),
        env.cleared_spend_ix(&relay.pubkey(), &notes[0], &to, fake_clearance),
    ];
    assert!(env.send(&ixs, &[&relay]).unwrap_err().contains(&code(102)));
}

#[test]
fn one_notes_clearance_does_not_spend_another_note() {
    let (mut env, notes) = scenario();
    let relay = env.funded();
    let to = Pubkey::new_unique();
    let req = env
        .prove(&notes[0], 0, &[1], env.binding_for(&relay.pubkey(), &to))
        .unwrap();
    let curator = env.curator.pubkey();
    let pool = env.pool;
    let ixs = [
        env.verify_clearance_ix(&relay.pubkey(), &curator, &pool, &req),
        env.cleared_spend_ix(&relay.pubkey(), &notes[2], &to, env.clearance_of(&notes[0])),
    ];
    assert!(env.send(&ixs, &[&relay]).unwrap_err().contains(&code(103)));
}

#[test]
fn a_note_cannot_be_spent_twice() {
    let (mut env, notes) = scenario();
    let relay = env.funded();
    let to = Pubkey::new_unique();
    env.clear_and_spend(&relay, &notes[0], 0, &[1], &to)
        .unwrap();
    // A second clearance can be written, but the spend after it hits
    // mirror-pool's nullifier guard: the spend record already exists.
    let err = env
        .clear_and_spend(&relay, &notes[0], 0, &[1], &to)
        .unwrap_err();
    assert!(err.contains(&code(15)), "{err}");
    // And a spend reusing the existing clearance hits mirror-pool's nullifier guard.
    let ix = env.cleared_spend_ix(&relay.pubkey(), &notes[0], &to, env.clearance_of(&notes[0]));
    let err = env.send(&[ix], &[&relay]).unwrap_err();
    assert!(err.contains(&code(15)), "{err}");
}
