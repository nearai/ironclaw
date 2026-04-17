//! Sensitive-data scrubber for DingTalk-bound strings.
//!
//! Every string that flows out of the DingTalk channel to the AI card
//! (status lines, tool-parameter summaries, error bodies, reasoning
//! excerpts) passes through this module. The goal is **defense in depth**:
//!
//! - Tool execution + audit paths are untouched — full fidelity is retained
//!   per the `LLM data is never deleted` rule (see `ironclaw/CLAUDE.md`).
//! - UI-bound strings get secrets/PII/paths/URLs/long-body values redacted
//!   before they reach `CardState.*_buffer` or DingTalk's streaming API.
//!
//! Enforcement is two-layered per the plan:
//! 1. Construction of [`ScrubbedText`] is module-private (`new()` is
//!    internal), and the renderer only accepts `ScrubbedText` — callers
//!    cannot accidentally push a raw `String` into the PUT payload.
//! 2. A grep-based pre-commit rule backs this up for cross-module churn.
//!
//! Plan reference: `docs/plans/2026-04-18-001-feat-dingtalk-anti-silence-ux-plan.md` (Unit 4).

use std::collections::HashSet;

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::types::ChannelLevel;
use crate::tools::Tool;

/// A string that has been run through the sensitive-data scrubber and is
/// therefore safe to write into the DingTalk AI card `content` field.
///
/// Construction is module-private: the only way to get a `ScrubbedText` is
/// to go through the scrub helpers below. `Display` / `AsRef<str>` expose
/// the inner string read-only.
#[derive(Debug, Clone)]
pub struct ScrubbedText(String);

impl ScrubbedText {
    /// Module-private constructor. Callers outside this module must use
    /// the `scrub_*` free functions.
    fn new(text: String) -> Self {
        ScrubbedText(text)
    }

    /// Borrow the scrubbed content as a string slice. This is the only
    /// read accessor exposed — intentionally no `into_inner` that hands
    /// out a mutable String.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the scrubbed payload is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl AsRef<str> for ScrubbedText {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ScrubbedText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// ─── Secret value-shape detectors ──────────────────────────────────────────
//
// Order matters: the most specific patterns run first so that e.g. a JWT
// inside a generic `authorization` header is caught as a JWT, not as a
// generic secret.

static RE_AWS_ACCESS_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"A(?:KIA|SIA|CCA|IDA|GPA|ROA|NPA)[0-9A-Z]{16}").unwrap());

static RE_PRIVATE_KEY_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
    )
    .unwrap()
});

static RE_GCP_SA: LazyLock<Regex> = LazyLock::new(|| {
    // Minimal GCP service-account JSON marker.
    Regex::new(r#""type"\s*:\s*"service_account""#).unwrap()
});

static RE_JWT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"ey[A-Za-z0-9_-]{10,}\.ey[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+").unwrap()
});

static RE_GITHUB_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"gh[pousr]_[A-Za-z0-9]{36,}").unwrap());

static RE_SLACK_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"xox[abp]-[A-Za-z0-9-]+").unwrap());

// ─── PII patterns ──────────────────────────────────────────────────────────

static RE_EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap()
});

// Chinese national ID: 17 digits + 1 digit or X.
static RE_CN_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d{17}[0-9Xx]\b").unwrap());

// Mainland Chinese mobile: 11 digits starting with 1.
static RE_CN_PHONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b1[3-9]\d{9}\b").unwrap());

// E.164-ish international phone (loose): + followed by 8..15 digits.
static RE_E164: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\+\d{8,15}").unwrap());

// RFC1918 internal IPv4 ranges.
static RE_INTERNAL_IP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:10\.\d{1,3}\.\d{1,3}\.\d{1,3}|172\.(?:1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3}|192\.168\.\d{1,3}\.\d{1,3})\b").unwrap()
});

// Internal hostname hints.
static RE_INTERNAL_HOST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Za-z0-9.-]+\.(?:internal|intranet|local|corp)\b").unwrap());

// ─── Public API ────────────────────────────────────────────────────────────

/// Apply the full sensitive-value scrubber to an arbitrary string. Most
/// callers should use the higher-level helpers below; this is the shared
/// backbone.
fn scrub_raw(text: &str) -> String {
    let mut out = text.to_string();
    // Secret shapes (strictest first).
    out = RE_PRIVATE_KEY_BLOCK
        .replace_all(&out, "<private_key:redacted>")
        .into_owned();
    out = RE_GCP_SA
        .replace_all(&out, "\"type\":\"<gcp_sa:redacted>\"")
        .into_owned();
    out = RE_AWS_ACCESS_KEY
        .replace_all(&out, "<aws_key:redacted>")
        .into_owned();
    out = RE_JWT.replace_all(&out, "<jwt:redacted>").into_owned();
    out = RE_GITHUB_TOKEN
        .replace_all(&out, "<github_token:redacted>")
        .into_owned();
    out = RE_SLACK_TOKEN
        .replace_all(&out, "<slack_token:redacted>")
        .into_owned();
    // PII.
    out = RE_CN_ID.replace_all(&out, "<id:redacted>").into_owned();
    out = RE_CN_PHONE.replace_all(&out, "<phone:redacted>").into_owned();
    out = RE_E164.replace_all(&out, "<phone:redacted>").into_owned();
    out = RE_EMAIL.replace_all(&out, "<email:redacted>").into_owned();
    // Network identifiers.
    out = RE_INTERNAL_IP
        .replace_all(&out, "<internal_ip>")
        .into_owned();
    out = RE_INTERNAL_HOST
        .replace_all(&out, "<internal_host>")
        .into_owned();
    out
}

/// Escape markdown special characters so a scrubbed payload can't break
/// DingTalk card rendering (backticks, pipes, asterisks, newlines).
///
/// Kept as a free helper so tests can exercise it directly.
pub fn markdown_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '`' => out.push_str("\\`"),
            '*' => out.push_str("\\*"),
            '|' => out.push_str("\\|"),
            '\n' => out.push(' '),
            '\r' => {} // drop CR
            other => out.push(other),
        }
    }
    out
}

fn looks_like_path(s: &str) -> bool {
    s.starts_with('/') && s.matches('/').count() >= 2
}

fn basename_only(path: &str) -> String {
    path.rsplit('/')
        .find(|c| !c.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn host_only(url: &str) -> String {
    // Cheap URL -> host extraction without pulling in the `url` crate.
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host_part = after_scheme.split('/').next().unwrap_or(after_scheme);
    host_part.split('?').next().unwrap_or(host_part).to_string()
}

fn truncate_ellipsis(s: &str, cap_chars: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= cap_chars {
            out.push('…');
            return out;
        }
        out.push(c);
    }
    out
}

/// Build a scrubbed one-line summary for a tool invocation. Drives the
/// status line's `🔧 Using tool · <summary> (Ns)` rendering.
///
/// Group-chat behavior: if the tool has NOT opted into
/// [`Tool::safe_for_group_display`], the summary is reduced to the
/// group_display_name with zero parameter detail (bystander privacy).
pub fn scrub_param_summary(
    tool: &dyn Tool,
    params: &Value,
    channel_level: ChannelLevel,
) -> ScrubbedText {
    // Group chat + tool not opted in → opaque "external service" rendering.
    if channel_level == ChannelLevel::Group && !tool.safe_for_group_display() {
        return ScrubbedText::new(markdown_escape(tool.group_display_name()));
    }

    // Default-deny for tools that haven't declared any sensitive_params:
    // in DM we still want to surface SOMETHING (the tool's display_name +
    // a short summary) so users see what's happening, but we run the full
    // value-shape scrubber below so any accidental secret in the summary
    // is redacted.
    let raw_summary = tool.summary_for_ui(params);
    let scrubbed = scrub_raw(&raw_summary);
    let mut rendered = String::new();
    rendered.push_str(tool.display_name());
    if !scrubbed.is_empty() && scrubbed != tool.name() {
        rendered.push_str(": ");
        rendered.push_str(&scrubbed);
    }

    // Cap length at ~60 characters to keep the status line one-line on
    // mobile DingTalk; the ellipsis is part of the visible payload.
    let capped = truncate_ellipsis(&rendered, 60);
    ScrubbedText::new(markdown_escape(&capped))
}

/// Scrub a full filesystem path down to its basename (defense in depth —
/// callers SHOULD already avoid logging full paths, but we enforce it).
pub fn scrub_path_like(text: &str) -> String {
    if looks_like_path(text) {
        basename_only(text)
    } else {
        text.to_string()
    }
}

/// Scrub a URL down to its host component.
pub fn scrub_url_like(text: &str) -> String {
    if text.starts_with("http://") || text.starts_with("https://") {
        host_only(text)
    } else {
        text.to_string()
    }
}

/// Scrub an arbitrary error body for rendering to the card. Strips JSON
/// keys that commonly leak internal topology in addition to the
/// standard secret/PII scrubber.
pub fn scrub_error_body(err: &str) -> ScrubbedText {
    let base = scrub_raw(err);
    ScrubbedText::new(markdown_escape(&base))
}

/// Scrub a reasoning-summary excerpt. Applies the standard secret/PII
/// pass, then also redacts any substring that matches a previously-seen
/// sensitive value for this card — prevents reasoning from leaking a
/// secret that appeared earlier in a tool param/return.
pub fn scrub_reasoning_excerpt(text: &str, seen_sensitive: &HashSet<String>) -> ScrubbedText {
    let mut out = scrub_raw(text);
    for needle in seen_sensitive {
        // Only redact substrings ≥8 chars to avoid false-positive
        // redaction of common tokens.
        if needle.len() >= 8 && out.contains(needle.as_str()) {
            out = out.replace(needle.as_str(), "<seen_sensitive:redacted>");
        }
    }
    let capped = truncate_ellipsis(&out, 80);
    ScrubbedText::new(markdown_escape(&capped))
}

/// Test-only constructor used by the caller-level integration test
/// harness to inject fixtures. NOT exposed outside the crate.
#[cfg(any(test, feature = "integration"))]
pub(crate) fn scrubbed_for_test(s: impl Into<String>) -> ScrubbedText {
    ScrubbedText::new(s.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_aws_access_key() {
        let raw = "headers: {Authorization: AKIAIOSFODNN7EXAMPLE}";
        let out = scrub_raw(raw);
        assert!(out.contains("<aws_key:redacted>"));
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn scrubs_jwt() {
        let raw = "token=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let out = scrub_raw(raw);
        assert!(out.contains("<jwt:redacted>"));
    }

    #[test]
    fn scrubs_private_key_block() {
        let raw = "-----BEGIN RSA PRIVATE KEY-----\nMIIE\n-----END RSA PRIVATE KEY-----";
        let out = scrub_raw(raw);
        assert!(out.contains("<private_key:redacted>"));
        assert!(!out.contains("MIIE"));
    }

    #[test]
    fn scrubs_chinese_id_card() {
        let raw = "staff id 320106198001014567 applied";
        let out = scrub_raw(raw);
        assert!(out.contains("<id:redacted>"));
        assert!(!out.contains("320106198001014567"));
    }

    #[test]
    fn scrubs_mainland_phone() {
        let raw = "contact 13800001234 please";
        let out = scrub_raw(raw);
        assert!(out.contains("<phone:redacted>"));
    }

    #[test]
    fn scrubs_email() {
        let raw = "user foo@example.com reported";
        let out = scrub_raw(raw);
        assert!(out.contains("<email:redacted>"));
    }

    #[test]
    fn scrubs_internal_host_and_ip() {
        let raw = "target https://scheduler.internal:8080 on 10.0.1.5";
        let out = scrub_raw(raw);
        assert!(out.contains("<internal_host>"));
        assert!(out.contains("<internal_ip>"));
    }

    #[test]
    fn markdown_escape_breaks_tables_and_code() {
        let raw = "a|b `c` *d*\nline2";
        let out = markdown_escape(raw);
        assert!(out.contains(r"\|"));
        assert!(out.contains(r"\`"));
        assert!(out.contains(r"\*"));
        assert!(!out.contains('\n'));
    }

    #[test]
    fn basename_helper_strips_dirs() {
        assert_eq!(basename_only("/etc/passwd"), "passwd");
        assert_eq!(basename_only("/usr/local/bin/"), "bin");
        assert_eq!(basename_only("noslash"), "noslash");
    }

    #[test]
    fn url_host_strips_path_and_query() {
        assert_eq!(
            host_only("https://api.example.com/v1/foo?token=xxx"),
            "api.example.com"
        );
        assert_eq!(host_only("api.example.com/x"), "api.example.com");
    }

    #[test]
    fn seen_sensitive_redacts_substring() {
        let mut seen = HashSet::new();
        seen.insert("SECRET_ABC_12345".to_string());
        let out = scrub_reasoning_excerpt("I noticed SECRET_ABC_12345 in the payload", &seen);
        assert!(out.as_str().contains("<seen_sensitive:redacted>"));
        assert!(!out.as_str().contains("SECRET_ABC_12345"));
    }

    #[test]
    fn seen_sensitive_ignores_short_substrings() {
        let mut seen = HashSet::new();
        seen.insert("abc".to_string());
        let out = scrub_reasoning_excerpt("abc appears everywhere", &seen);
        // "abc" < 8 chars → should NOT trigger aggressive redaction.
        assert!(out.as_str().contains("abc"));
    }

    #[test]
    fn scrubbed_text_only_exposes_read_access() {
        let t = ScrubbedText::new("hello".to_string());
        assert_eq!(t.as_str(), "hello");
        assert_eq!(format!("{t}"), "hello");
        assert!(!t.is_empty());
    }
}
