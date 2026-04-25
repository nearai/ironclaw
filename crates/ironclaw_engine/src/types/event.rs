//! Event sourcing types.
//!
//! Every significant action within a thread is recorded as an event.
//! This enables replay, debugging, reflection, and trace-based testing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::capability::LeaseId;

/// Generate a short human-readable summary of tool parameters for display.
///
/// For `http`: shows the URL with query/userinfo/fragment stripped (signed
/// URLs and query-string secrets must not leak into debug SSE). For
/// `web_search`: shows the query. For `shell`: shows the command with
/// auth-bearing flag values and embedded URL query strings redacted. For
/// other tools: shows the first string argument, truncated. Returns
/// `None` for empty or unrecognizable params.
pub fn summarize_params(action_name: &str, params: &serde_json::Value) -> Option<String> {
    let summary = match action_name {
        "http" | "web_fetch" => params
            .get("url")
            .and_then(|v| v.as_str())
            .map(|u| truncate(&strip_url_sensitive_parts(u), 80)),
        "web_search" | "llm_context" => params
            .get("query")
            .and_then(|v| v.as_str())
            .map(|q| truncate(q, 60)),
        "memory_search" => params
            .get("query")
            .and_then(|v| v.as_str())
            .map(|q| truncate(q, 60)),
        "memory_write" => params
            .get("target")
            .and_then(|v| v.as_str())
            .map(|t| t.to_string()),
        "memory_read" => params
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| p.to_string()),
        "shell" => params
            .get("command")
            .and_then(|v| v.as_str())
            .map(|c| truncate(&redact_shell_command_for_display(c), 60)),
        "message" => params
            .get("content")
            .and_then(|v| v.as_str())
            .map(|c| truncate(c, 40)),
        _ => {
            // Generic: show the first string value whose key is not
            // sensitive-looking. The fallback previously returned the
            // first string unconditionally, which for MCP / unknown
            // tools could surface `token`, `api_key`, `password`, etc.
            // into debug-panel SSE and `ActionExecuted` events.
            if let Some(obj) = params.as_object() {
                obj.iter()
                    .filter(|(k, _)| !is_sensitive_param_key(k))
                    .find_map(|(_, v)| v.as_str())
                    .map(|s| truncate(s, 50))
            } else {
                None
            }
        }
    };
    summary.filter(|s| !s.is_empty())
}

/// Strip query string, fragment, and userinfo from a URL so debug
/// summaries never surface query-string secrets, signed-URL tokens, or
/// `user:password@host` credentials.
///
/// Parses conservatively: recognizes `scheme://` URLs, splits on `?` /
/// `#`, and drops `user[:pass]@` from the authority. Non-URL strings
/// pass through unchanged — the caller truncates afterward.
fn strip_url_sensitive_parts(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let (scheme, rest) = url.split_at(scheme_end + 3);
    // Cut off query and fragment first.
    let end = rest
        .find(['?', '#'])
        .map(|i| {
            let marker = &rest[i..=i];
            // Replace with a visible placeholder so the reader can tell
            // something was stripped, rather than silently truncating.
            (i, marker)
        })
        .map(|(i, marker)| {
            if marker == "?" {
                format!("{}?…", &rest[..i])
            } else {
                format!("{}#…", &rest[..i])
            }
        })
        .unwrap_or_else(|| rest.to_string());
    // Drop `user[:pass]@` from the authority section (before the first
    // `/` that starts the path, if any).
    let (authority, path_and_rest) = match end.find('/') {
        Some(i) => end.split_at(i),
        None => (end.as_str(), ""),
    };
    let authority_clean = authority
        .rfind('@')
        .map(|at| &authority[at + 1..])
        .unwrap_or(authority);
    format!("{scheme}{authority_clean}{path_and_rest}")
}

/// Redact auth-bearing argument values and URL query strings inside a
/// shell command before it reaches debug surfaces.
///
/// Covers the common secret-leaking shapes seen in agent-written `curl`
/// / `wget` / `http` invocations:
///
/// * Quoted and unquoted values after `-H`/`--header`, `-u`/`--user`,
///   `--token`, `--api-key`, `--password`, `--auth`, `--bearer`.
/// * `Authorization:` / `X-Api-Key:` style headers inside a single
///   `-H '…'` argument.
/// * URL query strings embedded anywhere in the command.
///
/// Anything stripped is replaced with `<REDACTED>` (quoted values) or a
/// trailing `?…` (URL query) so the reader can see something was
/// removed. The caller still truncates to the display width.
fn redact_shell_command_for_display(cmd: &str) -> String {
    use regex::Regex;
    // Lazily initialized — summary rendering is hot on debug-panel SSE
    // and we want to avoid recompiling regexes per call.
    use std::sync::OnceLock;
    static PATTERNS: OnceLock<[Regex; 4]> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [
            // (1) Quoted values after auth-bearing flags:
            //     -H "Authorization: Bearer …"   -> -H "<REDACTED>"
            //     --header 'X-Api-Key: …'        -> --header '<REDACTED>'
            //     -u 'user:pass'                 -> -u '<REDACTED>'
            Regex::new(
                r#"(?i)(-H|--header|-u|--user|--token|--api-?key|--password|--auth|--bearer)(\s+|=)(["'])[^"']*(["'])"#,
            )
            .unwrap(), // safety: hardcoded literal
            // (2) Unquoted values after the same flags (stops at whitespace).
            Regex::new(
                r#"(?i)(-H|--header|-u|--user|--token|--api-?key|--password|--auth|--bearer)(\s+|=)([^\s"'][^\s]*)"#,
            )
            .unwrap(), // safety: hardcoded literal
            // (3) Authorization-style header inside a single `-H '…'`
            //     argument where the quoted-value regex above already
            //     fired — belt-and-suspenders for header spellings that
            //     bypass the quoted-value match (e.g. bare word args).
            Regex::new(r#"(?i)(Authorization|X-Api-Key|X-Auth-Token|Bearer)\s*:\s*[^\s"']+"#)
                .unwrap(), // safety: hardcoded literal
            // (4) URL query string anywhere in the command.
            //     Matches `scheme://host/path?…` up to whitespace or quote.
            Regex::new(r#"([a-zA-Z][a-zA-Z0-9+.\-]*://[^\s"'?#]*)\?[^\s"']*"#).unwrap(), // safety: hardcoded literal
        ]
    });
    let mut out = patterns[0]
        .replace_all(cmd, "$1$2$3<REDACTED>$4")
        .into_owned();
    out = patterns[1].replace_all(&out, "$1$2<REDACTED>").into_owned();
    out = patterns[2].replace_all(&out, "$1: <REDACTED>").into_owned();
    out = patterns[3].replace_all(&out, "$1?…").into_owned();
    out
}

/// Returns `true` if a parameter key name looks like it carries a
/// secret. Used by [`summarize_params`] to skip the generic fallback on
/// keys whose values should not appear in debug surfaces.
///
/// The engine crate can't consult the host's `Tool::sensitive_params()`
/// (that trait lives in the main `ironclaw` crate), so this denylist is
/// a best-effort defense for unknown/MCP tools. Known tools get
/// per-tool extraction (url/query/command/etc.) above and never hit
/// this path.
fn is_sensitive_param_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "token",
        "secret",
        "password",
        "passwd",
        "api_key",
        "apikey",
        "auth",
        "credential",
        "bearer",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
        || lower == "key"
        || lower.ends_with("_key")
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Find a safe UTF-8 boundary
        let mut end = max.min(s.len());
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end]) // safety: end is validated by is_char_boundary loop above
    }
}
use crate::types::step::{StepId, TokenUsage};
use crate::types::thread::{ThreadId, ThreadState};

/// Strongly-typed event identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventId(pub Uuid);

impl EventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

/// A recorded event in a thread's execution history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadEvent {
    pub id: EventId,
    pub thread_id: ThreadId,
    pub timestamp: DateTime<Utc>,
    pub kind: EventKind,
}

impl ThreadEvent {
    pub fn new(thread_id: ThreadId, kind: EventKind) -> Self {
        Self {
            id: EventId::new(),
            thread_id,
            timestamp: Utc::now(),
            kind,
        }
    }
}

/// The specific kind of event that occurred.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventKind {
    // ── Thread lifecycle ────────────────────────────────────
    StateChanged {
        from: ThreadState,
        to: ThreadState,
        reason: Option<String>,
    },

    // ── Step lifecycle ──────────────────────────────────────
    StepStarted {
        step_id: StepId,
    },
    StepCompleted {
        step_id: StepId,
        tokens: TokenUsage,
    },
    StepFailed {
        step_id: StepId,
        error: String,
    },

    // ── Action execution ────────────────────────────────────
    ActionExecuted {
        step_id: StepId,
        action_name: String,
        call_id: String,
        duration_ms: u64,
        /// Short human-readable summary of parameters (e.g., URL for http tool).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        params_summary: Option<String>,
    },
    ActionFailed {
        step_id: StepId,
        action_name: String,
        call_id: String,
        error: String,
        #[serde(default)]
        duration_ms: u64,
        /// Short human-readable summary of parameters.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        params_summary: Option<String>,
    },

    // ── Capability leases ───────────────────────────────────
    LeaseGranted {
        lease_id: LeaseId,
        capability_name: String,
    },
    LeaseRevoked {
        lease_id: LeaseId,
        reason: String,
    },
    LeaseExpired {
        lease_id: LeaseId,
    },

    // ── Messages ────────────────────────────────────────────
    MessageAdded {
        role: String,
        content_preview: String,
    },

    // ── Thread tree ─────────────────────────────────────────
    ChildSpawned {
        child_id: ThreadId,
        goal: String,
    },
    ChildCompleted {
        child_id: ThreadId,
    },

    // ── Approval flow ───────────────────────────────────────
    ApprovalRequested {
        action_name: String,
        call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parameters: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        allow_always: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gate_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        params_summary: Option<String>,
    },
    ApprovalReceived {
        call_id: String,
        approved: bool,
    },

    // ── Self-improvement ──────────────────────────────────────
    SelfImprovementStarted,
    SelfImprovementComplete {
        prompt_updated: bool,
        patterns_added: usize,
    },
    SelfImprovementFailed {
        error: String,
    },

    // ── Skill activation ───────────────────────────────────────
    SkillActivated {
        skill_names: Vec<String>,
    },

    // ── Code execution instrumentation ────────────────────────
    /// Emitted when a code (REPL) execution attempt fails. Enables aggregate
    /// analysis of code execution failure modes to determine whether the
    /// runtime (Monty), the LLM, or tool dispatch is the primary source of
    /// failures.
    CodeExecutionFailed {
        step_id: StepId,
        /// Classified failure category.
        category: crate::types::step::CodeExecutionFailure,
        /// The error message text (truncated to 500 chars).
        error: String,
        /// Hash of the Python code that was executed, for dedup/correlation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code_hash: Option<String>,
        /// Duration of the code execution attempt in milliseconds.
        #[serde(default)]
        duration_ms: u64,
    },

    /// CodeAct execution trace — raw code + stdout retained for observers
    /// (debug panel, trace replay). The in-context chat summary is too
    /// lossy; this variant keeps the full evidence.
    CodeExecuted {
        step_id: StepId,
        code: String,
        stdout: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        return_value: Option<serde_json::Value>,
        #[serde(default)]
        duration_ms: u64,
    },

    // ── Orchestrator versioning ───────────────────────────────
    OrchestratorRollback {
        from_version: u64,
        to_version: u64,
        reason: String,
    },

    /// Unknown event kind — catch-all for forward compatibility during
    /// rolling deploys. Older binaries deserializing events written by
    /// newer binaries will produce this variant instead of failing.
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::{EventKind, summarize_params};
    use crate::types::step::StepId;

    #[test]
    fn summarize_params_generic_fallback_skips_sensitive_keys() {
        // Regression: PR #2850 — the generic fallback previously returned
        // `obj.values().find_map(..)`, which surfaced tokens/api keys from
        // MCP tools into `ActionExecuted` events + debug-panel SSE.
        let params = serde_json::json!({
            "api_key": "sk-very-secret-value",
            "endpoint": "https://example.com/rpc",
        });
        let summary = summarize_params("unknown_mcp_tool", &params);
        assert_eq!(summary.as_deref(), Some("https://example.com/rpc"));
        assert!(
            !summary.as_deref().unwrap_or("").contains("sk-very-secret"),
            "sensitive value leaked into params_summary"
        );
    }

    #[test]
    fn summarize_params_generic_fallback_returns_none_when_only_sensitive_keys() {
        let params = serde_json::json!({
            "token": "abc",
            "password": "p@ss",
            "Authorization": "Bearer xyz",
            "refresh_key": "rk",
        });
        assert_eq!(summarize_params("unknown_mcp_tool", &params), None);
    }

    #[test]
    fn summarize_params_http_strips_query_string_and_userinfo() {
        // Regression: PR #2850 review — query-string secrets and signed
        // URL tokens were reaching debug-panel SSE via the `http` /
        // `web_fetch` per-tool branches.
        let params = serde_json::json!({
            "url": "https://user:secret@api.example.com/v1/thing?api_key=sk-abc123&foo=bar#frag",
        });
        let summary = summarize_params("http", &params).expect("http summary should be present");
        assert!(
            !summary.contains("sk-abc123"),
            "query-string secret leaked: {summary}"
        );
        assert!(!summary.contains("secret@"), "userinfo leaked: {summary}");
        assert!(
            !summary.contains("api_key"),
            "query param key leaked: {summary}"
        );
        assert!(
            summary.starts_with("https://api.example.com/v1/thing?"),
            "unexpected http summary shape: {summary}"
        );

        let params = serde_json::json!({
            "url": "https://cdn.example.com/download?Signature=abc&Expires=1234",
        });
        let summary =
            summarize_params("web_fetch", &params).expect("web_fetch summary should be present");
        assert!(
            !summary.contains("Signature"),
            "signed-URL param leaked: {summary}"
        );
        assert!(
            !summary.contains("abc"),
            "signed-URL value leaked: {summary}"
        );
    }

    #[test]
    fn summarize_params_http_preserves_non_url_strings() {
        // Defense-in-depth: if `url` ever carries a non-URL shape, we
        // shouldn't mangle it; leakage risk comes from query/userinfo,
        // not from arbitrary strings.
        let params = serde_json::json!({ "url": "not-really-a-url" });
        let summary = summarize_params("http", &params).expect("summary present");
        assert_eq!(summary, "not-really-a-url");
    }

    #[test]
    fn summarize_params_shell_redacts_auth_headers_and_query_strings() {
        // Regression: PR #2850 review — `shell` summary carried raw
        // command text into `ToolCompleted.parameters`, leaking
        // `Authorization: Bearer …` / `--token …` / URL query secrets.
        let params = serde_json::json!({
            "command": "curl -H \"Authorization: Bearer sk-abc123\" https://api.example.com/v1",
        });
        let summary = summarize_params("shell", &params).expect("summary present");
        assert!(
            !summary.contains("sk-abc123"),
            "bearer token leaked: {summary}"
        );
        assert!(
            !summary.contains("Bearer "),
            "bearer header leaked: {summary}"
        );
        assert!(
            summary.contains("<REDACTED>"),
            "redaction marker missing: {summary}"
        );

        let params = serde_json::json!({
            "command": "curl -u alice:topsecret https://api.example.com/",
        });
        let summary = summarize_params("shell", &params).expect("summary present");
        assert!(
            !summary.contains("topsecret"),
            "basic-auth leaked: {summary}"
        );
        assert!(
            !summary.contains("alice:"),
            "basic-auth user leaked: {summary}"
        );

        let params = serde_json::json!({
            "command": "http GET https://api.example.com/v1/data?api_key=sk-xyz",
        });
        let summary = summarize_params("shell", &params).expect("summary present");
        assert!(
            !summary.contains("sk-xyz"),
            "query-string secret leaked: {summary}"
        );
        assert!(
            !summary.contains("api_key=sk"),
            "query key/value leaked: {summary}"
        );

        let params = serde_json::json!({
            "command": "curl --token sk-123 --api-key ak-456 https://example.com/",
        });
        let summary = summarize_params("shell", &params).expect("summary present");
        assert!(!summary.contains("sk-123"), "--token leaked: {summary}");
        assert!(!summary.contains("ak-456"), "--api-key leaked: {summary}");
    }

    #[test]
    fn action_failed_defaults_missing_duration_ms_when_deserializing_legacy_payload() {
        let step_id = StepId::new();
        let legacy_payload = serde_json::json!({
            "ActionFailed": {
                "step_id": step_id,
                "action_name": "web_search",
                "call_id": "call_123",
                "error": "permission denied"
            }
        });

        let event_kind: EventKind =
            serde_json::from_value(legacy_payload).expect("legacy ActionFailed should deserialize");

        match event_kind {
            EventKind::ActionFailed {
                step_id: actual_step_id,
                action_name,
                call_id,
                error,
                duration_ms,
                params_summary,
            } => {
                assert_eq!(actual_step_id, step_id);
                assert_eq!(action_name, "web_search");
                assert_eq!(call_id, "call_123");
                assert_eq!(error, "permission denied");
                assert_eq!(duration_ms, 0);
                assert_eq!(params_summary, None);
            }
            other => panic!("expected ActionFailed, got {other:?}"),
        }
    }
}
