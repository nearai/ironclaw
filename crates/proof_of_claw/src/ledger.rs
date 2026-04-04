//! Ledger hardware approval gate — ERC-7730 clear-signing integration.
//!
//! High-value agent actions require physical Ledger approval via the
//! Ledger DMK/DSK device. This module manages the approval flow.

use anyhow::Result;

/// Gate that requests physical Ledger approval for high-value actions.
pub struct LedgerApprovalGate {
    origin_token: Option<String>,
}

impl LedgerApprovalGate {
    pub fn new(origin_token: Option<String>) -> Self {
        Self { origin_token }
    }

    /// Request Ledger approval for an action.
    ///
    /// In the current stub implementation, always returns `true`.
    /// Real implementation will:
    /// 1. Derive the action hash to present on the Ledger screen
    /// 2. Use `eth_signTypedData_v4` or Ledger's signer API
    /// 3. Verify the signature against the agent's Ledger-published origin token
    pub async fn request_approval(&self, action_description: &str, value_wei: u64) -> Result<bool> {
        tracing::info!(
            "Ledger approval requested: {} (value={value_wei} wei, token={:?})",
            action_description,
            self.origin_token.as_ref().map(|t| &t[..8.min(t.len())])
        );

        // TODO: Integrate with Ledger's EIP-712 signing flow
        // 1. Build EIP-712 domain from ORIGIN_TOKEN (ERC-7730 context)
        // 2. Serialize action parameters as EIP-712 typed data
        // 3. Prompt Ledger device to display and sign
        // 4. Verify signature matches origin token public key
        Ok(true)
    }

    /// Returns the origin token (for ERC-7730 clear-signing).
    pub fn origin_token(&self) -> Option<&str> {
        self.origin_token.as_deref()
    }
}
