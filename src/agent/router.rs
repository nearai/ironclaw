//! Message routing to appropriate handlers.

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

/// Routes messages to appropriate handlers based on intent.
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

    /// Route a message to determine its intent.
    pub fn route(&self, message: &IncomingMessage) -> MessageIntent {
        let content = message.content.trim();

        // Check for commands
        if content.starts_with(&self.command_prefix) {
            return self.parse_command(content);
        }

        // Try to extract intent from natural language
        self.extract_intent(content)
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
            Some(cmd) => MessageIntent::Command {
                command: cmd.to_string(),
                args: parts[1..].iter().map(|s| s.to_string()).collect(),
            },
            None => MessageIntent::Unknown,
        }
    }

    fn extract_intent(&self, content: &str) -> MessageIntent {
        let lower = content.to_lowercase();

        // Job creation patterns - must be explicit about creating a job
        // More specific patterns to avoid capturing general conversation
        let is_job_creation = lower.starts_with("create job ")
            || lower.starts_with("new job ")
            || lower.starts_with("schedule job ")
            || lower.starts_with("run job ")
            || (lower.contains("create") && lower.contains("job"));

        if is_job_creation {
            return MessageIntent::CreateJob {
                title: extract_title(content),
                description: content.to_string(),
                category: extract_category(content),
            };
        }

        // Status check patterns
        if lower.contains("status")
            || lower.contains("how is")
            || lower.contains("progress")
            || lower.starts_with("check ")
        {
            return MessageIntent::CheckJobStatus {
                job_id: extract_job_id(content),
            };
        }

        // Cancel patterns
        if lower.contains("cancel") || lower.contains("stop") || lower.contains("abort") {
            if let Some(job_id) = extract_job_id(content) {
                return MessageIntent::CancelJob { job_id };
            }
        }

        // List patterns
        if lower.starts_with("list") || lower.contains("show jobs") || lower.contains("my jobs") {
            return MessageIntent::ListJobs { filter: None };
        }

        // Help patterns
        if lower.contains("stuck") || lower.contains("not working") || lower.contains("fix") {
            if let Some(job_id) = extract_job_id(content) {
                return MessageIntent::HelpJob { job_id };
            }
        }

        // Default to chat
        MessageIntent::Chat {
            content: content.to_string(),
        }
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract a title from content.
fn extract_title(content: &str) -> String {
    // Take first sentence or first N characters
    let first_sentence = content.split('.').next().unwrap_or(content);
    let title = first_sentence.chars().take(100).collect::<String>();
    if title.len() < first_sentence.len() {
        format!("{}...", title)
    } else {
        title
    }
}

/// Extract a category from content.
fn extract_category(content: &str) -> Option<String> {
    let lower = content.to_lowercase();

    let categories = [
        ("code", "development"),
        ("program", "development"),
        ("website", "web"),
        ("api", "development"),
        ("data", "data"),
        ("write", "writing"),
        ("design", "design"),
        ("research", "research"),
    ];

    for (keyword, category) in categories {
        if lower.contains(keyword) {
            return Some(category.to_string());
        }
    }

    None
}

/// Extract a job ID from content.
fn extract_job_id(content: &str) -> Option<String> {
    // Look for UUID patterns
    let uuid_regex = regex::Regex::new(
        r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
    )
    .ok()?;

    uuid_regex.find(content).map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_routing() {
        let router = Router::new();

        let msg = IncomingMessage::new("test", "user", "/status abc-123");
        let intent = router.route(&msg);

        assert!(matches!(intent, MessageIntent::CheckJobStatus { .. }));
    }

    #[test]
    fn test_natural_language_routing() {
        let router = Router::new();

        let msg = IncomingMessage::new("test", "user", "Can you create a website for me?");
        let intent = router.route(&msg);

        assert!(matches!(intent, MessageIntent::CreateJob { .. }));
    }

    #[test]
    fn test_chat_fallback() {
        let router = Router::new();

        let msg = IncomingMessage::new("test", "user", "Hello, how are you?");
        let intent = router.route(&msg);

        assert!(matches!(intent, MessageIntent::Chat { .. }));
    }

    #[test]
    fn test_extract_job_id() {
        let content = "Check status of job 550e8400-e29b-41d4-a716-446655440000";
        let id = extract_job_id(content);
        assert_eq!(id, Some("550e8400-e29b-41d4-a716-446655440000".to_string()));
    }
}
