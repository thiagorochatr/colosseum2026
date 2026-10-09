//! The member-facing half of the tool: join a pool, act through it, settle it.
//!
//! Every command here reads what it needs from the chain. Nothing depends on a
//! file this tool wrote earlier except the note itself, and nothing depends on a
//! server we run. A member who keeps their note file can act from a fresh
//! machine; a member who loses it has lost the deposit, and no operator can
//! change that in either direction.

use crate::chain::Chain;
use crate::history::{self, History};
use crate::lookup;
use crate::note::StoredNote;
use anyhow::{anyhow, Context, Result};
use ark_std::rand::SeedableRng;
use mirror_pool_program::{
    instruction::Instruction as MirrorIx,
    pda::{pool_address, spend_address, vault_address},
    processor::{SELECTOR_INVOKE, SELECTOR_INVOKE_SIGNED, SELECTOR_TRANSFER},
    spend::{Spend, STATUS_PENDING},
    Pool,
};
use solana_keypair::Keypair;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::path::Path;

/// What the pool account says about itself.
pub struct PoolState {
    pub pool: Pubkey,
    pub vault: Pubkey,
    pub denomination: u64,
    pub k_floor: u32,
    pub deposits: u64,
    pub spends: u64,
    pub root: [u8; 32],
    /// The pool's own settlement timeout, already resolved against the
    /// program's default for a pool that never set one.
    pub settle_timeout: i64,
}

pub fn read_pool(chain: &Chain, program_id: &Pubkey, denomination: u64) -> Result<PoolState> {
    let (pool, _) = pool_address(program_id, denomination);
    let (vault, _) = vault_address(program_id, &pool);
    let mut data = chain.account_data(&pool)?.ok_or_else(|| {
        anyhow!(
            "no pool exists at {pool} for denomination {denomination}. \
             Pool creation is permissionless — `mirror init-pool` makes one."
        )
    })?;
    let state =
        Pool::load(&mut data).map_err(|e| anyhow!("{pool} is not a pool account: {e:?}"))?;
    Ok(PoolState {
        pool,
        vault,
        denomination: state.denomination(),
        k_floor: state.k_floor(),
        deposits: state.deposit_count(),
        spends: state.spend_count(),
        root: state
            .current_root()
            .map_err(|e| anyhow!("{e:?}"))?
            .to_bytes(),
        settle_timeout: state.settle_timeout_seconds().map_or(
            mirror_pool_program::processor::SETTLE_TIMEOUT_SECONDS,
            i64::from,
        ),
    })
}

fn send(chain: &Chain, ix: Instruction, signers: &[&Keypair]) -> Result<String> {
    let blockhash = chain.latest_blockhash()?;
    let message = solana_message::Message::new(&[ix], Some(&signers[0].pubkey()));
    let tx = Transaction::new(signers, message, blockhash);
    chain.send(&tx)
}

// ---------------------------------------------------------------------------
// init-pool

pub fn init_pool(
    chain: &Chain,
    program_id: &Pubkey,
    denomination: u64,
    k_floor: u32,
    settle_timeout_seconds: u32,
    payer: &Keypair,
) -> Result<()> {
    let (pool, _) = pool_address(program_id, denomination);
    let (vault, _) = vault_address(program_id, &pool);
    if chain.account_data(&pool)?.is_some() {
        println!("the pool for denomination {denomination} already exists at {pool}");
        println!("nothing to do — one pool per denomination is the whole point");
        return Ok(());
    }
    // Zero means the program's default, and anything else has to be a timeout
    // somebody could live with. Refused here as well as on chain so the caller
    // learns it before paying for a transaction, and refused rather than
    // clamped because the value is immutable once the pool exists.
    if settle_timeout_seconds != 0
        && !(mirror_pool_program::state::MIN_SETTLE_TIMEOUT_SECONDS
            ..=mirror_pool_program::state::MAX_SETTLE_TIMEOUT_SECONDS)
            .contains(&settle_timeout_seconds)
    {
        return Err(anyhow!(
            "a settlement timeout of {settle_timeout_seconds}s is outside the permitted \
             range of {}s to {}s. A pool's timeout is fixed at creation and cannot be \
             changed, so this is refused rather than adjusted. Pass 0 for the program's \
             default of {}s.",
            mirror_pool_program::state::MIN_SETTLE_TIMEOUT_SECONDS,
            mirror_pool_program::state::MAX_SETTLE_TIMEOUT_SECONDS,
            mirror_pool_program::processor::SETTLE_TIMEOUT_SECONDS,
        ));
    }
    let effective_timeout = match settle_timeout_seconds {
        0 => mirror_pool_program::processor::SETTLE_TIMEOUT_SECONDS,
        v => i64::from(v),
    };

    // A floor above what one settlement can carry is a floor no crowd can ever
    // satisfy. Settlement locks three accounts per member plus three for the
    // pool itself, so the largest batch a transaction can hold is fixed by the
    // runtime and not by this program — and a pool asking for more than that can
    // only ever settle through the liveness timeout, one wait at a time, which
    // is the opposite of what a high floor is chosen for.
    //
    // Refused rather than warned about: a pool's floor is fixed at creation, so
    // by the time anyone notices, the fix is a different pool.
    let settleable = (lookup::MAX_ACCOUNT_LOCKS - 3) / 3;
    if k_floor as usize > settleable {
        return Err(anyhow!(
            "a floor of {k_floor} is higher than one settlement can carry. A transaction \
             may lock {} accounts and each member costs three, so at most {settleable} \
             members settle together — a pool with this floor would never meet it by \
             crowd, and could only ever settle on the {}s timeout. Choose {settleable} \
             or fewer.",
            lookup::MAX_ACCOUNT_LOCKS,
            effective_timeout,
        ));
    }
    let ix = Instruction::new_with_bytes(
        *program_id,
        &MirrorIx::InitPool {
            denomination,
            entry_fee: 0,
            k_floor,
            settle_timeout_seconds,
        }
        .pack(),
        vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(pool, false),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(solana_system_interface::program::ID, false),
        ],
    );
    let sig = send(chain, ix, &[payer])?;
    println!("pool    {pool}");
    println!("vault   {vault}");
    println!(
        "timeout {effective_timeout}s{}",
        if settle_timeout_seconds == 0 {
            "   (the program's default: this pool set none)"
        } else {
            ""
        }
    );
    println!("signature {sig}");
    Ok(())
}

// ---------------------------------------------------------------------------
// deposit

pub fn deposit(
    chain: &Chain,
    program_id: &Pubkey,
    note_path: &Path,
    depositor: &Keypair,
) -> Result<()> {
    let stored = StoredNote::read(note_path)?;
    let note = stored.note()?;
    let state = read_pool(chain, program_id, stored.denomination)?;
    let commitment = stored.commitment_bytes()?;

    // Refuse a second deposit of the same note before it costs anything. The
    // program allows duplicate commitments — the only party a duplicate harms is
    // whoever paid for it — but a member doing it by accident is out a
    // denomination for a leaf they cannot distinguish from their first.
    let history = history::scan(chain, program_id, &state.pool, false)?;
    if history.index_of(&commitment).is_some() {
        return Err(anyhow!(
            "this note's commitment is already a leaf of this pool. Depositing it \
             again would escrow a second denomination against a note you cannot \
             tell apart from the first."
        ));
    }

    let balance = chain.balance(&depositor.pubkey())?;
    if balance < state.denomination {
        return Err(anyhow!(
            "{} holds {} lamports and this pool escrows {} per deposit",
            depositor.pubkey(),
            balance,
            state.denomination
        ));
    }

    let ix = Instruction::new_with_bytes(
        *program_id,
        &MirrorIx::Deposit { commitment }.pack(),
        vec![
            AccountMeta::new(depositor.pubkey(), true),
            AccountMeta::new(state.pool, false),
            AccountMeta::new(state.vault, false),
            AccountMeta::new_readonly(solana_system_interface::program::ID, false),
        ],
    );
    let sig = send(chain, ix, &[depositor])?;
    println!(
        "deposited {} lamports into {}",
        state.denomination, state.pool
    );
    println!("leaf      {}", state.deposits);
    println!("signature {sig}");
    println!();
    println!(
        "Keep {} safe. It is the only way to spend this note,",
        note_path.display()
    );
    println!("and nobody — including this pool's authors — can reissue it.");
    let _ = note;
    Ok(())
}

// ---------------------------------------------------------------------------
// tree

/// Rebuilds the accumulator from chain history and checks it against the pool.
///
/// The check is the point. A rebuilt tree whose root matches the one the program
/// holds is proof that the history is complete and correctly ordered, which is
/// exactly the precondition a membership proof needs. A mismatch means any proof
/// built here would fail on-chain for reasons that would be very hard to read
/// backwards from the failure.
pub fn tree(chain: &Chain, program_id: &Pubkey, denomination: u64) -> Result<History> {
    let state = read_pool(chain, program_id, denomination)?;
    println!("pool          {}", state.pool);
    println!("vault         {}", state.vault);
    println!("denomination  {}", state.denomination);
    println!("k floor       {}", state.k_floor);
    println!("timeout       {}s", state.settle_timeout);
    println!(
        "notes         {} deposited, {} settled, {} outstanding",
        state.deposits,
        state.spends,
        state.deposits.saturating_sub(state.spends)
    );
    println!();
    // The nominal figures above are the ones a reader walks away with, so the
    // measured discount travels with them rather than living only in a
    // document nobody has open.
    println!("{}", mirror_provenance::PUBLISHED_HEADLINE.note());
    println!();
    println!("rebuilding the accumulator from chain history:");

    let history = history::scan(chain, program_id, &state.pool, true)?;
    let rebuilt = history.tree()?;
    let root = rebuilt.root().map_err(|e| anyhow!("{e:?}"))?;

    println!();
    println!("  leaves recovered  {}", history.commitments.len());
    println!("  pool reports      {}", state.deposits);
    println!("  rebuilt root      {}", hex::encode(root.to_bytes()));
    println!("  on-chain root     {}", hex::encode(state.root));

    if history.commitments.len() as u64 != state.deposits {
        return Err(anyhow!(
            "recovered {} leaves but the pool has inserted {}. The history is \
             incomplete — most likely the endpoint has pruned it. `mirror \
             check-endpoint` tests for exactly that.",
            history.commitments.len(),
            state.deposits
        ));
    }
    if root.to_bytes() != state.root {
        return Err(anyhow!(
            "the rebuilt root does not match the pool's. The leaf set or its \
             order is wrong, and any proof built against this tree would be \
             rejected on-chain."
        ));
    }
    println!();
    println!("  the rebuilt tree matches the chain — proofs built from it will verify");
    Ok(history)
}

// ---------------------------------------------------------------------------
// spend

/// What a member is asking the pool to do.
pub enum Action {
    /// Pay the beneficiary, no CPI.
    Transfer,
    /// Call `target` with `payload`. `pool_signs` picks the selector that hands
    /// the vault to the callee as a signer.
    Invoke {
        target: Pubkey,
        payload: Vec<u8>,
        accounts: u8,
        pool_signs: bool,
    },
}

impl Action {
    fn selector(&self) -> u64 {
        match self {
            Action::Transfer => SELECTOR_TRANSFER,
            Action::Invoke {
                pool_signs: false, ..
            } => SELECTOR_INVOKE,
            Action::Invoke {
                pool_signs: true, ..
            } => SELECTOR_INVOKE_SIGNED,
        }
    }
    fn target(&self) -> [u8; 32] {
        match self {
            Action::Transfer => [0u8; 32],
            Action::Invoke { target, .. } => target.to_bytes(),
        }
    }
    fn payload(&self) -> &[u8] {
        match self {
            Action::Transfer => &[],
            Action::Invoke { payload, .. } => payload,
        }
    }
    fn accounts(&self) -> u8 {
        match self {
            Action::Transfer => 0,
            Action::Invoke { accounts, .. } => *accounts,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spend(
    chain: &Chain,
    program_id: &Pubkey,
    note_path: &Path,
    beneficiary: &Pubkey,
    relay: &Keypair,
    relay_fee: u64,
    action: Action,
) -> Result<()> {
    let stored = StoredNote::read(note_path)?;
    let note = stored.note()?;
    let state = read_pool(chain, program_id, stored.denomination)?;

    if relay.pubkey() == *beneficiary {
        eprintln!(
            "warning: the relay and the beneficiary are the same key, which links \
             the payout to whoever paid the fee."
        );
    }
    if relay_fee >= state.denomination {
        return Err(anyhow!(
            "a relay fee of {relay_fee} is not less than the denomination {}",
            state.denomination
        ));
    }
    if state.deposits < state.k_floor as u64 {
        return Err(anyhow!(
            "this pool holds {} notes and its floor is {}. It refuses to act \
             below the floor, because a crowd of one is not a crowd.",
            state.deposits,
            state.k_floor
        ));
    }

    println!("rebuilding the accumulator so this note can be proved a member:");
    let history = history::scan(chain, program_id, &state.pool, true)?;
    let commitment = stored.commitment_bytes()?;
    let index = history.index_of(&commitment).ok_or_else(|| {
        anyhow!(
            "this note is not a leaf of the pool. Either it was never deposited, \
             or it belongs to a different denomination."
        )
    })?;
    let tree = history.tree()?;
    let root = tree.root().map_err(|e| anyhow!("{e:?}"))?;
    if root.to_bytes() != state.root {
        return Err(anyhow!(
            "the rebuilt root does not match the pool's, so a proof against it \
             would be rejected. Run `mirror tree` to see the mismatch."
        ));
    }
    println!(
        "  this note is leaf {index} of {}",
        history.commitments.len()
    );

    // The mistake that undoes the whole protocol, caught before it lands.
    //
    // The relay signs, so its key is public on the spend transaction. If that
    // same key signed a deposit into this pool, an observer joins the two by
    // reading two public transactions and the anonymity set collapses to one for
    // this action — the proof stays sound and protects nothing. It is an easy
    // mistake to make, because the obvious key to hand `--relay` is the one
    // already funded, which is the one that deposited.
    if history.depositors.contains(&relay.pubkey()) {
        return Err(anyhow!(
            "the relay {} has also deposited into this pool.\n\n\
             The relay signs this spend, so its key is public on it. A key that \
             appears on both a deposit and a spend links them, and this action's \
             anonymity set collapses to one — the proof would be sound and \
             pointless.\n\n\
             Use a key with no history with this pool, funded from somewhere that \
             does not lead back to your deposit.",
            relay.pubkey()
        ));
    }

    let binding = mirror_core::action_binding(
        action.selector(),
        &action.target(),
        &beneficiary.to_bytes(),
        &relay.pubkey().to_bytes(),
        relay_fee,
        action.accounts(),
        action.payload(),
    );
    let merkle_proof = tree.proof(index).map_err(|e| anyhow!("{e:?}"))?;
    let witness = mirror_circuit::Witness {
        note,
        merkle_proof: &merkle_proof,
        root,
        action_binding: binding,
    };

    println!("deriving the proving key from the published seed (this takes a moment)");
    let keys = crate::soak::keys()?;
    println!("proving membership");
    let mut rng = ark_std::rand::rngs::StdRng::from_entropy();
    let proof = mirror_circuit::prove(&keys, &witness, &mut rng).map_err(|e| anyhow!("{e:?}"))?;
    let nullifier = proof.public_inputs[1];
    let (spend_pda, _) = spend_address(program_id, &state.pool, &nullifier);

    let ix = Instruction::new_with_bytes(
        *program_id,
        &MirrorIx::SubmitSpend {
            proof_a: proof.proof_a,
            proof_b: proof.proof_b,
            proof_c: proof.proof_c,
            root: proof.public_inputs[0],
            nullifier,
            selector: action.selector(),
            target_program: action.target(),
            beneficiary: beneficiary.to_bytes(),
            relay_fee,
            action_accounts: action.accounts(),
            payload: action.payload().to_vec(),
        }
        .pack(),
        vec![
            AccountMeta::new(relay.pubkey(), true),
            AccountMeta::new(state.pool, false),
            AccountMeta::new(spend_pda, false),
            AccountMeta::new_readonly(solana_system_interface::program::ID, false),
        ],
    );
    let sig = send(chain, ix, &[relay])?;

    println!();
    println!("submitted. The note is spent and the action is recorded.");
    println!("  nullifier {}", hex::encode(nullifier));
    println!("  record    {spend_pda}");
    println!("  signature {sig}");
    println!();
    println!("Nothing has been paid out yet — settlement executes the batch, which is");
    println!("what gives every member's action one timestamp. Run `mirror settle`, or");
    println!("wait for anyone else to.");
    Ok(())
}

// ---------------------------------------------------------------------------
// settle

pub fn settle(
    chain: &Chain,
    program_id: &Pubkey,
    denomination: u64,
    settler: &Keypair,
    now: i64,
    allow_below_floor: bool,
) -> Result<()> {
    let state = read_pool(chain, program_id, denomination)?;
    println!("looking for spends waiting to settle:");
    let history = history::scan(chain, program_id, &state.pool, true)?;

    let mut ready: Vec<(Pubkey, Pubkey, Pubkey, i64, u64)> = Vec::new();
    let mut skipped_actions: Vec<([u8; 32], u64, u8)> = Vec::new();
    for submitted in &history.spends {
        let Some(mut data) = chain.account_data(&submitted.spend)? else {
            continue;
        };
        let Ok(record) = Spend::load(&mut data) else {
            continue;
        };
        if record.status() != STATUS_PENDING {
            continue;
        }
        if record.selector() != SELECTOR_TRANSFER {
            // A CPI action needs the account list its callee expects, which this
            // command cannot infer from the chain — the record binds how many
            // accounts, never which. Settling those is a caller's job.
            skipped_actions.push((
                submitted.nullifier,
                record.selector(),
                record.action_accounts(),
            ));
            continue;
        }
        ready.push((
            submitted.spend,
            Pubkey::new_from_array(record.beneficiary()),
            Pubkey::new_from_array(record.relay()),
            record.submitted_at(),
            record.relay_fee(),
        ));
    }

    if !skipped_actions.is_empty() {
        println!(
            "  {} pending action(s) left alone: a CPI needs the account list its callee",
            skipped_actions.len()
        );
        println!("  expects, and the record binds how many accounts, never which.");
        for (nullifier, selector, accounts) in &skipped_actions {
            println!(
                "    nullifier {} — selector {selector}, {accounts} account(s)",
                hex::encode(nullifier)
            );
        }
    }
    if ready.is_empty() {
        println!("  nothing to settle");
        return Ok(());
    }

    // Members are paid `denomination - relay_fee`, and that payout is public — so
    // a batch mixing fees settles into visibly different amounts and an observer
    // partitions it by value. The program refuses such a batch; this picks one
    // fee and settles the members who paid it, rather than assembling a
    // transaction that will be rejected.
    //
    // The largest group goes first, because the batch that hides its members
    // best is the biggest one available.
    let mut by_fee: std::collections::BTreeMap<u64, Vec<_>> = std::collections::BTreeMap::new();
    for entry in ready {
        by_fee.entry(entry.4).or_default().push(entry);
    }
    let fee_groups = by_fee.len();
    let (chosen_fee, mut ready) = by_fee
        .into_iter()
        .max_by_key(|(_, group)| group.len())
        .expect("there is at least one pending spend");
    if fee_groups > 1 {
        println!();
        println!(
            "  the pending spends carry {fee_groups} different relay fees. Settling the {} that",
            ready.len()
        );
        println!(
            "  paid {chosen_fee}: a batch of mixed fees pays visibly different amounts, which"
        );
        println!("  lets an observer partition it by value. Run again for the others.");
    }

    // A lookup table takes the packet out of the way, and what takes over is the
    // number of accounts a transaction may lock. Members are added while the
    // batch still fits under it, so a settler is never handed a transaction the
    // cluster will refuse — and never has to know the limit exists.
    let base = vec![
        AccountMeta::new(settler.pubkey(), true),
        AccountMeta::new(state.pool, false),
        AccountMeta::new(state.vault, false),
    ];
    let mut metas = base.clone();
    let mut taken = 0usize;
    for (spend, beneficiary, relay, _, _) in &ready {
        let mut next = metas.clone();
        next.push(AccountMeta::new(*spend, false));
        next.push(AccountMeta::new(*beneficiary, false));
        next.push(AccountMeta::new(*relay, false));
        if lookup::locks_for(&next, program_id, &settler.pubkey()) > lookup::MAX_ACCOUNT_LOCKS {
            break;
        }
        metas = next;
        taken += 1;
    }
    if taken == 0 {
        return Err(anyhow!(
            "a single spend already locks more than the {} accounts a transaction may hold",
            lookup::MAX_ACCOUNT_LOCKS
        ));
    }
    let deferred = ready.len() - taken;
    if deferred > 0 {
        println!();
        println!(
            "  {} spend(s) pending; settling {taken} of them, which is what fits under the",
            ready.len()
        );
        println!(
            "  {}-account lock limit. Run this again for the rest — the cost is a",
            lookup::MAX_ACCOUNT_LOCKS
        );
        println!("  second timestamp, which is a real cost to the anonymity of both halves.");
    }
    ready.truncate(taken);

    // The crowd rule, checked against the batch actually being sent rather than
    // against everything pending. Those differ whenever the lock limit truncates
    // a batch, and checking the wrong one builds a transaction the program then
    // refuses with a bare error code — which is exactly what this check exists
    // to spare a caller.
    let crowd = ready.len() as u32 >= state.k_floor;
    let timeout = state.settle_timeout;
    if !crowd {
        let youngest = ready.iter().map(|(_, _, _, at, _)| *at).max().unwrap_or(0);
        let waited = now.saturating_sub(youngest);
        if waited < timeout {
            println!();
            println!(
                "  this batch carries {} spend(s) and the pool's floor is {}.",
                ready.len(),
                state.k_floor
            );
            if deferred > 0 {
                println!(
                    "  {} more are pending but cannot join: the floor is above what one",
                    deferred
                );
                println!("  transaction can settle, so this pool can only settle on the timeout.");
            }
            println!(
                "  A batch below the floor may settle once every spend in it has waited {timeout}s;"
            );
            println!(
                "  the youngest has waited {waited}s, so {}s remain.",
                timeout - waited
            );
            println!();
            println!("  This is a liveness guarantee rather than a restriction: nobody can");
            println!("  hold a member's funds waiting for a crowd that never arrives.");
            return Ok(());
        }

        // Past the timeout the program will accept this batch whatever its size,
        // and a settler about to publish an under-floor batch should be told what
        // it buys the members in it — which, at one member, is nothing. The
        // timeout is a solvency guarantee and it is not an anonymity one.
        println!();
        println!(
            "  WARNING: this batch carries {} spend(s), below the pool's floor of {}.",
            ready.len(),
            state.k_floor
        );
        println!("  It settles because every spend in it has waited out the {timeout}s timeout,");
        println!("  which is a solvency guarantee and not an anonymity one. The members in");
        if ready.len() == 1 {
            println!("  this batch get an anonymity set of exactly one: the action will be");
            println!("  attributable to whoever submitted it. Wait for company if you can.");
        } else {
            println!(
                "  this batch get an anonymity set of {}, not {}.",
                ready.len(),
                state.k_floor
            );
        }

        // The consent, and it is asked for here rather than assumed.
        //
        // Warning and then settling anyway is how a settler ends up publishing
        // an under-floor batch they never meant to compose — the notice scrolls
        // past and the transaction lands. The program refuses this without the
        // flag; so does the tool, and for the same reason.
        if !allow_below_floor {
            println!();
            println!("  Not settling. This is not a failure: the program refuses an under-floor");
            println!("  batch that nobody asked for, and so does this command. If it is what you");
            println!("  intend — usually because the members' funds would otherwise stay escrowed");
            println!("  — re-run with --allow-below-floor.");
            println!();
            println!("  The settlement will be marked on chain as having landed below the floor,");
            println!("  so the members can tell afterwards what crowd they actually got.");
            return Ok(());
        }
    }

    let ix = Instruction::new_with_bytes(
        *program_id,
        &MirrorIx::SettleEpoch {
            count: ready.len() as u8,
            // Only ever true on the path that has already printed the warning
            // and been told to go ahead: a crowd-sized batch never asks for it.
            allow_below_floor: !crowd,
        }
        .pack(),
        metas,
    );

    // A legacy transaction names every account by its full 32 bytes, so a batch
    // outgrows the 1232-byte packet at ten members. Rather than settle ten and
    // leave the rest, a batch that does not fit is settled through a lookup
    // table, which names the same accounts by one byte each.
    //
    // Legacy stays the default for batches that fit, and that is deliberate: a
    // table costs four extra transactions, a slot of latency and rent, and none
    // of it buys anything for a batch already inside the packet.
    let sig = match legacy_settlement_bytes(&ix, &settler.pubkey()) {
        bytes if bytes <= PACKET_DATA_SIZE => {
            println!();
            println!(
                "settling {} spends in one legacy transaction, {bytes} bytes",
                ready.len()
            );
            send(chain, ix, &[settler])?
        }
        bytes => {
            println!();
            println!(
                "  {} spends do not fit a legacy transaction: {bytes} bytes, {} over the \
                 {PACKET_DATA_SIZE}-byte packet.",
                ready.len(),
                bytes - PACKET_DATA_SIZE
            );
            println!("  Settling through a lookup table instead.");
            settle_through_lookup_table(chain, ix, settler)?
        }
    };
    println!();
    println!("settled {} spends in one transaction", ready.len());
    println!("  signature {sig}");
    println!();
    println!("Every payout in that batch shares one timestamp and one ordering, which");
    println!("is what stops arrival time from telling the members apart.");
    println!();
    // The batch size is an anonymity set, and printing one without the measured
    // discount is exactly the lift this guards against.
    println!(
        "The {} members above are that batch's nominal set.",
        ready.len()
    );
    println!("{}", mirror_provenance::PUBLISHED_HEADLINE.note());
    Ok(())
}

/// The 1280-byte IPv6 minimum MTU, less a 40-byte IPv6 header and an 8-byte
/// fragment header.
const PACKET_DATA_SIZE: usize = 1280 - 40 - 8;

/// What this instruction would weigh as a legacy transaction signed by one key.
fn legacy_settlement_bytes(ix: &Instruction, payer: &Pubkey) -> usize {
    let message = solana_message::Message::new(std::slice::from_ref(ix), Some(payer));
    1 + 64 + message.serialize().len()
}

/// Publishes a lookup table, settles through it, and then takes it back down.
///
/// The table is the interesting part of the cost, and not only because of the
/// rent. While it exists it is a public, durable account listing every address
/// the settlement is about to touch — published *before* the settlement lands,
/// signed by the settler. Leaving it there would turn a one-transaction event
/// into a permanent on-chain index of the batch, which is a strange thing for a
/// privacy pool to leave behind.
///
/// So it is deactivated immediately, and closing it — which returns the rent and
/// removes the list — is reported as the errand it is: the runtime enforces a
/// cooldown of roughly `DEACTIVATION_COOLDOWN_SLOTS` slots first, because a
/// transaction already in flight may still be resolving against the table.
fn settle_through_lookup_table(
    chain: &Chain,
    ix: Instruction,
    settler: &Keypair,
) -> Result<String> {
    use solana_message::{v0, AddressLookupTableAccount, VersionedMessage};
    use solana_transaction::versioned::VersionedTransaction;

    let authority = settler.pubkey();
    let addresses = lookup::addresses_for(&ix.accounts, &ix.program_id);

    let recent_slot = chain.recent_slot()?;
    let (create_ix, table) = lookup::create(&authority, &authority, recent_slot)?;
    send(chain, create_ix, &[settler])?;
    println!("  lookup table {table}");

    for chunk in addresses.chunks(lookup::ADDRESSES_PER_EXTEND) {
        let extend_ix = lookup::extend(&table, &authority, &authority, chunk)?;
        send(chain, extend_ix, &[settler])?;
    }
    println!("  {} addresses published", addresses.len());

    // A table cannot be used in the slot it was extended in: the runtime resolves
    // it against a slot strictly later than the last write. Waiting for the
    // cluster to move on is not politeness, it is the difference between a
    // settlement that lands and one rejected for an address the table already
    // holds.
    let extended_at = chain.recent_slot()?;
    while chain.recent_slot()? <= extended_at {
        std::thread::sleep(std::time::Duration::from_millis(400));
    }

    let alt = AddressLookupTableAccount {
        key: table,
        addresses,
    };
    let message = v0::Message::try_compile(&authority, &[ix], &[alt], chain.latest_blockhash()?)
        .map_err(|e| anyhow!("compiling the settlement against the lookup table: {e}"))?;
    let tx = VersionedTransaction::try_new(VersionedMessage::V0(message), &[settler])
        .map_err(|e| anyhow!("signing the settlement: {e}"))?;
    let bytes = bincode::serialize(&tx)?.len();
    println!("  settlement is {bytes} bytes of {PACKET_DATA_SIZE}, one signature");
    let sig = chain.send_versioned(&tx)?;

    send(chain, lookup::deactivate(&table, &authority)?, &[settler])?;
    println!(
        "  table deactivated. Close it after ~{} slots to reclaim the rent and remove",
        lookup::DEACTIVATION_COOLDOWN_SLOTS
    );
    println!("  the published address list:");
    println!("    mirror close-table --table {table}");
    Ok(sig)
}

/// Takes a lookup table back down, returning its rent and removing the address
/// list it published.
///
/// Separate from settling because the runtime will not allow it to be the same
/// errand: a table must sit deactivated for a cooldown before it can close, so
/// that a transaction already in flight cannot have the table pulled out from
/// under it. Deactivation happens at settlement; this is the second half.
///
/// Running it is optional in the sense that nothing breaks if nobody does, and
/// not optional in the sense that skipping it leaves rent stranded and, more to
/// the point, leaves a permanent public list of every address that settlement
/// touched.
pub fn close_table(chain: &Chain, table: &Pubkey, authority: &Keypair) -> Result<()> {
    let before = chain.balance(&authority.pubkey())?;
    let ix = lookup::close(table, &authority.pubkey(), &authority.pubkey())?;
    match send(chain, ix, &[authority]) {
        Ok(sig) => {
            let reclaimed = chain.balance(&authority.pubkey())?.saturating_sub(before);
            println!("closed {table}");
            println!("  signature {sig}");
            println!("  reclaimed {reclaimed} lamports, and the published address list is gone");
            Ok(())
        }
        Err(e) => {
            let message = e.to_string();
            if message.contains("not been deactivated") || message.contains("still in use") {
                Err(anyhow!(
                    "{table} is not closable yet. A deactivated table waits about {} slots \
                     before it can be closed, because a transaction already in flight may \
                     still be resolving against it. Try again shortly.",
                    lookup::DEACTIVATION_COOLDOWN_SLOTS
                ))
            } else {
                Err(e)
            }
        }
    }
}

/// Creates a note and writes it, printing what to do next.
pub fn note_new(denomination: u64, path: &Path) -> Result<()> {
    let note = crate::note::generate(denomination)?;
    let stored = StoredNote::from_note(&note, denomination)?;
    stored.write(path)?;
    println!("wrote {}", path.display());
    println!("  denomination {denomination}");
    println!("  commitment   {}", stored.commitment);
    println!();
    println!("This file is the note. Treat it exactly as you would a keypair:");
    println!("anyone holding it can spend the deposit, and losing it loses the");
    println!("deposit with no way back.");
    println!();
    println!("Next: mirror deposit --note {}", path.display());
    Ok(())
}

/// The current wall clock, for the crowd-rule explanation.
pub fn now() -> Result<i64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("reading the clock")?
        .as_secs() as i64)
}
