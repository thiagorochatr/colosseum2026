//! A Solana JSON-RPC client that fails loudly.
//!
//! Two properties matter here more than throughput.
//!
//! **Every error propagates.** There is no `unwrap_or(0)` and no
//! `unwrap_or_default()` anywhere in this module. A throttled call returns an
//! error that becomes `Unresolved::RpcFailure`, which the census counts on its
//! own line and excludes from the class distribution. The alternative — a
//! default value — turns a rate limit into the claim "this address has no
//! funding history", which is the single easiest way for a provenance
//! measurement to be wrong in the direction that flatters it.
//!
//! **The endpoint is checked before the run.** Third-party "public" endpoints
//! silently truncate history: one reports a first available block 2.5 days
//! behind the tip and returns `{"result": null}` — not an error — for a 2021
//! transaction. A collector pointed at it records "not found" for every old
//! funding event and produces a truncated graph that looks exactly like a
//! finding. [`RpcClient::check_preconditions`] refuses to run against one.

use crate::edge::TransactionView;
use serde::Deserialize;
use std::time::{Duration, Instant};

/// A slot old enough that only an archival endpoint can serve it.
///
/// Slot 50,000,000 was produced on 2020-11-19. An endpoint that cannot return
/// it cannot answer questions about where anybody's money came from.
pub const ARCHIVAL_PROBE_SLOT: u64 = 50_000_000;

#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error("transport failure: {0}")]
    Transport(String),
    #[error("rpc returned an error: {0}")]
    Rpc(String),
    #[error("unexpected response shape: {0}")]
    Shape(String),
    #[error("endpoint is not archival: {0}")]
    NotArchival(String),
}

/// What the endpoint claimed about itself, recorded in the run manifest so a
/// reader can see which endpoint produced the numbers and that it passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointCheck {
    pub endpoint: String,
    pub first_available_block: u64,
    pub archival_probe_ok: bool,
}

pub struct RpcClient {
    endpoint: String,
    agent: ureq::Agent,
    /// Minimum spacing between calls.
    ///
    /// The public endpoint sustains a measured 0.28–0.55 requests per second,
    /// far below its documented allowance; the documented figure is not
    /// reproducible. Pacing is therefore set from measurement, and the measured
    /// range is what gets reported rather than the advertised one.
    min_interval: Duration,
    last_call: Option<Instant>,
    calls: u64,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    result: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
}

/// One entry from `getSignaturesForAddress`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureInfo {
    pub signature: String,
    pub slot: u64,
    pub block_time: Option<i64>,
    pub err: bool,
}

/// The fields of `getAccountInfo` the program/PDA rule needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountOwner {
    pub owner: String,
    pub executable: bool,
}

/// The system program. An account owned by anything else is a program account
/// or a PDA, never a person's wallet.
pub const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";

impl RpcClient {
    pub fn new(endpoint: impl Into<String>, requests_per_second: f64) -> Self {
        let interval = if requests_per_second > 0.0 {
            Duration::from_secs_f64(1.0 / requests_per_second)
        } else {
            Duration::ZERO
        };
        RpcClient {
            endpoint: endpoint.into(),
            agent: ureq::AgentBuilder::new()
                . // A full signature page is a large response, and the slowest ones on a
              // busy address exceeded thirty seconds — which arrived as a
              // transport failure and was counted against the run rather than
              // retried.
              timeout(Duration::from_secs(90))
                .build(),
            min_interval: interval,
            last_call: None,
            calls: 0,
        }
    }

    pub fn calls_made(&self) -> u64 {
        self.calls
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn pace(&mut self) {
        if let Some(last) = self.last_call {
            let elapsed = last.elapsed();
            if elapsed < self.min_interval {
                std::thread::sleep(self.min_interval - elapsed);
            }
        }
        self.last_call = Some(Instant::now());
    }

    /// One call, retrying through rate limits.
    ///
    /// A 429 is not evidence about the chain, so abandoning a trace on the first
    /// one converts throttling into an `RpcFailure` — and enough of those push
    /// the run past the threshold where it refuses to publish at all. Which is
    /// what happened on the first real run of this collector: the chain client
    /// had backoff and this one did not, and the analysis correctly refused a
    /// headline computed from our own throttling.
    ///
    /// Persistent throttling still surfaces as an error. The point is not to
    /// hide it, only to distinguish a transient limit from a real one.
    fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.calls + 1,
            "method": method,
            "params": params,
        });

        // Providers meter by compute units, not request count, and a full
        // signature page is expensive in those terms — so a 429 arrives even at
        // well under one request per second, and the budget needs seconds to
        // refill rather than milliseconds.
        let mut wait = std::time::Duration::from_secs(2);
        let mut response: Option<JsonRpcResponse> = None;
        for attempt in 0..8 {
            self.pace();
            self.calls += 1;
            match self.agent.post(&self.endpoint).send_json(body.clone()) {
                Ok(r) => {
                    response = Some(r.into_json().map_err(|e| RpcError::Shape(e.to_string()))?);
                    break;
                }
                Err(ureq::Error::Status(429 | 502 | 503, _)) if attempt < 7 => {
                    std::thread::sleep(wait);
                    wait *= 2;
                }
                Err(e) => return Err(RpcError::Transport(e.to_string())),
            }
        }
        let response = response
            .ok_or_else(|| RpcError::Transport(format!("{method}: throttled past retries")))?;

        if let Some(err) = response.error {
            return Err(RpcError::Rpc(err.to_string()));
        }
        response
            .result
            .ok_or_else(|| RpcError::Shape(format!("{method}: neither result nor error")))
    }

    /// Refuses to proceed against an endpoint that cannot see the whole chain.
    ///
    /// Both probes are hard. A truncated endpoint does not error on old data, it
    /// returns nothing, so without these the run would look healthy and be
    /// systematically wrong.
    pub fn check_preconditions(&mut self) -> Result<EndpointCheck, RpcError> {
        let first = self.call("getFirstAvailableBlock", serde_json::json!([]))?;
        let first_available_block = first
            .as_u64()
            .ok_or_else(|| RpcError::Shape("getFirstAvailableBlock: not a number".into()))?;
        if first_available_block != 0 {
            return Err(RpcError::NotArchival(format!(
                "first available block is {first_available_block}, not 0; this endpoint \
                 retains only recent history and would report old funding events as absent"
            )));
        }

        let block = self.call(
            "getBlock",
            serde_json::json!([
                ARCHIVAL_PROBE_SLOT,
                { "encoding": "json", "transactionDetails": "none", "rewards": false,
                  "maxSupportedTransactionVersion": 0 }
            ]),
        );
        let archival_probe_ok = matches!(&block, Ok(v) if !v.is_null());
        if !archival_probe_ok {
            return Err(RpcError::NotArchival(format!(
                "slot {ARCHIVAL_PROBE_SLOT} is unavailable; the endpoint cannot serve \
                 the history this measurement depends on"
            )));
        }

        Ok(EndpointCheck {
            endpoint: self.endpoint.clone(),
            first_available_block,
            archival_probe_ok,
        })
    }

    /// One page of signatures, newest first. `before` pages backwards.
    ///
    /// There is no forward cursor and `limit` caps at 1,000, so for an address
    /// with fewer than 1,000 lifetime signatures the **last element of the first
    /// page is the birth transaction**. That is what the birth-edge rule reads.
    pub fn signatures_for_address(
        &mut self,
        address: &str,
        before: Option<&str>,
        limit: u32,
    ) -> Result<Vec<SignatureInfo>, RpcError> {
        let mut config = serde_json::json!({ "limit": limit });
        if let Some(b) = before {
            config["before"] = serde_json::Value::String(b.to_string());
        }
        let result = self.call(
            "getSignaturesForAddress",
            serde_json::json!([address, config]),
        )?;
        let array = result
            .as_array()
            .ok_or_else(|| RpcError::Shape("getSignaturesForAddress: not an array".into()))?;

        array
            .iter()
            .map(|v| {
                Ok(SignatureInfo {
                    signature: v
                        .get("signature")
                        .and_then(|s| s.as_str())
                        .ok_or_else(|| RpcError::Shape("signature missing".into()))?
                        .to_string(),
                    slot: v
                        .get("slot")
                        .and_then(|s| s.as_u64())
                        .ok_or_else(|| RpcError::Shape("slot missing".into()))?,
                    block_time: v.get("blockTime").and_then(|s| s.as_i64()),
                    err: v.get("err").map(|e| !e.is_null()).unwrap_or(false),
                })
            })
            .collect()
    }

    /// The narrow projection of a transaction the edge extractor needs.
    ///
    /// A `null` result is an error here, not an empty view. Against a truncated
    /// endpoint `null` is what every old transaction returns, and treating it as
    /// "no such transaction" is precisely the failure this module exists to
    /// prevent.
    pub fn transaction(&mut self, signature: &str) -> Result<TransactionView, RpcError> {
        let result = self.call(
            "getTransaction",
            serde_json::json!([
                signature,
                { "encoding": "jsonParsed", "maxSupportedTransactionVersion": 0 }
            ]),
        )?;
        if result.is_null() {
            return Err(RpcError::Rpc(format!(
                "getTransaction({signature}) returned null; on an archival endpoint this \
                 means the signature does not exist, but on a truncated one it means the \
                 history was pruned — which is why preconditions are checked first"
            )));
        }
        parse_transaction(signature, &result)
    }

    /// Owner and executability, for the program/PDA terminal rule.
    pub fn account_owner(&mut self, address: &str) -> Result<Option<AccountOwner>, RpcError> {
        let result = self.call(
            "getAccountInfo",
            serde_json::json!([address, { "encoding": "base64" }]),
        )?;
        let value = result.get("value").unwrap_or(&serde_json::Value::Null);
        if value.is_null() {
            // A genuinely absent account. Distinct from a failure, and the
            // caller treats it as such.
            return Ok(None);
        }
        Ok(Some(AccountOwner {
            owner: value
                .get("owner")
                .and_then(|o| o.as_str())
                .ok_or_else(|| RpcError::Shape("account owner missing".into()))?
                .to_string(),
            executable: value
                .get("executable")
                .and_then(|e| e.as_bool())
                .unwrap_or(false),
        }))
    }
}

/// Projects a `getTransaction` response into the fields the edge extractor uses.
///
/// Split out so it can be tested against a committed fixture without a network.
pub fn parse_transaction(
    signature: &str,
    value: &serde_json::Value,
) -> Result<TransactionView, RpcError> {
    let meta = value
        .get("meta")
        .ok_or_else(|| RpcError::Shape("meta missing".into()))?;

    // Under `transactionDetails: "accounts"` the path is
    // `transaction.accountKeys`; under `"full"` it is
    // `transaction.message.accountKeys`. Accept either rather than depending on
    // a request parameter staying in sync with a parser.
    let keys_value = value
        .pointer("/transaction/message/accountKeys")
        .or_else(|| value.pointer("/transaction/accountKeys"))
        .ok_or_else(|| RpcError::Shape("accountKeys missing".into()))?;
    let account_keys: Vec<String> = keys_value
        .as_array()
        .ok_or_else(|| RpcError::Shape("accountKeys is not an array".into()))?
        .iter()
        .map(|k| {
            // jsonParsed renders each key as an object; base64/json as a string.
            k.get("pubkey")
                .and_then(|p| p.as_str())
                .or_else(|| k.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| RpcError::Shape("account key is neither object nor string".into()))
        })
        .collect::<Result<_, _>>()?;

    let numbers = |field: &str| -> Result<Vec<u64>, RpcError> {
        meta.get(field)
            .and_then(|v| v.as_array())
            .ok_or_else(|| RpcError::Shape(format!("{field} missing")))?
            .iter()
            .map(|v| {
                v.as_u64()
                    .ok_or_else(|| RpcError::Shape(format!("{field} entry is not a number")))
            })
            .collect()
    };

    Ok(TransactionView {
        signature: signature.to_string(),
        slot: value
            .get("slot")
            .and_then(|s| s.as_u64())
            .ok_or_else(|| RpcError::Shape("slot missing".into()))?,
        block_time: value.get("blockTime").and_then(|t| t.as_i64()),
        account_keys,
        pre_balances: numbers("preBalances")?,
        post_balances: numbers("postBalances")?,
        fee: meta
            .get("fee")
            .and_then(|f| f.as_u64())
            .ok_or_else(|| RpcError::Shape("fee missing".into()))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `jsonParsed` response whose keys are objects, which is the shape the
    /// collector actually receives.
    fn json_parsed_fixture() -> serde_json::Value {
        serde_json::json!({
            "slot": 250_000_000u64,
            "blockTime": 1_700_000_000i64,
            "transaction": {
                "message": {
                    "accountKeys": [
                        { "pubkey": "Payer1111111111111111111111111111111111111", "signer": true, "writable": true, "source": "transaction" },
                        { "pubkey": "Recip111111111111111111111111111111111111", "signer": false, "writable": true, "source": "transaction" },
                        { "pubkey": "Lut11111111111111111111111111111111111111", "signer": false, "writable": true, "source": "lookupTable" }
                    ]
                }
            },
            "meta": {
                "fee": 5000u64,
                "preBalances": [1_000_000u64, 0u64, 7u64],
                "postBalances": [ 895_000u64, 100_000u64, 7u64]
            }
        })
    }

    #[test]
    fn a_json_parsed_transaction_projects_correctly() {
        let v = json_parsed_fixture();
        let t = parse_transaction("sig123", &v).unwrap();
        assert_eq!(t.account_keys.len(), 3);
        assert_eq!(t.pre_balances.len(), 3);
        assert!(t.is_well_formed());
        assert_eq!(t.fee, 5000);
        assert_eq!(t.slot, 250_000_000);

        let edge = t
            .edge_crediting("Recip111111111111111111111111111111111111")
            .expect("recipient credited");
        assert_eq!(edge.value, 100_000);
        assert_eq!(
            edge.sources,
            vec!["Payer1111111111111111111111111111111111111"]
        );
    }

    /// Lookup-table addresses arrive folded into `accountKeys`, and the balance
    /// arrays align with the full resolved list. If that ever stopped being
    /// true, deltas would be attributed to the wrong accounts.
    #[test]
    fn lookup_table_keys_are_included_and_balances_align() {
        let t = parse_transaction("sig123", &json_parsed_fixture()).unwrap();
        assert!(
            t.account_keys.iter().any(|k| k.starts_with("Lut1")),
            "lookup-table key was dropped"
        );
        assert_eq!(t.account_keys.len(), t.post_balances.len());
    }

    #[test]
    fn the_flat_account_keys_shape_is_also_accepted() {
        let v = serde_json::json!({
            "slot": 1u64,
            "transaction": { "accountKeys": ["aaa", "bbb"] },
            "meta": { "fee": 0u64, "preBalances": [10u64, 0u64], "postBalances": [5u64, 5u64] }
        });
        let t = parse_transaction("s", &v).unwrap();
        assert_eq!(t.account_keys, vec!["aaa", "bbb"]);
    }

    #[test]
    fn a_malformed_response_is_an_error_not_a_default() {
        // Every one of these would become a silent zero under unwrap_or.
        let no_meta = serde_json::json!({ "slot": 1u64, "transaction": { "accountKeys": ["a"] } });
        assert!(parse_transaction("s", &no_meta).is_err());

        let no_balances = serde_json::json!({
            "slot": 1u64,
            "transaction": { "accountKeys": ["a"] },
            "meta": { "fee": 0u64 }
        });
        assert!(parse_transaction("s", &no_balances).is_err());

        let no_slot = serde_json::json!({
            "transaction": { "accountKeys": ["a"] },
            "meta": { "fee": 0u64, "preBalances": [1u64], "postBalances": [1u64] }
        });
        assert!(parse_transaction("s", &no_slot).is_err());
    }

    #[test]
    fn the_system_program_constant_is_right() {
        assert_eq!(SYSTEM_PROGRAM.len(), 32);
        assert!(SYSTEM_PROGRAM.chars().all(|c| c == '1'));
    }

    #[test]
    fn pacing_is_derived_from_the_measured_rate() {
        let slow = RpcClient::new("http://example.invalid", 0.4);
        assert!(slow.min_interval >= Duration::from_millis(2_400));
        let fast = RpcClient::new("http://example.invalid", 10.0);
        assert!(fast.min_interval <= Duration::from_millis(100));
    }
}
