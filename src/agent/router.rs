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
    /// Known job intents (`/job`, `/status`, `/cancel`, `/list`,
    /// `/help <id>`) return `Some(intent)`. Mention-shaped inputs
    /// (`/skill-name`, `/server:prompt-name`) return `None` so the
    /// dispatcher's mention extractors handle them. Everything else
    /// (`/foo!`, `/foo/bar`, plain typos) returns `Some(Command {...})`
    /// so the `Unknown command` handler fires instead of paying for an
    /// LLM turn.
    ///
    /// Inbound safety scan runs before this function is called (see
    /// `thread_ops::process_user_input`), so the fall-through path
    /// isn't an unscanned ingress.
    pub fn route_command(&self, message: &IncomingMessage) -> Option<MessageIntent> {
        let content = message.content.trim();

        if !content.starts_with(&self.command_prefix) {
            return None;
        }
        match self.parse_command(content) {
            MessageIntent::Unknown => {
                if first_token_is_mention_shape(content) {
                    return None;
                }
                let rest = content
                    .strip_prefix(&self.command_prefix)
                    .unwrap_or(content);
                let mut parts = rest.split_whitespace();
                let command = parts.next().unwrap_or("").to_string();
                let args: Vec<String> = parts.map(|s| s.to_string()).collect();
                Some(MessageIntent::Command { command, args })
            }
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

/// Return true when `content` starts with a `/<identifier>` (optionally
/// followed by `:<identifier>`) token — the shape the dispatcher's skill
/// and prompt mention extractors expect.
///
/// Reuses the prompt extractor's byte class so the router can't drift
/// from what the extractor actually accepts (per `.claude/rules/types.md`
/// "same-shape predicates drifting" bug class).
fn first_token_is_mention_shape(content: &str) -> bool {
    let Some(rest) = content.trim_start().strip_prefix('/') else {
        return false;
    };
    let first = rest.split_whitespace().next().unwrap_or("");
    let mut halves = first.splitn(2, ':');
    let head = halves.next().unwrap_or("");
    let tail = halves.next();
    is_ident(head) && tail.is_none_or(is_ident)
}

fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(crate::tools::mcp::prompt_mentions::is_prompt_ident_byte)
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

    /// Mention-shaped slash prefixes must fall through so the
    /// dispatcher's skill / prompt extractors get a chance:
    /// - `/skill-name` → `extract_skill_mentions` force-activates
    /// - `/server:prompt` → `resolve_prompt_mentions` splices the block
    #[test]
    fn test_mention_shaped_slash_prefix_falls_through_to_dispatcher() {
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

    /// Non-mention-shaped `/xxx` takes the stock Unknown command
    /// path. Otherwise every typo burns a full LLM turn for no benefit.
    #[test]
    fn test_non_mention_slash_prefix_returns_unknown_command() {
        let router = Router::new();

        for content in [
            "/foo!",        // trailing punctuation — not an identifier
            "/foo/bar",     // two segments with slash — not `:`-shaped
            "/!!!",         // no identifier at all
            "/foo:bar:baz", // three colons — mention shape is two halves
        ] {
            let msg = IncomingMessage::new("test", "user", content);
            let intent = router.route_command(&msg);
            assert!(
                matches!(intent, Some(MessageIntent::Command { .. })),
                "non-mention-shaped `{content}` must synthesize an Unknown command, got: {intent:?}",
            );
        }
    }

    #[test]
    fn test_first_token_is_mention_shape() {
        assert!(first_token_is_mention_shape("/github"));
        assert!(first_token_is_mention_shape("/github fetch issues"));
        assert!(first_token_is_mention_shape("/notion:search"));
        assert!(first_token_is_mention_shape(
            "/notion:create-page title=foo"
        ));
        assert!(first_token_is_mention_shape("/my_skill.v2"));

        assert!(!first_token_is_mention_shape("hello"));
        assert!(!first_token_is_mention_shape("/"));
        assert!(!first_token_is_mention_shape("/foo!"));
        assert!(!first_token_is_mention_shape("/foo/bar"));
        assert!(!first_token_is_mention_shape("/foo:bar:baz"));
        assert!(!first_token_is_mention_shape("/:foo"));
        assert!(!first_token_is_mention_shape("/foo:"));
    }
}
