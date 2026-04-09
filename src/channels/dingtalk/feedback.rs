//! Feedback learning and shared knowledge module for the DingTalk channel.
//!
//! Provides local persistence for user feedback events, session notes, and
//! global/scoped learned rules. All I/O is non-blocking — errors are logged at
//! `debug` level and never propagate back to the message pipeline.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tracing::debug;
use uuid::Uuid;

use crate::error::ChannelError;

// ──────────────────────────────────────────────────────────────────────────────
// Data types
// ──────────────────────────────────────────────────────────────────────────────

/// The kind of feedback event captured.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackType {
    Upvote,
    Downvote,
    SessionNote,
    GlobalRule,
}

/// A single feedback event appended to the JSONL log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackEvent {
    /// ISO 8601 timestamp (e.g. "2024-01-15T10:30:00Z").
    pub timestamp: String,
    pub user_id: String,
    pub event_type: FeedbackType,
    pub content: String,
    pub session_id: Option<String>,
}

/// Scope that determines where a learned rule applies.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "scope_type", content = "scope_value", rename_all = "snake_case")]
pub enum RuleScope {
    /// Applies only to a specific session.
    Session(String),
    /// Applies only to a specific user or conversation.
    Target(String),
    /// Applies globally to all sessions.
    Global,
}

/// A persistable rule learned from user feedback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnedRule {
    pub id: String,
    pub content: String,
    pub scope: RuleScope,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    pub enabled: bool,
}

// ──────────────────────────────────────────────────────────────────────────────
// LearnCommand
// ──────────────────────────────────────────────────────────────────────────────

/// Commands that can be extracted from `/learn …` messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LearnCommand {
    WhoAmI,
    ListGlobal,
    AddNote(String),
    AddGlobalRule(String),
    DeleteRule(String),
}

/// Parse a `/learn` command from a message string.
///
/// Returns `None` if the message does not start with `/learn`.
pub fn parse_learn_command(content: &str) -> Option<LearnCommand> {
    let rest = content.trim().strip_prefix("/learn")?;
    let rest = rest.trim();

    if rest.is_empty() || rest == "whoami" {
        return Some(LearnCommand::WhoAmI);
    }

    if rest == "list" || rest == "list global" {
        return Some(LearnCommand::ListGlobal);
    }

    if let Some(note_text) = rest.strip_prefix("note ") {
        let text = note_text.trim();
        if !text.is_empty() {
            return Some(LearnCommand::AddNote(text.to_string()));
        }
    }

    if let Some(global_rest) = rest.strip_prefix("global ") {
        let global_rest = global_rest.trim();
        if let Some(id) = global_rest.strip_prefix("delete ") {
            let id = id.trim();
            if !id.is_empty() {
                return Some(LearnCommand::DeleteRule(id.to_string()));
            }
        } else if !global_rest.is_empty() {
            return Some(LearnCommand::AddGlobalRule(global_rest.to_string()));
        }
    }

    // Fall back to WhoAmI for unrecognised sub-commands.
    Some(LearnCommand::WhoAmI)
}

// ──────────────────────────────────────────────────────────────────────────────
// FeedbackStore
// ──────────────────────────────────────────────────────────────────────────────

/// Persistent store for DingTalk feedback events, session notes, and rules.
///
/// All state is kept under `base_dir/` (typically `dingtalk-state/`).
///
/// Layout:
/// ```text
/// dingtalk-state/
///   feedback-events.jsonl      — append-only feedback log
///   global-rules.json          — array of LearnedRule (scope = Global | Target)
///   session-notes/
///     {session_id}.json        — array of LearnedRule with scope = Session(…)
/// ```
pub struct FeedbackStore {
    base_dir: PathBuf,
}

impl FeedbackStore {
    /// Create a new store rooted at `base_dir`, creating the directory if needed.
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Lazily create the base directory (and session-notes sub-dir).
    async fn ensure_dirs(&self) -> Result<(), ChannelError> {
        tokio::fs::create_dir_all(&self.base_dir)
            .await
            .map_err(|e| ChannelError::Http(format!("feedback mkdir: {e}")))?;
        tokio::fs::create_dir_all(self.base_dir.join("session-notes"))
            .await
            .map_err(|e| ChannelError::Http(format!("feedback mkdir session-notes: {e}")))?;
        Ok(())
    }

    /// Path to the feedback events log.
    fn events_path(&self) -> PathBuf {
        self.base_dir.join("feedback-events.jsonl")
    }

    /// Path to the global/target rules file.
    fn global_rules_path(&self) -> PathBuf {
        self.base_dir.join("global-rules.json")
    }

    /// Path to a session note file.
    fn session_notes_path(&self, session_id: &str) -> PathBuf {
        self.base_dir
            .join("session-notes")
            .join(format!("{session_id}.json"))
    }

    // ── Raw file helpers ──────────────────────────────────────────────────────

    /// Read a JSON array of `LearnedRule` from a file, or return empty vec.
    async fn read_rules(path: &PathBuf) -> Vec<LearnedRule> {
        match tokio::fs::read_to_string(path).await {
            Ok(text) => serde_json::from_str::<Vec<LearnedRule>>(&text).unwrap_or_default(),
            Err(e) => {
                debug!(path = %path.display(), error = %e, "feedback: could not read rules file");
                Vec::new()
            }
        }
    }

    /// Atomically write a JSON array of `LearnedRule` to a file.
    async fn write_rules(path: &PathBuf, rules: &[LearnedRule]) -> Result<(), ChannelError> {
        let json = serde_json::to_string_pretty(rules)
            .map_err(|e| ChannelError::Http(format!("feedback serialize rules: {e}")))?;
        tokio::fs::write(path, json)
            .await
            .map_err(|e| ChannelError::Http(format!("feedback write rules: {e}")))?;
        Ok(())
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Append a feedback event to `feedback-events.jsonl`.
    pub async fn record_feedback(&self, event: &FeedbackEvent) -> Result<(), ChannelError> {
        if let Err(e) = self.ensure_dirs().await {
            debug!(error = %e, "feedback: could not ensure dirs");
            return Ok(()); // non-blocking
        }

        let line = match serde_json::to_string(event) {
            Ok(l) => l,
            Err(e) => {
                debug!(error = %e, "feedback: could not serialize event");
                return Ok(());
            }
        };

        let mut file = match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.events_path())
            .await
        {
            Ok(f) => f,
            Err(e) => {
                debug!(error = %e, "feedback: could not open events file");
                return Ok(());
            }
        };

        let entry = format!("{line}\n");
        if let Err(e) = file.write_all(entry.as_bytes()).await {
            debug!(error = %e, "feedback: could not write event");
        }

        Ok(())
    }

    /// Write a note for a session.
    ///
    /// Notes are stored as `LearnedRule` objects with `scope = Session(session_id)`.
    pub async fn add_session_note(
        &self,
        session_id: &str,
        content: &str,
    ) -> Result<(), ChannelError> {
        if let Err(e) = self.ensure_dirs().await {
            debug!(error = %e, "feedback: could not ensure dirs for session note");
            return Ok(());
        }

        let path = self.session_notes_path(session_id);
        let mut rules = Self::read_rules(&path).await;

        let rule = LearnedRule {
            id: Uuid::new_v4().to_string(),
            content: content.to_string(),
            scope: RuleScope::Session(session_id.to_string()),
            created_at: now_iso8601(),
            enabled: true,
        };
        rules.push(rule);

        if let Err(e) = Self::write_rules(&path, &rules).await {
            debug!(error = %e, "feedback: could not write session note");
        }

        Ok(())
    }

    /// Add a global rule and persist it.
    ///
    /// Returns the newly created `LearnedRule`.
    pub async fn add_global_rule(&self, content: &str) -> Result<LearnedRule, ChannelError> {
        self.ensure_dirs().await?;

        let path = self.global_rules_path();
        let mut rules = Self::read_rules(&path).await;

        let rule = LearnedRule {
            id: Uuid::new_v4().to_string(),
            content: content.to_string(),
            scope: RuleScope::Global,
            created_at: now_iso8601(),
            enabled: true,
        };
        rules.push(rule.clone());

        Self::write_rules(&path, &rules).await?;

        Ok(rule)
    }

    /// Return all active (enabled) rules applicable to the given context.
    ///
    /// Priority order (highest first): session notes → target rules → global rules.
    pub async fn get_active_rules(
        &self,
        session_id: Option<&str>,
        target_id: Option<&str>,
    ) -> Result<Vec<LearnedRule>, ChannelError> {
        let global_path = self.global_rules_path();
        let global_rules = Self::read_rules(&global_path).await;

        let mut result: Vec<LearnedRule> = Vec::new();

        // 1. Session-scoped rules (highest priority).
        if let Some(sid) = session_id {
            let session_path = self.session_notes_path(sid);
            let session_rules = Self::read_rules(&session_path).await;
            for rule in session_rules {
                if rule.enabled {
                    result.push(rule);
                }
            }
        }

        // 2. Target-scoped rules from global-rules.json.
        if let Some(tid) = target_id {
            for rule in &global_rules {
                if rule.enabled {
                    if let RuleScope::Target(ref t) = rule.scope {
                        if t == tid {
                            result.push(rule.clone());
                        }
                    }
                }
            }
        }

        // 3. Global rules.
        for rule in &global_rules {
            if rule.enabled && rule.scope == RuleScope::Global {
                result.push(rule.clone());
            }
        }

        Ok(result)
    }

    /// Delete a rule by ID from either the global-rules file or any session file.
    ///
    /// Returns `true` if the rule was found and removed.
    pub async fn delete_rule(&self, rule_id: &str) -> Result<bool, ChannelError> {
        // Try global-rules.json first.
        let global_path = self.global_rules_path();
        let mut global_rules = Self::read_rules(&global_path).await;
        let before = global_rules.len();
        global_rules.retain(|r| r.id != rule_id);
        if global_rules.len() < before {
            if let Err(e) = Self::write_rules(&global_path, &global_rules).await {
                debug!(error = %e, "feedback: could not write global rules after delete");
            }
            return Ok(true);
        }

        // Scan session-notes directory.
        let notes_dir = self.base_dir.join("session-notes");
        let mut read_dir = match tokio::fs::read_dir(&notes_dir).await {
            Ok(rd) => rd,
            Err(e) => {
                debug!(error = %e, "feedback: could not read session-notes dir");
                return Ok(false);
            }
        };

        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let path = entry.path();
            let mut rules = Self::read_rules(&path).await;
            let before = rules.len();
            rules.retain(|r| r.id != rule_id);
            if rules.len() < before {
                if let Err(e) = Self::write_rules(&path, &rules).await {
                    debug!(error = %e, path = %path.display(), "feedback: could not write session notes after delete");
                }
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Build a human-readable context string for `/learn whoami`.
    pub async fn get_context_info(&self, user_id: &str, session_id: Option<&str>) -> String {
        let global_path = self.global_rules_path();
        let global_rules = Self::read_rules(&global_path).await;
        let global_count = global_rules
            .iter()
            .filter(|r| r.enabled && r.scope == RuleScope::Global)
            .count();

        let session_count = if let Some(sid) = session_id {
            let path = self.session_notes_path(sid);
            Self::read_rules(&path)
                .await
                .iter()
                .filter(|r| r.enabled)
                .count()
        } else {
            0
        };

        let mut lines = vec![
            format!("**User ID:** {user_id}"),
            format!("**Global rules:** {global_count} active"),
        ];

        if let Some(sid) = session_id {
            lines.push(format!("**Session:** {sid}"));
            lines.push(format!("**Session notes:** {session_count} active"));
        }

        lines.push(String::from(
            "\nUse `/learn note <text>` to add a session note.",
        ));
        lines.push(String::from(
            "Use `/learn global <text>` to add a global rule.",
        ));
        lines.push(String::from(
            "Use `/learn global delete <id>` to remove a rule.",
        ));
        lines.push(String::from("Use `/learn list` to list global rules."));

        lines.join("\n")
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

fn now_iso8601() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_learn_command ───────────────────────────────────────────────────

    #[test]
    fn test_whoami_bare() {
        assert_eq!(parse_learn_command("/learn"), Some(LearnCommand::WhoAmI));
    }

    #[test]
    fn test_whoami_explicit() {
        assert_eq!(
            parse_learn_command("/learn whoami"),
            Some(LearnCommand::WhoAmI)
        );
    }

    #[test]
    fn test_list_global() {
        assert_eq!(
            parse_learn_command("/learn list"),
            Some(LearnCommand::ListGlobal)
        );
        assert_eq!(
            parse_learn_command("/learn list global"),
            Some(LearnCommand::ListGlobal)
        );
    }

    #[test]
    fn test_add_note() {
        assert_eq!(
            parse_learn_command("/learn note always be concise"),
            Some(LearnCommand::AddNote("always be concise".to_string()))
        );
    }

    #[test]
    fn test_parse_add_global_rule() {
        assert_eq!(
            parse_learn_command("/learn global reply in English"),
            Some(LearnCommand::AddGlobalRule("reply in English".to_string()))
        );
    }

    #[test]
    fn test_delete_rule() {
        assert_eq!(
            parse_learn_command("/learn global delete abc-123"),
            Some(LearnCommand::DeleteRule("abc-123".to_string()))
        );
    }

    #[test]
    fn test_non_learn_prefix() {
        assert_eq!(parse_learn_command("hello world"), None);
        assert_eq!(parse_learn_command("/help"), None);
    }

    #[test]
    fn test_whitespace_trimmed() {
        assert_eq!(
            parse_learn_command("  /learn note   trim me  "),
            Some(LearnCommand::AddNote("trim me".to_string()))
        );
    }

    // ── FeedbackStore ─────────────────────────────────────────────────────────

    fn make_store(dir: &tempfile::TempDir) -> FeedbackStore {
        FeedbackStore::new(dir.path().join("dingtalk-state"))
    }

    #[tokio::test]
    async fn test_record_feedback_creates_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = make_store(&tmp);

        let event = FeedbackEvent {
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            user_id: "u1".to_string(),
            event_type: FeedbackType::Upvote,
            content: "great answer".to_string(),
            session_id: Some("s1".to_string()),
        };

        store.record_feedback(&event).await.unwrap();

        let path = store.events_path();
        assert!(path.exists(), "feedback-events.jsonl should be created");

        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(contents.contains("great answer"));
        assert!(contents.contains("upvote"));
    }

    #[tokio::test]
    async fn test_add_session_note() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = make_store(&tmp);

        store.add_session_note("sess-42", "be brief").await.unwrap();

        let path = store.session_notes_path("sess-42");
        assert!(path.exists());

        let rules: Vec<LearnedRule> =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].content, "be brief");
        assert_eq!(rules[0].scope, RuleScope::Session("sess-42".to_string()));
        assert!(rules[0].enabled);
    }

    #[tokio::test]
    async fn test_add_global_rule() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = make_store(&tmp);

        let rule = store.add_global_rule("always greet in Chinese").await.unwrap();

        assert_eq!(rule.content, "always greet in Chinese");
        assert_eq!(rule.scope, RuleScope::Global);
        assert!(rule.enabled);
        assert!(!rule.id.is_empty());

        // Persisted correctly.
        let path = store.global_rules_path();
        let rules: Vec<LearnedRule> =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_eq!(rules.len(), 1);
    }

    #[tokio::test]
    async fn test_get_active_rules_priority() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = make_store(&tmp);

        store.add_global_rule("global rule A").await.unwrap();
        store.add_session_note("sess-1", "session note B").await.unwrap();

        let rules = store
            .get_active_rules(Some("sess-1"), None)
            .await
            .unwrap();

        // Session note should come before global rule.
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].content, "session note B");
        assert_eq!(rules[1].content, "global rule A");
    }

    #[tokio::test]
    async fn test_delete_global_rule() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = make_store(&tmp);

        let rule = store.add_global_rule("to be deleted").await.unwrap();
        let id = rule.id.clone();

        let deleted = store.delete_rule(&id).await.unwrap();
        assert!(deleted);

        let rules = store.get_active_rules(None, None).await.unwrap();
        assert!(rules.is_empty());
    }

    #[tokio::test]
    async fn test_delete_missing_rule() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = make_store(&tmp);

        let deleted = store.delete_rule("nonexistent-id").await.unwrap();
        assert!(!deleted);
    }

    #[tokio::test]
    async fn test_get_context_info() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = make_store(&tmp);

        store.add_global_rule("rule X").await.unwrap();

        let info = store.get_context_info("user-99", Some("sess-5")).await;
        assert!(info.contains("user-99"));
        assert!(info.contains("Global rules:** 1 active"));
        assert!(info.contains("sess-5"));
    }
}
