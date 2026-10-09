//! Different members, different validators, one timestamp.
//!
//! `PROOF.md` shows the pool delegating stake as a member's authority, once, in
//! a batch whose other members were making payments. That establishes the
//! capability and leaves the harder question open: a batch of *identical*
//! actions is the easy case for an anonymity set, because there is nothing to
//! tell the members apart in the first place. The question worth answering is
//! what happens when the actions **diverge in content** — when each member picks
//! a different validator, which is exactly the behavioural pattern that
//! fingerprints a staker across time.
//!
//! So this run settles N delegations to N *different* validators in one
//! transaction, and reads every stake account back off the cluster to check that
//! each member got the validator they asked for.
//!
//! It also measures what that costs, because it does cost something. A
//! transaction names each distinct account once, and a batch of divergent
//! actions names more distinct accounts than a batch of uniform ones — one extra
//! vote account per member. The 1232-byte packet is therefore reached sooner,
//! and **the anonymity set a divergent batch can hold is smaller than the one a
//! uniform batch can hold**. That is a real trade-off in the design and it is
//! better measured than left for a reader to discover.
//!
//! Nothing here is simulated. The ceiling is found by serializing real
//! instructions, and the batch at that ceiling is settled against a live
//! cluster.

use crate::chain::Chain;
use crate::lookup::MAX_ACCOUNT_LOCKS as MAX_LOCKS;
use crate::note::StoredNote;
use crate::soak::{keys, read_keypair, Settlement, Step};
use crate::stake;
use anyhow::{anyhow, Context, Result};
use mirror_circuit::{prove, Keys, Witness};
use mirror_core::{MerkleTree, Note};
use mirror_pool_program::{
    instruction::Instruction as MirrorIx,
    pda::{pool_address, spend_address, vault_address},
    processor::SELECTOR_INVOKE_SIGNED,
};
use solana_keypair::Keypair;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_signer::Signer;
use solana_transaction::Transaction;

/// A pool is unique per denomination, so this constant also selects the pool.
/// Bump it for a clean run; the note ledger is keyed by it, so a new
/// denomination starts a new ledger too.
const DENOMINATION: u64 = 47_000_011; // 0.047 SOL
const ENTRY_FEE: u64 = 0;
const RELAY_FEE: u64 = 200_000;

/// What the operator puts into each stake account before the pool delegates it.
///
/// Devnet's minimum delegation is 1 SOL, which no sane pool denomination
/// reaches: a pool whose notes each cleared the floor would need six-plus SOL to
/// fill a single batch of this size. The operator funds the account and the
/// member's escrow is added to it. What the *pool* supplies is the authority,
/// which is the part being demonstrated.
const STAKE_FUNDING: u64 = 1_100_000_000;

/// Enough for a relay to pay for its own `submit_spend` and stay rent-exempt.
/// It is repaid at settlement out of the denomination.
const RELAY_FUNDING: u64 = 5_000_000;

/// The 1280-byte IPv6 minimum MTU, less a 40-byte IPv6 header and an 8-byte
/// fragment header. Written out rather than imported because the crates this
/// binary already depends on do not re-export it, and a number a reader can
/// derive is worth more than one they have to trust.
const PACKET_DATA_SIZE: usize = 1280 - 40 - 8;

/// What a transaction carrying a single non-budget instruction is given unless
/// it asks for more. Asking costs a second instruction, and a second instruction
/// costs bytes a full settlement has none of.
const DEFAULT_COMPUTE_BUDGET: u64 = 200_000;

/// One member's delegation, and the validator they chose.
pub struct Member {
    pub stake: Pubkey,
    pub vote: Pubkey,
    pub relay: Pubkey,
}

/// What the run measured about how many members fit.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Ceiling {
    /// Members per settlement when each chooses a different validator.
    pub divergent: usize,
    /// Members per settlement when they all choose the same one.
    pub uniform: usize,
    /// Wire size of a full divergent settlement.
    pub divergent_bytes: usize,
    /// Wire size of one member more than fits.
    pub over_bytes: usize,
    /// What adding a `SetComputeUnitLimit` instruction costs in bytes.
    ///
    /// Measured because the two limits turn out to be coupled. Raising the
    /// compute budget is the obvious answer to a settlement that runs out of it,
    /// and it is paid for in the one currency a full batch has none of.
    pub budget_bytes: usize,

    /// The same two ceilings when the packet is taken out of the way by a lookup
    /// table, so what binds is the 64-account lock limit instead of 1232 bytes.
    ///
    /// A table names an account with one byte instead of thirty-two, which ends
    /// the byte argument and starts a different one. These are the answer to the
    /// question the earlier version of this report left open rather than
    /// guessed at.
    ///
    /// Both figures assume the `SetComputeUnitLimit` instruction is present,
    /// because for this shape it has to be: a batch large enough to be worth a
    /// table costs more compute than the 200,000 a transaction is given by
    /// default. That instruction brings its own program, and a program is an
    /// account — so raising the budget still costs a member, just in locks now
    /// rather than in bytes.
    #[serde(default)]
    pub divergent_through_table: usize,
    #[serde(default)]
    pub uniform_through_table: usize,
    /// Locks held by a full divergent settlement through a table, against the 64
    /// a transaction may hold.
    #[serde(default)]
    pub locks_at_divergent_table: usize,
}

/// Everything the report is rendered from.
///
/// Written after the run succeeds and read back by `--render-only`, so that
/// improving a sentence in the report costs nothing. A document generated from a
/// live run is worth more than a hand-written one, and it is only worth more
/// while regenerating it is cheap — otherwise the prose drifts away from the
/// generator that is supposed to produce it, quietly.
/// Addresses are stored as base58 text rather than as the byte arrays a
/// `Pubkey` serializes into, because this file is meant to be read: every value
/// in it can be pasted into an explorer and checked against the cluster, which
/// is the only reason to keep it at all.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Outcome {
    pub pool: String,
    pub vault: String,
    pub denomination: u64,
    pub ceiling: Ceiling,
    pub roster: Vec<MemberRecord>,
    pub signature: String,
    pub bytes: usize,
    pub compute_units: u64,
    pub steps: Vec<StepRecord>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct MemberRecord {
    pub stake: String,
    pub vote: String,
    pub relay: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct StepRecord {
    pub name: String,
    pub signature: String,
    pub note: String,
    /// The finalized slot, so a pruned transaction stays findable.
    ///
    /// `#[serde(default)]` because the committed result files predate this
    /// field. `--render-only` has to keep rebuilding this document from the run
    /// that produced it, and a schema change that made those files unreadable
    /// would break the one property that makes the numbers checkable.
    #[serde(default)]
    pub slot: Option<u64>,
}

pub struct Crowd {
    client: Chain,
    program_id: Pubkey,
    payer: Keypair,
    pool: Pubkey,
    vault: Pubkey,
    pub steps: Vec<Step>,
}

impl Crowd {
    pub fn new(url: &str, program_id: Pubkey, payer: Keypair) -> Self {
        let (pool, _) = pool_address(&program_id, DENOMINATION);
        let (vault, _) = vault_address(&program_id, &pool);
        Crowd {
            client: Chain::new(url),
            program_id,
            payer,
            pool,
            vault,
            steps: Vec::new(),
        }
    }

    pub fn pool(&self) -> Pubkey {
        self.pool
    }
    pub fn vault(&self) -> Pubkey {
        self.vault
    }

    fn send(&self, ix: Instruction, signers: &[&Keypair]) -> Result<String> {
        let blockhash = self.client.latest_blockhash()?;
        let message = solana_message::Message::new(&[ix], Some(&signers[0].pubkey()));
        let tx = Transaction::new(signers, message, blockhash);
        self.client.send(&tx)
    }

    fn record(&mut self, name: &'static str, signature: String, note: String) {
        println!("  {name:<28} {signature}");
        // Read back from the cluster, for the same reason the soak does it:
        // devnet prunes, and a signature with no slot beside it is not
        // recoverable once `getTransaction` has forgotten the transaction.
        let slot = self.client.slot(&signature).ok();
        self.steps.push(Step {
            name,
            signature,
            note,
            slot,
        });
    }

    fn pool_metas(&self) -> Vec<AccountMeta> {
        vec![
            AccountMeta::new(self.payer.pubkey(), true),
            AccountMeta::new(self.pool, false),
            AccountMeta::new(self.vault, false),
            AccountMeta::new_readonly(solana_system_interface::program::ID, false),
        ]
    }

    /// Creates the pool, unless a previous run already did.
    ///
    /// Notes left in the pool are allowed here, and that is deliberate. A run
    /// against a live cluster gets interrupted — a public endpoint times out
    /// mid-confirmation and the deposits are already made — and a tool that
    /// refuses to continue turns a transient network error into escrow nobody
    /// releases. What must not happen is a run *reporting* a batch it did not
    /// assemble, and that is checked where it can be checked properly: the
    /// leftovers must number exactly what this run needs, the ledger must hold
    /// their secrets, and every settled delegation is read back off the cluster
    /// at the end.
    pub fn init_pool(&mut self, k_floor: u32, members: usize) -> Result<()> {
        if let Some(mut data) = self.client.account_data(&self.pool)? {
            let pool = mirror_pool_program::Pool::load(&mut data)
                .map_err(|e| anyhow!("the account at the pool address is not a pool: {e:?}"))?;
            let outstanding = pool
                .outstanding_notes()
                .map_err(|e| anyhow!("reading the pool: {e:?}"))?;
            if outstanding as usize > members {
                return Err(anyhow!(
                    "the pool for denomination {DENOMINATION} holds {outstanding} unspent \
                     note(s) and this run needs {members}, so it cannot be the remains of \
                     this run. Raise DENOMINATION in crowd.rs for a clean pool — the note \
                     ledger is keyed by it, so a new denomination starts a new ledger too."
                ));
            }
            if pool.k_floor() != k_floor {
                return Err(anyhow!(
                    "the existing pool's floor is {} and this run needs {k_floor}. \
                     A pool's floor is fixed at creation, so raise DENOMINATION for a \
                     fresh one.",
                    pool.k_floor()
                ));
            }
            if outstanding > 0 {
                println!("  pool exists and holds {outstanding} note(s) from an interrupted run");
            } else {
                println!("  pool already exists and is empty, reusing it");
            }
            return Ok(());
        }
        let ix = Instruction::new_with_bytes(
            self.program_id,
            &MirrorIx::InitPool {
                denomination: DENOMINATION,
                entry_fee: ENTRY_FEE,
                k_floor,
                // The program's default. A run that needed its own timeout
                // would be measuring a pool nobody else would create.
                settle_timeout_seconds: 0,
            }
            .pack(),
            self.pool_metas(),
        );
        let payer = self.payer.insecure_clone();
        let sig = self.send(ix, &[&payer])?;
        self.record(
            "init_pool",
            sig,
            format!("denomination {DENOMINATION}, k_floor {k_floor}"),
        );
        Ok(())
    }

    fn leaf_count(&self) -> Result<u64> {
        let mut data = self
            .client
            .account_data(&self.pool)
            .context("reading the pool account")?
            .ok_or_else(|| anyhow!("the pool account does not exist"))?;
        let pool = mirror_pool_program::Pool::load(&mut data).map_err(|e| anyhow!("{e:?}"))?;
        Ok(pool.deposit_count())
    }

    /// Deposits until the pool holds `target` notes, returning the host tree and
    /// every note in it.
    ///
    /// Stated as a target rather than a count so that resuming is the same code
    /// path as starting: a run interrupted after four of six deposits makes two
    /// more, and one interrupted after all six makes none.
    ///
    /// The ledger is written after every single deposit rather than at the end,
    /// because the program keeps only the accumulator's frontier: a note whose
    /// secrets are lost after its commitment reached the tree is escrow nobody
    /// can ever release. An interrupted run must leave a file that still matches
    /// the chain.
    pub fn deposit(
        &mut self,
        target: usize,
        ledger: &std::path::Path,
    ) -> Result<(MerkleTree, Vec<Note>)> {
        let mut tree = MerkleTree::new().map_err(|e| anyhow!("{e}"))?;
        let mut notes = Vec::new();

        let mut stored: Vec<StoredNote> = if ledger.exists() {
            serde_json::from_str(&std::fs::read_to_string(ledger)?)?
        } else {
            Vec::new()
        };
        let on_chain = self.leaf_count()? as usize;
        if stored.len() != on_chain {
            return Err(anyhow!(
                "the note ledger holds {} notes but the pool has {on_chain} leaves. \
                 They must agree, or the host tree cannot match the program's root. \
                 Use a fresh denomination, or restore the ledger.",
                stored.len()
            ));
        }
        for note in &stored {
            let note = note.note()?;
            tree.insert(note.commitment().map_err(|e| anyhow!("{e}"))?)
                .map_err(|e| anyhow!("{e}"))?;
            notes.push(note);
        }

        if stored.len() > target {
            return Err(anyhow!(
                "the pool already holds {} notes and this run wants {target}",
                stored.len()
            ));
        }
        if let Some(parent) = ledger.parent() {
            std::fs::create_dir_all(parent)?;
        }
        for _ in stored.len()..target {
            let note = crate::note::generate(DENOMINATION)?;
            let commitment = note.commitment().map_err(|e| anyhow!("{e}"))?;
            let ix = Instruction::new_with_bytes(
                self.program_id,
                &MirrorIx::Deposit {
                    commitment: commitment.to_bytes(),
                }
                .pack(),
                self.pool_metas(),
            );
            let payer = self.payer.insecure_clone();
            let sig = self.send(ix, &[&payer])?;
            self.record("deposit", sig, format!("note {}", notes.len() + 1));
            tree.insert(commitment).map_err(|e| anyhow!("{e}"))?;
            notes.push(note);
            stored.push(StoredNote::from_note(&note, DENOMINATION)?);
            std::fs::write(ledger, serde_json::to_string(&stored)?)?;
        }
        Ok((tree, notes))
    }

    /// The pending spend this note already has, if an interrupted run submitted
    /// one.
    ///
    /// Rebuilt from the chain rather than from a file, because it can be: a
    /// note's nullifier is a function of the note alone, the record's address is
    /// a function of the nullifier, and the record itself carries the two
    /// accounts settlement has to name. A state file would be a second source of
    /// truth for facts the chain already holds, and the failure it invites — a
    /// file that disagrees with the cluster — is worse than the one it prevents.
    ///
    /// A record that is already settled returns `None`: that note is finished,
    /// and re-submitting it would burn a nullifier that is already burnt.
    pub fn existing_spend(&self, note: &Note) -> Result<Option<Settlement>> {
        let nullifier = note.nullifier().map_err(|e| anyhow!("{e}"))?.to_bytes();
        let (spend_pda, _) = spend_address(&self.program_id, &self.pool, &nullifier);
        let Some(mut data) = self.client.account_data(&spend_pda)? else {
            return Ok(None);
        };
        let record = mirror_pool_program::spend::Spend::load(&mut data)
            .map_err(|e| anyhow!("the account at {spend_pda} is not a spend record: {e:?}"))?;
        if record.status() != mirror_pool_program::spend::STATUS_PENDING {
            return Ok(None);
        }
        let beneficiary = Pubkey::new_from_array(record.beneficiary());
        Ok(Some(Settlement {
            spend: spend_pda,
            beneficiary,
            relay: Pubkey::new_from_array(record.relay()),
            target: Some(Pubkey::new_from_array(record.target_program())),
            // The vote account is not recoverable from the record, because the
            // proof never bound it — `DelegateStake` carries the validator in an
            // account slot rather than in its instruction data. The caller
            // supplies it from the plan, and the read-back after settlement is
            // what turns that into a checked claim.
            action_accounts: Vec::new(),
        }))
    }

    /// Gives a relay enough to pay for the one transaction it signs.
    ///
    /// Every member gets their own relay here, which is the worst case for the
    /// packet and the one a settler has to plan against: a batch whose members
    /// shared a relay names fewer distinct keys and fits more of them.
    pub fn fund_relay(&mut self, relay: &Pubkey) -> Result<()> {
        let ix = solana_system_interface::instruction::transfer(
            &self.payer.pubkey(),
            relay,
            RELAY_FUNDING,
        );
        let payer = self.payer.insecure_clone();
        self.send(ix, &[&payer])?;
        Ok(())
    }

    /// A stake account whose **staker** is the pool's vault and whose
    /// **withdrawer** is not.
    pub fn create_stake_account(&mut self) -> Result<Pubkey> {
        let stake_kp = Keypair::new();
        let [create, initialise] = stake::create(
            &self.payer.pubkey(),
            &stake_kp.pubkey(),
            &self.vault,
            &self.payer.pubkey(),
            STAKE_FUNDING,
        )?;
        let blockhash = self.client.latest_blockhash()?;
        let message =
            solana_message::Message::new(&[create, initialise], Some(&self.payer.pubkey()));
        let payer = self.payer.insecure_clone();
        let tx = Transaction::new(&[&payer, &stake_kp], message, blockhash);
        let sig = self.client.send(&tx)?;
        self.record(
            "create stake account",
            sig,
            format!("{STAKE_FUNDING} lamports, staker = the pool's vault"),
        );
        Ok(stake_kp.pubkey())
    }

    /// Submits one member's spend: delegate `stake` to `vote`, authorised by the
    /// pool, signed by a relay that is not the member.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_delegation(
        &mut self,
        keys: &Keys,
        tree: &MerkleTree,
        notes: &[Note],
        index: usize,
        stake_account: &Pubkey,
        vote: &Pubkey,
        relay: &Keypair,
    ) -> Result<Pubkey> {
        use ark_std::rand::SeedableRng;
        let stake_program = stake::program_id()?;
        let merkle_proof = tree.proof(index as u64).map_err(|e| anyhow!("{e}"))?;
        // The stake account is the beneficiary, so the member's escrow lands in
        // the stake they authorised rather than beside it.
        //
        // Note what the binding does *not* cover: the vote account. The proof
        // fixes the selector, the target program, the beneficiary, the fee, the
        // account *count* and the payload — and `DelegateStake` carries the
        // validator in an account slot, not in its four bytes of instruction
        // data. Which validator a member gets is therefore chosen at settlement.
        // That is the account-slot limit `THREAT_MODEL.md` states, met head-on:
        // the check below is a read-back of every stake account, because the
        // proof cannot make this promise and something has to.
        let binding = mirror_core::action_binding(
            SELECTOR_INVOKE_SIGNED,
            &stake_program.to_bytes(),
            &stake_account.to_bytes(),
            &relay.pubkey().to_bytes(),
            RELAY_FEE,
            stake::DELEGATE_ACCOUNTS,
            &stake::DELEGATE_STAKE,
        );
        let witness = Witness {
            note: notes[index],
            merkle_proof: &merkle_proof,
            root: tree.root().map_err(|e| anyhow!("{e}"))?,
            action_binding: binding,
        };
        let mut rng = ark_std::rand::rngs::StdRng::from_seed([index as u8 + 1; 32]);
        let proof = prove(keys, &witness, &mut rng).map_err(|e| anyhow!("{e}"))?;
        let nullifier = proof.public_inputs[1];
        let (spend_pda, _) = spend_address(&self.program_id, &self.pool, &nullifier);

        let ix = Instruction::new_with_bytes(
            self.program_id,
            &MirrorIx::SubmitSpend {
                proof_a: proof.proof_a,
                proof_b: proof.proof_b,
                proof_c: proof.proof_c,
                root: proof.public_inputs[0],
                nullifier,
                selector: SELECTOR_INVOKE_SIGNED,
                target_program: stake_program.to_bytes(),
                beneficiary: stake_account.to_bytes(),
                relay_fee: RELAY_FEE,
                action_accounts: stake::DELEGATE_ACCOUNTS,
                payload: stake::DELEGATE_STAKE.to_vec(),
            }
            .pack(),
            vec![
                AccountMeta::new(relay.pubkey(), true),
                AccountMeta::new(self.pool, false),
                AccountMeta::new(spend_pda, false),
                AccountMeta::new_readonly(solana_system_interface::program::ID, false),
            ],
        );
        let sig = self.send(ix, &[relay])?;
        self.record(
            "submit_spend",
            sig,
            format!("note {index}, relay-signed, delegate to {vote}"),
        );
        Ok(spend_pda)
    }

    fn settle_ix(&self, batch: &[Settlement]) -> Instruction {
        let mut metas = vec![
            AccountMeta::new(self.payer.pubkey(), true),
            AccountMeta::new(self.pool, false),
            AccountMeta::new(self.vault, false),
        ];
        for entry in batch {
            metas.push(AccountMeta::new(entry.spend, false));
            metas.push(AccountMeta::new(entry.beneficiary, false));
            metas.push(AccountMeta::new(entry.relay, false));
            if let Some(target) = entry.target {
                metas.push(AccountMeta::new_readonly(target, false));
                // The vault is among these, and it reaches the callee as a
                // *signer*. The program sets that flag itself from seeds no
                // settler holds, which is why the meta here says otherwise.
                metas.extend(entry.action_accounts.iter().cloned());
            }
        }
        Instruction::new_with_bytes(
            self.program_id,
            &MirrorIx::SettleEpoch {
                count: batch.len() as u8,
                // False, and that is an assertion rather than a default: these
                // batches are supposed to meet the floor. If one ever does not,
                // the program refuses it and the run stops — which is the
                // failure we would want, instead of evidence quietly recording
                // a crowd that was not there.
                allow_below_floor: false,
            }
            .pack(),
            metas,
        )
    }

    /// What this settlement would weigh on the wire.
    ///
    /// A legacy transaction is a compact array of 64-byte signatures followed by
    /// the serialized message, and `Message::serialize` is what a validator
    /// receives rather than an approximation of it. Settlement is signed by the
    /// settler alone, so the signature array is one entry and its length prefix
    /// one byte.
    pub fn settlement_bytes(&self, batch: &[Settlement]) -> usize {
        let ix = self.settle_ix(batch);
        let message = solana_message::Message::new(&[ix], Some(&self.payer.pubkey()));
        1 + 64 + message.serialize().len()
    }

    /// What the same settlement would weigh with a `SetComputeUnitLimit`
    /// instruction in front of it.
    ///
    /// The difference is the price of asking for more compute. It is worth a
    /// number rather than an argument, because for a batch of cross-program
    /// actions the compute budget and the packet stop being independent limits:
    /// the escape from one is paid for out of the other.
    pub fn settlement_bytes_with_budget(&self, batch: &[Settlement]) -> usize {
        // `ComputeBudgetInstruction::SetComputeUnitLimit`: a u8 discriminant of
        // 2 followed by a little-endian u32. The program takes no accounts, so
        // what this costs is its own key plus the instruction's framing.
        let budget: Pubkey = "ComputeBudget111111111111111111111111111111"
            .parse()
            .expect("the compute budget program id is a const");
        let mut data = vec![2u8];
        data.extend_from_slice(&u32::MAX.to_le_bytes());
        let raise = Instruction::new_with_bytes(budget, &data, vec![]);
        let message = solana_message::Message::new(
            &[raise, self.settle_ix(batch)],
            Some(&self.payer.pubkey()),
        );
        1 + 64 + message.serialize().len()
    }

    pub fn settle(&mut self, batch: &[Settlement]) -> Result<(String, usize)> {
        let bytes = self.settlement_bytes(batch);
        if bytes > PACKET_DATA_SIZE {
            return Err(anyhow!(
                "this settlement is {bytes} bytes, {} over the {PACKET_DATA_SIZE}-byte \
                 packet limit, and no validator would accept it",
                bytes - PACKET_DATA_SIZE
            ));
        }
        let ix = self.settle_ix(batch);
        let payer = self.payer.insecure_clone();
        let sig = self.send(ix, &[&payer])?;
        self.record(
            "settle_epoch",
            sig.clone(),
            format!(
                "{} delegations to {} different validators, one transaction, {bytes} bytes",
                batch.len(),
                batch.len()
            ),
        );
        Ok((sig, bytes))
    }

    pub fn compute_units(&self, signature: &str) -> Result<u64> {
        self.client.compute_units(signature)
    }

    /// Reads a stake account back off the cluster and returns the validator it
    /// actually backs.
    pub fn delegated_voter(&self, stake_account: &Pubkey) -> Result<Pubkey> {
        let data = self
            .client
            .account_data(stake_account)?
            .ok_or_else(|| anyhow!("the stake account {stake_account} vanished"))?;
        stake::delegated_voter(&data)
    }

    /// How many members fit in one settlement, found by serializing rather than
    /// by arithmetic.
    ///
    /// Placeholder keys are legitimate here because transaction size depends on
    /// how many *distinct* accounts a message names, not on which — and the real
    /// batch is measured again before it is sent, so an error in that reasoning
    /// surfaces as a refusal rather than as a wrong number in a document.
    ///
    /// Both shapes are measured because the difference between them is the
    /// finding. `divergent` gives every member their own vote account;
    /// `uniform` shares one across the batch, which is the same run of members
    /// all choosing the same validator.
    pub fn measure_ceiling(&self) -> Ceiling {
        let shape = |n: usize, share_vote: bool| -> Vec<Settlement> {
            let shared_vote = Pubkey::new_unique();
            (0..n)
                .map(|_| {
                    let stake_account = Pubkey::new_unique();
                    let vote = if share_vote {
                        shared_vote
                    } else {
                        Pubkey::new_unique()
                    };
                    Settlement {
                        spend: Pubkey::new_unique(),
                        beneficiary: stake_account,
                        relay: Pubkey::new_unique(),
                        target: Some(stake::program_id().expect("the stake program id is a const")),
                        action_accounts: stake::delegate_accounts(
                            &stake_account,
                            &vote,
                            &self.vault,
                        )
                        .expect("the delegate account list is built from consts"),
                    }
                })
                .collect()
        };
        let largest = |share_vote: bool| -> usize {
            // Searched from the top down rather than counted upward, so a
            // hypothetical non-monotonic encoding would be caught by the
            // round-trip check against the batch that actually lands rather
            // than hidden by an early exit.
            (1..=32)
                .rfind(|n| self.settlement_bytes(&shape(*n, share_vote)) <= PACKET_DATA_SIZE)
                .unwrap_or(0)
        };
        let largest_through_table = |share_vote: bool| -> usize {
            (1..=64)
                .rfind(|n| self.settlement_locks_with_budget(&shape(*n, share_vote)) <= MAX_LOCKS)
                .unwrap_or(0)
        };
        let divergent = largest(false);
        let uniform = largest(true);
        let full = shape(divergent, false);
        let divergent_bytes = self.settlement_bytes(&full);
        let divergent_through_table = largest_through_table(false);
        Ceiling {
            divergent,
            uniform,
            divergent_bytes,
            over_bytes: self.settlement_bytes(&shape(divergent + 1, false)),
            budget_bytes: self.settlement_bytes_with_budget(&full) - divergent_bytes,
            divergent_through_table,
            uniform_through_table: largest_through_table(true),
            locks_at_divergent_table: self
                .settlement_locks_with_budget(&shape(divergent_through_table, false)),
        }
    }

    /// How many accounts a settlement would lock, with the compute-budget
    /// instruction counted.
    ///
    /// This is the limit that takes over once a lookup table ends the byte
    /// argument, and it is a different kind of limit: bytes are spent per
    /// account *name*, locks are held per *distinct* account. That is the whole
    /// reason a crowd agreeing on one validator fits more members — the shared
    /// vote account is named once and locked once, where divergent members each
    /// bring one nobody else in the batch holds.
    ///
    /// The compute-budget program is included because it is an account like any
    /// other. A batch this size cannot execute inside the default 200,000 CU, so
    /// the instruction is not optional here, and neither is the lock it costs.
    pub fn settlement_locks_with_budget(&self, batch: &[Settlement]) -> usize {
        let ix = self.settle_ix(batch);
        let mut metas = ix.accounts.clone();
        metas.push(AccountMeta::new_readonly(compute_budget_program(), false));
        crate::lookup::locks_for(&metas, &ix.program_id, &self.payer.pubkey())
    }
}

/// The compute-budget program, which a settlement large enough to need a lookup
/// table also needs.
fn compute_budget_program() -> Pubkey {
    "ComputeBudget111111111111111111111111111111"
        .parse()
        .expect("the compute budget program id is a const")
}

pub fn run(program: &str, url: &str, keypair: &str, out: &std::path::Path) -> Result<()> {
    let program_id: Pubkey = program
        .parse()
        .map_err(|e| anyhow!("bad program id: {e}"))?;
    let payer = read_keypair(keypair)?;
    println!("program {program_id}");
    println!("payer   {}", payer.pubkey());
    println!("cluster {url}\n");

    let mut crowd = Crowd::new(url, program_id, payer.insecure_clone());

    // The ceiling first, before anything is spent. How many stake accounts to
    // create is the answer to this question, and creating the wrong number means
    // either a batch that no validator will accept or SOL left in accounts the
    // run has no use for.
    let ceiling = crowd.measure_ceiling();
    println!("how many members fit in one settlement, by serializing the real instruction:");
    println!(
        "  all delegating to the same validator   {} members",
        ceiling.uniform
    );
    println!(
        "  each delegating to a different one     {} members  ({} bytes, {} to spare)",
        ceiling.divergent,
        ceiling.divergent_bytes,
        PACKET_DATA_SIZE - ceiling.divergent_bytes
    );
    println!(
        "  one more than that                     {} bytes, {} over the {PACKET_DATA_SIZE}-byte limit\n",
        ceiling.over_bytes,
        ceiling.over_bytes - PACKET_DATA_SIZE
    );
    let members = ceiling.divergent;
    if members < 2 {
        return Err(anyhow!(
            "only {members} member(s) fit, so there is no crowd to measure"
        ));
    }

    // Validators the cluster itself counts as active, so a reader checking this
    // run finds the delegations pointing at validators they could have chosen.
    //
    // Fixed to a file on the first run and reused after, because the ranking
    // moves between epochs and a resumed run that re-drew the list would settle
    // members against validators other than the ones they asked for — silently,
    // since the proof binds the account *count* and never the accounts.
    let plan_path = format!("data/crowd-plan-{DENOMINATION}.json");
    let votes = crowd_votes(url, members, std::path::Path::new(&plan_path))?;
    println!("validators drawn from the cluster's active set:");
    for (i, v) in votes.iter().enumerate() {
        println!("  member {i}  {v}");
    }

    let cost = members as u64 * (STAKE_FUNDING + DENOMINATION + RELAY_FUNDING);
    println!(
        "\nthis run will spend about {:.2} SOL: {members} stake accounts, {members} deposits, \
         {members} relays\n",
        cost as f64 / 1e9
    );

    println!("deriving the proving key from the published seed (this takes a moment)");
    let keys = keys()?;

    println!("\nsetting up:");
    crowd.init_pool(members as u32, members)?;
    let ledger_path = format!("data/crowd-notes-{DENOMINATION}.json");
    let ledger = std::path::Path::new(&ledger_path);
    let (tree, notes) = crowd.deposit(members, ledger)?;

    let mut batch = Vec::new();
    let mut roster = Vec::new();
    for (i, vote) in votes.iter().enumerate() {
        // A spend this note already carries is reused rather than remade. The
        // nullifier is spend-once-ever, so a second submission for the same note
        // is refused by the program — which would strand the run at exactly the
        // point a flaky endpoint left it.
        let entry = match crowd.existing_spend(&notes[i])? {
            Some(found) => {
                println!("  member {i:<21} spend {} already submitted", found.spend);
                Settlement {
                    action_accounts: stake::delegate_accounts(
                        &found.beneficiary,
                        vote,
                        &crowd.vault(),
                    )?,
                    ..found
                }
            }
            None => {
                let stake_account = crowd.create_stake_account()?;
                let relay = Keypair::new();
                crowd.fund_relay(&relay.pubkey())?;
                let spend = crowd.submit_delegation(
                    &keys,
                    &tree,
                    &notes,
                    i,
                    &stake_account,
                    vote,
                    &relay,
                )?;
                Settlement {
                    spend,
                    beneficiary: stake_account,
                    relay: relay.pubkey(),
                    target: Some(stake::program_id()?),
                    action_accounts: stake::delegate_accounts(
                        &stake_account,
                        vote,
                        &crowd.vault(),
                    )?,
                }
            }
        };
        roster.push(Member {
            stake: entry.beneficiary,
            vote: *vote,
            relay: entry.relay,
        });
        batch.push(entry);
    }

    println!("\nsettling all {members} in one transaction:");
    let (signature, bytes) = crowd.settle(&batch)?;
    let cu = crowd.compute_units(&signature)?;
    println!("  {bytes} bytes of {PACKET_DATA_SIZE}, {cu} CU of {DEFAULT_COMPUTE_BUDGET}");
    println!(
        "  raising the compute budget would cost {} bytes and there are {} to spare",
        ceiling.budget_bytes,
        PACKET_DATA_SIZE - bytes
    );
    if bytes != ceiling.divergent_bytes {
        return Err(anyhow!(
            "the settlement that landed was {bytes} bytes and the measurement predicted \
             {}, so the ceiling above describes a different transaction than the one sent",
            ceiling.divergent_bytes
        ));
    }

    // The read-back. The proof binds how many accounts the call takes and never
    // which, so nothing before this point can promise a member got the validator
    // they asked for — the account state is the only place that answer exists.
    println!("\nreading every stake account back off the cluster:");
    for (i, member) in roster.iter().enumerate() {
        let voter = crowd.delegated_voter(&member.stake)?;
        if voter != member.vote {
            return Err(anyhow!(
                "member {i} asked to delegate to {} and the stake account backs {voter}",
                member.vote
            ));
        }
        println!("  member {i}  {} → {voter}", member.stake);
    }

    let distinct: std::collections::HashSet<_> = roster.iter().map(|m| m.vote).collect();
    if distinct.len() != members {
        return Err(anyhow!(
            "the batch names {} distinct validators for {members} members, so it does not \
             demonstrate divergence",
            distinct.len()
        ));
    }
    println!(
        "\n{members} members, {} distinct validators, one transaction, one timestamp.",
        distinct.len()
    );

    let outcome = Outcome {
        pool: crowd.pool().to_string(),
        vault: crowd.vault().to_string(),
        denomination: DENOMINATION,
        ceiling,
        roster: roster
            .iter()
            .map(|m| MemberRecord {
                stake: m.stake.to_string(),
                vote: m.vote.to_string(),
                relay: m.relay.to_string(),
            })
            .collect(),
        signature,
        bytes,
        compute_units: cu,
        steps: crowd
            .steps
            .iter()
            .map(|s| StepRecord {
                name: s.name.to_string(),
                signature: s.signature.clone(),
                note: s.note.clone(),
                slot: s.slot,
            })
            .collect(),
    };
    let result_path = format!("data/crowd-result-{DENOMINATION}.json");
    std::fs::write(&result_path, serde_json::to_string_pretty(&outcome)?)?;
    write_report(&outcome, out)?;
    println!("wrote {} and {result_path}", out.display());
    Ok(())
}

fn write_report(outcome: &Outcome, out: &std::path::Path) -> Result<()> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, report(outcome))?;
    Ok(())
}

/// Re-renders the report from the last run's recorded results.
///
/// Touches no cluster and spends nothing. The numbers are exactly the ones the
/// run measured; only the sentences around them can change.
pub fn render_only(out: &std::path::Path) -> Result<()> {
    let result_path = format!("data/crowd-result-{DENOMINATION}.json");
    let text = std::fs::read_to_string(&result_path).with_context(|| {
        format!("{result_path} does not exist — there is no recorded run to render")
    })?;
    let mut outcome: Outcome = serde_json::from_str(&text)?;

    // The through-a-table ceilings are a property of the instruction's shape and
    // not of the run, so they are recomputed here rather than trusted from the
    // file. A record written before these fields existed deserializes them as
    // zero, and rendering a zero into a published table is worse than any
    // staleness this is meant to avoid: it would read as a measured result.
    //
    // Nothing about this touches a cluster. It builds the same instruction the
    // settlement builds and counts what it names, which is what the packet
    // ceilings in the same struct already are.
    let derived = Crowd::new(
        "http://127.0.0.1:1",
        outcome
            .pool
            .parse()
            .unwrap_or_else(|_| solana_program::pubkey::Pubkey::new_unique()),
        Keypair::new(),
    )
    .measure_ceiling();
    outcome.ceiling.divergent_through_table = derived.divergent_through_table;
    outcome.ceiling.uniform_through_table = derived.uniform_through_table;
    outcome.ceiling.locks_at_divergent_table = derived.locks_at_divergent_table;

    write_report(&outcome, out)?;
    println!("rendered {} from {result_path}", out.display());
    Ok(())
}

/// Picks `n` distinct active validators, largest stake first, and remembers the
/// choice.
///
/// The file is the record of what each member *asked for*. It has to exist
/// somewhere outside the chain, because the chain does not hold it: a member's
/// proof binds the number of accounts their call takes and not which accounts
/// fill the slots, so before settlement there is nothing on-chain that says
/// which validator member 3 wanted. The read-back afterwards compares the
/// cluster against this file, which is what makes it a check rather than a
/// restatement.
fn crowd_votes(url: &str, n: usize, plan: &std::path::Path) -> Result<Vec<Pubkey>> {
    if plan.exists() {
        let saved: Vec<String> = serde_json::from_str(&std::fs::read_to_string(plan)?)?;
        if saved.len() != n {
            return Err(anyhow!(
                "{} names {} validators and this run needs {n}. It belongs to a run of a \
                 different size; delete it only if no spend has been submitted against it.",
                plan.display(),
                saved.len()
            ));
        }
        println!("(reusing the validator choices in {})", plan.display());
        return saved
            .iter()
            .map(|s| s.parse().map_err(|e| anyhow!("{s}: {e}")))
            .collect();
    }
    let all = Chain::new(url).active_vote_accounts()?;
    if all.len() < n {
        return Err(anyhow!(
            "the cluster reports {} active validators and this run needs {n}",
            all.len()
        ));
    }
    let chosen: Vec<Pubkey> = all.into_iter().take(n).collect();
    if let Some(parent) = plan.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let names: Vec<String> = chosen.iter().map(|v| v.to_string()).collect();
    std::fs::write(plan, serde_json::to_string_pretty(&names)?)?;
    Ok(chosen)
}

fn explorer(kind: &str, id: &str) -> String {
    format!("[`{id}`](https://explorer.solana.com/{kind}/{id}?cluster=devnet)")
}

fn report(outcome: &Outcome) -> String {
    let Outcome {
        ceiling,
        roster,
        bytes,
        steps,
        ..
    } = outcome;
    let (signature, cu) = (&outcome.signature, outcome.compute_units);
    let denomination = outcome.denomination;
    let bytes = *bytes;
    let members = roster.len();
    let mut md = String::new();
    md.push_str("# Different members, different validators, one timestamp\n\n");
    md.push_str(
        "A batch whose members all do the same thing is the easy case for an anonymity set: \
         there is nothing to tell them apart to begin with. This run does the hard one. Each \
         member delegates stake to a **different validator**, and all of it settles in a \
         single transaction.\n\n\
         Validator choice is the behavioural pattern worth hiding. It is stable, it is \
         public, and it fingerprints a staker across epochs far more reliably than an \
         amount does. What an observer gets from the transaction below is a set of \
         delegations landing at one timestamp with no way to say which member asked for \
         which.\n\n",
    );
    md.push_str(&format!(
        "Generated by `mirror crowd`. Pool {}, vault {}, denomination {denomination} lamports.\n\n",
        explorer("address", &outcome.pool),
        explorer("address", &outcome.vault)
    ));

    md.push_str("## The settlement\n\n");
    md.push_str(&format!("{}\n\n", explorer("tx", signature)));
    md.push_str(&format!(
        "| | |\n|---|---|\n| members | {members} |\n| distinct validators | {members} |\n\
         | wire size | {bytes} of {PACKET_DATA_SIZE} bytes |\n\
         | compute | {cu} of {DEFAULT_COMPUTE_BUDGET} CU |\n\n"
    ));

    md.push_str("## Who got what\n\n");
    md.push_str(
        "Read back from the cluster after settlement, not assumed from what was requested. \
         A member's proof binds how many accounts their call takes and never *which*, so \
         the stake account's own state is the only place the answer exists — this table is \
         the check that limit needs, and the run fails if any row disagrees.\n\n\
         Every member has their own relay, and no relay key appears twice. That is the \
         worst case for the packet and the case a settler has to plan against: a batch \
         whose members shared a relay would name fewer distinct keys and fit more of \
         them.\n\n",
    );
    md.push_str("| member | stake account | delegated to | relay |\n|---|---|---|---|\n");
    for (i, m) in roster.iter().enumerate() {
        md.push_str(&format!(
            "| {i} | {} | {} | {} |\n",
            explorer("address", &m.stake),
            explorer("address", &m.vote),
            explorer("address", &m.relay)
        ));
    }
    md.push('\n');

    md.push_str("## What divergence costs\n\n");
    md.push_str(&format!(
        "Divergence is not free, and the price is anonymity-set size. A transaction names \
         each distinct account once, so a member who picks their own validator adds a vote \
         account nobody else in the batch names. The 1232-byte packet is reached sooner.\n\n\
         | batch | members per settlement |\n|---|---|\n\
         | all delegating to the same validator | {} |\n\
         | each delegating to a different one | {} |\n\n\
         At {} members the settlement weighs {} bytes with {} to spare; one more member \
         weighs {} bytes, {} over the limit. Both figures come from serializing the real \
         instruction, and the settlement that landed above is {bytes} bytes — the same \
         number, which is what makes the measurement a prediction rather than a \
         description.\n\n",
        ceiling.uniform,
        ceiling.divergent,
        ceiling.divergent,
        ceiling.divergent_bytes,
        PACKET_DATA_SIZE - ceiling.divergent_bytes,
        ceiling.over_bytes,
        ceiling.over_bytes - PACKET_DATA_SIZE,
    ));
    md.push_str("## And compute, which is closer than it is for payments\n\n");
    md.push_str(&format!(
        "This settlement burned **{cu} CU of the {DEFAULT_COMPUTE_BUDGET}** a single \
         instruction gets by default — {:.0}% of the budget, for {members} payouts and \
         {members} cross-program invocations. That is a different regime from a batch of \
         plain transfers: `ten_spends_fit_in_one_settlement_and_the_packet_is_what_stops_\
         the_eleventh` settles ten of those in 19,545 CU, where compute is nowhere in the \
         conversation. A delegation costs roughly an order of magnitude more per member \
         than a payment does.\n\n\
         Each member's Groth16 proof was verified earlier, in their own `submit_spend`, \
         which costs about 101,000 CU. That is why the two phases exist: verifying \
         {members} proofs here would cost over 600,000 CU — comfortably past the 200,000 a \
         single instruction gets by default, and a large fraction of the 1.4M a whole \
         transaction may ever request.\n\n",
        cu as f64 * 100.0 / DEFAULT_COMPUTE_BUDGET as f64
    ));
    md.push_str(&format!(
        "The packet binds first — {} members is where the bytes run out, and the budget is \
         not exhausted there — but for a **legacy** transaction the two limits are barely \
         independent, and that is worth stating plainly. The usual answer to a settlement \
         that runs out of compute is to ask for more with a `SetComputeUnitLimit` \
         instruction. Measured against this very batch, that instruction costs **{} \
         bytes**, and a full legacy settlement has {} to spare. In a legacy transaction, \
         raising the budget means dropping a member.\n\n\
         **A lookup table lifts that, and here is how far.** Naming accounts by one \
         byte each takes the packet out of the way — `mirror settle` does it \
         automatically, and a batch of twenty plain transfers settled that way on devnet \
         at 333 bytes of 1232. What takes over for *delegations* is the 64-account lock \
         limit, and it is a different kind of limit: bytes are spent naming an account, \
         locks are held per **distinct** account.\n\n\
         | batch | legacy packet | through a lookup table |\n|---|---|---|\n\
         | all delegating to the same validator | {} | **{}** |\n\
         | each delegating to a different one | {} | **{}** |\n\n\
         A full divergent batch through a table holds {} of the 64 locks a transaction \
         may take. Both ceilings roughly double, and the gap between them widens from one \
         member to {} — because a shared vote account is named once either way but locked \
         only once too, so agreeing on a validator is worth more here than it was in the \
         packet.\n\n\
         **The compute budget instruction is counted in those two figures, because at \
         this size it is not optional.** A batch of {} delegations costs on the order of \
         {} CU at the per-member rate this run measured, well past the 200,000 a \
         transaction is given by default. Asking for more brings the compute-budget \
         program along, and a program is an account — so raising the budget still costs a \
         member. In the legacy packet that cost was {} bytes; through a table it is one \
         lock. The escape from one limit is paid out of the other in both regimes, which \
         is the finding rather than the inconvenience.\n\n\
         **What kind of number these two are.** They are computed the same way the packet \
         ceilings above are — by building the real instruction and counting what it \
         names — and not by settling a batch of that size. The 64-account limit itself is \
         not a guess: it was found on devnet, where 77 accounts returned \
         `TooManyAccountLocks`, and the twenty-transfer settlement cited above landed at \
         exactly 64. What has not been done is a delegation batch of {} settled through a \
         table on a live cluster, and this document does not claim one.\n\n",
        ceiling.divergent,
        ceiling.budget_bytes,
        PACKET_DATA_SIZE - ceiling.divergent_bytes,
        ceiling.uniform,
        ceiling.uniform_through_table,
        ceiling.divergent,
        ceiling.divergent_through_table,
        ceiling.locks_at_divergent_table,
        ceiling.uniform_through_table - ceiling.divergent_through_table,
        ceiling.divergent_through_table,
        ceiling.divergent_through_table * 23_800,
        ceiling.budget_bytes,
        ceiling.divergent_through_table,
    ));

    md.push_str("## Every step\n\n");
    md.push_str("| step | signature | slot | note |\n|---|---|---|---|\n");
    for step in steps {
        md.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            step.name,
            explorer("tx", &step.signature),
            step.slot.map_or_else(|| "—".to_string(), |n| n.to_string()),
            step.note
        ));
    }
    md.push('\n');
    md.push_str(
        "Devnet history is pruned, so a signature above may one day return null from \
         `getTransaction` without having failed. The slot is what tells those two apart: \
         `getSignatureStatuses` with `--search-transaction-history` still answers for a \
         pruned transaction. A dash means this run could not read the slot back, and that \
         row is the one to check by hand.\n\n",
    );

    md.push_str("## Reproducing this document\n\n");
    md.push_str(&format!(
        "Every value above was written by the run that produced it, and the run recorded \
         them in `data/crowd-result-{denomination}.json`. `mirror crowd --render-only` \
         rebuilds this file from that record without touching a cluster, so the prose \
         around a number can be improved without re-running a measurement — and a number \
         cannot be changed without re-running one.\n\n\
         Every address and signature here is on devnet and can be checked against the \
         cluster rather than against this file.\n\n"
    ));

    md.push_str("## What this run's own anonymity was, by our own metric\n\n");
    md.push_str(
        "It would be easy to publish this section's numbers and let a reader take them \
         for a privacy result. They are not one, and the honest way to show that is to \
         run the measurement this repository is built around against *this run* rather \
         than only against somebody else's pool.\n\n",
    );
    // Every note here was deposited by one wallet, so the funding-provenance
    // partition has exactly one class. Computed by the same code that produces
    // the published headline rather than asserted, because the point of the
    // section is that the number comes out *good* and means nothing.
    let own = mirror_provenance::Anonymity::from_class_sizes(&[members as u64]);
    let own_bracket = mirror_provenance::Bracket::new(&[members as u64], 0);
    if let (Some(a), Some(b)) = (own, own_bracket) {
        // The bracket travels with the figure here too, even though this run
        // resolved everybody and it therefore collapses onto the point. Showing
        // the collapse is worth a column: it is the difference between "no
        // unresolved members" and "unresolved members nobody accounted for",
        // and a table that omits the bracket whenever it is narrow teaches a
        // reader that the bracket is optional.
        md.push_str(&format!(
            "| quantity | this run | unresolved bracket |\n|---|---|---|\n\
             | nominal k | {} | — |\n\
             | provenance classes | {} | — |\n\
             | ρ, the loss factor | {:.4} | {:.4} … {:.4} |\n\
             | effective k (Shannon) | {:.2} | {:.2} … {:.2} |\n\
             | effective k (min-entropy) | {:.2} | {:.2} … {:.2} |\n\n",
            a.nominal_k,
            a.classes,
            a.loss_factor,
            b.lower.loss_factor,
            b.upper.loss_factor,
            a.eff_k_shannon,
            b.lower.eff_k_shannon,
            b.upper.eff_k_shannon,
            a.eff_k_min_entropy,
            b.lower.eff_k_min_entropy,
            b.upper.eff_k_min_entropy,
        ));
        md.push_str(&format!(
            "The bracket collapses onto the point because all {} members resolved and none \
             were left over — not because the figure needs no bracket. Every ρ this \
             repository publishes carries one.\n\n",
            b.resolved
        ));
        md.push_str(&format!(
            "**ρ = {:.4} is the best value the metric can return, and it is meaningless \
             here.** Every note in this pool was deposited by the same wallet, so the \
             partition has one class holding all {} members; an adversary who learns a \
             member's funding class learns nothing, and the metric correctly reports no \
             loss *through that channel*. What it cannot report is that the single class \
             is the operator, who funded every deposit and every relay and therefore knows \
             which member is which. Against that adversary the anonymity set is **one**, \
             and no funding-provenance number will ever say so, because provenance is not \
             the channel that failed.\n\n\
             This is the shape of the tautology `PROVENANCE_METHOD.md` §9.0 warns about, \
             met head-on: a metric applied to a population constructed by the person \
             reading it returns whatever that construction implies. The published headline \
             in `README.md` avoids it by pointing at a pool this project does not control \
             and did not fund — which is the only reason that number means anything and \
             this one does not.\n\n",
            a.loss_factor, a.nominal_k
        ));
    }

    md.push_str("## Scope\n\n");
    md.push_str(
        "Devnet, and one operator. This is a functional and quantitative result, not an \
         anonymity claim about a live crowd: the relays here were funded from the same \
         wallet that made the deposits, which is exactly the linkage `USAGE.md` tells a \
         real member to avoid. What the run establishes is that a batch of *divergent* \
         actions settles as one, that every member reached the validator they chose, and \
         what such a batch costs in packet space.\n\n\
         The withdraw authority on every stake account above is the operator, never the \
         pool. The pool's signature is available to every member, so an authority the vault \
         holds is an authority every member holds: delegation is safe on those terms — the \
         worst a member can do is re-delegate somebody else's stake — and withdrawal is \
         not. That is `THREAT_MODEL.md` applied rather than repeated.\n",
    );
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `Crowd` with no cluster behind it.
    ///
    /// `measure_ceiling` never touches the network — it serializes instructions
    /// and counts accounts — so the ceilings are testable without devnet, which
    /// is what makes them a pinned result rather than something re-derived on
    /// each run and believed.
    fn offline() -> Crowd {
        Crowd::new("http://127.0.0.1:1", Pubkey::new_unique(), Keypair::new())
    }

    /// The four ceilings, pinned.
    ///
    /// Legacy and through-a-table are different regimes and the numbers say so.
    /// A change to the account shape of `settle_epoch` moves these, and moving
    /// them silently is how a document comes to describe a settlement nobody can
    /// send.
    #[test]
    fn the_table_lifts_the_delegation_ceiling_and_locks_take_over() {
        let c = offline().measure_ceiling();

        // Legacy: the 1232-byte packet binds, and divergence costs a member.
        assert_eq!(c.uniform, 7, "legacy, one validator");
        assert_eq!(c.divergent, 6, "legacy, a validator each");

        // Through a table: the packet stops mattering and the 64-account lock
        // limit takes over. Both shapes roughly double.
        assert_eq!(c.uniform_through_table, 18, "table, one validator");
        assert_eq!(c.divergent_through_table, 13, "table, a validator each");

        assert!(
            c.locks_at_divergent_table <= MAX_LOCKS,
            "a full divergent batch holds {} locks, over the {MAX_LOCKS} limit",
            c.locks_at_divergent_table
        );
    }

    /// Divergence costs more under locks than under bytes, and the reason is
    /// structural rather than incidental.
    ///
    /// Bytes are spent naming an account; locks are held per *distinct*
    /// account. A shared vote account is named once either way, so agreeing on a
    /// validator buys one member in a legacy packet and five through a table.
    #[test]
    fn agreeing_on_a_validator_buys_more_members_through_a_table_than_without() {
        let c = offline().measure_ceiling();
        let legacy_gain = c.uniform - c.divergent;
        let table_gain = c.uniform_through_table - c.divergent_through_table;
        assert!(
            table_gain > legacy_gain,
            "divergence cost {legacy_gain} member(s) legacy and {table_gain} through a table; \
             the table was supposed to make the shared account matter more, not less"
        );
    }
}
