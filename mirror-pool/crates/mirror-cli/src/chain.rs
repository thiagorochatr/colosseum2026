//! The few RPC calls a live run needs, over the same blocking client the
//! provenance collector uses.
//!
//! `solana-client` is deliberately not a dependency. It pulls
//! `solana-transaction-status-client-types`, which sits on the far side of the
//! ecosystem's in-progress `wincode` migration and cannot coexist with the
//! version litesvm requires. Rather than fight that resolution a second time,
//! this speaks JSON-RPC directly — four methods, all of which we already needed
//! to understand.

use anyhow::{anyhow, Result};
use base64::Engine;
use solana_program::pubkey::Pubkey;
use solana_transaction::Transaction;

pub struct Chain {
    endpoint: String,
    agent: ureq::Agent,
}

impl Chain {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Chain {
            endpoint: endpoint.into(),
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(60))
                .build(),
        }
    }

    /// One call, retrying through rate limits.
    ///
    /// The public endpoints throttle aggressively and a 429 is not a failure of
    /// the thing being measured — treating it as one would abandon a run that is
    /// otherwise fine. Backoff is exponential and bounded, and a persistent 429
    /// still surfaces as an error rather than as a silent default.
    fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": method, "params": params,
        });
        // Ten attempts with a bounded exponential backoff, which is roughly
        // four minutes of patience before giving up. Seven was not enough, and
        // the reason is specific rather than a matter of taste: rebuilding a
        // pool's accumulator means one `getTransaction` per signature, and the
        // public endpoints throttle that method far harder than they throttle a
        // balance read. A client that gives up during the burst leaves a member
        // unable to spend a note they own, which is the one failure this tool
        // must not have.
        let mut wait = std::time::Duration::from_millis(500);
        let mut last: Option<String> = None;
        let mut response: Option<serde_json::Value> = None;
        for _ in 0..10 {
            match self.agent.post(&self.endpoint).send_json(body.clone()) {
                Ok(r) => {
                    response = Some(r.into_json()?);
                    break;
                }
                Err(ureq::Error::Status(429, _)) => {
                    std::thread::sleep(wait);
                    // Capped, because doubling ten times is over eight minutes
                    // of sleeping for the last attempt alone, and a member
                    // watching a command hang learns nothing from it.
                    wait = (wait * 2).min(std::time::Duration::from_secs(30));
                    last = Some("rate limited".into());
                }
                Err(e) => return Err(anyhow!("{method}: {e}")),
            }
        }
        let response = response.ok_or_else(|| {
            anyhow!(
                "{method}: gave up after retries ({})",
                last.unwrap_or_default()
            )
        })?;
        if let Some(err) = response.get("error") {
            return Err(anyhow!("{method}: {err}"));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow!("{method}: no result"))
    }

    pub fn latest_blockhash(&self) -> Result<solana_program::hash::Hash> {
        let v = self.call(
            "getLatestBlockhash",
            serde_json::json!([{ "commitment": "confirmed" }]),
        )?;
        let s = v
            .pointer("/value/blockhash")
            .and_then(|b| b.as_str())
            .ok_or_else(|| anyhow!("blockhash missing"))?;
        s.parse().map_err(|e| anyhow!("bad blockhash: {e}"))
    }

    /// Sends and waits for confirmation, returning the signature.
    ///
    /// A failed simulation is surfaced with its logs rather than a bare error,
    /// because the program's own error code is the thing worth recording.
    pub fn send(&self, tx: &Transaction) -> Result<String> {
        let wire = bincode::serialize(tx)?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&wire);
        let sig = self.call(
            "sendTransaction",
            serde_json::json!([
                encoded,
                { "encoding": "base64", "preflightCommitment": "confirmed" }
            ]),
        )?;
        let sig = sig
            .as_str()
            .ok_or_else(|| anyhow!("signature missing"))?
            .to_string();
        self.confirm(&sig)?;
        Ok(sig)
    }

    /// Sends a versioned transaction, which is the only kind that can resolve
    /// accounts through a lookup table.
    ///
    /// Separate from `send` rather than generic over both, because the wire
    /// encodings differ and a legacy transaction silently serialized as
    /// versioned — or the reverse — produces a signature verification failure
    /// with nothing in it that points at the encoding.
    pub fn send_versioned(
        &self,
        tx: &solana_transaction::versioned::VersionedTransaction,
    ) -> Result<String> {
        let wire = bincode::serialize(tx)?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&wire);
        let sig = self.call(
            "sendTransaction",
            serde_json::json!([
                encoded,
                { "encoding": "base64", "preflightCommitment": "confirmed" }
            ]),
        )?;
        let sig = sig
            .as_str()
            .ok_or_else(|| anyhow!("signature missing"))?
            .to_string();
        self.confirm(&sig)?;
        Ok(sig)
    }

    /// A slot recent enough to derive a lookup table's address from, and settled
    /// enough that the address lookup table program will accept it.
    ///
    /// **Finalized, deliberately.** A table's address is seeded with a slot, and
    /// the program validates that slot against the `SlotHashes` sysvar — which
    /// holds rooted slots only. A *confirmed* slot is routinely ahead of what is
    /// rooted, so deriving from one produces a table address the program rejects
    /// with `is not a recent slot`, an error that names the right cause and
    /// sounds like the opposite of the problem: the slot was too new, not too
    /// old.
    pub fn recent_slot(&self) -> Result<u64> {
        let v = self.call(
            "getSlot",
            serde_json::json!([{ "commitment": "finalized" }]),
        )?;
        v.as_u64().ok_or_else(|| anyhow!("slot missing"))
    }

    fn confirm(&self, signature: &str) -> Result<()> {
        for _ in 0..40 {
            let v = self.call(
                "getSignatureStatuses",
                serde_json::json!([[signature], { "searchTransactionHistory": true }]),
            )?;
            if let Some(status) = v.pointer("/value/0") {
                if !status.is_null() {
                    if let Some(err) = status.get("err") {
                        if !err.is_null() {
                            return Err(anyhow!("transaction failed: {err}"));
                        }
                    }
                    let confirmed = status
                        .get("confirmationStatus")
                        .and_then(|c| c.as_str())
                        .unwrap_or("");
                    if confirmed == "confirmed" || confirmed == "finalized" {
                        return Ok(());
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1_200));
        }
        Err(anyhow!("timed out waiting for {signature}"))
    }

    pub fn account_data(&self, key: &Pubkey) -> Result<Option<Vec<u8>>> {
        let v = self.call(
            "getAccountInfo",
            // Commitment must be stated. The default is finalized, so a read
            // immediately after a confirmed write sees nothing and the caller
            // concludes the account was never created.
            serde_json::json!([
                key.to_string(),
                { "encoding": "base64", "commitment": "confirmed" }
            ]),
        )?;
        let value = v.get("value").unwrap_or(&serde_json::Value::Null);
        if value.is_null() {
            return Ok(None);
        }
        let b64 = value
            .pointer("/data/0")
            .and_then(|d| d.as_str())
            .ok_or_else(|| anyhow!("account data missing"))?;
        Ok(Some(base64::engine::general_purpose::STANDARD.decode(b64)?))
    }

    /// The log lines a landed transaction produced.
    ///
    /// Used to read what a *callee* said about a CPI, which is the only way to
    /// check a claim about the inner call from outside: the outer signature
    /// proves the transaction landed and says nothing about who signed the
    /// instruction the pool made inside it.
    pub fn transaction_logs(&self, signature: &str) -> Result<Vec<String>> {
        let v = self.call(
            "getTransaction",
            serde_json::json!([
                signature,
                { "commitment": "confirmed", "maxSupportedTransactionVersion": 0 }
            ]),
        )?;
        if v.is_null() {
            return Err(anyhow!(
                "getTransaction: {signature} is not visible yet on this endpoint"
            ));
        }
        let logs = v
            .pointer("/meta/logMessages")
            .and_then(|l| l.as_array())
            .ok_or_else(|| anyhow!("getTransaction: {signature} carries no log messages"))?;
        Ok(logs
            .iter()
            .filter_map(|l| l.as_str().map(str::to_owned))
            .collect())
    }

    /// Every signature that touched `address`, oldest first.
    ///
    /// Pages until the cluster runs out. The RPC returns newest first and pages
    /// backwards through `before`, so the reversal at the end is what turns this
    /// into an insertion order — and insertion order is the whole point, because
    /// a Merkle accumulator rebuilt in the wrong order produces a different root
    /// and no proof against it will ever verify.
    ///
    /// Failed transactions are dropped here rather than by the caller. They
    /// changed no state, so a rebuild that included them would insert leaves the
    /// chain never inserted.
    pub fn signatures_for_address(&self, address: &Pubkey) -> Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        let mut before: Option<String> = None;
        loop {
            let params = match &before {
                Some(b) => serde_json::json!([
                    address.to_string(),
                    { "limit": 1000, "before": b, "commitment": "confirmed" }
                ]),
                None => serde_json::json!([
                    address.to_string(),
                    { "limit": 1000, "commitment": "confirmed" }
                ]),
            };
            let page = self.call("getSignaturesForAddress", params)?;
            let entries = page
                .as_array()
                .ok_or_else(|| anyhow!("getSignaturesForAddress: not an array"))?;
            if entries.is_empty() {
                break;
            }
            for entry in entries {
                let sig = entry
                    .get("signature")
                    .and_then(|s| s.as_str())
                    .ok_or_else(|| anyhow!("getSignaturesForAddress: entry without signature"))?;
                before = Some(sig.to_string());
                if entry.get("err").map(|e| !e.is_null()).unwrap_or(false) {
                    continue;
                }
                out.push(sig.to_string());
            }
            if entries.len() < 1000 {
                break;
            }
        }
        out.reverse();
        Ok(out)
    }

    /// A landed transaction, decoded.
    ///
    /// Asked for as base64 and deserialised here rather than read out of the
    /// RPC's parsed JSON, because parsed instruction data arrives base58-encoded
    /// and decoding that would mean carrying an alphabet this crate otherwise
    /// has no use for.
    pub fn transaction(&self, signature: &str) -> Result<Option<Transaction>> {
        let v = self.call(
            "getTransaction",
            serde_json::json!([
                signature,
                {
                    "commitment": "confirmed",
                    "encoding": "base64",
                    "maxSupportedTransactionVersion": 0
                }
            ]),
        )?;
        if v.is_null() {
            return Ok(None);
        }
        let encoded = v
            .pointer("/transaction/0")
            .and_then(|t| t.as_str())
            .ok_or_else(|| anyhow!("getTransaction: {signature} carried no transaction"))?;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| anyhow!("getTransaction: {signature} is not base64: {e}"))?;
        // A versioned transaction will not deserialise into this shape. Treating
        // that as "not one of ours" is correct: this program is only ever called
        // from legacy transactions built by this CLI, and a versioned one that
        // happened to touch the pool carries nothing we need.
        Ok(bincode::deserialize::<Transaction>(&raw).ok())
    }

    pub fn balance(&self, key: &Pubkey) -> Result<u64> {
        let v = self.call(
            "getBalance",
            serde_json::json!([key.to_string(), { "commitment": "confirmed" }]),
        )?;
        v.pointer("/value")
            .and_then(|b| b.as_u64())
            .ok_or_else(|| anyhow!("balance missing"))
    }

    /// The rent-exempt floor for an account of `space` bytes.
    ///
    /// Asked of the cluster rather than computed here. The rent parameters are
    /// chain state, so a constant baked into this binary would be a second
    /// source of truth that is right until it is not — and the number is used
    /// to assert that a vault settled to its floor exactly, which is a claim
    /// worth grounding in what the cluster itself says.
    pub fn rent_exempt_minimum(&self, space: usize) -> Result<u64> {
        let v = self.call(
            "getMinimumBalanceForRentExemption",
            serde_json::json!([space, { "commitment": "confirmed" }]),
        )?;
        v.as_u64()
            .ok_or_else(|| anyhow!("rent-exempt minimum missing"))
    }

    /// Vote accounts of validators the cluster currently counts as active,
    /// ordered by stake, largest first.
    ///
    /// Only the `current` list is read. A delinquent validator's vote account
    /// still exists and `DelegateStake` would still accept it, so a run that
    /// drew from `delinquent` would succeed and prove the same thing — but the
    /// claim being made is about members choosing between *real* validators, and
    /// a reader checking the run against the cluster should find the ones they
    /// would have chosen from too.
    ///
    /// The order is the cluster's, made deterministic by sorting on stake and
    /// then on the key, so two runs against the same epoch pick the same
    /// validators and a reader can reproduce the selection instead of taking the
    /// list on trust.
    pub fn active_vote_accounts(&self) -> Result<Vec<Pubkey>> {
        let v = self.call(
            "getVoteAccounts",
            serde_json::json!([{ "commitment": "confirmed" }]),
        )?;
        let current = v
            .get("current")
            .and_then(|c| c.as_array())
            .ok_or_else(|| anyhow!("getVoteAccounts: no current validators"))?;
        let mut ranked: Vec<(u64, String)> = current
            .iter()
            .filter_map(|entry| {
                let key = entry.get("votePubkey")?.as_str()?.to_string();
                let stake = entry.get("activatedStake")?.as_u64()?;
                Some((stake, key))
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        ranked
            .into_iter()
            .map(|(_, key)| key.parse().map_err(|e| anyhow!("vote account {key}: {e}")))
            .collect()
    }

    /// What a landed transaction actually cost in compute.
    ///
    /// Read back from the cluster rather than simulated. A simulation runs
    /// against a different slot with different account states, and the number
    /// this is used for — how close a full settlement comes to the budget — is
    /// only interesting if it is the number the validator metered.
    pub fn compute_units(&self, signature: &str) -> Result<u64> {
        let v = self.call(
            "getTransaction",
            serde_json::json!([
                signature,
                { "commitment": "confirmed", "maxSupportedTransactionVersion": 0 }
            ]),
        )?;
        v.pointer("/meta/computeUnitsConsumed")
            .and_then(|c| c.as_u64())
            .ok_or_else(|| anyhow!("getTransaction: {signature} reports no compute units"))
    }

    /// The finalized slot a transaction landed in.
    ///
    /// Recorded beside every published signature because devnet history is
    /// pruned. A signature whose `getTransaction` has aged out returns null, and
    /// a reader checking the evidence months later cannot tell that from a
    /// signature that never existed. The slot is what makes the difference
    /// recoverable: `getSignatureStatuses --search-transaction-history` still
    /// answers for a pruned transaction, and the slot is what its answer is
    /// checked against.
    pub fn slot(&self, signature: &str) -> Result<u64> {
        let v = self.call(
            "getTransaction",
            serde_json::json!([
                signature,
                { "commitment": "confirmed", "maxSupportedTransactionVersion": 0 }
            ]),
        )?;
        v.pointer("/slot")
            .and_then(|s| s.as_u64())
            .ok_or_else(|| anyhow!("getTransaction: {signature} reports no slot"))
    }
}
