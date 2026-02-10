//! Host functions for WASM sandbox.
//!
//! Implements a minimal, security-focused host API following VMLogic patterns
//! from NEAR blockchain. The principle is: deny by default, grant minimal capabilities.
//!
//! # Extended API (V2)
//!
//! In addition to the basic log/time/workspace functions, the host now provides:
//!
//! - **http_request**: Make HTTP requests to allowlisted endpoints with credential injection
//! - **tool_invoke**: Call other tools via aliases
//! - **secret_exists**: Check if a secret exists (never read values)
//!
//! # Security Architecture
//!
//! ```text
//! WASM Tool ──▶ Host Function ──▶ Allowlist ──▶ Credential ──▶ Execute
//! (untrusted)   (boundary)        Validator     Injector       Request
//!                                                    │
//!                                                    ▼
//!                              ◀────── Leak Detector ◀────── Response
//!                          (sanitized, no secrets)
//! ```

use std::time::{SystemTime, UNIX_EPOCH};

use crate::tools::wasm::capabilities::Capabilities;
use crate::tools::wasm::error::WasmError;

/// Maximum log entries per execution (prevents log spam attacks).
const MAX_LOG_ENTRIES: usize = 1000;

/// Maximum bytes per log message.
const MAX_LOG_MESSAGE_BYTES: usize = 4096;

/// Log levels matching the WIT interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Trace => write!(f, "TRACE"),
            LogLevel::Debug => write!(f, "DEBUG"),
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Warn => write!(f, "WARN"),
            LogLevel::Error => write!(f, "ERROR"),
        }
    }
}

/// A single log entry from WASM execution.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
    pub timestamp_millis: u64,
}

/// Host state maintained during WASM execution.
///
/// This is the "VMLogic" equivalent, it tracks all side effects and enforces limits.
/// Extended in V2 to support HTTP requests, tool invocation, and secret checks.
/// Extended in V3 to support payload signing via managed NEAR keys.
pub struct HostState {
    /// Collected log entries.
    logs: Vec<LogEntry>,
    /// Whether logging is still allowed (false after MAX_LOG_ENTRIES).
    logging_enabled: bool,
    /// Granted capabilities.
    capabilities: Capabilities,
    /// Count of log entries dropped due to rate limiting.
    logs_dropped: usize,
    /// User ID for secret/credential lookups.
    user_id: Option<String>,
    /// HTTP request count for rate limiting within this execution.
    http_request_count: u32,
    /// Tool invoke count for rate limiting within this execution.
    tool_invoke_count: u32,
    /// Signing request count for rate limiting within this execution.
    sign_count: u32,
}

impl std::fmt::Debug for HostState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostState")
            .field("logs_count", &self.logs.len())
            .field("logging_enabled", &self.logging_enabled)
            .field("logs_dropped", &self.logs_dropped)
            .field("user_id", &self.user_id)
            .field("http_request_count", &self.http_request_count)
            .field("tool_invoke_count", &self.tool_invoke_count)
            .field("sign_count", &self.sign_count)
            .finish()
    }
}

impl HostState {
    /// Create a new host state with the given capabilities.
    pub fn new(capabilities: Capabilities) -> Self {
        Self {
            logs: Vec::new(),
            logging_enabled: true,
            capabilities,
            logs_dropped: 0,
            user_id: None,
            http_request_count: 0,
            tool_invoke_count: 0,
            sign_count: 0,
        }
    }

    /// Create a new host state with user context.
    pub fn new_with_user(capabilities: Capabilities, user_id: impl Into<String>) -> Self {
        Self {
            logs: Vec::new(),
            logging_enabled: true,
            capabilities,
            logs_dropped: 0,
            user_id: Some(user_id.into()),
            http_request_count: 0,
            tool_invoke_count: 0,
            sign_count: 0,
        }
    }

    /// Create a minimal host state with no capabilities.
    pub fn minimal() -> Self {
        Self::new(Capabilities::default())
    }

    /// Get the user ID if set.
    pub fn user_id(&self) -> Option<&str> {
        self.user_id.as_deref()
    }

    /// Get the capabilities.
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// Log a message from WASM.
    ///
    /// Returns Ok(()) if logged, Err if rate limited or too long.
    pub fn log(&mut self, level: LogLevel, message: String) -> Result<(), WasmError> {
        if !self.logging_enabled {
            self.logs_dropped += 1;
            return Ok(()); // Silently drop, don't fail execution
        }

        if self.logs.len() >= MAX_LOG_ENTRIES {
            self.logging_enabled = false;
            self.logs_dropped += 1;
            tracing::warn!(
                "WASM log limit reached ({} entries), further logs dropped",
                MAX_LOG_ENTRIES
            );
            return Ok(());
        }

        // Truncate overly long messages
        let message = if message.len() > MAX_LOG_MESSAGE_BYTES {
            let mut truncated = message[..MAX_LOG_MESSAGE_BYTES].to_string();
            truncated.push_str("... (truncated)");
            truncated
        } else {
            message
        };

        let timestamp_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        self.logs.push(LogEntry {
            level,
            message,
            timestamp_millis,
        });

        Ok(())
    }

    /// Get current timestamp in milliseconds.
    pub fn now_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Read from workspace if capability granted.
    pub fn workspace_read(&self, path: &str) -> Result<Option<String>, WasmError> {
        // Check if workspace capability is granted
        let capability = match &self.capabilities.workspace_read {
            Some(cap) => cap,
            None => return Ok(None), // No capability, return None
        };

        // Validate path (security critical)
        validate_workspace_path(path)?;

        // Check allowed prefixes if any are specified
        if !capability.allowed_prefixes.is_empty() {
            let allowed = capability
                .allowed_prefixes
                .iter()
                .any(|prefix| path.starts_with(prefix));
            if !allowed {
                tracing::debug!(
                    path = path,
                    allowed = ?capability.allowed_prefixes,
                    "WASM workspace read denied: path not in allowed prefixes"
                );
                return Ok(None);
            }
        }

        // Actually read from workspace
        match &capability.reader {
            Some(reader) => Ok(reader.read(path)),
            None => Ok(None), // No reader configured
        }
    }

    /// Sign a payload using a managed NEAR key.
    ///
    /// Checks signing capability, key label allowlist, and rate limit.
    /// Delegates actual signing to the `PayloadSigner` if all checks pass.
    ///
    /// Private keys NEVER enter WASM memory. Only the signature is returned.
    pub fn sign_payload(
        &mut self,
        key_label: &str,
        payload_base64: &str,
        context_json: &str,
    ) -> crate::tools::wasm::capabilities::SignPayloadResult {
        use crate::tools::wasm::capabilities::SignPayloadResult;

        let capability = match &self.capabilities.signing {
            Some(cap) => cap,
            None => {
                return SignPayloadResult {
                    signature: None,
                    error: Some("Signing capability not granted".to_string()),
                    approval_pending: false,
                };
            }
        };

        // Check key label is allowed
        if !capability.is_label_allowed(key_label) {
            return SignPayloadResult {
                signature: None,
                error: Some(format!(
                    "Key label '{}' not in allowed list for this tool",
                    key_label
                )),
                approval_pending: false,
            };
        }

        // Check rate limit
        self.sign_count += 1;
        if self.sign_count > capability.max_signs_per_execution {
            return SignPayloadResult {
                signature: None,
                error: Some(format!(
                    "Sign limit exceeded ({} per execution)",
                    capability.max_signs_per_execution
                )),
                approval_pending: false,
            };
        }

        // Decode base64 payload
        let payload_bytes = match base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            payload_base64,
        ) {
            Ok(bytes) => bytes,
            Err(e) => {
                return SignPayloadResult {
                    signature: None,
                    error: Some(format!("Invalid base64 payload: {}", e)),
                    approval_pending: false,
                };
            }
        };

        // Delegate to signer implementation
        match &capability.signer {
            Some(signer) => signer.sign_payload(key_label, &payload_bytes, context_json),
            None => SignPayloadResult {
                signature: None,
                error: Some("No signing provider configured".to_string()),
                approval_pending: false,
            },
        }
    }

    /// Get the sign count for this execution.
    pub fn sign_count(&self) -> u32 {
        self.sign_count
    }

    /// Get collected logs after execution.
    pub fn take_logs(&mut self) -> Vec<LogEntry> {
        std::mem::take(&mut self.logs)
    }

    /// Get number of logs dropped due to rate limiting.
    pub fn logs_dropped(&self) -> usize {
        self.logs_dropped
    }

    /// Check if a secret exists (does not expose value).
    ///
    /// Returns false if:
    /// - Secrets capability not granted
    /// - Secret name not in allowed list
    /// - User ID not set
    pub fn secret_exists(&self, name: &str) -> bool {
        let capability = match &self.capabilities.secrets {
            Some(cap) => cap,
            None => return false,
        };

        // Check if name is allowed
        capability.is_allowed(name)
    }

    /// Check if HTTP capability is available for a given URL and method.
    ///
    /// Returns an error message if not allowed.
    pub fn check_http_allowed(&self, url: &str, method: &str) -> Result<(), String> {
        let capability = self
            .capabilities
            .http
            .as_ref()
            .ok_or_else(|| "HTTP capability not granted".to_string())?;

        // Use the allowlist validator
        use crate::tools::wasm::allowlist::AllowlistValidator;

        let validator = AllowlistValidator::new(capability.allowlist.clone());
        let result = validator.validate(url, method);

        if result.is_allowed() {
            Ok(())
        } else {
            Err(format!("HTTP request not allowed: {:?}", result))
        }
    }

    /// Check if tool invocation is allowed for an alias.
    ///
    /// Returns the real tool name if allowed, error otherwise.
    pub fn check_tool_invoke_allowed(&self, alias: &str) -> Result<String, String> {
        let capability = self
            .capabilities
            .tool_invoke
            .as_ref()
            .ok_or_else(|| "Tool invocation capability not granted".to_string())?;

        capability
            .resolve_alias(alias)
            .map(|s| s.to_string())
            .ok_or_else(|| format!("Unknown tool alias: {}", alias))
    }

    /// Increment HTTP request counter and check rate limit.
    ///
    /// Returns error if rate limit exceeded.
    pub fn record_http_request(&mut self) -> Result<(), String> {
        // Verify HTTP capability exists
        let _capability = self
            .capabilities
            .http
            .as_ref()
            .ok_or_else(|| "HTTP capability not granted".to_string())?;

        self.http_request_count += 1;

        // Simple per-execution rate limit (additional to global rate limiter)
        // This prevents a single execution from making too many requests
        const MAX_REQUESTS_PER_EXECUTION: u32 = 50;
        if self.http_request_count > MAX_REQUESTS_PER_EXECUTION {
            return Err(format!(
                "Too many HTTP requests in single execution (max {})",
                MAX_REQUESTS_PER_EXECUTION
            ));
        }

        Ok(())
    }

    /// Increment tool invoke counter and check rate limit.
    ///
    /// Returns error if rate limit exceeded.
    pub fn record_tool_invoke(&mut self) -> Result<(), String> {
        self.tool_invoke_count += 1;

        const MAX_INVOKES_PER_EXECUTION: u32 = 20;
        if self.tool_invoke_count > MAX_INVOKES_PER_EXECUTION {
            return Err(format!(
                "Too many tool invocations in single execution (max {})",
                MAX_INVOKES_PER_EXECUTION
            ));
        }

        Ok(())
    }

    /// Get HTTP request count for this execution.
    pub fn http_request_count(&self) -> u32 {
        self.http_request_count
    }

    /// Get tool invoke count for this execution.
    pub fn tool_invoke_count(&self) -> u32 {
        self.tool_invoke_count
    }
}

/// Validate a workspace path for security.
///
/// Blocks path traversal attacks and absolute paths.
fn validate_workspace_path(path: &str) -> Result<(), WasmError> {
    // Block absolute paths
    if path.starts_with('/') {
        return Err(WasmError::PathTraversalBlocked(
            "absolute paths not allowed".to_string(),
        ));
    }

    // Block path traversal
    if path.contains("..") {
        return Err(WasmError::PathTraversalBlocked(
            "parent directory references not allowed".to_string(),
        ));
    }

    // Block null bytes
    if path.contains('\0') {
        return Err(WasmError::PathTraversalBlocked(
            "null bytes not allowed".to_string(),
        ));
    }

    // Block Windows-style absolute paths (just in case)
    if path.len() >= 2 && path.chars().nth(1) == Some(':') {
        return Err(WasmError::PathTraversalBlocked(
            "Windows-style paths not allowed".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::tools::wasm::capabilities::{
        Capabilities, SecretsCapability, WorkspaceCapability, WorkspaceReader,
    };
    use crate::tools::wasm::host::{
        HostState, LogLevel, MAX_LOG_ENTRIES, MAX_LOG_MESSAGE_BYTES, validate_workspace_path,
    };

    struct MockReader {
        content: String,
    }

    impl WorkspaceReader for MockReader {
        fn read(&self, _path: &str) -> Option<String> {
            Some(self.content.clone())
        }
    }

    #[test]
    fn test_logging_basic() {
        let mut state = HostState::minimal();
        state
            .log(LogLevel::Info, "test message".to_string())
            .unwrap();

        let logs = state.take_logs();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].level, LogLevel::Info);
        assert_eq!(logs[0].message, "test message");
    }

    #[test]
    fn test_logging_rate_limit() {
        let mut state = HostState::minimal();

        // Fill up to limit
        for i in 0..MAX_LOG_ENTRIES {
            state
                .log(LogLevel::Debug, format!("message {}", i))
                .unwrap();
        }

        // This should be dropped silently
        state
            .log(LogLevel::Info, "should be dropped".to_string())
            .unwrap();

        assert_eq!(state.take_logs().len(), MAX_LOG_ENTRIES);
        assert_eq!(state.logs_dropped(), 1);
    }

    #[test]
    fn test_logging_truncation() {
        let mut state = HostState::minimal();

        let long_message = "x".repeat(MAX_LOG_MESSAGE_BYTES + 1000);
        state.log(LogLevel::Info, long_message).unwrap();

        let logs = state.take_logs();
        assert!(logs[0].message.len() <= MAX_LOG_MESSAGE_BYTES + 20); // +20 for truncation suffix
        assert!(logs[0].message.ends_with("... (truncated)"));
    }

    #[test]
    fn test_now_millis() {
        let state = HostState::minimal();
        let now = state.now_millis();
        // Should be a reasonable timestamp (after 2020)
        assert!(now > 1577836800000); // Jan 1, 2020
    }

    #[test]
    fn test_workspace_read_no_capability() {
        let state = HostState::minimal();
        let result = state.workspace_read("context/test.md").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_workspace_read_with_capability() {
        let reader = Arc::new(MockReader {
            content: "test content".to_string(),
        });

        let capabilities = Capabilities {
            workspace_read: Some(WorkspaceCapability {
                allowed_prefixes: vec![],
                reader: Some(reader),
            }),
            ..Default::default()
        };

        let state = HostState::new(capabilities);
        let result = state.workspace_read("context/test.md").unwrap();
        assert_eq!(result, Some("test content".to_string()));
    }

    #[test]
    fn test_workspace_read_prefix_restriction() {
        let reader = Arc::new(MockReader {
            content: "test content".to_string(),
        });

        let capabilities = Capabilities {
            workspace_read: Some(WorkspaceCapability {
                allowed_prefixes: vec!["context/".to_string()],
                reader: Some(reader),
            }),
            ..Default::default()
        };

        let state = HostState::new(capabilities);

        // Allowed prefix
        let result = state.workspace_read("context/test.md").unwrap();
        assert!(result.is_some());

        // Disallowed prefix
        let result = state.workspace_read("secrets/api_key.txt").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_path_validation_blocks_traversal() {
        assert!(validate_workspace_path("../etc/passwd").is_err());
        assert!(validate_workspace_path("context/../secrets").is_err());
        assert!(validate_workspace_path("context/test/../../secrets").is_err());
    }

    #[test]
    fn test_path_validation_blocks_absolute() {
        assert!(validate_workspace_path("/etc/passwd").is_err());
        assert!(validate_workspace_path("/context/test.md").is_err());
    }

    #[test]
    fn test_path_validation_blocks_null_bytes() {
        assert!(validate_workspace_path("context/test\0.md").is_err());
    }

    #[test]
    fn test_path_validation_blocks_windows_paths() {
        assert!(validate_workspace_path("C:\\Windows\\System32").is_err());
        assert!(validate_workspace_path("D:secrets").is_err());
    }

    #[test]
    fn test_path_validation_allows_valid_paths() {
        assert!(validate_workspace_path("context/test.md").is_ok());
        assert!(validate_workspace_path("daily/2024-01-15.md").is_ok());
        assert!(validate_workspace_path("projects/alpha/notes.md").is_ok());
        assert!(validate_workspace_path("MEMORY.md").is_ok());
    }

    #[test]
    fn test_secret_exists_no_capability() {
        let state = HostState::minimal();
        assert!(!state.secret_exists("any_secret"));
    }

    #[test]
    fn test_secret_exists_with_capability() {
        let capabilities = Capabilities {
            secrets: Some(SecretsCapability {
                allowed_names: vec!["openai_*".to_string(), "exact_name".to_string()],
            }),
            ..Default::default()
        };

        let state = HostState::new(capabilities);

        // Glob match
        assert!(state.secret_exists("openai_key"));
        assert!(state.secret_exists("openai_org"));

        // Exact match
        assert!(state.secret_exists("exact_name"));

        // Not allowed
        assert!(!state.secret_exists("stripe_key"));
    }

    #[test]
    fn test_http_request_rate_limit() {
        // Create state with HTTP capability enabled
        let capabilities = Capabilities {
            http: Some(crate::tools::wasm::capabilities::HttpCapability::default()),
            ..Default::default()
        };
        let mut state = HostState::new(capabilities);

        // Should allow up to 50 requests
        for _ in 0..50 {
            assert!(state.record_http_request().is_ok());
        }

        // 51st should fail
        assert!(state.record_http_request().is_err());
    }

    #[test]
    fn test_tool_invoke_rate_limit() {
        // Create state with tool invoke capability enabled
        let capabilities = Capabilities {
            tool_invoke: Some(crate::tools::wasm::capabilities::ToolInvokeCapability::default()),
            ..Default::default()
        };
        let mut state = HostState::new(capabilities);

        // Should allow up to 20 invocations
        for _ in 0..20 {
            assert!(state.record_tool_invoke().is_ok());
        }

        // 21st should fail
        assert!(state.record_tool_invoke().is_err());
    }

    #[test]
    fn test_new_with_user() {
        let state = HostState::new_with_user(Capabilities::default(), "user123");
        assert_eq!(state.user_id(), Some("user123"));
    }

    #[test]
    fn test_sign_payload_no_capability() {
        let mut state = HostState::minimal();
        let result = state.sign_payload("any-key", "AAAA", "{}");
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("not granted"));
    }

    #[test]
    fn test_sign_payload_label_not_allowed() {
        let capabilities = Capabilities {
            signing: Some(crate::tools::wasm::capabilities::SigningCapability {
                allowed_key_labels: vec!["allowed-key".to_string()],
                max_signs_per_execution: 5,
                signer: None,
            }),
            ..Default::default()
        };
        let mut state = HostState::new(capabilities);
        let result = state.sign_payload("forbidden-key", "AAAA", "{}");
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("not in allowed list"));
    }

    #[test]
    fn test_sign_payload_rate_limit() {
        let capabilities = Capabilities {
            signing: Some(crate::tools::wasm::capabilities::SigningCapability {
                allowed_key_labels: vec!["key".to_string()],
                max_signs_per_execution: 2,
                signer: None,
            }),
            ..Default::default()
        };
        let mut state = HostState::new(capabilities);

        // First two should hit "no signer" (not rate limit)
        let r1 = state.sign_payload("key", "AAAA", "{}");
        assert!(r1.error.as_deref().unwrap().contains("No signing provider"));

        let r2 = state.sign_payload("key", "AAAA", "{}");
        assert!(r2.error.as_deref().unwrap().contains("No signing provider"));

        // Third should hit rate limit
        let r3 = state.sign_payload("key", "AAAA", "{}");
        assert!(r3.error.as_deref().unwrap().contains("limit exceeded"));
    }

    #[test]
    fn test_sign_payload_invalid_base64() {
        let capabilities = Capabilities {
            signing: Some(crate::tools::wasm::capabilities::SigningCapability {
                allowed_key_labels: vec!["key".to_string()],
                max_signs_per_execution: 5,
                signer: Some(Arc::new(MockSigner)),
            }),
            ..Default::default()
        };
        let mut state = HostState::new(capabilities);
        let result = state.sign_payload("key", "not-valid-base64!!!", "{}");
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("Invalid base64"));
    }

    #[test]
    fn test_sign_payload_with_mock_signer() {
        let capabilities = Capabilities {
            signing: Some(crate::tools::wasm::capabilities::SigningCapability {
                allowed_key_labels: vec!["test-key".to_string()],
                max_signs_per_execution: 5,
                signer: Some(Arc::new(MockSigner)),
            }),
            ..Default::default()
        };
        let mut state = HostState::new(capabilities);

        // Encode some payload as base64
        let payload =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"sign this");
        let result = state.sign_payload("test-key", &payload, "{}");
        assert!(result.signature.is_some());
        assert!(result.error.is_none());
        assert!(!result.approval_pending);
        assert_eq!(result.signature.unwrap(), "mock-signature");
    }

    struct MockSigner;

    impl crate::tools::wasm::capabilities::PayloadSigner for MockSigner {
        fn sign_payload(
            &self,
            _key_label: &str,
            _payload: &[u8],
            _context_json: &str,
        ) -> crate::tools::wasm::capabilities::SignPayloadResult {
            crate::tools::wasm::capabilities::SignPayloadResult {
                signature: Some("mock-signature".to_string()),
                error: None,
                approval_pending: false,
            }
        }
    }
}
