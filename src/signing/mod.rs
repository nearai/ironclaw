//! Cryptographic signing of tool calls via signet-core.
//!
//! Provides tamper-evident audit logging: every tool call is signed with
//! Ed25519 and appended to a hash-chained JSONL audit log at `~/.signet/audit/`.

use std::collections::HashSet;
use std::path::PathBuf;

use ed25519_dalek::SigningKey;
use signet_core::audit;
use signet_core::{Action, Receipt};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SigningError {
    #[error("Failed to initialize signing key: {0}")]
    KeyInit(String),

    #[error("Failed to sign action: {0}")]
    SignFailed(String),

    #[error("Failed to append audit record: {0}")]
    AuditAppend(String),
}

/// Cryptographic signing service for tool calls.
///
/// Wraps signet-core to sign every dispatched tool call and append the
/// receipt to a hash-chained audit log. Keys and audit data live under
/// `~/.signet/` (configurable via `SIGNET_HOME`).
pub struct SigningService {
    signing_key: SigningKey,
    signet_dir: PathBuf,
    skip_tools: HashSet<String>,
}

impl SigningService {
    /// Load or auto-generate the "ironclaw" signing key.
    ///
    /// On first run, generates a new Ed25519 keypair and saves it to
    /// `~/.signet/keys/ironclaw.key` + `~/.signet/keys/ironclaw.pub`.
    pub fn init(skip_tools: HashSet<String>) -> Result<Self, SigningError> {
        let signet_dir = signet_core::default_signet_dir();
        let keys_dir = signet_dir.join("keys");

        // Ensure the keys directory exists
        std::fs::create_dir_all(&keys_dir)
            .map_err(|e| SigningError::KeyInit(format!("Failed to create keys dir: {e}")))?;

        let signing_key = match signet_core::load_signing_key(&keys_dir, "ironclaw", None) {
            Ok(key) => {
                tracing::debug!("Loaded existing signing key 'ironclaw'");
                key
            }
            Err(_) => {
                tracing::info!("No signing key found, generating new Ed25519 keypair");
                signet_core::generate_and_save(&keys_dir, "ironclaw", None, None, None)
                    .map_err(|e| SigningError::KeyInit(e.to_string()))?;

                // Load the key we just generated
                signet_core::load_signing_key(&keys_dir, "ironclaw", None)
                    .map_err(|e| SigningError::KeyInit(e.to_string()))?
            }
        };

        // Ensure audit directory exists
        let audit_dir = signet_dir.join("audit");
        std::fs::create_dir_all(&audit_dir)
            .map_err(|e| SigningError::AuditAppend(format!("Failed to create audit dir: {e}")))?;

        Ok(Self {
            signing_key,
            signet_dir,
            skip_tools,
        })
    }

    /// Sign a tool call and append to the audit chain.
    ///
    /// Returns `None` if the tool is in the skiplist.
    /// Signing failures are logged as warnings but never propagated —
    /// they must not block tool execution.
    pub fn sign_action(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
        output_summary: &str,
        success: bool,
        user_id: &str,
    ) -> Option<Receipt> {
        if self.skip_tools.contains(tool_name) {
            return None;
        }

        let action = Action {
            tool: tool_name.to_string(),
            params: input.clone(),
            params_hash: String::new(), // computed by sign()
            target: user_id.to_string(),
            transport: "dispatch".to_string(),
            session: None,
            call_id: None,
            response_hash: if success {
                None
            } else {
                // Embed failure indicator in response_hash for audit trail
                Some(format!("error:{}", truncate_safe(output_summary, 256)))
            },
            trace_id: None,
            parent_receipt_id: None,
        };

        let receipt = match signet_core::sign(&self.signing_key, &action, "ironclaw", user_id) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(tool = %tool_name, error = %e, "Failed to sign tool call");
                return None;
            }
        };

        // Append to hash-chained audit log
        let receipt_json = match serde_json::to_value(&receipt) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(tool = %tool_name, error = %e, "Failed to serialize receipt");
                return Some(receipt);
            }
        };

        let audit_dir = self.signet_dir.join("audit");
        if let Err(e) = audit::append(&audit_dir, &receipt_json) {
            tracing::warn!(tool = %tool_name, error = %e, "Failed to append audit record");
        }

        Some(receipt)
    }

    /// Verify the integrity of the audit chain.
    pub fn verify_chain(&self) -> Result<audit::ChainStatus, SigningError> {
        let audit_dir = self.signet_dir.join("audit");
        audit::verify_chain(&audit_dir).map_err(|e| SigningError::AuditAppend(e.to_string()))
    }
}

/// Truncate a string at a safe char boundary.
pub fn truncate_safe(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Walk backwards from max_bytes to find a char boundary
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_action_produces_receipt() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Override SIGNET_HOME for test isolation
        let _guard = crate::config::helpers::lock_env();
        // SAFETY: under ENV_MUTEX
        unsafe { std::env::set_var("SIGNET_HOME", dir.path().as_os_str()) };

        let service =
            SigningService::init(HashSet::new()).expect("should init with auto-generated key");
        let receipt = service.sign_action(
            "shell",
            &serde_json::json!({"command": "ls"}),
            "file list output",
            true,
            "user-1",
        );
        assert!(receipt.is_some(), "non-skipped tool should produce receipt");

        unsafe { std::env::remove_var("SIGNET_HOME") };
    }

    #[test]
    fn test_skiplist_excludes_tool() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = crate::config::helpers::lock_env();
        // SAFETY: under ENV_MUTEX
        unsafe { std::env::set_var("SIGNET_HOME", dir.path().as_os_str()) };

        let skip = HashSet::from(["echo".to_string()]);
        let service = SigningService::init(skip).expect("should init");
        let receipt = service.sign_action(
            "echo",
            &serde_json::json!({"text": "hello"}),
            "hello",
            true,
            "user-1",
        );
        assert!(receipt.is_none(), "skipped tool should return None");

        // Non-skipped tool should still produce receipt
        let receipt = service.sign_action(
            "shell",
            &serde_json::json!({"command": "ls"}),
            "output",
            true,
            "user-1",
        );
        assert!(receipt.is_some(), "non-skipped tool should produce receipt");

        unsafe { std::env::remove_var("SIGNET_HOME") };
    }

    #[test]
    fn test_chain_integrity_after_multiple_signs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = crate::config::helpers::lock_env();
        // SAFETY: under ENV_MUTEX
        unsafe { std::env::set_var("SIGNET_HOME", dir.path().as_os_str()) };

        let service = SigningService::init(HashSet::new()).expect("should init");
        for i in 0..5 {
            service.sign_action(
                "shell",
                &serde_json::json!({"command": format!("cmd-{i}")}),
                "ok",
                true,
                "user-1",
            );
        }
        let status = service
            .verify_chain()
            .expect("chain verification should succeed");
        assert!(status.valid, "chain should be valid after sequential signs");
        assert_eq!(status.total_records, 5, "should have 5 audit records");

        unsafe { std::env::remove_var("SIGNET_HOME") };
    }

    #[test]
    fn test_failed_action_records_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = crate::config::helpers::lock_env();
        // SAFETY: under ENV_MUTEX
        unsafe { std::env::set_var("SIGNET_HOME", dir.path().as_os_str()) };

        let service = SigningService::init(HashSet::new()).expect("should init");
        let receipt = service.sign_action(
            "http_fetch",
            &serde_json::json!({"url": "https://example.com"}),
            "connection refused",
            false,
            "user-1",
        );
        assert!(receipt.is_some(), "failed actions should still be signed");

        unsafe { std::env::remove_var("SIGNET_HOME") };
    }

    #[test]
    fn test_truncate_safe_ascii() {
        assert_eq!(truncate_safe("hello", 10), "hello");
        assert_eq!(truncate_safe("hello", 3), "hel");
    }

    #[test]
    fn test_truncate_safe_multibyte() {
        // "你好世界" = 12 bytes (3 bytes per char)
        let s = "你好世界";
        assert_eq!(truncate_safe(s, 6), "你好");
        assert_eq!(truncate_safe(s, 7), "你好"); // mid-char boundary, truncates back
        assert_eq!(truncate_safe(s, 100), s);
    }
}
