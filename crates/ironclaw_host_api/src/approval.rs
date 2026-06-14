//! Approval contracts for user-mediated authority.
//!
//! Approval is a scoped grant to continue a specific action, not a vague
//! confirmation. [`ApprovalRequest`] carries the exact action that needs a
//! decision and may optionally describe a reusable [`ApprovalScope`] such as a
//! capability, path prefix, or network target. Matching must be exact or
//! policy-defined by the host; callers must not infer broader authority from a
//! one-off approval.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

use crate::{
    Action, ApprovalRequestId, CapabilityDisplayText, CapabilityId, CorrelationId, HostApiError,
    NetworkTargetPattern, Principal, ResourceEstimate, ResourceScope, ScopedPath, Timestamp,
    truncate_capability_display_text,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: ApprovalRequestId,
    pub correlation_id: CorrelationId,
    pub requested_by: Principal,
    pub action: Box<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_fingerprint: Option<InvocationFingerprint>,
    pub reason: String,
    pub reusable_scope: Option<ApprovalScope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InvocationFingerprint(String);

impl InvocationFingerprint {
    pub fn for_dispatch(
        scope: &ResourceScope,
        capability: &CapabilityId,
        estimate: &ResourceEstimate,
        input: &serde_json::Value,
    ) -> Result<Self, HostApiError> {
        Self::for_action("dispatch", scope, capability, estimate, input)
    }

    pub fn for_spawn(
        scope: &ResourceScope,
        capability: &CapabilityId,
        estimate: &ResourceEstimate,
        input: &serde_json::Value,
    ) -> Result<Self, HostApiError> {
        Self::for_action("spawn_capability", scope, capability, estimate, input)
    }

    fn for_action(
        kind: &'static str,
        scope: &ResourceScope,
        capability: &CapabilityId,
        estimate: &ResourceEstimate,
        input: &serde_json::Value,
    ) -> Result<Self, HostApiError> {
        #[derive(Serialize)]
        struct Payload<'a> {
            version: u8,
            kind: &'static str,
            scope: &'a ResourceScope,
            capability: &'a CapabilityId,
            estimate: &'a ResourceEstimate,
            input: &'a serde_json::Value,
        }

        let canonical_input = canonical_json_v1(input)?;
        let payload = Payload {
            version: 1,
            kind,
            scope,
            capability,
            estimate,
            input: &canonical_input,
        };
        let bytes = serde_json::to_vec(&payload)
            .map_err(|error| HostApiError::invariant(error.to_string()))?;
        Ok(Self(sha256_digest_token(&bytes)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const MAX_CANONICAL_JSON_DEPTH: usize = 64;
pub const SHELL_COMMAND_DISPLAY_MAX_BYTES: usize = 2 * 1024;

/// Canonicalize JSON values with recursively sorted object keys.
///
/// This helper is shared by host API fingerprints and host-runtime surface
/// fingerprints so equivalent JSON inputs hash identically across process runs.
/// Arrays preserve order; object keys sort lexicographically; scalar values are
/// returned unchanged. Deeply nested inputs fail closed.
pub fn canonical_json_v1(value: &serde_json::Value) -> Result<serde_json::Value, HostApiError> {
    canonical_json_at_depth(value, 0)
}

pub fn shell_command_display_text(command: &str) -> CapabilityDisplayText {
    let sanitized = redact_shell_command_for_display(command);
    truncate_capability_display_text(&sanitized, SHELL_COMMAND_DISPLAY_MAX_BYTES)
}

fn redact_shell_command_for_display(cmd: &str) -> String {
    use regex::Regex;

    static PATTERNS: OnceLock<[Regex; 4]> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        // The patterns are hardcoded literals; a panic here would be a
        // developer error caught by the unit test below.
        [
            Regex::new(
                r#"(?i)(-u|--user|--token|--api-?key|--password|--auth|--bearer)(\s+|=)(["'])[^"']*(["'])"#,
            )
            .expect("hardcoded shell redaction regex is valid"), // safety: static regex literal is covered by redaction tests.
            Regex::new(
                r#"(?i)(-u|--user|--token|--api-?key|--password|--auth|--bearer)(\s+|=)([^\s"'][^\s]*)"#,
            )
            .expect("hardcoded shell redaction regex is valid"), // safety: static regex literal is covered by redaction tests.
            Regex::new(
                r#"(?i)(Authorization|X-Api-Key|X-Auth-Token|Bearer)\s*:\s*(?:[a-zA-Z]+\s+[^\s"']+|[^\s"']+)"#,
            )
                .expect("hardcoded shell redaction regex is valid"), // safety: static regex literal is covered by redaction tests.
            Regex::new(r#"[a-zA-Z][a-zA-Z0-9+.\-]*://[^\s"']+"#)
                .expect("hardcoded shell redaction regex is valid"), // safety: static regex literal is covered by redaction tests.
        ]
    });
    let mut out = patterns[0]
        .replace_all(cmd, "$1$2$3[redacted]$4")
        .into_owned();
    out = patterns[1].replace_all(&out, "$1$2[redacted]").into_owned();
    out = patterns[2].replace_all(&out, "$1: [redacted]").into_owned();
    out = patterns[3]
        .replace_all(&out, |captures: &regex::Captures<'_>| {
            sanitize_url_for_display(&captures[0])
        })
        .into_owned();
    sanitize_text(&out)
}

pub fn sanitize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut redact_next_value = false;
    for token in text.split_inclusive(char::is_whitespace) {
        let trimmed = token.trim_end();
        if trimmed.is_empty() {
            push_safe_text(&mut out, token);
            continue;
        }
        let suffix = &token[trimmed.len()..];
        if is_url_like(trimmed) {
            out.push_str(&sanitize_url_for_display(trimmed));
            push_safe_text(&mut out, suffix);
            redact_next_value = false;
            continue;
        }
        let redact_current =
            redact_next_value || is_secret_like(trimmed) || is_unsafe_path_like(trimmed);
        if redact_current {
            out.push_str("[redacted]");
            push_safe_text(&mut out, suffix);
        } else {
            push_safe_text(&mut out, token);
        }
        redact_next_value = credential_key_expects_value(trimmed) && !suffix.is_empty();
    }
    out
}

fn is_url_like(token: &str) -> bool {
    let Some(scheme_end) = token.find("://") else {
        return false;
    };
    let scheme = &token[..scheme_end];
    let Some(first) = scheme.chars().next() else {
        return false;
    };
    !scheme.eq_ignore_ascii_case("file")
        && first.is_ascii_alphabetic()
        && scheme.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '.' | '-')
        })
}

pub fn sanitize_url_for_display(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return sanitize_text(url);
    };
    let (scheme, rest) = url.split_at(scheme_end + 3);
    let (rest, suffix) = match rest.find(['?', '#']) {
        Some(index) => {
            let marker = &rest[index..=index];
            let replacement = if marker == "?" { "?..." } else { "#..." };
            (&rest[..index], replacement)
        }
        None => (rest, ""),
    };
    let (authority, path) = match rest.find('/') {
        Some(index) => rest.split_at(index),
        None => (rest, ""),
    };
    let authority = authority
        .rfind('@')
        .map(|index| &authority[index + 1..])
        .unwrap_or(authority);
    let path = redact_secret_url_path_segments(path);
    strip_unsupported_control_chars(&format!("{scheme}{authority}{path}{suffix}"))
}

fn redact_secret_url_path_segments(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    path.split('/')
        .enumerate()
        .map(|(index, segment)| {
            if index == 0 {
                String::new()
            } else if is_secret_url_path_segment(segment) {
                "[redacted]".to_string()
            } else {
                segment.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn is_secret_url_path_segment(segment: &str) -> bool {
    let segment = segment.trim_matches(token_boundary_punctuation);
    if is_secret_url_path_segment_value(segment) {
        return true;
    }
    let Ok(decoded) = urlencoding::decode(segment) else {
        return false;
    };
    decoded.as_ref() != segment
        && is_secret_url_path_segment_value(decoded.trim_matches(token_boundary_punctuation))
}

fn is_secret_url_path_segment_value(segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    is_secret_like(segment)
        || lower.starts_with("secret")
        || lower.starts_with("token")
        || lower.starts_with("api-key")
        || lower.starts_with("apikey")
        || lower.starts_with("api_key")
        || lower.starts_with("password")
        || lower.starts_with("credential")
        || lower.starts_with("bearer")
}

fn push_safe_text(out: &mut String, text: &str) {
    out.extend(
        text.chars().filter(|character| {
            *character == '\n' || *character == '\t' || !character.is_control()
        }),
    );
}

fn strip_unsupported_control_chars(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    push_safe_text(&mut out, text);
    out
}

fn is_secret_like(token: &str) -> bool {
    let trimmed = token.trim_matches(token_boundary_punctuation);
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("sk-")
        || lower.starts_with("ghp_")
        || lower.starts_with("gho_")
        || lower.starts_with("ghu_")
        || lower.starts_with("ghs_")
        || lower.starts_with("xoxb-")
        || lower.starts_with("xoxa-")
        || lower.starts_with("xoxp-")
        || looks_like_aws_access_key(trimmed)
        || looks_like_jwt(trimmed)
        || lower.contains("api_key=")
        || lower.contains("api_key:")
        || lower.contains("apikey=")
        || lower.contains("apikey:")
        || lower.contains("access_token=")
        || lower.contains("access_token:")
        || lower.contains("secret=")
        || lower.contains("secret:")
        || lower.contains("password=")
        || lower.contains("password:")
        || lower.contains("token=")
        || lower.contains("token:")
}

fn is_unsafe_path_like(token: &str) -> bool {
    let token = token.trim_matches(token_boundary_punctuation);
    token.to_ascii_lowercase().starts_with("file:/")
        || token_contains_absolute_posix_path(token)
        || token.starts_with("\\\\")
        || token.contains("\\\\")
        || token.get(1..3) == Some(":\\")
}

fn credential_key_expects_value(token: &str) -> bool {
    let lower = token
        .trim_matches(non_credential_boundary_punctuation)
        .to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "api_key:"
            | "api_key="
            | "apikey:"
            | "apikey="
            | "access_token:"
            | "access_token="
            | "secret:"
            | "secret="
            | "password:"
            | "password="
            | "token:"
            | "token="
    )
}

fn non_credential_boundary_punctuation(character: char) -> bool {
    matches!(
        character,
        '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
    )
}

fn looks_like_aws_access_key(token: &str) -> bool {
    (token.starts_with("AKIA") || token.starts_with("ASIA"))
        && token.len() >= 16
        && token
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

fn looks_like_jwt(token: &str) -> bool {
    token.starts_with("eyJ")
        && token.matches('.').count() >= 2
        && token.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

fn token_contains_absolute_posix_path(token: &str) -> bool {
    let mut previous = None;
    let mut characters = token.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '/'
            && previous.is_none_or(token_boundary_punctuation)
            && !matches!(previous, Some('/'))
            && !matches!(characters.peek(), Some('/'))
        {
            return true;
        }
        previous = Some(character);
    }
    false
}

fn token_boundary_punctuation(character: char) -> bool {
    matches!(
        character,
        '"' | '\'' | '`' | ',' | ';' | ':' | '=' | '(' | ')' | '[' | ']' | '{' | '}'
    )
}

/// Return a stable `sha256:<lower-hex>` digest token for already-canonical bytes.
pub fn sha256_digest_token(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", to_lower_hex(&digest))
}

fn canonical_json_at_depth(
    value: &serde_json::Value,
    depth: usize,
) -> Result<serde_json::Value, HostApiError> {
    if depth > MAX_CANONICAL_JSON_DEPTH {
        return Err(HostApiError::invariant(
            "canonical_json: max depth exceeded",
        ));
    }

    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| canonical_json_at_depth(item, depth + 1))
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        serde_json::Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            let mut canonical = serde_json::Map::new();
            for (key, value) in entries {
                canonical.insert(key.clone(), canonical_json_at_depth(value, depth + 1)?);
            }
            Ok(serde_json::Value::Object(canonical))
        }
        _ => Ok(value.clone()),
    }
}

fn to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalScope {
    pub principal: Principal,
    pub action_pattern: ActionPattern,
    pub expires_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ActionPattern {
    ExactAction {
        action: Box<Action>,
    },
    Capability {
        capability: CapabilityId,
    },
    PathPrefix {
        action_kind: FileActionKind,
        prefix: ScopedPath,
    },
    NetworkTarget {
        target: NetworkTargetPattern,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileActionKind {
    Read,
    List,
    Write,
    Delete,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_command_display_text_redacts_auth_and_url_query() {
        let display = shell_command_display_text(
            "curl -H 'Authorization: Bearer sk-secret' https://example.test/path?token=secret && echo ok",
        );

        assert!(display.text.contains("curl -H 'Authorization: [redacted]'"));
        assert!(display.text.contains("https://example.test/path?..."));
        assert!(display.text.contains("echo ok"));
        assert!(!display.text.contains("sk-secret"));
        assert!(!display.text.contains("token=secret"));
    }

    #[test]
    fn shell_command_display_text_redacts_bare_secrets_and_host_paths() {
        let display = shell_command_display_text(
            "cat /home/alice/.ssh/id_rsa && echo sk-secret && token: ghp_secret",
        );

        assert!(display.text.contains("cat [redacted]"));
        assert!(display.text.contains("echo [redacted]"));
        assert!(display.text.contains("token: [redacted]"));
        assert!(!display.text.contains("/home/alice"));
        assert!(!display.text.contains("sk-secret"));
        assert!(!display.text.contains("ghp_secret"));
    }

    #[test]
    fn shell_command_display_text_redacts_secret_url_path_segments() {
        let display = shell_command_display_text(
            "curl https://example.test/reset/sk-secret/token123?debug=true",
        );

        assert!(
            display
                .text
                .contains("https://example.test/reset/[redacted]/[redacted]?...")
        );
        assert!(!display.text.contains("sk-secret"));
        assert!(!display.text.contains("token123"));
        assert!(!display.text.contains("debug=true"));
    }

    #[test]
    fn shell_command_display_text_redacts_percent_encoded_secret_url_path_segments() {
        let display = shell_command_display_text(
            "curl https://example.test/reset/sk%2Dsecret/token%31%32%33?debug=true",
        );

        assert!(
            display
                .text
                .contains("https://example.test/reset/[redacted]/[redacted]?...")
        );
        assert!(!display.text.contains("sk%2Dsecret"));
        assert!(!display.text.contains("token%31%32%33"));
        assert!(!display.text.contains("debug=true"));
    }

    #[test]
    fn shell_command_display_text_keeps_benign_headers_visible() {
        let display = shell_command_display_text(
            "curl -H 'Accept: application/json' -H 'Authorization: Bearer sk-secret' https://example.test",
        );

        assert!(display.text.contains("-H 'Accept: application/json'"));
        assert!(display.text.contains("-H 'Authorization: [redacted]'"));
        assert!(!display.text.contains("sk-secret"));
    }
}
