//! Submission types for the turn-based agent loop.
//!
//! Submissions are the different types of input the agent can receive
//! and process as part of the turn-based development loop.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Parses user input into Submission types.
pub struct SubmissionParser;

impl SubmissionParser {
    /// Parse message content into a Submission.
    ///
    /// If `skill_commands` is provided (list of registered skill command names),
    /// unrecognized `/foo` commands will be checked against it to enable
    /// `/review <args>` style skill activation.
    pub fn parse(content: &str) -> Submission {
        Self::parse_with_skill_commands(content, &[])
    }

    /// Parse with awareness of registered skill slash commands.
    pub fn parse_with_skill_commands(content: &str, skill_commands: &[String]) -> Submission {
        let trimmed = content.trim();
        let lower = trimmed.to_lowercase();

        // Control commands (exact match or prefix)
        if lower == "/undo" {
            return Submission::Undo;
        }
        if lower == "/redo" {
            return Submission::Redo;
        }
        if lower == "/interrupt" || lower == "/stop" {
            return Submission::Interrupt;
        }
        if lower == "/compact" {
            return Submission::Compact;
        }
        if lower == "/clear" {
            return Submission::Clear;
        }
        if lower == "/heartbeat" {
            return Submission::Heartbeat;
        }
        if lower == "/summarize" || lower == "/summary" {
            return Submission::Summarize;
        }
        if lower == "/suggest" {
            return Submission::Suggest;
        }
        if lower == "/thread new" || lower == "/new" {
            return Submission::NewThread;
        }

        // /thread <uuid> - switch thread
        if let Some(rest) = lower.strip_prefix("/thread ") {
            let rest = rest.trim();
            if rest != "new" {
                if let Ok(id) = Uuid::parse_str(rest) {
                    return Submission::SwitchThread { thread_id: id };
                }
            }
        }

        // /resume <uuid> - resume from checkpoint
        if let Some(rest) = lower.strip_prefix("/resume ") {
            if let Ok(id) = Uuid::parse_str(rest.trim()) {
                return Submission::Resume { checkpoint_id: id };
            }
        }

        // Skill commands
        if let Some(rest) = lower.strip_prefix("/skill ") {
            let rest = rest.trim();
            if let Some(submission) = Self::parse_skill_command(rest, trimmed) {
                return submission;
            }
        }

        // Check if this is a dynamic skill slash command (e.g. /review <args>)
        if lower.starts_with('/') {
            if let Some(submission) =
                Self::parse_dynamic_skill_command(&lower, trimmed, skill_commands)
            {
                return submission;
            }
        }

        // Approval responses (simple yes/no/always for pending approvals)
        // These are short enough to check explicitly
        match lower.as_str() {
            "yes" | "y" | "approve" | "ok" => {
                return Submission::ApprovalResponse {
                    approved: true,
                    always: false,
                };
            }
            "always" | "yes always" | "approve always" => {
                return Submission::ApprovalResponse {
                    approved: true,
                    always: true,
                };
            }
            "no" | "n" | "deny" | "reject" | "cancel" => {
                return Submission::ApprovalResponse {
                    approved: false,
                    always: false,
                };
            }
            _ => {}
        }

        // Default: user input
        Submission::UserInput {
            content: content.to_string(),
        }
    }

    /// Parse `/skill <subcommand>` forms.
    fn parse_skill_command(rest: &str, _original: &str) -> Option<Submission> {
        // /skill list
        if rest == "list" {
            return Some(Submission::SkillList);
        }

        // /skill deactivate
        if rest == "deactivate" || rest == "off" {
            return Some(Submission::SkillDeactivate);
        }

        // /skill load <url>
        if let Some(url) = rest.strip_prefix("load ") {
            let url = url.trim();
            if !url.is_empty() {
                return Some(Submission::SkillLoad {
                    url: url.to_string(),
                });
            }
        }

        // /skill remove <name>
        if let Some(name) = rest.strip_prefix("remove ") {
            let name = name.trim();
            if !name.is_empty() {
                return Some(Submission::SkillRemove {
                    name: name.to_string(),
                });
            }
        }

        // /skill info <name>
        if let Some(name) = rest.strip_prefix("info ") {
            let name = name.trim();
            if !name.is_empty() {
                return Some(Submission::SkillInfo {
                    name: name.to_string(),
                });
            }
        }

        // /skill activate <name> [args]
        if let Some(rest) = rest.strip_prefix("activate ") {
            let rest = rest.trim();
            if !rest.is_empty() {
                let (name, args) = split_first_word(rest);
                return Some(Submission::SkillActivate {
                    name: name.to_string(),
                    args: args.map(|s| s.to_string()),
                });
            }
        }

        // /skill <name> [args] (shorthand for activate)
        if !rest.is_empty() {
            let (name, args) = split_first_word(rest);
            return Some(Submission::SkillActivate {
                name: name.to_string(),
                args: args.map(|s| s.to_string()),
            });
        }

        None
    }

    /// Check if a `/command args` matches a registered skill command.
    fn parse_dynamic_skill_command(
        lower: &str,
        original: &str,
        skill_commands: &[String],
    ) -> Option<Submission> {
        // Extract the command word (without the leading /)
        let without_slash = &lower[1..];
        let (cmd, _) = split_first_word(without_slash);

        if skill_commands.iter().any(|sc| sc == cmd) {
            // Get args from the original (preserving case)
            let original_without_slash = &original.trim()[1..];
            let (_, args) = split_first_word(original_without_slash);
            return Some(Submission::SkillActivateByCommand {
                command: cmd.to_string(),
                args: args.map(|s| s.to_string()),
            });
        }

        None
    }
}

/// Split a string into the first word and the rest.
fn split_first_word(s: &str) -> (&str, Option<&str>) {
    match s.find(char::is_whitespace) {
        Some(idx) => {
            let rest = s[idx..].trim();
            if rest.is_empty() {
                (&s[..idx], None)
            } else {
                (&s[..idx], Some(rest))
            }
        }
        None => (s, None),
    }
}

/// A submission to the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Submission {
    /// User text input (starts a new turn).
    UserInput {
        /// The user's message content.
        content: String,
    },

    /// Response to an execution approval request (with explicit request ID).
    ExecApproval {
        /// ID of the approval request being responded to.
        request_id: Uuid,
        /// Whether the execution was approved.
        approved: bool,
        /// If true, auto-approve this tool for the rest of the session.
        always: bool,
    },

    /// Simple approval response (yes/no/always) for the current pending approval.
    ApprovalResponse {
        /// Whether the execution was approved.
        approved: bool,
        /// If true, auto-approve this tool for the rest of the session.
        always: bool,
    },

    /// Interrupt the current turn.
    Interrupt,

    /// Request context compaction.
    Compact,

    /// Undo the last turn.
    Undo,

    /// Redo a previously undone turn (if available).
    Redo,

    /// Resume from a specific checkpoint.
    Resume {
        /// ID of the checkpoint to resume from.
        checkpoint_id: Uuid,
    },

    /// Clear the current thread and start fresh.
    Clear,

    /// Switch to a different thread.
    SwitchThread {
        /// ID of the thread to switch to.
        thread_id: Uuid,
    },

    /// Create a new thread.
    NewThread,

    /// Trigger a manual heartbeat check.
    Heartbeat,

    /// Summarize the current thread.
    Summarize,

    /// Suggest next steps based on the current thread.
    Suggest,

    /// Load a skill from a URL.
    SkillLoad {
        /// URL to load the skill manifest from.
        url: String,
    },

    /// Activate a skill by name.
    SkillActivate {
        /// Skill name.
        name: String,
        /// Optional arguments.
        args: Option<String>,
    },

    /// Activate a skill via its registered slash command.
    SkillActivateByCommand {
        /// The slash command that matched.
        command: String,
        /// Optional arguments.
        args: Option<String>,
    },

    /// Deactivate the currently active skill.
    SkillDeactivate,

    /// List installed skills.
    SkillList,

    /// Remove an installed skill.
    SkillRemove {
        /// Skill name.
        name: String,
    },

    /// Show info about an installed skill.
    SkillInfo {
        /// Skill name.
        name: String,
    },
}

impl Submission {
    /// Create a user input submission.
    pub fn user_input(content: impl Into<String>) -> Self {
        Self::UserInput {
            content: content.into(),
        }
    }

    /// Create an approval submission.
    pub fn approval(request_id: Uuid, approved: bool) -> Self {
        Self::ExecApproval {
            request_id,
            approved,
            always: false,
        }
    }

    /// Create an "always approve" submission.
    pub fn always_approve(request_id: Uuid) -> Self {
        Self::ExecApproval {
            request_id,
            approved: true,
            always: true,
        }
    }

    /// Create an interrupt submission.
    pub fn interrupt() -> Self {
        Self::Interrupt
    }

    /// Create a compact submission.
    pub fn compact() -> Self {
        Self::Compact
    }

    /// Create an undo submission.
    pub fn undo() -> Self {
        Self::Undo
    }

    /// Create a redo submission.
    pub fn redo() -> Self {
        Self::Redo
    }

    /// Check if this submission starts a new turn.
    pub fn starts_turn(&self) -> bool {
        matches!(self, Self::UserInput { .. })
    }

    /// Check if this submission is a control command.
    pub fn is_control(&self) -> bool {
        matches!(
            self,
            Self::Interrupt
                | Self::Compact
                | Self::Undo
                | Self::Redo
                | Self::Clear
                | Self::NewThread
                | Self::Heartbeat
                | Self::Summarize
                | Self::Suggest
                | Self::SkillLoad { .. }
                | Self::SkillDeactivate
                | Self::SkillList
                | Self::SkillRemove { .. }
                | Self::SkillInfo { .. }
        )
    }
}

/// Result of processing a submission.
#[derive(Debug, Clone)]
pub enum SubmissionResult {
    /// Turn completed with a response.
    Response {
        /// The agent's response.
        content: String,
    },

    /// Need approval before continuing.
    NeedApproval {
        /// ID of the approval request.
        request_id: Uuid,
        /// Tool that needs approval.
        tool_name: String,
        /// Description of what the tool will do.
        description: String,
        /// Parameters being passed.
        parameters: serde_json::Value,
    },

    /// Successfully processed (for control commands).
    Ok {
        /// Optional message.
        message: Option<String>,
    },

    /// Error occurred.
    Error {
        /// Error message.
        message: String,
    },

    /// Turn was interrupted.
    Interrupted,
}

impl SubmissionResult {
    /// Create a response result.
    pub fn response(content: impl Into<String>) -> Self {
        Self::Response {
            content: content.into(),
        }
    }

    /// Create an OK result.
    pub fn ok() -> Self {
        Self::Ok { message: None }
    }

    /// Create an OK result with a message.
    pub fn ok_with_message(message: impl Into<String>) -> Self {
        Self::Ok {
            message: Some(message.into()),
        }
    }

    /// Create an error result.
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_submission_types() {
        let input = Submission::user_input("Hello");
        assert!(input.starts_turn());
        assert!(!input.is_control());

        let undo = Submission::undo();
        assert!(!undo.starts_turn());
        assert!(undo.is_control());
    }

    #[test]
    fn test_parser_user_input() {
        let submission = SubmissionParser::parse("Hello, how are you?");
        assert!(
            matches!(submission, Submission::UserInput { content } if content == "Hello, how are you?")
        );
    }

    #[test]
    fn test_parser_undo() {
        let submission = SubmissionParser::parse("/undo");
        assert!(matches!(submission, Submission::Undo));

        let submission = SubmissionParser::parse("/UNDO");
        assert!(matches!(submission, Submission::Undo));
    }

    #[test]
    fn test_parser_redo() {
        let submission = SubmissionParser::parse("/redo");
        assert!(matches!(submission, Submission::Redo));
    }

    #[test]
    fn test_parser_interrupt() {
        let submission = SubmissionParser::parse("/interrupt");
        assert!(matches!(submission, Submission::Interrupt));

        let submission = SubmissionParser::parse("/stop");
        assert!(matches!(submission, Submission::Interrupt));
    }

    #[test]
    fn test_parser_compact() {
        let submission = SubmissionParser::parse("/compact");
        assert!(matches!(submission, Submission::Compact));
    }

    #[test]
    fn test_parser_clear() {
        let submission = SubmissionParser::parse("/clear");
        assert!(matches!(submission, Submission::Clear));
    }

    #[test]
    fn test_parser_new_thread() {
        let submission = SubmissionParser::parse("/thread new");
        assert!(matches!(submission, Submission::NewThread));

        let submission = SubmissionParser::parse("/new");
        assert!(matches!(submission, Submission::NewThread));
    }

    #[test]
    fn test_parser_switch_thread() {
        let uuid = Uuid::new_v4();
        let submission = SubmissionParser::parse(&format!("/thread {}", uuid));
        assert!(matches!(submission, Submission::SwitchThread { thread_id } if thread_id == uuid));
    }

    #[test]
    fn test_parser_resume() {
        let uuid = Uuid::new_v4();
        let submission = SubmissionParser::parse(&format!("/resume {}", uuid));
        assert!(
            matches!(submission, Submission::Resume { checkpoint_id } if checkpoint_id == uuid)
        );
    }

    #[test]
    fn test_parser_heartbeat() {
        let submission = SubmissionParser::parse("/heartbeat");
        assert!(matches!(submission, Submission::Heartbeat));
    }

    #[test]
    fn test_parser_summarize() {
        let submission = SubmissionParser::parse("/summarize");
        assert!(matches!(submission, Submission::Summarize));

        let submission = SubmissionParser::parse("/summary");
        assert!(matches!(submission, Submission::Summarize));
    }

    #[test]
    fn test_parser_suggest() {
        let submission = SubmissionParser::parse("/suggest");
        assert!(matches!(submission, Submission::Suggest));
    }

    #[test]
    fn test_parser_invalid_commands_become_user_input() {
        // Invalid UUID should become user input
        let submission = SubmissionParser::parse("/thread not-a-uuid");
        assert!(matches!(submission, Submission::UserInput { .. }));

        // Unknown command should become user input
        let submission = SubmissionParser::parse("/unknown");
        assert!(matches!(submission, Submission::UserInput { content } if content == "/unknown"));
    }

    #[test]
    fn test_parser_skill_list() {
        let submission = SubmissionParser::parse("/skill list");
        assert!(matches!(submission, Submission::SkillList));
    }

    #[test]
    fn test_parser_skill_load() {
        let submission = SubmissionParser::parse(
            "/skill load https://github.com/alice/skills/blob/main/review.toml",
        );
        assert!(matches!(submission, Submission::SkillLoad { url } if url.contains("github.com")));
    }

    #[test]
    fn test_parser_skill_activate() {
        let submission = SubmissionParser::parse("/skill activate pr-review");
        assert!(
            matches!(submission, Submission::SkillActivate { name, args } if name == "pr-review" && args.is_none())
        );
    }

    #[test]
    fn test_parser_skill_activate_with_args() {
        let submission = SubmissionParser::parse(
            "/skill activate pr-review https://github.com/org/repo/pull/123",
        );
        assert!(
            matches!(submission, Submission::SkillActivate { name, args } if name == "pr-review" && args.is_some())
        );
    }

    #[test]
    fn test_parser_skill_shorthand() {
        // /skill <name> is shorthand for /skill activate <name>
        let submission = SubmissionParser::parse("/skill pr-review");
        assert!(
            matches!(submission, Submission::SkillActivate { name, .. } if name == "pr-review")
        );
    }

    #[test]
    fn test_parser_skill_deactivate() {
        let submission = SubmissionParser::parse("/skill deactivate");
        assert!(matches!(submission, Submission::SkillDeactivate));

        let submission = SubmissionParser::parse("/skill off");
        assert!(matches!(submission, Submission::SkillDeactivate));
    }

    #[test]
    fn test_parser_skill_remove() {
        let submission = SubmissionParser::parse("/skill remove pr-review");
        assert!(matches!(submission, Submission::SkillRemove { name } if name == "pr-review"));
    }

    #[test]
    fn test_parser_skill_info() {
        let submission = SubmissionParser::parse("/skill info pr-review");
        assert!(matches!(submission, Submission::SkillInfo { name } if name == "pr-review"));
    }

    #[test]
    fn test_parser_dynamic_skill_command() {
        let skill_commands = vec!["review".to_string(), "debug".to_string()];
        let submission = SubmissionParser::parse_with_skill_commands(
            "/review https://github.com/org/repo/pull/123",
            &skill_commands,
        );
        assert!(matches!(
            submission,
            Submission::SkillActivateByCommand { command, args }
                if command == "review" && args.as_deref() == Some("https://github.com/org/repo/pull/123")
        ));
    }

    #[test]
    fn test_parser_dynamic_skill_command_no_args() {
        let skill_commands = vec!["debug".to_string()];
        let submission = SubmissionParser::parse_with_skill_commands("/debug", &skill_commands);
        assert!(matches!(
            submission,
            Submission::SkillActivateByCommand { command, args }
                if command == "debug" && args.is_none()
        ));
    }

    #[test]
    fn test_parser_unknown_slash_not_skill() {
        let skill_commands = vec!["review".to_string()];
        // /unknown is not a skill command, becomes user input
        let submission = SubmissionParser::parse_with_skill_commands("/unknown", &skill_commands);
        assert!(matches!(submission, Submission::UserInput { .. }));
    }
}
