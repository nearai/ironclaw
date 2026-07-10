//! Episodic memory: durable per-conversation summaries + a recent-digest that
//! rides the system prompt. See docs/superpowers/specs/2026-07-09-episodic-memory-design.md

use chrono::{DateTime, Utc};

/// A structured summary of one conversation (thread).
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub conversation_id: String,
    pub channel: String,
    pub timestamp: DateTime<Utc>,
    pub title: String,
    pub gist: String,
    pub decisions: Vec<String>,
    pub open_threads: Vec<String>,
    pub user_notes: Vec<String>,
}

/// Workspace-relative path for a per-session file.
pub fn sessions_path(stem: &str) -> String {
    format!("memory/sessions/{stem}.md")
}

/// Workspace-relative path for the recent-digest.
pub const RECENT_PATH: &str = "memory/recent.md";

impl SessionSummary {
    /// `YYYY-MM-DD-<conversation_id>` — the per-session file stem.
    pub fn file_stem(&self) -> String {
        format!(
            "{}-{}",
            self.timestamp.format("%Y-%m-%d"),
            self.conversation_id
        )
    }

    fn bullets(items: &[String]) -> String {
        if items.is_empty() {
            "_none_\n".to_string()
        } else {
            items.iter().map(|i| format!("- {i}\n")).collect()
        }
    }

    /// Full per-session file: YAML frontmatter + structured body.
    pub fn to_markdown(&self) -> String {
        format!(
            "---\nconversation_id: {}\nchannel: {}\ntimestamp: {}\ntitle: {}\n---\n\n\
             # {}\n\n{}\n\n## Decisions\n{}\n## Open threads\n{}\n## User notes\n{}",
            self.conversation_id,
            self.channel,
            self.timestamp.to_rfc3339(),
            self.title,
            self.title,
            self.gist,
            Self::bullets(&self.decisions),
            Self::bullets(&self.open_threads),
            Self::bullets(&self.user_notes),
        )
    }

    /// Terse one-block digest for `recent.md`: title + gist + open threads only.
    pub fn digest_entry(&self) -> String {
        let threads = if self.open_threads.is_empty() {
            String::new()
        } else {
            format!("  \n  _open:_ {}", self.open_threads.join("; "))
        };
        format!(
            "### {} — {}\n{}{}\n",
            self.timestamp.format("%Y-%m-%d"),
            self.title,
            self.gist,
            threads,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample() -> SessionSummary {
        SessionSummary {
            conversation_id: "abc123".into(),
            channel: "gateway".into(),
            timestamp: chrono::Utc.with_ymd_and_hms(2026, 7, 9, 14, 30, 0).unwrap(),
            title: "Voice setup".into(),
            gist: "Wired local STT/TTS into OWUI.".into(),
            decisions: vec!["Use Wyoming engines".into()],
            open_threads: vec!["Test on the phone PWA".into()],
            user_notes: vec!["Prefers all-local".into()],
        }
    }

    #[test]
    fn file_stem_is_date_and_conv() {
        assert_eq!(sample().file_stem(), "2026-07-09-abc123");
        assert_eq!(
            sessions_path(&sample().file_stem()),
            "memory/sessions/2026-07-09-abc123.md"
        );
    }

    #[test]
    fn to_markdown_has_frontmatter_and_body() {
        let md = sample().to_markdown();
        assert!(md.starts_with("---\n"));
        assert!(md.contains("conversation_id: abc123"));
        assert!(md.contains("title: Voice setup"));
        assert!(md.contains("## Open threads"));
        assert!(md.contains("Test on the phone PWA"));
    }

    #[test]
    fn digest_entry_is_terse() {
        let d = sample().digest_entry();
        assert!(d.contains("Voice setup"));
        assert!(d.contains("Wired local STT/TTS"));
        assert!(d.contains("Test on the phone PWA")); // open threads kept
        assert!(!d.contains("Prefers all-local")); // user_notes NOT in the terse digest
    }
}
