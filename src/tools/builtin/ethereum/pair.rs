//! WalletConnect pairing tool.

use std::sync::Arc;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::builtin::ethereum::session::WalletConnectSession;
use crate::tools::tool::{ApprovalRequirement, RiskLevel, Tool, ToolError, ToolOutput};

/// Tool that initiates or checks a WalletConnect pairing session.
pub struct WalletPairTool {
    session: Arc<WalletConnectSession>,
}

impl WalletPairTool {
    /// Create a new `WalletPairTool` with the given session.
    pub fn new(session: Arc<WalletConnectSession>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl Tool for WalletPairTool {
    fn name(&self) -> &str {
        "wallet_pair"
    }

    fn description(&self) -> &str {
        "Pair with an external Ethereum wallet via WalletConnect. \
         Returns the connected wallet address and chain ID, or initiates a new pairing session."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "chain_id": {
                    "type": "integer",
                    "description": "Target EVM chain ID (e.g. 1 for mainnet, 137 for Polygon). Defaults to 1."
                }
            },
            "required": []
        })
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }

    fn risk_level_for(&self, _params: &serde_json::Value) -> RiskLevel {
        RiskLevel::Low
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        // Check if already paired -- return address info directly.
        // Use active_address() directly to avoid TOCTOU race with is_paired().
        if let Some(address) = self.session.active_address().await {
            let chain_id = self.session.active_chain_id().await.unwrap_or(1);
            let content = format!(
                "Already paired with wallet {} on chain {}",
                address, chain_id
            );
            return Ok(ToolOutput::text(content, start.elapsed()));
        }

        let chain_id = params.get("chain_id").and_then(|v| v.as_u64()).unwrap_or(1);

        let uri = self.session.initiate_pairing(chain_id).await?;

        Ok(ToolOutput::text(
            format!("Pairing initiated. Present this WalletConnect URI to the user:\n{uri}"),
            start.elapsed(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool() -> WalletPairTool {
        let session = Arc::new(WalletConnectSession::new_disconnected());
        WalletPairTool::new(session)
    }

    #[test]
    fn test_tool_metadata() {
        let tool = make_tool();
        assert_eq!(tool.name(), "wallet_pair");
        assert_eq!(
            tool.requires_approval(&serde_json::Value::Null),
            ApprovalRequirement::Never
        );
        assert_eq!(
            tool.risk_level_for(&serde_json::Value::Null),
            RiskLevel::Low
        );
    }

    #[test]
    fn test_parameters_schema_is_valid() {
        let tool = make_tool();
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["chain_id"].is_object());
    }
}
