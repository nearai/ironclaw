//! Configuration builder for Claude CLI process invocation.
//!
//! Provides a builder pattern for constructing CLI arguments across different
//! execution modes (oneshot, sidecar, resume).

use std::path::PathBuf;

/// Permission mode for the Claude CLI.
#[derive(Debug, Clone)]
pub enum PermissionMode {
    /// Default mode - prompts for permissions.
    Default,
    /// Accept all edit operations.
    AcceptEdits,
    /// Bypass all permission checks (use with caution).
    DangerouslySkipPermissions,
}

/// Configuration for spawning Claude CLI processes.
///
/// Use the builder methods to configure, then call one of the `*_args()` methods
/// to generate the appropriate CLI arguments for the desired execution mode.
///
/// ```rust,no_run
/// use ironclaw::claude_cli::config::{ClaudeCodeConfig, PermissionMode};
///
/// let config = ClaudeCodeConfig::new()
///     .model("claude-sonnet-4-5-20250929")
///     .max_turns(50)
///     .cwd("/workspace")
///     .dangerously_skip_permissions();
///
/// let args = config.oneshot_args("Fix the bug in main.rs");
/// ```
#[derive(Debug, Clone)]
pub struct ClaudeCodeConfig {
    /// Path to the `claude` binary. Defaults to `"claude"`.
    pub binary: String,
    /// Model to use.
    pub model: Option<String>,
    /// Maximum agentic turns.
    pub max_turns: Option<u32>,
    /// System prompt override.
    pub system_prompt: Option<String>,
    /// Appended system prompt.
    pub append_system_prompt: Option<String>,
    /// Working directory for the process.
    pub cwd: Option<PathBuf>,
    /// Allowed tools.
    pub allowed_tools: Vec<String>,
    /// Disallowed tools.
    pub disallowed_tools: Vec<String>,
    /// Permission mode.
    pub permission_mode: Option<PermissionMode>,
    /// Whether to emit verbose output.
    pub verbose: bool,
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            binary: "claude".to_string(),
            model: None,
            max_turns: None,
            system_prompt: None,
            append_system_prompt: None,
            cwd: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            permission_mode: None,
            verbose: false,
        }
    }
}

impl ClaudeCodeConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn binary(mut self, binary: impl Into<String>) -> Self {
        self.binary = binary.into();
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn max_turns(mut self, n: u32) -> Self {
        self.max_turns = Some(n);
        self
    }

    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn append_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.append_system_prompt = Some(prompt.into());
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn allowed_tools(mut self, tools: Vec<String>) -> Self {
        self.allowed_tools = tools;
        self
    }

    pub fn disallowed_tools(mut self, tools: Vec<String>) -> Self {
        self.disallowed_tools = tools;
        self
    }

    pub fn permission_mode(mut self, mode: PermissionMode) -> Self {
        self.permission_mode = Some(mode);
        self
    }

    pub fn dangerously_skip_permissions(self) -> Self {
        self.permission_mode(PermissionMode::DangerouslySkipPermissions)
    }

    pub fn verbose(mut self, v: bool) -> Self {
        self.verbose = v;
        self
    }

    /// Build CLI arguments for oneshot mode: `-p "prompt" --output-format stream-json`.
    pub fn oneshot_args(&self, prompt: &str) -> Vec<String> {
        let mut args = vec![
            "-p".to_string(),
            prompt.to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
        ];
        self.push_common_args(&mut args);
        args
    }

    /// Build CLI arguments for sidecar (interactive) mode with stdin/stdout streaming.
    pub fn sidecar_args(&self) -> Vec<String> {
        let mut args = vec![
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--input-format".to_string(),
            "stream-json".to_string(),
        ];
        self.push_common_args(&mut args);
        args
    }

    /// Build CLI arguments for resume mode: `--resume <session_id> -p "prompt"`.
    pub fn resume_args(&self, session_id: &str, prompt: &str) -> Vec<String> {
        let mut args = vec![
            "-p".to_string(),
            prompt.to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--resume".to_string(),
            session_id.to_string(),
        ];
        self.push_common_args(&mut args);
        args
    }

    fn push_common_args(&self, args: &mut Vec<String>) {
        if let Some(ref model) = self.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }
        if let Some(max_turns) = self.max_turns {
            args.push("--max-turns".to_string());
            args.push(max_turns.to_string());
        }
        if let Some(ref prompt) = self.system_prompt {
            args.push("--system-prompt".to_string());
            args.push(prompt.clone());
        }
        if let Some(ref prompt) = self.append_system_prompt {
            args.push("--append-system-prompt".to_string());
            args.push(prompt.clone());
        }
        for tool in &self.allowed_tools {
            args.push("--allowedTools".to_string());
            args.push(tool.clone());
        }
        for tool in &self.disallowed_tools {
            args.push("--disallowedTools".to_string());
            args.push(tool.clone());
        }
        if let Some(ref mode) = self.permission_mode {
            match mode {
                PermissionMode::Default => {}
                PermissionMode::AcceptEdits => {
                    args.push("--permission-mode".to_string());
                    args.push("accept-edits".to_string());
                }
                PermissionMode::DangerouslySkipPermissions => {
                    args.push("--dangerously-skip-permissions".to_string());
                }
            }
        }
        if self.verbose {
            args.push("--verbose".to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ClaudeCodeConfig::new();
        assert_eq!(config.binary, "claude");
        assert!(config.model.is_none());
        assert!(config.max_turns.is_none());
        assert!(!config.verbose);
    }

    #[test]
    fn test_oneshot_args_minimal() {
        let config = ClaudeCodeConfig::new();
        let args = config.oneshot_args("hello");
        assert_eq!(args, vec!["-p", "hello", "--output-format", "stream-json"]);
    }

    #[test]
    fn test_oneshot_args_full() {
        let config = ClaudeCodeConfig::new()
            .model("claude-sonnet-4-5-20250929")
            .max_turns(10)
            .dangerously_skip_permissions();
        let args = config.oneshot_args("do something");
        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"do something".to_string()));
        assert!(args.contains(&"--model".to_string()));
        assert!(args.contains(&"claude-sonnet-4-5-20250929".to_string()));
        assert!(args.contains(&"--max-turns".to_string()));
        assert!(args.contains(&"10".to_string()));
        assert!(args.contains(&"--dangerously-skip-permissions".to_string()));
    }

    #[test]
    fn test_sidecar_args() {
        let config = ClaudeCodeConfig::new().model("test-model");
        let args = config.sidecar_args();
        assert!(args.contains(&"--output-format".to_string()));
        assert!(args.contains(&"stream-json".to_string()));
        assert!(args.contains(&"--input-format".to_string()));
        assert!(args.contains(&"--model".to_string()));
        assert!(args.contains(&"test-model".to_string()));
        // No -p flag in sidecar mode
        assert!(!args.contains(&"-p".to_string()));
    }

    #[test]
    fn test_resume_args() {
        let config = ClaudeCodeConfig::new();
        let args = config.resume_args("session-42", "continue work");
        assert!(args.contains(&"--resume".to_string()));
        assert!(args.contains(&"session-42".to_string()));
        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"continue work".to_string()));
    }

    #[test]
    fn test_builder_chain() {
        let config = ClaudeCodeConfig::new()
            .binary("/usr/local/bin/claude")
            .model("test")
            .max_turns(5)
            .system_prompt("You are helpful")
            .append_system_prompt("Be concise")
            .cwd("/tmp")
            .allowed_tools(vec!["Bash".to_string()])
            .disallowed_tools(vec!["Write".to_string()])
            .verbose(true);

        assert_eq!(config.binary, "/usr/local/bin/claude");
        assert_eq!(config.model.as_deref(), Some("test"));
        assert_eq!(config.max_turns, Some(5));
        assert_eq!(config.system_prompt.as_deref(), Some("You are helpful"));
        assert_eq!(config.append_system_prompt.as_deref(), Some("Be concise"));
        assert_eq!(config.cwd.as_ref().unwrap().to_str(), Some("/tmp"));
        assert_eq!(config.allowed_tools, vec!["Bash"]);
        assert_eq!(config.disallowed_tools, vec!["Write"]);
        assert!(config.verbose);
    }

    #[test]
    fn test_allowed_tools_in_args() {
        let config =
            ClaudeCodeConfig::new().allowed_tools(vec!["Bash".to_string(), "Read".to_string()]);
        let args = config.oneshot_args("test");
        // Each tool gets its own --allowedTools flag
        let tool_flag_count = args
            .iter()
            .filter(|a| *a == "--allowedTools")
            .count();
        assert_eq!(tool_flag_count, 2);
        assert!(args.contains(&"Bash".to_string()));
        assert!(args.contains(&"Read".to_string()));
    }

    #[test]
    fn test_accept_edits_permission() {
        let config = ClaudeCodeConfig::new().permission_mode(PermissionMode::AcceptEdits);
        let args = config.oneshot_args("test");
        assert!(args.contains(&"--permission-mode".to_string()));
        assert!(args.contains(&"accept-edits".to_string()));
    }

    #[test]
    fn test_default_permission_no_flag() {
        let config = ClaudeCodeConfig::new().permission_mode(PermissionMode::Default);
        let args = config.oneshot_args("test");
        assert!(!args.contains(&"--permission-mode".to_string()));
        assert!(!args.contains(&"--dangerously-skip-permissions".to_string()));
    }
}
