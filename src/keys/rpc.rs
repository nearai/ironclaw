//! Lightweight NEAR JSON-RPC client.
//!
//! Thin reqwest wrapper for the subset of NEAR RPC we need:
//! - view_access_key (nonce + block_hash for transaction building)
//! - send_transaction (submit signed transaction)
//! - tx_status (poll for result)
//! - view_account (check balance)

use serde::{Deserialize, Serialize};

use crate::keys::KeyError;
use crate::keys::types::NearNetwork;

/// NEAR RPC client.
#[derive(Debug, Clone)]
pub struct NearRpcClient {
    client: reqwest::Client,
    rpc_url: String,
}

impl NearRpcClient {
    pub fn new(network: &NearNetwork) -> Self {
        Self {
            client: reqwest::Client::new(),
            rpc_url: network.rpc_url().to_string(),
        }
    }

    pub fn with_url(url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            rpc_url: url.to_string(),
        }
    }

    /// Fetch access key info (nonce + block hash) for signing a transaction.
    pub async fn view_access_key(
        &self,
        account_id: &str,
        public_key: &str,
    ) -> Result<AccessKeyView, KeyError> {
        let response: RpcResponse<AccessKeyView> = self
            .call(
                "query",
                serde_json::json!({
                    "request_type": "view_access_key",
                    "finality": "final",
                    "account_id": account_id,
                    "public_key": public_key,
                }),
            )
            .await?;

        Ok(response.result)
    }

    /// Submit a signed transaction (fire and forget, returns tx hash).
    pub async fn send_transaction_async(&self, signed_tx_base64: &str) -> Result<String, KeyError> {
        let response: RpcResponse<serde_json::Value> = self
            .call("broadcast_tx_async", serde_json::json!([signed_tx_base64]))
            .await?;

        response
            .result
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| KeyError::RpcError {
                reason: "unexpected response from broadcast_tx_async".to_string(),
            })
    }

    /// Submit a signed transaction and wait for result.
    pub async fn send_transaction(&self, signed_tx_base64: &str) -> Result<TxOutcome, KeyError> {
        let response: RpcResponse<TxOutcome> = self
            .call("broadcast_tx_commit", serde_json::json!([signed_tx_base64]))
            .await?;

        Ok(response.result)
    }

    /// Check transaction status.
    pub async fn tx_status(&self, tx_hash: &str, sender_id: &str) -> Result<TxOutcome, KeyError> {
        let response: RpcResponse<TxOutcome> = self
            .call("tx", serde_json::json!([tx_hash, sender_id]))
            .await?;

        Ok(response.result)
    }

    /// View account information.
    pub async fn view_account(&self, account_id: &str) -> Result<AccountView, KeyError> {
        let response: RpcResponse<AccountView> = self
            .call(
                "query",
                serde_json::json!({
                    "request_type": "view_account",
                    "finality": "final",
                    "account_id": account_id,
                }),
            )
            .await?;

        Ok(response.result)
    }

    /// Make a JSON-RPC 2.0 call.
    async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<RpcResponse<T>, KeyError> {
        let request = RpcRequest {
            jsonrpc: "2.0",
            id: "ironclaw",
            method,
            params,
        };

        let response = self
            .client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(KeyError::RpcError {
                reason: format!("HTTP {}: {}", status, truncate(&body, 200)),
            });
        }

        let body = response.text().await?;
        let parsed: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| KeyError::RpcError {
                reason: format!("invalid JSON response: {}", e),
            })?;

        // Check for JSON-RPC error
        if let Some(error) = parsed.get("error") {
            let cause = error
                .get("cause")
                .and_then(|c| c.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");
            let message = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(KeyError::RpcError {
                reason: format!("{}: {}", cause, message),
            });
        }

        serde_json::from_value(parsed).map_err(|e| KeyError::RpcError {
            reason: format!("failed to parse RPC response: {}", e),
        })
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// JSON-RPC 2.0 request.
#[derive(Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'a str,
    id: &'a str,
    method: &'a str,
    params: serde_json::Value,
}

/// JSON-RPC 2.0 response.
#[derive(Deserialize)]
struct RpcResponse<T> {
    result: T,
}

/// Access key view from RPC.
#[derive(Debug, Clone, Deserialize)]
pub struct AccessKeyView {
    pub nonce: u64,
    pub block_hash: String,
    pub permission: serde_json::Value,
}

/// Transaction outcome from RPC.
#[derive(Debug, Clone, Deserialize)]
pub struct TxOutcome {
    pub status: serde_json::Value,
    pub transaction: Option<serde_json::Value>,
    pub transaction_outcome: Option<serde_json::Value>,
    pub receipts_outcome: Option<Vec<serde_json::Value>>,
}

impl TxOutcome {
    /// Check if the transaction succeeded.
    pub fn is_success(&self) -> bool {
        if let Some(obj) = self.status.as_object() {
            obj.contains_key("SuccessValue") || obj.contains_key("SuccessReceiptId")
        } else {
            false
        }
    }

    /// Get the failure reason if the transaction failed.
    pub fn failure_reason(&self) -> Option<String> {
        if let Some(obj) = self.status.as_object() {
            if let Some(failure) = obj.get("Failure") {
                return Some(format!("{}", failure));
            }
        }
        None
    }
}

/// Account view from RPC.
#[derive(Debug, Clone, Deserialize)]
pub struct AccountView {
    pub amount: String,
    pub locked: String,
    pub storage_usage: u64,
    pub code_hash: String,
    pub block_height: u64,
    pub block_hash: String,
}

impl AccountView {
    /// Parse the balance as u128 (yoctoNEAR).
    pub fn balance_yocto(&self) -> Result<u128, KeyError> {
        self.amount.parse::<u128>().map_err(|e| KeyError::RpcError {
            reason: format!("failed to parse account balance '{}': {}", self.amount, e),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::keys::rpc::{AccessKeyView, AccountView, TxOutcome};

    #[test]
    fn test_tx_outcome_success() {
        let outcome = TxOutcome {
            status: serde_json::json!({"SuccessValue": ""}),
            transaction: None,
            transaction_outcome: None,
            receipts_outcome: None,
        };
        assert!(outcome.is_success());
        assert!(outcome.failure_reason().is_none());
    }

    #[test]
    fn test_tx_outcome_failure() {
        let outcome = TxOutcome {
            status: serde_json::json!({"Failure": {"ActionError": "..."}}),
            transaction: None,
            transaction_outcome: None,
            receipts_outcome: None,
        };
        assert!(!outcome.is_success());
        assert!(outcome.failure_reason().is_some());
    }

    #[test]
    fn test_access_key_view_deserialize() {
        let json = serde_json::json!({
            "nonce": 42,
            "block_hash": "11111111111111111111111111111111",
            "permission": "FullAccess"
        });
        let view: AccessKeyView = serde_json::from_value(json).unwrap();
        assert_eq!(view.nonce, 42);
    }

    #[test]
    fn test_account_view_balance() {
        let view = AccountView {
            amount: "1000000000000000000000000".to_string(), // 1 NEAR
            locked: "0".to_string(),
            storage_usage: 100,
            code_hash: "11111111111111111111111111111111".to_string(),
            block_height: 1000,
            block_hash: "11111111111111111111111111111111".to_string(),
        };
        assert_eq!(
            view.balance_yocto().unwrap(),
            1_000_000_000_000_000_000_000_000
        );
    }
}
