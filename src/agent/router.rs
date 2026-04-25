//! Message routing to appropriate handlers.
//!
//! The router handles explicit commands (starting with `/`).
//! Natural language intent classification is handled by `IntentClassifier`
//! which uses LLM + tools instead of brittle pattern matching.

use crate::channels::IncomingMessage;

/// Intent extracted from a message.
#[derive(Debug, Clone)]
pub enum MessageIntent {
    /// Create a new job.
    CreateJob {
        title: String,
        description: String,
        category: Option<String>,
    },
    /// Check status of a job.
    CheckJobStatus { job_id: Option<String> },
    /// Cancel a job.
    CancelJob { job_id: String },
    /// List jobs.
    ListJobs { filter: Option<String> },
    /// Help with a stuck job.
    HelpJob { job_id: String },
    /// General conversation/question.
    Chat { content: String },
    /// System command.
    Command { command: String, args: Vec<String> },
    /// Unknown intent.
    Unknown,
}

/// Routes messages to appropriate handlers based on explicit commands.
///
/// For natural language messages, use `IntentClassifier` instead.
pub struct Router {
    /// Command prefix (e.g., "/" or "!")
    command_prefix: String,
}

impl Router {
    /// Create a new router.
    pub fn new() -> Self {
        Self {
            command_prefix: "/".to_string(),
        }
    }

    /// Set the command prefix.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.command_prefix = prefix.into();
        self
    }

    /// Check if a message is an explicit command.
    pub fn is_command(&self, message: &IncomingMessage) -> bool {
        message.content.trim().starts_with(&self.command_prefix)
    }

    /// Route an explicit command to determine its intent.
    ///
    /// Returns `Some(intent)` only for *known* job intents (`/job`,
    /// `/status`, `/cancel`, `/list`, `/help <id>`). Unknown `/xxx`
    /// patterns — including `/skill-name` force-activations handled by
    /// `extract_skill_mentions` and `/server:prompt-name` mentions
    /// handled by `resolve_prompt_mentions` — return `None` so the
    /// dispatcher's mention extractors and, ultimately, the LLM get to
    /// handle them. Returning `None` is also correct for plain typos:
    /// the LLM can respond helpfully ("I don't know `/foo`, did you
    /// mean `/help`?") rather than the agent producing a stock error.
    pub fn route_command(&self, message: &IncomingMessage) -> Option<MessageIntent> {
        let content = message.content.trim();

        if !content.starts_with(&self.command_prefix) {
            return None;
        }
        match self.parse_command(content) {
            MessageIntent::Unknown => None,
            intent => Some(intent),
        }
    }

    fn parse_command(&self, content: &str) -> MessageIntent {
        let without_prefix = content
            .strip_prefix(&self.command_prefix)
            .unwrap_or(content);
        let parts: Vec<&str> = without_prefix.split_whitespace().collect();

        match parts.first().map(|s| s.to_lowercase()).as_deref() {
            Some("job") | Some("create") => {
                let rest = parts[1..].join(" ");
                MessageIntent::CreateJob {
                    title: rest.clone(),
                    description: rest,
                    category: None,
                }
            }
            Some("status") => {
                let job_id = parts.get(1).map(|s| s.to_string());
                MessageIntent::CheckJobStatus { job_id }
            }
            Some("cancel") => {
                if let Some(job_id) = parts.get(1) {
                    MessageIntent::CancelJob {
                        job_id: job_id.to_string(),
                    }
                } else {
                    MessageIntent::Unknown
                }
            }
            Some("list") | Some("jobs") => {
                let filter = parts.get(1).map(|s| s.to_string());
                MessageIntent::ListJobs { filter }
            }
            Some("help") => {
                if let Some(job_id) = parts.get(1) {
                    MessageIntent::HelpJob {
                        job_id: job_id.to_string(),
                    }
                } else {
                    MessageIntent::Command {
                        command: "help".to_string(),
                        args: vec![],
                    }
                }
            }
            // Unknown `/xxx` is not a Router concern. The dispatcher's
            // `extract_skill_mentions` handles `/skill-name` and
            // `resolve_prompt_mentions` handles `/server:prompt`; plain
            // typos are better answered by the LLM than by a canned
            // "Unknown command" error. Return `Unknown` so
            // `route_command` can lift it to `None`.
            Some(_) => MessageIntent::Unknown,
            None => MessageIntent::Unknown,
        }
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_routing() {
        let router = Router::new();

        let msg = IncomingMessage::new("test", "user", "/status abc-123");
        let intent = router.route_command(&msg);

        assert!(matches!(intent, Some(MessageIntent::CheckJobStatus { .. })));
    }

    #[test]
    fn test_is_command() {
        let router = Router::new();

        let cmd_msg = IncomingMessage::new("test", "user", "/status");
        assert!(router.is_command(&cmd_msg));

        let chat_msg = IncomingMessage::new("test", "user", "Hello there");
        assert!(!router.is_command(&chat_msg));
    }

    #[test]
    fn test_non_command_returns_none() {
        let router = Router::new();

        // Natural language messages return None - they should use IntentClassifier
        let msg = IncomingMessage::new("test", "user", "Can you create a website for me?");
        assert!(router.route_command(&msg).is_none());

        let msg2 = IncomingMessage::new("test", "user", "Hello, how are you?");
        assert!(router.route_command(&msg2).is_none());
    }

    #[test]
    fn test_command_create_job() {
        let router = Router::new();

        let msg = IncomingMessage::new("test", "user", "/job build a website");
        let intent = router.route_command(&msg);

        match intent {
            Some(MessageIntent::CreateJob { title, .. }) => {
                assert_eq!(title, "build a website");
            }
            _ => panic!("Expected CreateJob intent"),
        }
    }

    #[test]
    fn test_command_list_jobs() {
        let router = Router::new();

        let msg = IncomingMessage::new("test", "user", "/list active");
        let intent = router.route_command(&msg);

        match intent {
            Some(MessageIntent::ListJobs { filter }) => {
                assert_eq!(filter, Some("active".to_string()));
            }
            _ => panic!("Expected ListJobs intent"),
        }
    }

    /// Unknown `/xxx` at the start of a message must NOT be intercepted
    /// by the router — that path would short-circuit the dispatcher's
    /// mention extractors and surface a generic "Unknown command"
    /// error instead. Three shapes all fall through:
    /// - `/skill-name` → `extract_skill_mentions` force-activates
    /// - `/server:prompt` → `resolve_prompt_mentions` splices the block
    /// - plain typos → the LLM gets to respond helpfully
    #[test]
    fn test_unknown_slash_prefix_falls_through_to_dispatcher() {
        let router = Router::new();

        for content in [
            "/github fetch issues",
            "/notion:search docs",
            "/notion:create-page title=foo",
            "/unknown-thing",
        ] {
            let msg = IncomingMessage::new("test", "user", content);
            assert!(
                router.route_command(&msg).is_none(),
                "router must not intercept `{content}` — dispatcher handles it",
            );
        }
    }
}
