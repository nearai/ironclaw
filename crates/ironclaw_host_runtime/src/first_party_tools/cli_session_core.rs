//! Pure parsing/validation/command-construction for `builtin.cli_session`.
//! Placement-neutral: the built tmux invocation is handed to
//! `RuntimeProcessPort::run_command` unmodified by `cli_session::dispatch`,
//! exactly like `shell_core::parse_shell_request` feeds `builtin.shell`.

use serde_json::Value;
use thiserror::Error;

use crate::sandbox_process::shell_single_quote;

const SESSION_MARKER: &str = "---IRONCLAW-CLI-SESSIONS---";
const MAX_SESSION_NAME_LEN: usize = 64;
const SESSION_NAME_PREFIX: &str = "ic-";

#[derive(Debug, Error, PartialEq, Eq)]
pub(super) enum CliSessionError {
    #[error("invalid parameters: {0}")]
    InvalidParameters(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CliSessionAction {
    Start,
    Send,
    Read,
    Kill,
}

impl CliSessionAction {
    fn parse(value: &str) -> Result<Self, CliSessionError> {
        match value {
            "start" => Ok(Self::Start),
            "send" => Ok(Self::Send),
            "read" => Ok(Self::Read),
            "kill" => Ok(Self::Kill),
            other => Err(CliSessionError::InvalidParameters(format!(
                "action must be one of start, send, read, kill; got {other:?}"
            ))),
        }
    }

    fn includes_session_footer(self) -> bool {
        matches!(self, Self::Start | Self::Read)
    }
}

/// Validated, namespaced tmux session identifier. The `ic-` prefix keeps
/// model-created sessions distinguishable from any other tmux usage inside
/// the same container; the charset restriction is defense-in-depth
/// alongside `shell_single_quote`, which quotes every argument regardless.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CliSessionName(String);

impl CliSessionName {
    pub(super) fn parse(raw: &str) -> Result<Self, CliSessionError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(CliSessionError::InvalidParameters(
                "session must not be empty".to_string(),
            ));
        }
        if trimmed.chars().count() > MAX_SESSION_NAME_LEN {
            return Err(CliSessionError::InvalidParameters(format!(
                "session must be at most {MAX_SESSION_NAME_LEN} characters"
            )));
        }
        let first = trimmed.chars().next().expect("checked non-empty above");
        if !first.is_ascii_alphanumeric() {
            return Err(CliSessionError::InvalidParameters(
                "session must start with an ASCII letter or digit".to_string(),
            ));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(CliSessionError::InvalidParameters(
                "session may contain only ASCII letters, digits, '-', and '_'".to_string(),
            ));
        }
        Ok(Self(format!("{SESSION_NAME_PREFIX}{trimmed}")))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CliSessionRequest {
    pub action: CliSessionAction,
    pub session: CliSessionName,
    pub command: Option<String>,
    pub text: Option<String>,
}

pub(super) fn parse_cli_session_request(
    params: &Value,
) -> Result<CliSessionRequest, CliSessionError> {
    let action = params.get("action").and_then(Value::as_str).ok_or_else(|| {
        CliSessionError::InvalidParameters("missing 'action' parameter".to_string())
    })?;
    let action = CliSessionAction::parse(action)?;
    let session = params.get("session").and_then(Value::as_str).ok_or_else(|| {
        CliSessionError::InvalidParameters("missing 'session' parameter".to_string())
    })?;
    let session = CliSessionName::parse(session)?;
    let command = optional_nonempty_string(params, "command")?;
    let text = optional_nonempty_string(params, "text")?;
    match action {
        CliSessionAction::Start if command.is_none() => {
            return Err(CliSessionError::InvalidParameters(
                "'command' is required for action 'start'".to_string(),
            ));
        }
        CliSessionAction::Send if text.is_none() => {
            return Err(CliSessionError::InvalidParameters(
                "'text' is required for action 'send'".to_string(),
            ));
        }
        _ => {}
    }
    Ok(CliSessionRequest {
        action,
        session,
        command,
        text,
    })
}

fn optional_nonempty_string(params: &Value, key: &str) -> Result<Option<String>, CliSessionError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let value = value.as_str().ok_or_else(|| {
                CliSessionError::InvalidParameters(format!("{key} must be a string"))
            })?;
            if value.contains('\0') {
                return Err(CliSessionError::InvalidParameters(format!(
                    "{key} must not contain a NUL byte"
                )));
            }
            Ok(Some(value.to_string()))
        }
    }
}

/// Build the exact `sh -c` command string handed to
/// `RuntimeProcessPort::run_command`. `start`/`read` append a `tmux
/// list-sessions` status footer after `SESSION_MARKER`; `dispatch` splits on
/// that marker to populate `active_sessions`. Every tmux argument — session
/// name AND free-text — is quoted via `shell_single_quote` (Phase A Task A3,
/// `crate::sandbox_process`) so shell metacharacters in either can never
/// leave the quoted literal.
pub(super) fn build_tmux_command(request: &CliSessionRequest) -> String {
    let session = shell_single_quote(request.session.as_str());
    let primary = match request.action {
        CliSessionAction::Start => {
            let command =
                shell_single_quote(request.command.as_deref().expect("validated by parse"));
            format!("tmux new-session -d -s {session} {command}")
        }
        CliSessionAction::Send => {
            let text = shell_single_quote(request.text.as_deref().expect("validated by parse"));
            format!(
                "tmux send-keys -t {session} -l -- {text} && tmux send-keys -t {session} Enter"
            )
        }
        CliSessionAction::Read => format!("tmux capture-pane -t {session} -p"),
        CliSessionAction::Kill => format!("tmux kill-session -t {session}"),
    };
    if request.action.includes_session_footer() {
        format!(
            "{primary}; printf '\\n{SESSION_MARKER}\\n'; tmux list-sessions -F '#S' 2>/dev/null || true"
        )
    } else {
        primary
    }
}

/// Split a captured `RuntimeProcessPort` output into the primary action
/// output and the `tmux list-sessions`-derived session list, when the built
/// command included the footer. Returns `(output, None)` untouched when no
/// marker is present (send/kill, or a start/read exec that failed before the
/// footer ran).
pub(super) fn split_session_footer(output: &str) -> (&str, Option<Vec<String>>) {
    let Some((primary, footer)) = output.split_once(SESSION_MARKER) else {
        return (output, None);
    };
    let primary = primary.trim_end_matches('\n');
    let sessions = footer
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != SESSION_MARKER)
        .map(str::to_string)
        .collect();
    (primary, Some(sessions))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_name_namespaces_and_accepts_valid_charset() {
        let name = CliSessionName::parse("dev-server_1").unwrap();
        assert_eq!(name.as_str(), "ic-dev-server_1");
    }

    #[test]
    fn session_name_rejects_empty_too_long_and_bad_first_char() {
        assert!(matches!(
            CliSessionName::parse(""),
            Err(CliSessionError::InvalidParameters(_))
        ));
        assert!(matches!(
            CliSessionName::parse(&"a".repeat(65)),
            Err(CliSessionError::InvalidParameters(_))
        ));
        assert!(matches!(
            CliSessionName::parse("-leading-dash"),
            Err(CliSessionError::InvalidParameters(_))
        ));
    }

    #[test]
    fn session_name_rejects_shell_metacharacters() {
        for bad in [
            "sess; rm -rf /",
            "sess$(whoami)",
            "sess`id`",
            "sess with space",
            "sess/../etc",
            "sess'quote",
        ] {
            assert!(
                CliSessionName::parse(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    // Single-quote escaping itself is pinned by Phase A Task A3's tests on
    // `crate::sandbox_process::shell_single_quote` — not duplicated here.
    // `build_tmux_command`'s tests below still exercise quoting behavior
    // end-to-end (embedded quotes in session names / free text), just via
    // the imported helper rather than a local reimplementation.

    #[test]
    fn build_tmux_command_for_start_appends_session_footer() {
        let request = CliSessionRequest {
            action: CliSessionAction::Start,
            session: CliSessionName::parse("devserver").unwrap(),
            command: Some("npm run dev".to_string()),
            text: None,
        };
        assert_eq!(
            build_tmux_command(&request),
            "tmux new-session -d -s 'ic-devserver' 'npm run dev'; \
             printf '\\n---IRONCLAW-CLI-SESSIONS---\\n'; \
             tmux list-sessions -F '#S' 2>/dev/null || true"
        );
    }

    #[test]
    fn build_tmux_command_for_send_quotes_injected_text_literally_and_omits_footer() {
        let request = CliSessionRequest {
            action: CliSessionAction::Send,
            session: CliSessionName::parse("devserver").unwrap(),
            command: None,
            text: Some("echo 'hi'; rm -rf /".to_string()),
        };
        assert_eq!(
            build_tmux_command(&request),
            "tmux send-keys -t 'ic-devserver' -l -- 'echo '\\''hi'\\''; rm -rf /' \
             && tmux send-keys -t 'ic-devserver' Enter"
        );
    }

    #[test]
    fn build_tmux_command_for_read_appends_session_footer() {
        let request = CliSessionRequest {
            action: CliSessionAction::Read,
            session: CliSessionName::parse("devserver").unwrap(),
            command: None,
            text: None,
        };
        assert_eq!(
            build_tmux_command(&request),
            "tmux capture-pane -t 'ic-devserver' -p; \
             printf '\\n---IRONCLAW-CLI-SESSIONS---\\n'; \
             tmux list-sessions -F '#S' 2>/dev/null || true"
        );
    }

    #[test]
    fn build_tmux_command_for_kill_has_no_footer() {
        let request = CliSessionRequest {
            action: CliSessionAction::Kill,
            session: CliSessionName::parse("devserver").unwrap(),
            command: None,
            text: None,
        };
        assert_eq!(
            build_tmux_command(&request),
            "tmux kill-session -t 'ic-devserver'"
        );
    }

    #[test]
    fn parse_cli_session_request_requires_command_for_start_and_text_for_send() {
        let missing_command = parse_cli_session_request(&serde_json::json!({
            "action": "start", "session": "s"
        }))
        .unwrap_err();
        assert!(matches!(missing_command, CliSessionError::InvalidParameters(_)));

        let missing_text = parse_cli_session_request(&serde_json::json!({
            "action": "send", "session": "s"
        }))
        .unwrap_err();
        assert!(matches!(missing_text, CliSessionError::InvalidParameters(_)));
    }

    #[test]
    fn parse_cli_session_request_rejects_nul_bytes_in_text() {
        let error = parse_cli_session_request(&serde_json::json!({
            "action": "send", "session": "s", "text": "hi\0there"
        }))
        .unwrap_err();
        assert!(matches!(error, CliSessionError::InvalidParameters(_)));
    }

    #[test]
    fn split_session_footer_extracts_sessions_and_trims_trailing_newline_from_primary() {
        let raw = "line1\nline2\n---IRONCLAW-CLI-SESSIONS---\nic-devserver\nic-other\n";
        let (primary, sessions) = split_session_footer(raw);
        assert_eq!(primary, "line1\nline2");
        assert_eq!(
            sessions,
            Some(vec!["ic-devserver".to_string(), "ic-other".to_string()])
        );
    }

    #[test]
    fn split_session_footer_returns_none_when_marker_absent() {
        let (primary, sessions) = split_session_footer("no marker here");
        assert_eq!(primary, "no marker here");
        assert_eq!(sessions, None);
    }

    #[test]
    fn split_session_footer_treats_first_marker_occurrence_as_the_boundary_even_if_command_output_repeats_it()
     {
        // Accepted-risk pin (not a bug to fix here): `split_once` finds the
        // FIRST occurrence of SESSION_MARKER. If the wrapped command's own
        // captured pane output happens to print the literal marker string
        // (e.g. `cat`-ing a file that contains it), `split_session_footer`
        // treats that occurrence as the boundary, and everything after it —
        // including the real trailing `tmux list-sessions` footer — is
        // folded into `active_sessions` alongside whatever text followed the
        // fake marker. Model-facing `output` is truncated at that point.
        let raw = "real output\n---IRONCLAW-CLI-SESSIONS---\nnot a real session name\n---IRONCLAW-CLI-SESSIONS---\nic-real\n";
        let (primary, sessions) = split_session_footer(raw);
        assert_eq!(primary, "real output");
        assert_eq!(
            sessions,
            Some(vec![
                "not a real session name".to_string(),
                "ic-real".to_string(),
            ])
        );
    }
}
