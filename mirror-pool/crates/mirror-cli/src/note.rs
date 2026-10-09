//! A member's note, on disk.
//!
//! The secret is two field elements. Losing them loses the deposit — there is no
//! authority that can reissue a note, which is the same property that means no
//! authority can freeze one. So the file is deliberately boring: plain JSON, one
//! note per file, readable by anything, with the commitment stored alongside so
//! a member can check what they hold without a prover.
//!
//! It is not encrypted. Encrypting it would put a password between a member and
//! their funds and imply a security property this crate cannot deliver on its
//! own — the file is as sensitive as a keypair and wants the same handling.

use anyhow::{anyhow, Context, Result};
use mirror_core::{Field, Note};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize)]
pub struct StoredNote {
    /// The pool this note belongs to. A note is only spendable in the pool whose
    /// denomination it was committed at.
    pub denomination: u64,
    /// Nullifier preimage, 32 bytes big-endian hex.
    pub k: String,
    /// Blinding factor, 32 bytes big-endian hex.
    pub r: String,
    /// Stored so a member can verify a deposit landed without running a prover.
    pub commitment: String,
}

impl StoredNote {
    pub fn from_note(note: &Note, denomination: u64) -> Result<Self> {
        Ok(StoredNote {
            denomination,
            k: hex::encode(note.k.to_bytes()),
            r: hex::encode(note.r.to_bytes()),
            commitment: hex::encode(note.commitment().map_err(|e| anyhow!("{e:?}"))?.to_bytes()),
        })
    }

    pub fn note(&self) -> Result<Note> {
        let k = field_from_hex(&self.k, "k")?;
        let r = field_from_hex(&self.r, "r")?;
        let note = Note::new(k, r, Field::from_u64(self.denomination));
        // A file whose commitment disagrees with its secrets has been edited or
        // corrupted, and spending it would burn a nullifier for a leaf that is
        // not in the tree. Caught here, where it is still only a bad file.
        let recomputed = hex::encode(note.commitment().map_err(|e| anyhow!("{e:?}"))?.to_bytes());
        if recomputed != self.commitment.to_lowercase() {
            return Err(anyhow!(
                "this note file is inconsistent: its secrets commit to {recomputed}, \
                 but the file records {}",
                self.commitment
            ));
        }
        Ok(note)
    }

    pub fn commitment_bytes(&self) -> Result<[u8; 32]> {
        let raw = hex::decode(&self.commitment).context("the commitment is not hex")?;
        raw.try_into()
            .map_err(|_| anyhow!("the commitment is not 32 bytes"))
    }

    pub fn read(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading the note at {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Writes the note, refusing to overwrite.
    ///
    /// An overwrite here destroys a deposit, so the safe default is the only
    /// default: there is no `--force`.
    pub fn write(&self, path: &Path) -> Result<()> {
        if path.exists() {
            return Err(anyhow!(
                "{} already exists. A note file is the only copy of a deposit's \
                 secret, so this will not overwrite one — choose another path.",
                path.display()
            ));
        }
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text)
            .with_context(|| format!("writing the note to {}", path.display()))?;
        Ok(())
    }
}

fn field_from_hex(s: &str, what: &str) -> Result<Field> {
    let raw = hex::decode(s).with_context(|| format!("{what} is not hex"))?;
    let arr: [u8; 32] = raw
        .try_into()
        .map_err(|_| anyhow!("{what} is not 32 bytes"))?;
    Field::from_bytes(arr).map_err(|e| anyhow!("{what} is not a canonical field element: {e:?}"))
}

/// Draws a fresh note from a CSPRNG seeded by the operating system.
///
/// Rejection sampling rather than reduction: a reduced value would make some
/// field elements twice as likely as others, and while that is not exploitable
/// at this size it is free to avoid. A draw at or above the modulus is redrawn.
pub fn generate(denomination: u64) -> Result<Note> {
    use ark_std::rand::{RngCore, SeedableRng};
    let mut rng = ark_std::rand::rngs::StdRng::from_entropy();
    let mut draw = || -> Field {
        loop {
            let mut bytes = [0u8; 32];
            rng.fill_bytes(&mut bytes);
            // Clearing the top byte would bias the distribution; redrawing does
            // not, and the expected number of redraws is under one in ten.
            if let Ok(f) = Field::from_bytes(bytes) {
                return f;
            }
        }
    };
    Ok(Note::new(draw(), draw(), Field::from_u64(denomination)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DENOM: u64 = 20_000_019;

    #[test]
    fn a_generated_note_round_trips_through_its_file_form() {
        let note = generate(DENOM).unwrap();
        let stored = StoredNote::from_note(&note, DENOM).unwrap();
        let text = serde_json::to_string(&stored).unwrap();
        let read: StoredNote = serde_json::from_str(&text).unwrap();
        assert_eq!(read.note().unwrap(), note);
    }

    /// A note file whose commitment no longer matches its secrets must not be
    /// spendable. Spending it would burn a nullifier against a leaf the tree
    /// does not hold, which costs the member the deposit and tells them nothing
    /// about why.
    #[test]
    fn a_note_whose_commitment_disagrees_with_its_secrets_is_refused() {
        let note = generate(DENOM).unwrap();
        let mut stored = StoredNote::from_note(&note, DENOM).unwrap();
        stored.commitment = hex::encode([0u8; 32]);
        let err = stored.note().unwrap_err().to_string();
        assert!(err.contains("inconsistent"), "{err}");
    }

    /// The denomination is part of the commitment, so a file that claims the
    /// wrong pool cannot silently be spent in it.
    #[test]
    fn changing_the_denomination_invalidates_the_note() {
        let note = generate(DENOM).unwrap();
        let mut stored = StoredNote::from_note(&note, DENOM).unwrap();
        stored.denomination = DENOM + 1;
        assert!(stored.note().is_err());
    }

    #[test]
    fn a_non_canonical_secret_is_refused_rather_than_reduced() {
        let note = generate(DENOM).unwrap();
        let mut stored = StoredNote::from_note(&note, DENOM).unwrap();
        stored.k = hex::encode([0xffu8; 32]);
        assert!(stored.note().is_err());
    }

    /// Writing over an existing note destroys a deposit, so there is no path
    /// that does it — not even an explicit one.
    #[test]
    fn writing_refuses_to_overwrite_an_existing_note() {
        let dir = std::env::temp_dir().join(format!("mirror-note-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.json");
        let _ = std::fs::remove_file(&path);

        let first = StoredNote::from_note(&generate(DENOM).unwrap(), DENOM).unwrap();
        first.write(&path).unwrap();
        let second = StoredNote::from_note(&generate(DENOM).unwrap(), DENOM).unwrap();
        let err = second.write(&path).unwrap_err().to_string();
        assert!(err.contains("already exists"), "{err}");

        // The file on disk is still the first note, not the second.
        assert_eq!(
            StoredNote::read(&path).unwrap().commitment,
            first.commitment
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn two_draws_differ() {
        let a = generate(DENOM).unwrap();
        let b = generate(DENOM).unwrap();
        assert_ne!(a.k, b.k);
        assert_ne!(a.r, b.r);
    }
}
