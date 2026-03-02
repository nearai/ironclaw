//! Secret leak detection for WASM sandbox.
//!
//! Scans data at the sandbox boundary to prevent secret exfiltration.
//! Uses Aho-Corasick for fast multi-pattern matching plus regex for
//! complex patterns.
//!
//! # Security Model
//!
//! Leak detection happens at TWO points:
//!
//! 1. **Before outbound requests** - Prevents WASM from exfiltrating secrets
//!    by encoding them in URLs, headers, or request bodies
//! 2. **After responses/outputs** - Prevents accidental exposure in logs,
//!    tool outputs, or data returned to WASM
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                         WASM HTTP Request Flow                              │
//! │                                                                              │
//! │   WASM ──► Allowlist ──► Leak Scan ──► Credential ──► Execute ──► Response │
//! │            Validator     (request)     Injector       Request      │        │
//! │                                                                    ▼        │
//! │                                      WASM ◀── Leak Scan ◀── Response       │
//! │                                               (response)                    │
//! └─────────────────────────────────────────────────────────────────────────────┘
//!
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                           Scan Result Actions                               │
//! │                                                                              │
//! │   LeakDetector.scan() ──► LeakScanResult                                   │
//! │                               │                                             │
//! │                               ├─► clean: pass through                       │
//! │                               ├─► warn: log, pass                           │
//! │                               ├─► redact: mask secret                       │
//! │                               └─► block: reject entirely                    │
//! └─────────────────────────────────────────────────────────────────────────────┘
//! ```

use std::ops::Range;

use aho_corasick::AhoCorasick;
use regex::Regex;

/// Action to take when a leak is detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeakAction {
    /// Block the output entirely (for critical secrets).
    Block,
    /// Redact the secret, replacing it with [REDACTED].
    Redact,
    /// Log a warning but allow the output.
    Warn,
}

impl std::fmt::Display for LeakAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LeakAction::Block => write!(f, "block"),
            LeakAction::Redact => write!(f, "redact"),
            LeakAction::Warn => write!(f, "warn"),
        }
    }
}

/// Severity of a detected leak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LeakSeverity {
    Low,
    Medium,
    High,
    Critical,
}

impl std::fmt::Display for LeakSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LeakSeverity::Low => write!(f, "low"),
            LeakSeverity::Medium => write!(f, "medium"),
            LeakSeverity::High => write!(f, "high"),
            LeakSeverity::Critical => write!(f, "critical"),
        }
    }
}

/// A pattern for detecting secret leaks.
#[derive(Debug, Clone)]
pub struct LeakPattern {
    pub name: String,
    pub regex: Regex,
    pub severity: LeakSeverity,
    pub action: LeakAction,
}

/// A detected potential secret leak.
#[derive(Debug, Clone)]
pub struct LeakMatch {
    pub pattern_name: String,
    pub severity: LeakSeverity,
    pub action: LeakAction,
    /// Location in the scanned content.
    pub location: Range<usize>,
    /// A preview of the match with the secret partially masked.
    pub masked_preview: String,
}

/// Result of scanning content for leaks.
#[derive(Debug)]
pub struct LeakScanResult {
    /// All detected potential leaks.
    pub matches: Vec<LeakMatch>,
    /// Whether any match requires blocking.
    pub should_block: bool,
    /// Content with secrets redacted (if redaction was applied).
    pub redacted_content: Option<String>,
}

impl LeakScanResult {
    /// Check if content is clean (no leaks detected).
    pub fn is_clean(&self) -> bool {
        self.matches.is_empty()
    }

    /// Get the highest severity found.
    pub fn max_severity(&self) -> Option<LeakSeverity> {
        self.matches.iter().map(|m| m.severity).max()
    }
}

/// Detector for secret leaks in output data.
pub struct LeakDetector {
    patterns: Vec<LeakPattern>,
    /// For fast prefix matching of known patterns
    prefix_matcher: Option<AhoCorasick>,
    known_prefixes: Vec<(String, usize)>, // (prefix, pattern_index)
}

impl LeakDetector {
    /// Create a new detector with default patterns.
    pub fn new() -> Self {
        Self::with_patterns(default_patterns())
    }

    /// Create a detector with custom patterns.
    pub fn with_patterns(patterns: Vec<LeakPattern>) -> Self {
        // Build prefix matcher for patterns that start with a known prefix
        let mut prefixes = Vec::new();
        for (idx, pattern) in patterns.iter().enumerate() {
            if let Some(prefix) = extract_literal_prefix(pattern.regex.as_str())
                && prefix.len() >= 3
            {
                prefixes.push((prefix, idx));
            }
        }

        let prefix_matcher = if !prefixes.is_empty() {
            let prefix_strings: Vec<&str> = prefixes.iter().map(|(s, _)| s.as_str()).collect();
            AhoCorasick::builder()
                .ascii_case_insensitive(false)
                .build(&prefix_strings)
                .ok()
        } else {
            None
        };

        Self {
            patterns,
            prefix_matcher,
            known_prefixes: prefixes,
        }
    }

    /// Scan content for potential secret leaks.
    pub fn scan(&self, content: &str) -> LeakScanResult {
        let mut matches = Vec::new();
        let mut should_block = false;
        let mut redact_ranges = Vec::new();

        // Use prefix matcher for quick elimination
        let candidate_indices: Vec<usize> = if let Some(ref matcher) = self.prefix_matcher {
            let mut indices = Vec::new();
            for mat in matcher.find_iter(content) {
                let found_prefix = &self.known_prefixes[mat.pattern().as_usize()].0;
                // Add all patterns whose prefix overlaps with the found prefix.
                // This handles two cases:
                // 1. A short prefix shadows a longer one (e.g. "sk-" shadows "sk-ant-api")
                // 2. Duplicate prefixes mapping to different patterns (e.g. "-----BEGIN" for PEM and SSH)
                for (other_prefix, other_idx) in &self.known_prefixes {
                    if (other_prefix.starts_with(found_prefix.as_str())
                        || found_prefix.starts_with(other_prefix.as_str()))
                        && !indices.contains(other_idx)
                    {
                        indices.push(*other_idx);
                    }
                }
            }
            // Also include patterns without prefixes
            for (idx, _) in self.patterns.iter().enumerate() {
                if !self.known_prefixes.iter().any(|(_, i)| *i == idx) && !indices.contains(&idx) {
                    indices.push(idx);
                }
            }
            indices
        } else {
            (0..self.patterns.len()).collect()
        };

        // Check candidate patterns
        for idx in candidate_indices {
            let pattern = &self.patterns[idx];
            for mat in pattern.regex.find_iter(content) {
                let matched_text = mat.as_str();
                let location = mat.start()..mat.end();

                let leak_match = LeakMatch {
                    pattern_name: pattern.name.clone(),
                    severity: pattern.severity,
                    action: pattern.action,
                    location: location.clone(),
                    masked_preview: mask_secret(matched_text),
                };

                if pattern.action == LeakAction::Block {
                    should_block = true;
                }

                if pattern.action == LeakAction::Redact {
                    redact_ranges.push(location.clone());
                }

                matches.push(leak_match);
            }
        }

        // Sort by location for proper redaction
        matches.sort_by_key(|m| m.location.start);
        redact_ranges.sort_by_key(|r| r.start);

        // Build redacted content if needed
        let redacted_content = if !redact_ranges.is_empty() {
            Some(apply_redactions(content, &redact_ranges))
        } else {
            None
        };

        LeakScanResult {
            matches,
            should_block,
            redacted_content,
        }
    }

    /// Scan content and return cleaned version based on action.
    ///
    /// Returns `Err` if content should be blocked, `Ok(content)` otherwise.
    pub fn scan_and_clean(&self, content: &str) -> Result<String, LeakDetectionError> {
        let result = self.scan(content);

        if result.should_block {
            // Find the blocking match for error message
            let blocking_match = result
                .matches
                .iter()
                .find(|m| m.action == LeakAction::Block);
            return Err(LeakDetectionError::SecretLeakBlocked {
                pattern: blocking_match
                    .map(|m| m.pattern_name.clone())
                    .unwrap_or_default(),
                preview: blocking_match
                    .map(|m| m.masked_preview.clone())
                    .unwrap_or_default(),
            });
        }

        // Log warnings
        for m in &result.matches {
            if m.action == LeakAction::Warn {
                tracing::warn!(
                    pattern = %m.pattern_name,
                    severity = %m.severity,
                    preview = %m.masked_preview,
                    "Potential secret leak detected (warning only)"
                );
            }
        }

        // Return redacted content if any, otherwise original
        Ok(result
            .redacted_content
            .unwrap_or_else(|| content.to_string()))
    }

    /// Scan an outbound HTTP request for potential secret leakage.
    ///
    /// This MUST be called before executing any HTTP request from WASM
    /// to prevent exfiltration of secrets via URL, headers, or body.
    ///
    /// Returns `Err` if any part contains a blocked secret pattern.
    pub fn scan_http_request(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<(), LeakDetectionError> {
        // Scan URL (most common exfiltration vector)
        self.scan_and_clean(url)?;

        // Scan each header value
        for (name, value) in headers {
            self.scan_and_clean(value)
                .map_err(|e| LeakDetectionError::SecretLeakBlocked {
                    pattern: format!("header:{}", name),
                    preview: e.to_string(),
                })?;
        }

        // Scan body if present. Use lossy UTF-8 conversion so a leading
        // non-UTF8 byte can't be used to skip scanning entirely.
        if let Some(body_bytes) = body {
            let body_str = String::from_utf8_lossy(body_bytes);
            self.scan_and_clean(&body_str)?;
        }

        Ok(())
    }

    /// Add a custom pattern at runtime.
    pub fn add_pattern(&mut self, pattern: LeakPattern) {
        self.patterns.push(pattern);
        // Note: prefix_matcher won't be updated; rebuild if needed
    }

    /// Get the number of patterns.
    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }
}

impl Default for LeakDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Error from leak detection.
#[derive(Debug, Clone, thiserror::Error)]
pub enum LeakDetectionError {
    #[error("Secret leak blocked: pattern '{pattern}' matched '{preview}'")]
    SecretLeakBlocked { pattern: String, preview: String },
}

/// Mask a secret for safe display.
///
/// Shows first 4 and last 4 characters, masks the middle.
fn mask_secret(secret: &str) -> String {
    let len = secret.len();
    if len <= 8 {
        return "*".repeat(len);
    }

    let prefix: String = secret.chars().take(4).collect();
    let suffix: String = secret.chars().skip(len - 4).collect();
    let middle_len = len - 8;
    format!("{}{}{}", prefix, "*".repeat(middle_len.min(8)), suffix)
}

/// Apply redaction ranges to content.
fn apply_redactions(content: &str, ranges: &[Range<usize>]) -> String {
    if ranges.is_empty() {
        return content.to_string();
    }

    let mut result = String::with_capacity(content.len());
    let mut last_end = 0;

    for range in ranges {
        if range.start > last_end {
            result.push_str(&content[last_end..range.start]);
        }
        result.push_str("[REDACTED]");
        last_end = range.end;
    }

    if last_end < content.len() {
        result.push_str(&content[last_end..]);
    }

    result
}

/// Extract a literal prefix from a regex pattern (if one exists).
fn extract_literal_prefix(pattern: &str) -> Option<String> {
    let mut prefix = String::new();

    for ch in pattern.chars() {
        match ch {
            // These start special regex constructs
            '[' | '(' | '.' | '*' | '+' | '?' | '{' | '|' | '^' | '$' => break,
            // Escape sequence
            '\\' => break,
            // Regular character
            _ => prefix.push(ch),
        }
    }

    if prefix.len() >= 3 {
        Some(prefix)
    } else {
        None
    }
}

/// Default leak detection patterns.
fn default_patterns() -> Vec<LeakPattern> {
    vec![
        // OpenAI API keys
        LeakPattern {
            name: "openai_api_key".to_string(),
            regex: Regex::new(r"sk-(?:proj-)?[a-zA-Z0-9]{20,}(?:T3BlbkFJ[a-zA-Z0-9_-]*)?").unwrap(),
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // Anthropic API keys
        LeakPattern {
            name: "anthropic_api_key".to_string(),
            regex: Regex::new(r"sk-ant-api[a-zA-Z0-9_-]{90,}").unwrap(),
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // AWS Access Key ID
        LeakPattern {
            name: "aws_access_key".to_string(),
            regex: Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(),
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // GitHub tokens
        LeakPattern {
            name: "github_token".to_string(),
            regex: Regex::new(r"gh[pousr]_[A-Za-z0-9_]{36,}").unwrap(),
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // GitHub fine-grained PAT
        LeakPattern {
            name: "github_fine_grained_pat".to_string(),
            regex: Regex::new(r"github_pat_[a-zA-Z0-9]{22}_[a-zA-Z0-9]{59}").unwrap(),
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // Stripe keys
        LeakPattern {
            name: "stripe_api_key".to_string(),
            regex: Regex::new(r"sk_(?:live|test)_[a-zA-Z0-9]{24,}").unwrap(),
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // NEAR AI session tokens
        LeakPattern {
            name: "nearai_session".to_string(),
            regex: Regex::new(r"sess_[a-zA-Z0-9]{32,}").unwrap(),
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // PEM private keys
        LeakPattern {
            name: "pem_private_key".to_string(),
            regex: Regex::new(r"-----BEGIN\s+(?:RSA\s+)?PRIVATE\s+KEY-----").unwrap(),
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // SSH private keys
        LeakPattern {
            name: "ssh_private_key".to_string(),
            regex: Regex::new(r"-----BEGIN\s+(?:OPENSSH|EC|DSA)\s+PRIVATE\s+KEY-----").unwrap(),
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // Google API keys
        LeakPattern {
            name: "google_api_key".to_string(),
            regex: Regex::new(r"AIza[0-9A-Za-z_-]{35}").unwrap(),
            severity: LeakSeverity::High,
            action: LeakAction::Block,
        },
        // Slack tokens
        LeakPattern {
            name: "slack_token".to_string(),
            regex: Regex::new(r"xox[baprs]-[0-9a-zA-Z-]{10,}").unwrap(),
            severity: LeakSeverity::High,
            action: LeakAction::Block,
        },
        // Twilio API keys
        LeakPattern {
            name: "twilio_api_key".to_string(),
            regex: Regex::new(r"SK[a-fA-F0-9]{32}").unwrap(),
            severity: LeakSeverity::High,
            action: LeakAction::Block,
        },
        // SendGrid API keys
        LeakPattern {
            name: "sendgrid_api_key".to_string(),
            regex: Regex::new(r"SG\.[a-zA-Z0-9_-]{22}\.[a-zA-Z0-9_-]{43}").unwrap(),
            severity: LeakSeverity::High,
            action: LeakAction::Block,
        },
        // Bearer tokens (redact instead of block, might be intentional)
        LeakPattern {
            name: "bearer_token".to_string(),
            regex: Regex::new(r"Bearer\s+[a-zA-Z0-9_-]{20,}").unwrap(),
            severity: LeakSeverity::High,
            action: LeakAction::Redact,
        },
        // Authorization header with key
        LeakPattern {
            name: "auth_header".to_string(),
            regex: Regex::new(r"(?i)authorization:\s*[a-zA-Z]+\s+[a-zA-Z0-9_-]{20,}").unwrap(),
            severity: LeakSeverity::High,
            action: LeakAction::Redact,
        },
        // High entropy hex (potential secrets, warn only)
        // Uses word boundary since look-around isn't supported in the regex crate.
        // This catches standalone 64-char hex strings (like SHA256 hashes used as secrets).
        LeakPattern {
            name: "high_entropy_hex".to_string(),
            regex: Regex::new(r"\b[a-fA-F0-9]{64}\b").unwrap(),
            severity: LeakSeverity::Medium,
            action: LeakAction::Warn,
        },
    ]
}

#[cfg(test)]
mod tests {
    use crate::safety::leak_detector::{LeakDetector, LeakSeverity};

    #[test]
    fn test_detect_openai_key() {
        let detector = LeakDetector::new();
        let content = "API key: sk-proj-abc123def456ghi789jkl012mno345pqrT3BlbkFJtest123";

        let result = detector.scan(content);
        assert!(!result.is_clean());
        assert!(result.should_block);
        assert!(
            result
                .matches
                .iter()
                .any(|m| m.pattern_name == "openai_api_key")
        );
    }

    #[test]
    fn test_detect_github_token() {
        let detector = LeakDetector::new();
        let content = "token: ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

        let result = detector.scan(content);
        assert!(!result.is_clean());
        assert!(
            result
                .matches
                .iter()
                .any(|m| m.pattern_name == "github_token")
        );
    }

    #[test]
    fn test_detect_aws_key() {
        let detector = LeakDetector::new();
        let key = format!("{}{}", "AKIA", "IOSFODNN7EXAMPLE");
        let content = format!("AWS_ACCESS_KEY_ID={key}");

        let result = detector.scan(&content);
        assert!(!result.is_clean());
        assert!(
            result
                .matches
                .iter()
                .any(|m| m.pattern_name == "aws_access_key")
        );
    }

    #[test]
    fn test_detect_pem_key() {
        let detector = LeakDetector::new();
        let content = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA...";

        let result = detector.scan(content);
        assert!(!result.is_clean());
        assert!(
            result
                .matches
                .iter()
                .any(|m| m.pattern_name == "pem_private_key")
        );
    }

    #[test]
    fn test_clean_content() {
        let detector = LeakDetector::new();
        let content = "Hello world! This is just regular text with no secrets.";

        let result = detector.scan(content);
        assert!(result.is_clean());
        assert!(!result.should_block);
    }

    #[test]
    fn test_redact_bearer_token() {
        let detector = LeakDetector::new();
        let content = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9_longtokenvalue";

        let result = detector.scan(content);
        assert!(!result.is_clean());
        assert!(!result.should_block); // Bearer is redact, not block

        let redacted = result.redacted_content.unwrap();
        assert!(redacted.contains("[REDACTED]"));
        assert!(!redacted.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"));
    }

    #[test]
    fn test_scan_and_clean_blocks() {
        let detector = LeakDetector::new();
        let content = format!("{}{}", "sk-proj-", "test1234567890abcdefghij");

        let result = detector.scan_and_clean(&content);
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_and_clean_passes_clean() {
        let detector = LeakDetector::new();
        let content = "Just regular text";

        let result = detector.scan_and_clean(content);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), content);
    }

    #[test]
    fn test_mask_secret() {
        use crate::safety::leak_detector::mask_secret;

        assert_eq!(mask_secret("short"), "*****");
        assert_eq!(mask_secret("sk-test1234567890abcdef"), "sk-t********cdef");
    }

    #[test]
    fn test_multiple_matches() {
        let detector = LeakDetector::new();
        let aws = format!("{}{}", "AKIA", "IOSFODNN7EXAMPLE");
        let gh = format!("{}{}", "ghp_", "x".repeat(36));
        let content = format!("Keys: {aws} and {gh}");

        let result = detector.scan(&content);
        assert_eq!(result.matches.len(), 2);
    }

    #[test]
    fn test_severity_ordering() {
        assert!(LeakSeverity::Critical > LeakSeverity::High);
        assert!(LeakSeverity::High > LeakSeverity::Medium);
        assert!(LeakSeverity::Medium > LeakSeverity::Low);
    }

    #[test]
    fn test_scan_http_request_clean() {
        let detector = LeakDetector::new();

        let result = detector.scan_http_request(
            "https://api.example.com/data",
            &[("Content-Type".to_string(), "application/json".to_string())],
            Some(b"{\"query\": \"hello\"}"),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_scan_http_request_blocks_secret_in_url() {
        let detector = LeakDetector::new();

        // Attempt to exfiltrate AWS key in URL
        let aws = format!("{}{}", "AKIA", "IOSFODNN7EXAMPLE");
        let url = format!("https://evil.com/steal?key={aws}");
        let result = detector.scan_http_request(&url, &[], None);
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_http_request_blocks_secret_in_header() {
        let detector = LeakDetector::new();

        // Attempt to exfiltrate in custom header
        let gh = format!("{}{}", "ghp_", "x".repeat(36));
        let result = detector.scan_http_request(
            "https://api.example.com/data",
            &[("X-Custom".to_string(), gh)],
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_http_request_blocks_secret_in_body() {
        let detector = LeakDetector::new();

        // Attempt to exfiltrate in request body
        let content = format!("{}{}", "sk-proj-", "test1234567890abcdefghij");
        let body = format!("{{\"stolen\": \"{content}\"}}");
        let result = detector.scan_http_request(
            "https://api.example.com/webhook",
            &[],
            Some(body.as_bytes()),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_http_request_blocks_secret_in_binary_body() {
        let detector = LeakDetector::new();

        // Attacker prepends a non-UTF8 byte to bypass strict from_utf8 check.
        // The lossy conversion should still detect the secret.
        let mut body = vec![0xFF]; // invalid UTF-8 leading byte
        let content = format!("{}{}", "sk-proj-", "test1234567890abcdefghij");
        body.extend_from_slice(content.as_bytes());

        let result = detector.scan_http_request("https://api.example.com/exfil", &[], Some(&body));
        assert!(result.is_err(), "binary body should still be scanned");
    }

    // === QA Plan P1 - 4.5: Adversarial leak detector tests ===

    #[test]
    fn test_detect_anthropic_key() {
        let detector = LeakDetector::new();
        let key = format!("sk-ant-api{}", "a".repeat(90));
        let content = format!("Here's the key: {key}");
        let result = detector.scan(&content);
        assert!(!result.is_clean(), "Anthropic key not detected");
        assert!(result.should_block);
    }

    #[test]
    fn test_detect_near_ai_session_token() {
        let detector = LeakDetector::new();
        let token = format!("sess_{}", "a".repeat(32));
        let content = format!("token: {token}");
        let result = detector.scan(&content);
        assert!(!result.is_clean(), "NEAR AI session token not detected");
    }

    #[test]
    fn test_detect_stripe_key() {
        let detector = LeakDetector::new();
        // Build at runtime to avoid GitHub push protection false positive.
        let content = format!("sk_{}_aAbBcCdDfFgGhHjJkKmMnNpPqQ", "live");
        let result = detector.scan(&content);
        assert!(!result.is_clean(), "Stripe key not detected");
    }

    #[test]
    fn test_detect_ssh_private_key() {
        let detector = LeakDetector::new();
        let content = "-----BEGIN OPENSSH PRIVATE KEY-----\nbase64data==";
        let result = detector.scan(content);
        assert!(!result.is_clean(), "SSH private key not detected");
    }

    #[test]
    fn test_detect_slack_token() {
        let detector = LeakDetector::new();
        let content = format!("{}{}", "xoxb-", "1234567890-abcdefghij");
        let result = detector.scan(&content);
        assert!(!result.is_clean(), "Slack token not detected");
    }

    #[test]
    fn test_secret_at_different_positions() {
        let detector = LeakDetector::new();
        let key = format!("{}{}", "AKIA", "IOSFODNN7EXAMPLE");

        // At start
        let result = detector.scan(&key);
        assert!(!result.is_clean(), "key at start not detected");

        // In middle
        let result = detector.scan(&format!("prefix text {key} suffix text"));
        assert!(!result.is_clean(), "key in middle not detected");

        // At end
        let result = detector.scan(&format!("end: {key}"));
        assert!(!result.is_clean(), "key at end not detected");
    }

    #[test]
    fn test_multiple_different_secret_types() {
        let detector = LeakDetector::new();
        let aws = format!("{}{}", "AKIA", "IOSFODNN7EXAMPLE");
        let gh = format!("{}{}", "ghp_", "x".repeat(36));
        let content = format!("AWS: {aws} and GitHub: {gh}");
        let result = detector.scan(&content);
        assert!(
            result.matches.len() >= 2,
            "expected 2+ matches for different secret types, got {}",
            result.matches.len()
        );
    }

    #[test]
    fn test_mask_secret_short_value() {
        use crate::safety::leak_detector::mask_secret;
        // Short secrets (<= 8 chars) should be fully masked
        assert_eq!(mask_secret("abc"), "***");
        assert_eq!(mask_secret(""), "");
        assert_eq!(mask_secret("12345678"), "********");
        // 9-char string shows first 4 + last 4 with one star in middle
        assert_eq!(mask_secret("123456789"), "1234*6789");
    }

    #[test]
    fn test_clean_text_not_flagged() {
        let detector = LeakDetector::new();
        // Common text that might look suspicious but isn't a real secret
        let clean_texts = [
            "The API returns a JSON response",
            "Use ssh to connect to the server",
            "Bearer authentication is required",
            "sk-this-is-too-short",
            "The key concept is immutability",
        ];
        for text in clean_texts {
            let result = detector.scan(text);
            // Should not block (may warn on some patterns, but not block)
            assert!(!result.should_block, "clean text falsely blocked: {text}");
        }
    }
}
