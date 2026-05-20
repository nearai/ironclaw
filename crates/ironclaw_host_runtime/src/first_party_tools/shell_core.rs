//! Reborn-local copy of the v1 shell execution implementation.
//!
//! This intentionally duplicates the v1 shell behavior for now. The follow-up
//! cleanup sweep can consolidate the two copies behind a better long-term
//! boundary without changing the v1 tool as part of this port.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::Stdio,
    sync::LazyLock,
    time::{Duration, Instant},
};

use ironclaw_safety::sensitive_paths::is_sensitive_path;
use serde_json::Value;
use thiserror::Error;
use tokio::{io::AsyncReadExt, process::Command};

pub(super) const MAX_OUTPUT_SIZE: usize = 64 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

const SAFE_ENV_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "COLORTERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "PWD",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_RUNTIME_DIR",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "NODE_PATH",
    "NPM_CONFIG_PREFIX",
    "EDITOR",
    "VISUAL",
    "SystemRoot",
    "SYSTEMROOT",
    "ComSpec",
    "PATHEXT",
    "APPDATA",
    "LOCALAPPDATA",
    "USERPROFILE",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "WINDIR",
];

static BLOCKED_COMMANDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "rm -rf /",
        "rm -rf /*",
        ":(){ :|:& };:",
        "dd if=/dev/zero",
        "mkfs",
        "chmod -R 777 /",
        "> /dev/sda",
        "curl | sh",
        "wget | sh",
        "curl | bash",
        "wget | bash",
    ])
});

static DANGEROUS_PATTERNS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "sudo ",
        "doas ",
        " | sh",
        " | bash",
        " | zsh",
        "eval ",
        "$(curl",
        "$(wget",
        "/etc/passwd",
        "/etc/shadow",
        "~/.ssh",
        ".bash_history",
        "id_rsa",
    ]
});

const FILE_READ_COMMANDS: &[&str] = &[
    "cat", "head", "tail", "less", "more", "tac", "nl", "bat", "batcat", "cp", "mv", "scp",
    "rsync", "source", ".", "vim", "vi", "nano", "code", "strings", "xxd", "hexdump", "od", "file",
    "stat", "wc", "diff", "cmp", "tar", "zip", "gzip", "bzip2", "xz", "zstd", "base64", "grep",
    "awk", "sed",
];

#[derive(Debug, Error)]
pub(super) enum ShellExecutionError {
    #[error("invalid parameters: {0}")]
    InvalidParameters(String),
    #[error("not authorized: {0}")]
    NotAuthorized(String),
    #[error("command timed out after {0:?}")]
    Timeout(Duration),
    #[error("execution failed: {0}")]
    ExecutionFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShellExecutionRequest {
    pub command: String,
    pub workdir: Option<String>,
    pub timeout_secs: Option<u64>,
    pub extra_env: HashMap<String, String>,
}

impl ShellExecutionRequest {
    fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            workdir: None,
            timeout_secs: None,
            extra_env: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShellExecutionOutput {
    pub output: String,
    pub exit_code: i64,
    pub success: bool,
    pub sandboxed: bool,
    pub duration: Duration,
}

#[derive(Debug, Clone)]
pub(super) struct ShellExecutor {
    working_dir: Option<PathBuf>,
    timeout: Duration,
    allow_dangerous: bool,
}

impl ShellExecutor {
    pub(super) fn new() -> Self {
        Self {
            working_dir: None,
            timeout: DEFAULT_TIMEOUT,
            allow_dangerous: false,
        }
    }

    pub(super) async fn execute_direct(
        &self,
        request: ShellExecutionRequest,
    ) -> Result<ShellExecutionOutput, ShellExecutionError> {
        validate_command(&request.command, self.allow_dangerous)?;
        let cwd = request
            .workdir
            .as_deref()
            .map(PathBuf::from)
            .or_else(|| self.working_dir.clone())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let timeout = request
            .timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(self.timeout);
        let start = Instant::now();
        let (output, exit_code) =
            execute_direct_command(&request.command, &cwd, timeout, &request.extra_env).await?;
        Ok(ShellExecutionOutput {
            output,
            exit_code: i64::from(exit_code),
            success: exit_code == 0,
            sandboxed: false,
            duration: start.elapsed(),
        })
    }
}

pub(super) fn parse_shell_request(
    params: &Value,
) -> Result<ShellExecutionRequest, ShellExecutionError> {
    let command = params
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ShellExecutionError::InvalidParameters("missing 'command' parameter".to_string())
        })?;
    let mut request = ShellExecutionRequest::new(command.to_string());
    request.workdir = parse_workdir(params)?;
    request.timeout_secs = parse_timeout(params)?;
    Ok(request)
}

fn parse_workdir(params: &Value) -> Result<Option<String>, ShellExecutionError> {
    match params.get("workdir") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let value = value.as_str().ok_or_else(|| {
                ShellExecutionError::InvalidParameters("workdir must be a string".to_string())
            })?;
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
    }
}

fn parse_timeout(params: &Value) -> Result<Option<u64>, ShellExecutionError> {
    match params.get("timeout") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let value = value.as_u64().ok_or_else(|| {
                ShellExecutionError::InvalidParameters(
                    "timeout must be a positive integer number of seconds".to_string(),
                )
            })?;
            if value == 0 {
                return Err(ShellExecutionError::InvalidParameters(
                    "timeout must be greater than 0".to_string(),
                ));
            }
            Ok(Some(value))
        }
    }
}

fn validate_command(cmd: &str, allow_dangerous: bool) -> Result<(), ShellExecutionError> {
    if let Some(reason) = blocked_reason(cmd, allow_dangerous) {
        return Err(ShellExecutionError::NotAuthorized(format!(
            "{}: {}",
            reason,
            truncate_for_error(cmd)
        )));
    }
    if let Some(reason) = detect_command_injection(cmd) {
        return Err(ShellExecutionError::NotAuthorized(format!(
            "Command injection detected ({}): {}",
            reason,
            truncate_for_error(cmd)
        )));
    }
    if let Some(reason) = check_sensitive_file_access(cmd) {
        return Err(ShellExecutionError::NotAuthorized(reason));
    }
    Ok(())
}

fn blocked_reason(cmd: &str, allow_dangerous: bool) -> Option<&'static str> {
    let normalized = normalize_command_text(cmd);
    for blocked in BLOCKED_COMMANDS.iter() {
        if normalized.contains(&normalize_command_text(blocked)) {
            return Some("Command contains blocked pattern");
        }
    }
    if !allow_dangerous {
        for pattern in DANGEROUS_PATTERNS.iter() {
            if normalized.contains(&normalize_command_text(pattern)) {
                return Some("Command contains potentially dangerous pattern");
            }
        }
    }
    None
}

fn normalize_command_text(cmd: &str) -> String {
    cmd.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn detect_command_injection(cmd: &str) -> Option<&'static str> {
    if cmd.bytes().any(|b| b == 0) {
        return Some("null byte in command");
    }

    let lower = cmd.to_lowercase();
    if (lower.contains("base64 -d") || lower.contains("base64 --decode"))
        && contains_shell_pipe(&lower)
    {
        return Some("base64 decode piped to shell");
    }
    if (lower.contains("printf") || lower.contains("echo -e") || lower.contains("echo $'"))
        && (lower.contains("\\x") || lower.contains("\\0"))
        && contains_shell_pipe(&lower)
    {
        return Some("encoded escape sequences piped to shell");
    }
    if (lower.contains("xxd -r") || has_command_token(&lower, "od ")) && contains_shell_pipe(&lower)
    {
        return Some("binary decode piped to shell");
    }
    if (has_command_token(&lower, "dig ")
        || has_command_token(&lower, "nslookup ")
        || has_command_token(&lower, "host "))
        && (lower.contains("$(") || lower.contains('`'))
    {
        return Some("potential DNS exfiltration via command substitution");
    }
    if (has_command_token(&lower, "nc ")
        || has_command_token(&lower, "ncat ")
        || has_command_token(&lower, "netcat "))
        && (lower.contains('|') || lower.contains('<'))
    {
        return Some("netcat with data piping");
    }
    if lower.contains("curl")
        && (lower.contains("-d @")
            || lower.contains("-d@")
            || lower.contains("--data @")
            || lower.contains("--data-binary @")
            || lower.contains("--upload-file"))
    {
        return Some("curl posting file contents");
    }
    if lower.contains("wget") && lower.contains("--post-file") {
        return Some("wget posting file contents");
    }
    if (lower.contains("| rev") || lower.contains("|rev")) && contains_shell_pipe(&lower) {
        return Some("string reversal piped to shell");
    }
    None
}

fn contains_shell_pipe(lower: &str) -> bool {
    has_pipe_to(lower, "sh")
        || has_pipe_to(lower, "bash")
        || has_pipe_to(lower, "zsh")
        || has_pipe_to(lower, "dash")
        || has_pipe_to(lower, "/bin/sh")
        || has_pipe_to(lower, "/bin/bash")
}

fn has_pipe_to(lower: &str, shell: &str) -> bool {
    for prefix in ["| ", "|"] {
        let pattern = format!("{prefix}{shell}");
        for (i, _) in lower.match_indices(&pattern) {
            let end = i + pattern.len();
            if end >= lower.len()
                || matches!(
                    lower.as_bytes()[end],
                    b' ' | b'\t' | b'\n' | b';' | b'|' | b'&' | b')'
                )
            {
                return true;
            }
        }
    }
    false
}

fn has_command_token(lower: &str, token: &str) -> bool {
    for (i, _) in lower.match_indices(token) {
        if i == 0 {
            return true;
        }
        let before = lower.as_bytes()[i - 1];
        if matches!(before, b' ' | b'\t' | b'|' | b';' | b'&' | b'\n' | b'(') {
            return true;
        }
    }
    false
}

fn check_sensitive_file_access(cmd: &str) -> Option<String> {
    for segment in split_shell_segments(cmd) {
        let segment = segment.trim();
        if let Some(reason) = check_segment_file_commands(segment) {
            return Some(reason);
        }
        if let Some(reason) = check_redirect_target(segment, '<', "input redirection") {
            return Some(reason);
        }
        if let Some(reason) = check_redirect_target(segment, '>', "output redirection") {
            return Some(reason);
        }
    }
    None
}

fn split_shell_segments(cmd: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut chars = cmd.char_indices().peekable();
    let mut quote = ShellQuote::None;
    let mut escaped = false;

    while let Some((i, ch)) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, ch) {
            (_, '\\') => {
                escaped = true;
            }
            (ShellQuote::None, '\'') => quote = ShellQuote::Single,
            (ShellQuote::Single, '\'') => quote = ShellQuote::None,
            (ShellQuote::None, '"') => quote = ShellQuote::Double,
            (ShellQuote::Double, '"') => quote = ShellQuote::None,
            (ShellQuote::None, ';' | '|') => {
                segments.push(&cmd[start..i]);
                if ch == '|' && matches!(chars.peek(), Some((_, '|'))) {
                    chars.next();
                    start = i + 2;
                } else {
                    start = i + ch.len_utf8();
                }
            }
            (ShellQuote::None, '&') if matches!(chars.peek(), Some((_, '&'))) => {
                segments.push(&cmd[start..i]);
                chars.next();
                start = i + 2;
            }
            _ => {}
        }
    }
    segments.push(&cmd[start..]);
    segments
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellQuote {
    None,
    Single,
    Double,
}

fn check_segment_file_commands(segment: &str) -> Option<String> {
    let segment = segment.trim().trim_start_matches('<').trim();
    let tokens = shell_words(segment);
    let mut tokens = tokens.iter().map(String::as_str);
    let cmd_name = tokens.next()?;
    let base_cmd = cmd_name.rsplit('/').next().unwrap_or(cmd_name);
    if !FILE_READ_COMMANDS
        .iter()
        .any(|&fc| base_cmd.eq_ignore_ascii_case(fc))
    {
        return None;
    }
    for token in tokens {
        if token.starts_with('-') {
            if let Some(eq_pos) = token.find('=') {
                let value = &token[eq_pos + 1..];
                let expanded = expand_tilde(strip_shell_quotes(value));
                if is_sensitive_path(&expanded) {
                    return Some(format!(
                        "Access denied: flag value in '{}' targets a sensitive credential path",
                        token
                    ));
                }
            }
            continue;
        }
        let unquoted = strip_shell_quotes(token);
        let expanded = expand_tilde(unquoted);
        if is_sensitive_path(&expanded) {
            return Some(format!(
                "Access denied: '{}' targets a sensitive credential path",
                unquoted
            ));
        }
    }
    None
}

fn strip_shell_quotes(token: &str) -> &str {
    let bytes = token.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &token[1..token.len() - 1];
        }
    }
    token
}

fn check_redirect_target(segment: &str, operator: char, label: &str) -> Option<String> {
    for target in redirect_targets(segment, operator) {
        let unquoted = strip_shell_quotes(&target);
        let expanded = expand_tilde(unquoted);
        if is_sensitive_path(&expanded) {
            return Some(format!(
                "Access denied: {} targets sensitive path '{}'",
                label, unquoted
            ));
        }
    }
    None
}

fn shell_words(segment: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = ShellQuote::None;
    let mut escaped = false;
    for ch in segment.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match (quote, ch) {
            (_, '\\') => escaped = true,
            (ShellQuote::None, '\'') => quote = ShellQuote::Single,
            (ShellQuote::Single, '\'') => quote = ShellQuote::None,
            (ShellQuote::None, '"') => quote = ShellQuote::Double,
            (ShellQuote::Double, '"') => quote = ShellQuote::None,
            (ShellQuote::None, ch) if ch.is_whitespace() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn redirect_targets(segment: &str, operator: char) -> Vec<String> {
    let mut targets = Vec::new();
    let mut chars = segment.char_indices().peekable();
    let mut quote = ShellQuote::None;
    let mut escaped = false;
    while let Some((i, ch)) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, ch) {
            (_, '\\') => escaped = true,
            (ShellQuote::None, '\'') => quote = ShellQuote::Single,
            (ShellQuote::Single, '\'') => quote = ShellQuote::None,
            (ShellQuote::None, '"') => quote = ShellQuote::Double,
            (ShellQuote::Double, '"') => quote = ShellQuote::None,
            (ShellQuote::None, ch) if ch == operator => {
                let mut after_start = i + ch.len_utf8();
                if operator == '>' && matches!(chars.peek(), Some((_, '>'))) {
                    chars.next();
                    after_start += 1;
                }
                if operator == '<' && matches!(chars.peek(), Some((_, '('))) {
                    chars.next();
                    if let Some(close) = segment[after_start + 1..].find(')') {
                        targets.extend(shell_words(
                            &segment[after_start + 1..after_start + 1 + close],
                        ));
                    }
                    continue;
                }
                if let Some(target) = shell_words(&segment[after_start..]).into_iter().next() {
                    targets.push(target);
                }
            }
            _ => {}
        }
    }
    targets
}

fn expand_tilde(token: &str) -> PathBuf {
    if let (Some(rest), Some(home)) = (token.strip_prefix("~/"), dirs::home_dir()) {
        return home.join(rest);
    }
    PathBuf::from(token)
}

async fn execute_direct_command(
    cmd: &str,
    workdir: &PathBuf,
    timeout: Duration,
    extra_env: &HashMap<String, String>,
) -> Result<(String, i32), ShellExecutionError> {
    let mut command = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", cmd]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        c
    };

    #[cfg(unix)]
    command.process_group(0);

    command.env_clear();
    for var in SAFE_ENV_VARS {
        if let Ok(val) = std::env::var(var) {
            command.env(var, val);
        }
    }
    command.envs(extra_env);
    command
        .current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|e| {
        ShellExecutionError::ExecutionFailed(format!("Failed to spawn command: {}", e))
    })?;

    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();

    let result = tokio::time::timeout(timeout, async {
        let stdout_fut = async {
            if let Some(out) = stdout_handle {
                read_stream_limited(out).await
            } else {
                String::new()
            }
        };

        let stderr_fut = async {
            if let Some(err) = stderr_handle {
                read_stream_limited(err).await
            } else {
                String::new()
            }
        };

        let (stdout, stderr, wait_result) = tokio::join!(stdout_fut, stderr_fut, child.wait());
        let status = wait_result?;
        let output = if stderr.is_empty() {
            stdout
        } else if stdout.is_empty() {
            stderr
        } else {
            format!("{}\n\n--- stderr ---\n{}", stdout, stderr)
        };
        Ok::<_, std::io::Error>((output, status.code().unwrap_or(-1)))
    })
    .await;

    match result {
        Ok(Ok((output, code))) => Ok((truncate_output(&output), code)),
        Ok(Err(e)) => Err(ShellExecutionError::ExecutionFailed(format!(
            "Command execution failed: {}",
            e
        ))),
        Err(_) => {
            terminate_child_tree(&mut child).await;
            Err(ShellExecutionError::Timeout(timeout))
        }
    }
}

async fn terminate_child_tree(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // SAFETY: Child was spawned into its own process group with pgid == pid.
        // Negative pid targets only that process group; result is best-effort.
        unsafe {
            let _ = kill_process_group(-(pid as i32), SIGKILL);
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[cfg(unix)]
const SIGKILL: i32 = 9;

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "kill"]
    fn kill_process_group(pid: i32, sig: i32) -> i32;
}

async fn read_stream_limited<R>(mut stream: R) -> String
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    (&mut stream)
        .take((MAX_OUTPUT_SIZE + 1) as u64)
        .read_to_end(&mut buf)
        .await
        .ok();
    tokio::io::copy(&mut stream, &mut tokio::io::sink())
        .await
        .ok();
    let output = String::from_utf8_lossy(&buf).to_string();
    truncate_output(&output)
}

fn truncate_output(s: &str) -> String {
    if s.len() <= MAX_OUTPUT_SIZE {
        s.to_string()
    } else {
        let half = MAX_OUTPUT_SIZE / 2;
        let head_end = floor_char_boundary(s, half);
        let tail_start = floor_char_boundary(s, s.len() - half);
        format!(
            "{}\n\n... [truncated {} bytes] ...\n\n{}",
            &s[..head_end],
            s.len() - MAX_OUTPUT_SIZE,
            &s[tail_start..]
        )
    }
}

fn truncate_for_error(s: &str) -> String {
    if s.chars().count() <= 100 {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(100).collect::<String>())
    }
}

fn floor_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut i = pos;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_shell_segments_ignores_operators_inside_quotes() {
        assert_eq!(
            split_shell_segments("echo 'a;b' && cat ~/.ssh/id_rsa").len(),
            2
        );
        assert_eq!(split_shell_segments("cat \"a;rm -rf /\"").len(), 1);
    }

    #[test]
    fn blocked_reason_collapses_whitespace() {
        assert_eq!(
            blocked_reason("rm    -rf    /", false),
            Some("Command contains blocked pattern")
        );
    }

    #[test]
    fn sensitive_path_detection_checks_shell_aware_tokens() {
        assert!(check_sensitive_file_access("cat \"~/server key.pem\"").is_some());
        assert!(check_sensitive_file_access("echo hi > '~/.ssh/config'").is_some());
    }
}
