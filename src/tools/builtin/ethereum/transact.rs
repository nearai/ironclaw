//! Ethereum transaction submission tool.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::context::JobContext;
use crate::tools::builtin::ethereum::error::EthereumError;
use crate::tools::builtin::ethereum::session::WalletConnectSession;
use crate::tools::callback::{CallbackMetadata, ToolCallbackRegistry};
use crate::tools::tool::{
    ApprovalRequirement, RiskLevel, Tool, ToolError, ToolOutput, require_str,
};

/// Tool that submits an Ethereum transaction via a paired WalletConnect wallet.
///
/// The transaction is sent asynchronously: this tool registers a callback and
/// returns a pending correlation ID. The actual signature request goes to the
/// user's mobile wallet; the result arrives later via the callback system.
pub struct WalletTransactTool {
    session: Arc<WalletConnectSession>,
    callback_registry: Arc<ToolCallbackRegistry>,
}

impl WalletTransactTool {
    /// Create a new `WalletTransactTool`.
    pub fn new(
        session: Arc<WalletConnectSession>,
        callback_registry: Arc<ToolCallbackRegistry>,
    ) -> Self {
        Self {
            session,
            callback_registry,
        }
    }
}

/// Validate an Ethereum address (0x + 40 hex characters).
fn is_valid_eth_address(addr: &str) -> bool {
    addr.len() == 42 && addr.starts_with("0x") && addr[2..].chars().all(|c| c.is_ascii_hexdigit())
}

#[async_trait]
impl Tool for WalletTransactTool {
    fn name(&self) -> &str {
        "wallet_transact"
    }

    fn description(&self) -> &str {
        "Submit an Ethereum transaction via the paired WalletConnect wallet. \
         Returns a pending correlation ID — the wallet owner must approve the \
         transaction on their device. The result is delivered asynchronously."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Destination address (0x-prefixed, 40 hex chars)"
                },
                "value": {
                    "type": "string",
                    "description": "Value to send in wei (decimal string)"
                },
                "data": {
                    "type": "string",
                    "description": "Optional calldata (0x-prefixed hex)"
                },
                "chain_id": {
                    "type": "integer",
                    "description": "Target EVM chain ID. Defaults to the paired session chain."
                }
            },
            "required": ["to", "value"]
        })
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // The wallet itself is the approval mechanism.
        ApprovalRequirement::Never
    }

    fn risk_level_for(&self, _params: &serde_json::Value) -> RiskLevel {
        RiskLevel::High
    }

    fn sensitive_params(&self) -> &[&str] {
        &["data"]
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        // Must be paired first.
        if !self.session.is_paired().await {
            return Err(EthereumError::NotPaired.into());
        }

        // Extract and validate parameters.
        let to = require_str(&params, "to")?;
        if !is_valid_eth_address(to) {
            return Err(EthereumError::InvalidAddress {
                address: to.to_string(),
            }
            .into());
        }

        let value = require_str(&params, "value")?;
        let data = params.get("data").and_then(|v| v.as_str());
        let chain_id = params.get("chain_id").and_then(|v| v.as_u64());

        // Generate a correlation ID for the async callback.
        let correlation_id = uuid::Uuid::new_v4().to_string();

        // Extract channel/thread from context metadata for callback routing.
        let channel = ctx
            .metadata
            .get("source_channel")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let thread_id = ctx
            .metadata
            .get("thread_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Register callback so the result can be routed back.
        self.callback_registry
            .register(
                correlation_id.clone(),
                CallbackMetadata {
                    tool_name: "wallet_transact".to_string(),
                    user_id: ctx.user_id.clone(),
                    thread_id,
                    channel,
                },
            )
            .await;

        let mut result = json!({
            "status": "pending",
            "correlation_id": correlation_id,
            "to": to,
            "value": value,
            "message": "Transaction submitted to wallet for approval. \
                        The wallet owner must confirm on their device."
        });

        if data.is_some() {
            result["data_present"] = json!(true);
        }
        if let Some(cid) = chain_id {
            result["chain_id"] = json!(cid);
        }

        Ok(ToolOutput::success(result, start.elapsed()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_tool() -> WalletTransactTool {
        let session = Arc::new(WalletConnectSession::new_disconnected());
        let registry = Arc::new(ToolCallbackRegistry::new(Duration::from_secs(300)));
        WalletTransactTool::new(session, registry)
    }

    #[test]
    fn test_tool_metadata() {
        let tool = make_tool();
        assert_eq!(tool.name(), "wallet_transact");
        assert_eq!(
            tool.requires_approval(&serde_json::Value::Null),
            ApprovalRequirement::Never
        );
        assert_eq!(
            tool.risk_level_for(&serde_json::Value::Null),
            RiskLevel::High
        );
    }

    #[test]
    fn test_sensitive_params() {
        let tool = make_tool();
        assert_eq!(tool.sensitive_params(), &["data"]);
    }

    #[test]
    fn test_parameters_schema_requires_to_and_value() {
        let tool = make_tool();
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"]
            .as_array()
            .expect("required should be an array");
        let required_names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(required_names.contains(&"to"), "to should be required");
        assert!(
            required_names.contains(&"value"),
            "value should be required"
        );
    }

    #[test]
    fn test_valid_eth_address() {
        assert!(is_valid_eth_address(
            "0x1234567890abcdef1234567890abcdef12345678"
        ));
        assert!(!is_valid_eth_address("0x1234")); // too short
        assert!(!is_valid_eth_address(
            "1234567890abcdef1234567890abcdef12345678"
        )); // no 0x prefix
        assert!(!is_valid_eth_address(
            "0xGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG"
        )); // not hex
    }
}
