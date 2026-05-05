//! Pre-execution scanning of shell tool arguments via the external
//! [Tirith](https://github.com/sheeki03/tirith) terminal-security CLI.
//!
//! Three-way decision exposed at the inline approval call sites:
//!
//! - [`TirithPreflightDecision::Allow`] — proceed with normal approval logic
//!   (tirith disabled, non-shell tool, allow verdict, or fail-open + operational
//!   failure).
//! - [`TirithPreflightDecision::Approval`] — surface a pause through the existing
//!   approval shape with a rich tirith reason. `allow_always = false`, so users
//!   cannot permanently allow-list a tirith finding.
//! - [`TirithPreflightDecision::Deny`] — hard rejection, used ONLY for
//!   fail-closed operational failures (missing binary, timeout, spawn error,
//!   unknown exit). Never an approval — letting a user click through a
//!   fail-closed scan would silently turn fail-closed into fail-open.
//!
//! The Tirith CLI ships its own daemon mode for IDE integration; this module
//! always passes `--no-daemon` so each preflight is a deterministic one-shot
//! subprocess.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::tools::builtin::shell;

const FINDINGS_CAP: usize = 50;
const REASON_CAP: usize = 500;
const COMMAND_TRUNC: usize = 80;
/// Maximum stdout bytes parsed from the tirith subprocess. The real CLI
/// emits a few KiB of JSON per scan; this cap defends against a broken
/// or shadowed binary that would otherwise emit unbounded output and
/// pin memory before the timeout fires.
const STDOUT_CAP_BYTES: u64 = 256 * 1024;

/// Runtime configuration for the tirith preflight.
#[derive(Debug, Clone)]
pub struct TirithConfig {
    /// Master switch. When `false`, [`tirith_preflight`] short-circuits to
    /// [`TirithPreflightDecision::Allow`] before any subprocess is spawned.
    pub enabled: bool,
    /// Bare name (resolved via `which::which`) or explicit path to the tirith
    /// binary. Tilde-prefixed paths are expanded via `dirs::home_dir`.
    pub bin: String,
    /// Maximum time the subprocess may run. Exceeding this counts as an
    /// operational failure (treated per [`Self::fail_open`]).
    pub timeout: Duration,
    /// `true` (default): operational failures map to [`TirithPreflightDecision::Allow`]
    /// so an unconfigured user with no tirith binary sees no behavior change.
    /// `false`: operational failures map to [`TirithPreflightDecision::Deny`]
    /// (hard rejection — never approvable).
    pub fail_open: bool,
}

impl Default for TirithConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bin: "tirith".to_string(),
            timeout: Duration::from_secs(5),
            fail_open: true,
        }
    }
}

/// Internal verdict from running the tirith subprocess.
#[derive(Debug, Clone)]
pub enum TirithVerdict {
    Allow,
    /// Block / Warn / WarnAck (exit 1 / 2 / 3 — Hermes #3428 pattern). All
    /// surface as approval prompts; only Allow skips the prompt.
    Approvable {
        action: TirithAction,
        reason: String,
        findings: Vec<TirithFinding>,
    },
    /// Operational failure (missing binary, timeout, spawn error, unknown
    /// exit) under `fail_open = false`.
    FailClosed {
        summary: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TirithAction {
    Block,
    Warn,
    WarnAck,
}

/// Three-way decision exposed to inline approval call sites.
///
/// Critical: fail-closed operational failures become [`Self::Deny`] — never
/// [`Self::Approval`]. Approval-clickthrough on a fail-closed scan would
/// defeat fail-closed semantics.
///
/// The helper does not know the channel context (relay vs DM, etc.). Relay-
/// channel rejection of an `Approval` outcome is a CALL-SITE concern: the
/// caller converts `Approval { reason }` into a rejection when the channel
/// requires it (e.g. v1 dispatcher's existing non-DM relay auto-deny).
#[derive(Debug, Clone)]
pub enum TirithPreflightDecision {
    Allow,
    Approval { reason: String },
    Deny { reason: String },
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TirithFinding {
    #[serde(default)]
    pub rule_id: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct TirithVerdictJson {
    #[serde(default)]
    findings: Vec<TirithFinding>,
    #[serde(default)]
    approval_description: Option<String>,
}

/// Resolve [`TirithConfig::bin`] to an executable path.
///
/// - Bare names (no path separator) delegate to `which::which`, which already
///   handles Unix executable bits AND Windows PATHEXT (`.exe` / `.bat` / `.cmd`).
/// - Tilde-prefixed paths (`~/...`) are expanded via `dirs::home_dir`.
/// - Explicit paths must exist as a regular file. On Unix the executable bit
///   is also checked; on Windows the OS gates exec on file extension and image
///   header, so `is_file()` is sufficient.
///
/// Returns `None` when the binary cannot be located or is not executable.
pub fn resolve_tirith_bin(configured: &str) -> Option<PathBuf> {
    let trimmed = configured.trim();
    if trimmed.is_empty() {
        return None;
    }

    let has_separator = trimmed.contains('/') || trimmed.contains('\\');

    if !has_separator {
        // Bare names — including the literal `~` (no path separator) — go
        // through the PATH lookup. `which::which("~")` will fail on every
        // platform we care about; that's the correct outcome for a config
        // value that doesn't name a binary.
        return which::which(trimmed).ok();
    }

    let expanded = if let Some(stripped) = trimmed.strip_prefix("~/") {
        match dirs::home_dir() {
            Some(home) => home.join(stripped),
            None => return None,
        }
    } else {
        PathBuf::from(trimmed)
    };

    if is_executable_file(&expanded) {
        Some(expanded)
    } else {
        None
    }
}

/// Cross-platform "is this a real, runnable file?" check for explicit paths.
#[cfg(unix)]
fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file() && (meta.permissions().mode() & 0o111 != 0),
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_executable_file(path: &std::path::Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file(),
        Err(_) => false,
    }
}

/// Run the tirith subprocess against `cmd` and map its outcome to a verdict.
///
/// Exit codes are the source of truth (0/1/2/3 → Allow/Block/Warn/WarnAck).
/// The JSON body is parsed only to enrich the reason string — a parse failure
/// on a non-zero exit still produces an `Approvable` verdict with a fallback
/// reason.
pub async fn check_command(cmd: &str, cfg: &TirithConfig) -> TirithVerdict {
    if !cfg.enabled {
        return TirithVerdict::Allow;
    }

    let Some(bin) = resolve_tirith_bin(&cfg.bin) else {
        tracing::debug!(
            configured = %cfg.bin,
            "tirith binary not found; treating per fail_open"
        );
        return fail(
            cfg.fail_open,
            format!("tirith binary `{}` not found", cfg.bin),
        );
    };

    let mut command = Command::new(&bin);
    command
        .arg("check")
        .arg("--json")
        .arg("--non-interactive")
        .arg("--no-daemon")
        .arg("--shell")
        .arg("posix")
        .arg("--")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // stderr is intentionally `null`. We only consume the exit code +
        // stdout JSON; if stderr were piped without a draining task the
        // child could block on a full pipe and starve until the timeout
        // — which under the default `fail_open = true` would silently
        // become an Allow.
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            tracing::debug!(error = %e, "tirith spawn failed");
            return fail(cfg.fail_open, format!("tirith spawn failed: {e}"));
        }
    };

    let mut stdout_buf = Vec::new();
    let stdout_handle = child.stdout.take();
    let read_fut = async {
        if let Some(out) = stdout_handle {
            // Parse at most STDOUT_CAP_BYTES of JSON, then continue
            // draining any further stdout into a sink so the child does
            // not block on a full pipe. Without the trailing drain, a
            // broken binary that prints more than the cap would deadlock
            // until the timeout fires; under fail_open=true that would
            // silently degrade to Allow.
            let mut limited = out.take(STDOUT_CAP_BYTES);
            let _ = limited.read_to_end(&mut stdout_buf).await;
            let mut tail = limited.into_inner();
            let mut sink = [0u8; 4096];
            while let Ok(n) = tail.read(&mut sink).await {
                if n == 0 {
                    break;
                }
            }
        }
        child.wait().await
    };

    let status = match tokio::time::timeout(cfg.timeout, read_fut).await {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            tracing::debug!(error = %e, "tirith wait failed");
            return fail(cfg.fail_open, format!("tirith wait failed: {e}"));
        }
        Err(_) => {
            tracing::debug!(
                timeout_ms = cfg.timeout.as_millis() as u64,
                "tirith timed out"
            );
            return fail(cfg.fail_open, "tirith timed out".to_string());
        }
    };

    let parsed: TirithVerdictJson = serde_json::from_slice(&stdout_buf).unwrap_or_default();
    let action = match status.code() {
        Some(0) => return TirithVerdict::Allow,
        Some(1) => TirithAction::Block,
        Some(2) => TirithAction::Warn,
        Some(3) => TirithAction::WarnAck,
        Some(other) => {
            tracing::debug!(exit = other, "tirith returned unknown exit code");
            return fail(cfg.fail_open, format!("tirith returned exit code {other}"));
        }
        None => {
            tracing::debug!("tirith terminated by signal");
            return fail(cfg.fail_open, "tirith terminated by signal".to_string());
        }
    };

    let mut findings = parsed.findings;
    if findings.len() > FINDINGS_CAP {
        findings.truncate(FINDINGS_CAP);
    }
    let reason = build_reason(parsed.approval_description.as_deref(), &findings, cmd);

    TirithVerdict::Approvable {
        action,
        reason,
        findings,
    }
}

fn fail(fail_open: bool, summary: String) -> TirithVerdict {
    if fail_open {
        TirithVerdict::Allow
    } else {
        // Sanitize even the operational-failure summary — it contains the
        // configured `bin` path, which a hostile environment could populate
        // with control bytes specifically to land in the user-facing
        // approval/denial surface.
        TirithVerdict::FailClosed {
            summary: sanitize_for_display(&summary),
        }
    }
}

fn build_reason(
    approval_description: Option<&str>,
    findings: &[TirithFinding],
    cmd: &str,
) -> String {
    let head = match approval_description {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => {
            if findings.is_empty() {
                "tirith flagged a security issue in the proposed command".to_string()
            } else {
                let mut parts = Vec::new();
                for f in findings.iter().take(3) {
                    let title = if f.title.trim().is_empty() {
                        f.rule_id.clone()
                    } else {
                        f.title.clone()
                    };
                    let sev = if f.severity.trim().is_empty() {
                        "FINDING".to_string()
                    } else {
                        f.severity.clone()
                    };
                    let desc_short = truncate(&f.description, 120);
                    if desc_short.is_empty() {
                        parts.push(format!("[{sev}] {title}"));
                    } else {
                        parts.push(format!("[{sev}] {title}: {desc_short}"));
                    }
                }
                parts.join("; ")
            }
        }
    };

    let cmd_short = truncate(cmd, COMMAND_TRUNC);
    let combined = if cmd_short.is_empty() {
        head
    } else {
        format!("{head} (command: {cmd_short})")
    };
    // Sanitize BEFORE the cap so the cap counts the actual displayed chars.
    // The reason flows into approval prompts and SSE/TUI status surfaces;
    // because tirith specifically catches terminal-control attacks, we must
    // strip ESC / bidi / zero-width / other control chars from the
    // suspect command and finding text before reflecting them back.
    truncate(&sanitize_for_display(&combined), REASON_CAP)
}

/// Neutralize text destined for user-facing approval / status surfaces.
///
/// Tirith's findings and the truncated command can contain the very
/// terminal-control payloads tirith was scanning for (ANSI escapes, bidi
/// override codepoints, zero-width characters, hidden multiline). Rendering
/// those raw in TUI / SSE / channel approval cards would defeat the
/// integration's purpose by letting the attack reach the operator's screen
/// unchanged. This helper:
///
/// - Collapses CR / LF / TAB / VT / FF and runs of spaces into a single space.
/// - Drops C0 (U+0000–U+001F) and DEL (U+007F) controls.
/// - Drops C1 controls (U+0080–U+009F).
/// - Drops Unicode bidi controls (U+200E / U+200F, U+202A–U+202E,
///   U+2066–U+2069) and the Mongolian Free Variation Selector U+180E.
/// - Drops zero-width / invisible separators (U+200B / U+200C / U+200D /
///   U+FEFF / U+00AD / U+115F / U+1160 / U+3164).
///
/// The result is always a single line of safe-to-print characters.
fn sanitize_for_display(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_space = false;
    for c in input.chars() {
        let mapped = match c {
            // Collapse all line breaks / tabs / vertical-whitespace to one space.
            '\n' | '\r' | '\t' | '\x0B' | '\x0C' => Some(' '),
            // C0 control range (excluding the whitespace handled above).
            c if (c as u32) < 0x20 => None,
            // DEL.
            '\x7F' => None,
            // C1 control range.
            c if matches!(c as u32, 0x80..=0x9F) => None,
            // Bidi formatting controls (override / isolate).
            '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => None,
            // Mongolian Free Variation Selector (used in some bidi attacks).
            '\u{180E}' => None,
            // Zero-width spacers and joiners.
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}' => None,
            // Soft hyphen.
            '\u{00AD}' => None,
            // Hangul fillers and Hangul Filler that can hide content.
            '\u{115F}' | '\u{1160}' | '\u{3164}' => None,
            other => Some(other),
        };
        if let Some(c) = mapped {
            if c == ' ' {
                if !prev_space && !out.is_empty() {
                    out.push(' ');
                    prev_space = true;
                }
            } else {
                out.push(c);
                prev_space = false;
            }
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Truncate `s` to at most `max_chars` Unicode scalar values, appending an
/// ellipsis when content was dropped.
///
/// Per the repo's iterator-first convention, walks the string once via
/// `chars()` rather than counting length up front. This also makes the cap
/// a stable user-visible size — passing `COMMAND_TRUNC = 80` means "up to
/// 80 displayed characters" instead of "up to 80 UTF-8 bytes" (which would
/// shrink the visible cap on multi-byte inputs).
fn truncate(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

/// Adapter for inline approval call sites. Returns a three-way decision.
///
/// - Tirith disabled, non-shell tool, or no `command` parameter → [`TirithPreflightDecision::Allow`].
/// - Tirith allow → [`TirithPreflightDecision::Allow`].
/// - Tirith block / warn / warn_ack → [`TirithPreflightDecision::Approval`].
/// - Tirith operational failure under `fail_open = false` → [`TirithPreflightDecision::Deny`].
///
/// The helper deliberately knows nothing about relay channels, sessions, or
/// auto-approve flags — those are call-site concerns layered on top of the
/// returned decision.
pub async fn tirith_preflight(
    tool_name: &str,
    parameters: &serde_json::Value,
    cfg: &TirithConfig,
) -> TirithPreflightDecision {
    if !cfg.enabled {
        return TirithPreflightDecision::Allow;
    }

    if tool_name != "shell" {
        return TirithPreflightDecision::Allow;
    }

    let Some(cmd) = shell::extract_command_param(parameters) else {
        return TirithPreflightDecision::Allow;
    };

    if cmd.trim().is_empty() {
        return TirithPreflightDecision::Allow;
    }

    match check_command(&cmd, cfg).await {
        TirithVerdict::Allow => TirithPreflightDecision::Allow,
        TirithVerdict::Approvable { reason, .. } => TirithPreflightDecision::Approval { reason },
        TirithVerdict::FailClosed { summary } => TirithPreflightDecision::Deny {
            reason: format!("Tirith unavailable in fail-closed mode: {summary}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::io::Write;

    #[test]
    fn empty_bin_returns_none() {
        assert!(resolve_tirith_bin("").is_none());
        assert!(resolve_tirith_bin("   ").is_none());
    }

    #[test]
    fn missing_explicit_path_returns_none() {
        assert!(resolve_tirith_bin("/nonexistent/tirith-xyz-abc").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn explicit_path_executable_file_resolves() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("fake-tirith");
        let mut f = std::fs::File::create(&path).expect("create");
        writeln!(f, "#!/bin/sh").expect("write");
        let mut perms = std::fs::metadata(&path).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
        let resolved = resolve_tirith_bin(path.to_str().unwrap()).expect("resolve");
        assert_eq!(resolved, path);
    }

    #[cfg(unix)]
    #[test]
    fn explicit_path_non_executable_file_returns_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("not-exec");
        std::fs::write(&path, "x").expect("write");
        // No chmod +x — should be rejected.
        assert!(resolve_tirith_bin(path.to_str().unwrap()).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn directory_rejected() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(resolve_tirith_bin(tmp.path().to_str().unwrap()).is_none());
    }

    #[test]
    fn truncate_handles_multi_byte_boundary() {
        let s = "héllo wörld";
        let out = truncate(s, 4);
        assert!(out.ends_with('…'));
        assert!(out.len() <= 8);
    }

    #[test]
    fn sanitize_strips_ansi_escape_sequences() {
        // Classic ANSI red-then-reset sequence wrapped around suspicious text.
        let input = "before \x1b[31mRED\x1b[0m after";
        let out = sanitize_for_display(input);
        assert!(!out.contains('\x1b'), "ESC must not survive: {out:?}");
        assert!(
            out.contains("RED"),
            "non-control text must be preserved: {out:?}"
        );
    }

    #[test]
    fn sanitize_strips_bidi_overrides() {
        // RLO + LRO + PDF + isolate codepoints — the bidi-attack family.
        let input = "echo\u{202E}drowssap\u{202C}";
        let out = sanitize_for_display(input);
        assert!(!out.contains('\u{202E}'));
        assert!(!out.contains('\u{202C}'));
        assert_eq!(out, "echodrowssap");
    }

    #[test]
    fn sanitize_strips_zero_width_characters() {
        let input = "ls\u{200B}\u{200C}\u{200D}\u{FEFF}-la";
        let out = sanitize_for_display(input);
        assert_eq!(out, "ls-la");
    }

    #[test]
    fn sanitize_collapses_newlines_to_single_space() {
        let input = "line1\nline2\r\nline3\n\n\nline4";
        let out = sanitize_for_display(input);
        assert_eq!(out, "line1 line2 line3 line4");
    }

    #[test]
    fn sanitize_drops_c1_controls_and_del() {
        // U+0080 (C1) and U+007F (DEL) flank visible text.
        let input = "a\u{0080}b\x7Fc";
        let out = sanitize_for_display(input);
        assert_eq!(out, "abc");
    }

    #[test]
    fn build_reason_sanitizes_ansi_in_finding_text() {
        // A malicious tirith finding (or, more realistically, the user-
        // controlled command echoed back into the description) carries an
        // ANSI sequence. The reason that reaches PendingGate.description /
        // ApprovalRequested.description must arrive sanitized.
        let findings = vec![TirithFinding {
            severity: "HIGH".into(),
            title: "ansi\x1b[31mPAYLOAD\x1b[0m".into(),
            description: "trailing\x1b[2J\x1b[H reset".into(),
            ..Default::default()
        }];
        let reason = build_reason(None, &findings, "echo\x1b[31m;rm -rf /\x1b[0m");
        assert!(!reason.contains('\x1b'), "ESC must not survive: {reason:?}");
        assert!(
            reason.contains("PAYLOAD"),
            "visible text must be preserved: {reason:?}"
        );
    }

    #[test]
    fn build_reason_sanitizes_bidi_in_command() {
        let reason = build_reason(Some("flagged"), &[], "echo \u{202E}desrever\u{202C} hi");
        assert!(!reason.contains('\u{202E}'));
        assert!(!reason.contains('\u{202C}'));
    }

    #[test]
    fn build_reason_prefers_approval_description() {
        let findings = vec![TirithFinding {
            severity: "HIGH".into(),
            title: "homograph".into(),
            description: "Cyrillic in URL".into(),
            ..Default::default()
        }];
        let reason = build_reason(Some("custom description"), &findings, "echo hi");
        assert!(reason.starts_with("custom description"));
        assert!(reason.contains("echo hi"));
    }

    #[test]
    fn build_reason_falls_back_to_findings() {
        let findings = vec![TirithFinding {
            severity: "HIGH".into(),
            title: "homograph".into(),
            description: "Cyrillic in URL".into(),
            ..Default::default()
        }];
        let reason = build_reason(None, &findings, "echo hi");
        assert!(reason.contains("[HIGH]"));
        assert!(reason.contains("homograph"));
    }

    #[test]
    fn build_reason_caps_findings_in_summary() {
        let mut findings = Vec::new();
        for i in 0..10 {
            findings.push(TirithFinding {
                severity: "HIGH".into(),
                title: format!("rule-{i}"),
                ..Default::default()
            });
        }
        let reason = build_reason(None, &findings, "");
        assert!(reason.contains("rule-0"));
        assert!(reason.contains("rule-2"));
        assert!(!reason.contains("rule-3"));
    }

    #[test]
    fn build_reason_caps_total_length() {
        let long_desc = "x".repeat(2000);
        let reason = build_reason(Some(&long_desc), &[], "");
        assert!(reason.chars().count() <= REASON_CAP + 1);
    }

    #[tokio::test]
    async fn disabled_short_circuits() {
        let cfg = TirithConfig {
            enabled: false,
            ..TirithConfig::default()
        };
        let decision = tirith_preflight("shell", &serde_json::json!({"command": "ls"}), &cfg).await;
        assert!(matches!(decision, TirithPreflightDecision::Allow));
    }

    #[tokio::test]
    async fn non_shell_tool_short_circuits() {
        let cfg = TirithConfig::default();
        let decision = tirith_preflight("http", &serde_json::json!({"url": "x"}), &cfg).await;
        assert!(matches!(decision, TirithPreflightDecision::Allow));
    }

    #[tokio::test]
    async fn no_command_param_short_circuits() {
        let cfg = TirithConfig::default();
        let decision = tirith_preflight("shell", &serde_json::json!({}), &cfg).await;
        assert!(matches!(decision, TirithPreflightDecision::Allow));
    }
}
