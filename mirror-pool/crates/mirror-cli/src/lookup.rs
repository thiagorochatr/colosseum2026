//! Address lookup tables, as much of them as a settlement needs.
//!
//! A legacy transaction names every account it touches by its full 32 bytes, so
//! a batch of members costs about 99 bytes each and the 1232-byte packet stops
//! it at ten. A v0 transaction may instead name accounts by a one-byte index
//! into a table published on chain, which drops the marginal cost of a member by
//! more than an order of magnitude and moves the ceiling somewhere else
//! entirely. `batch_ceiling.rs` measures the legacy number; this module is how
//! settlement stops being bound by it.
//!
//! The instruction layouts belong to somebody else's program and are written out
//! here rather than pulled in as a dependency, for the same reason `stake.rs`
//! does it: the crate that owns them drags in half the SDK, and a byte layout
//! whose derivation is written down can be checked by a reader.
//!
//! **A table is not free and it is not private.** It is a rent-paying account
//! that lists, publicly and durably, every address the settlement will touch —
//! published *before* the settlement lands. That is a real disclosure, and the
//! answer to it is `deactivate` + `close`: the rent comes back and the list
//! stops existing. A design that creates one of these per batch and never closes
//! it leaves both the lamports and the list behind, permanently, once per round.

use anyhow::{anyhow, Result};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

pub const PROGRAM: &str = "AddressLookupTab1e1111111111111111111111111";

/// How many addresses one `extend` can carry.
///
/// Bounded by the packet, not by the program: the instruction carries the
/// addresses in full, so 30 of them is 960 bytes and anything much larger stops
/// fitting. Extending in chunks is the normal way to fill a table.
pub const ADDRESSES_PER_EXTEND: usize = 20;

/// How many distinct accounts one transaction may lock.
///
/// **Measured against devnet, not read out of the SDK.** `solana-transaction`
/// exports `MAX_TX_ACCOUNT_LOCKS = 128`, and that is the raised limit, live only
/// where the feature gate for it is active. A settlement naming 77 accounts was
/// refused by devnet with `TooManyAccountLocks`, so the number a client must
/// plan against is the older one until it can see otherwise.
///
/// This is what actually bounds a settlement once a lookup table has taken the
/// packet out of the way. Twenty-four members through a table weigh 356 bytes of
/// 1232 — nowhere near the wire — and are refused for locking 77 accounts.
pub const MAX_ACCOUNT_LOCKS: usize = 64;

/// Slots a table must sit deactivated before it can be closed.
///
/// The cooldown exists because a transaction already in flight may still resolve
/// against the table, and closing it underneath such a transaction would change
/// what that transaction means. It is why reclaiming the rent is a later errand
/// rather than part of settling.
pub const DEACTIVATION_COOLDOWN_SLOTS: u64 = 513;

fn program_id() -> Result<Pubkey> {
    PROGRAM.parse().map_err(|e| anyhow!("{PROGRAM}: {e}"))
}

/// The address a table gets, given who owns it and which slot it was derived
/// against.
///
/// The recent slot is part of the seed, which is what stops one authority from
/// creating the same table twice and what ties a table to a moment. The runtime
/// checks the slot is recent, so a stale one is refused rather than silently
/// producing a different address.
pub fn address(authority: &Pubkey, recent_slot: u64) -> Result<(Pubkey, u8)> {
    Ok(Pubkey::find_program_address(
        &[authority.as_ref(), &recent_slot.to_le_bytes()],
        &program_id()?,
    ))
}

/// `CreateLookupTable { recent_slot, bump }` — discriminant 0.
pub fn create(
    authority: &Pubkey,
    payer: &Pubkey,
    recent_slot: u64,
) -> Result<(Instruction, Pubkey)> {
    let (table, bump) = address(authority, recent_slot)?;
    let mut data = 0u32.to_le_bytes().to_vec();
    data.extend_from_slice(&recent_slot.to_le_bytes());
    data.push(bump);
    Ok((
        Instruction::new_with_bytes(
            program_id()?,
            &data,
            vec![
                AccountMeta::new(table, false),
                AccountMeta::new_readonly(*authority, true),
                AccountMeta::new(*payer, true),
                AccountMeta::new_readonly(solana_system_interface::program::ID, false),
            ],
        ),
        table,
    ))
}

/// `ExtendLookupTable { new_addresses }` — discriminant 2, then a bincode
/// sequence: a u64 length followed by the addresses.
pub fn extend(
    table: &Pubkey,
    authority: &Pubkey,
    payer: &Pubkey,
    addresses: &[Pubkey],
) -> Result<Instruction> {
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend_from_slice(&(addresses.len() as u64).to_le_bytes());
    for a in addresses {
        data.extend_from_slice(&a.to_bytes());
    }
    Ok(Instruction::new_with_bytes(
        program_id()?,
        &data,
        vec![
            AccountMeta::new(*table, false),
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(solana_system_interface::program::ID, false),
        ],
    ))
}

/// `DeactivateLookupTable` — discriminant 3. Starts the cooldown; the table is
/// still usable until it elapses.
pub fn deactivate(table: &Pubkey, authority: &Pubkey) -> Result<Instruction> {
    Ok(Instruction::new_with_bytes(
        program_id()?,
        &3u32.to_le_bytes(),
        vec![
            AccountMeta::new(*table, false),
            AccountMeta::new_readonly(*authority, true),
        ],
    ))
}

/// `CloseLookupTable` — discriminant 4. Returns the rent and removes the
/// published address list.
pub fn close(table: &Pubkey, authority: &Pubkey, recipient: &Pubkey) -> Result<Instruction> {
    Ok(Instruction::new_with_bytes(
        program_id()?,
        &4u32.to_le_bytes(),
        vec![
            AccountMeta::new(*table, false),
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new(*recipient, false),
        ],
    ))
}

/// The addresses a table must hold for `metas` to resolve, in first-seen order,
/// excluding those that cannot come from a table.
///
/// Two kinds of account must stay in the transaction's static keys and so must
/// never enter the table. A **signer** cannot be served from a lookup table at
/// all — the runtime resolves the table after checking signatures. And the
/// **program being invoked** must be a static key, because the instruction
/// names it by an index into the static portion.
///
/// Getting either wrong produces a transaction that compiles and is then
/// rejected by the cluster, so the filtering happens here, once, rather than at
/// each call site.
/// How many distinct accounts a transaction would lock for `metas`.
///
/// The fee payer and the invoked program are locked too, so they count even
/// though neither can be served from a table.
pub fn locks_for(metas: &[AccountMeta], program: &Pubkey, payer: &Pubkey) -> usize {
    let mut seen: Vec<Pubkey> = vec![*program, *payer];
    for m in metas {
        if !seen.contains(&m.pubkey) {
            seen.push(m.pubkey);
        }
    }
    seen.len()
}

pub fn addresses_for(metas: &[AccountMeta], program: &Pubkey) -> Vec<Pubkey> {
    let mut out: Vec<Pubkey> = Vec::new();
    for m in metas {
        if m.is_signer || m.pubkey == *program || out.contains(&m.pubkey) {
            continue;
        }
        out.push(m.pubkey);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_carries_the_slot_and_the_bump_the_address_was_derived_from() {
        let authority = Pubkey::new_unique();
        let slot = 305_419_896u64;
        let (ix, table) = create(&authority, &authority, slot).unwrap();
        let (expected, bump) = address(&authority, slot).unwrap();
        assert_eq!(table, expected);
        assert_eq!(&ix.data[..4], 0u32.to_le_bytes());
        assert_eq!(&ix.data[4..12], slot.to_le_bytes());
        assert_eq!(ix.data[12], bump, "the bump must be the derived one");
        assert_eq!(ix.data.len(), 13);
    }

    /// The length prefix is a bincode `u64`, not a compact-u16. A four-byte
    /// prefix would leave the program reading addresses from the wrong offset
    /// and rejecting a table that was built correctly.
    #[test]
    fn extend_prefixes_the_addresses_with_a_u64_count() {
        let table = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let addresses: Vec<Pubkey> = (0..3).map(|_| Pubkey::new_unique()).collect();
        let ix = extend(&table, &authority, &authority, &addresses).unwrap();
        assert_eq!(&ix.data[..4], 2u32.to_le_bytes());
        assert_eq!(&ix.data[4..12], 3u64.to_le_bytes());
        assert_eq!(ix.data.len(), 12 + 3 * 32);
        for (i, a) in addresses.iter().enumerate() {
            let at = 12 + i * 32;
            assert_eq!(&ix.data[at..at + 32], a.to_bytes().as_slice());
        }
    }

    #[test]
    fn deactivate_and_close_are_bare_discriminants() {
        let table = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        assert_eq!(
            deactivate(&table, &authority).unwrap().data,
            3u32.to_le_bytes()
        );
        assert_eq!(
            close(&table, &authority, &authority).unwrap().data,
            4u32.to_le_bytes()
        );
    }

    /// The rent goes back to whoever is named, which is the whole point of
    /// closing rather than abandoning.
    #[test]
    fn close_names_the_account_the_rent_returns_to() {
        let table = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let recipient = Pubkey::new_unique();
        let ix = close(&table, &authority, &recipient).unwrap();
        assert_eq!(ix.accounts[2].pubkey, recipient);
        assert!(ix.accounts[2].is_writable);
    }

    /// A signer served from a lookup table makes a transaction the cluster
    /// refuses. So does the program being invoked. Both are filtered, and this
    /// is the test that says so.
    #[test]
    fn neither_a_signer_nor_the_invoked_program_enters_the_table() {
        let program = Pubkey::new_unique();
        let settler = Pubkey::new_unique();
        let pool = Pubkey::new_unique();
        let metas = vec![
            AccountMeta::new(settler, true),
            AccountMeta::new(pool, false),
            AccountMeta::new_readonly(program, false),
        ];
        let addresses = addresses_for(&metas, &program);
        assert_eq!(addresses, vec![pool]);
    }

    /// What a table is actually worth, measured rather than argued.
    ///
    /// This builds settlement-shaped instructions — a settler, a pool, a vault,
    /// then three accounts nobody else shares per member — and serializes them
    /// both ways. The legacy figure must reproduce what `batch_ceiling.rs`
    /// settles against the real program, which is what ties this arithmetic to
    /// an executed measurement; the v0 figure is the point.
    ///
    /// The assertion is deliberately not "the ceiling is N". Once accounts cost
    /// a byte each, the packet stops being the thing in the way at any batch
    /// size worth settling, and what takes over — compute, the account-lock
    /// limit, the program's own heap — depends on what the members are doing.
    /// Pinning a number here would be pinning the wrong constraint.
    #[test]
    fn a_lookup_table_takes_the_packet_out_of_the_way() {
        use solana_message::{v0, AddressLookupTableAccount, VersionedMessage};

        const PACKET_DATA_SIZE: usize = 1280 - 40 - 8;

        // `batch_ceiling.rs` settles ten of these against the real program at
        // 1228 bytes. If this model disagrees there, it is modelling something
        // else and the v0 number below means nothing.
        let legacy_at = |n: usize| -> usize {
            let (settler, program) = (Pubkey::new_unique(), Pubkey::new_unique());
            let ix = settle_shaped(&settler, &program, n);
            let msg = solana_message::Message::new(&[ix], Some(&settler));
            1 + 64 + msg.serialize().len()
        };
        assert_eq!(
            legacy_at(10),
            1228,
            "the legacy model no longer agrees with the executed measurement in \
             batch_ceiling.rs, so nothing else here can be trusted"
        );
        assert!(
            legacy_at(11) > PACKET_DATA_SIZE,
            "eleven members are supposed to be over the packet"
        );

        let versioned_at = |n: usize| -> usize {
            let (settler, program) = (Pubkey::new_unique(), Pubkey::new_unique());
            let ix = settle_shaped(&settler, &program, n);
            let alt = AddressLookupTableAccount {
                key: Pubkey::new_unique(),
                addresses: addresses_for(&ix.accounts, &program),
            };
            let msg = v0::Message::try_compile(
                &settler,
                &[ix],
                &[alt],
                solana_program::hash::Hash::default(),
            )
            .expect("a settlement must compile against its own table");
            1 + 64
                + bincode::serialize(&VersionedMessage::V0(msg))
                    .expect("serializes")
                    .len()
        };

        // Sixty members is six times what the packet allows without a table and
        // still nowhere near it — which is the finding, and it is a statement
        // about *bytes only*. Sixty members lock 183 accounts and the cluster
        // would refuse them; what a settlement can actually carry is bounded by
        // MAX_ACCOUNT_LOCKS, asserted just below, and a reader who took the byte
        // figure for a member count would be reading the wrong constraint.
        let sixty = versioned_at(60);
        assert!(
            sixty < PACKET_DATA_SIZE,
            "sixty members through a table weigh {sixty} bytes, over the packet"
        );

        // The limit that does bind, and the arithmetic settlement uses to stop
        // short of it. Twenty members is exactly 64 locks: three per member plus
        // the settler, the pool and the vault.
        let payer = Pubkey::new_unique();
        let program = Pubkey::new_unique();
        let twenty = settle_shaped(&payer, &program, 20);
        assert_eq!(
            locks_for(&twenty.accounts, &program, &payer),
            MAX_ACCOUNT_LOCKS,
            "twenty members is supposed to sit exactly on the lock limit"
        );
        let twenty_one = settle_shaped(&payer, &program, 21);
        assert!(
            locks_for(&twenty_one.accounts, &program, &payer) > MAX_ACCOUNT_LOCKS,
            "twenty-one members must be over the lock limit, or settlement stops \
             one short of what it could carry"
        );
        assert!(
            versioned_at(10) < legacy_at(10) / 2,
            "a table is supposed to cost far less per member than naming keys in full"
        );
    }

    /// A settlement's account shape: three fixed, then three per member that no
    /// other member shares. The same shape `batch_ceiling.rs` settles.
    fn settle_shaped(settler: &Pubkey, program: &Pubkey, n: usize) -> Instruction {
        let mut metas = vec![
            AccountMeta::new(*settler, true),
            AccountMeta::new(Pubkey::new_unique(), false),
            AccountMeta::new(Pubkey::new_unique(), false),
        ];
        for _ in 0..n {
            for _ in 0..3 {
                metas.push(AccountMeta::new(Pubkey::new_unique(), false));
            }
        }
        Instruction::new_with_bytes(*program, &[4u8, n as u8], metas)
    }

    /// A batch that names the same relay twice must not pay for it twice: the
    /// table holds distinct addresses, and the saving is per distinct key.
    #[test]
    fn a_repeated_account_is_listed_once() {
        let program = Pubkey::new_unique();
        let shared = Pubkey::new_unique();
        let other = Pubkey::new_unique();
        let metas = vec![
            AccountMeta::new(shared, false),
            AccountMeta::new(other, false),
            AccountMeta::new(shared, false),
        ];
        assert_eq!(addresses_for(&metas, &program), vec![shared, other]);
    }
}
