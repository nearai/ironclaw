//! Episodic memory: durable per-conversation summaries + a recent-digest that
//! rides the system prompt. See docs/superpowers/specs/2026-07-09-episodic-memory-design.md

use chrono::{DateTime, Utc};
use serde::Deserialize;

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

const RECENT_HEADER: &str = "# Recent conversations\n\n";

fn split_entries(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in body.lines() {
        if line.starts_with("### ") && !cur.trim().is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if line.starts_with("# Recent conversations") {
            continue;
        }
        cur.push_str(line);
        cur.push('\n');
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

fn has_open(entry: &str) -> bool {
    entry.contains("_open:_")
}

/// Rebuild `memory/recent.md`: prepend `new_entry` to the entries parsed out
/// of `existing`, cap the entry count, then cap total size — preferring to
/// drop fully-wrapped entries (no open threads) before dropping open ones.
pub fn build_recent(
    new_entry: &str,
    existing: &str,
    max_entries: usize,
    max_chars: usize,
) -> String {
    let mut entries = vec![new_entry.to_string()];
    entries.extend(split_entries(existing));
    // Count cap (newest-first order preserved).
    entries.truncate(max_entries);
    // Size cap: drop from the end; prefer dropping wrapped (no open threads) entries.
    loop {
        let total: usize = RECENT_HEADER.len() + entries.iter().map(|e| e.len()).sum::<usize>();
        if total <= max_chars || entries.len() <= 1 {
            break;
        }
        // find the last wrapped entry to drop; else drop the last entry.
        let idx = entries
            .iter()
            .rposition(|e| !has_open(e))
            .unwrap_or(entries.len() - 1);
        entries.remove(idx);
    }
    let mut out = String::from(RECENT_HEADER);
    for e in &entries {
        out.push_str(e.trim_end());
        out.push_str("\n\n");
    }
    out
}

/// Tolerant view of the model's JSON reply.
#[derive(Deserialize, Default)]
struct RawSummary {
    #[serde(default)]
    title: String,
    #[serde(default)]
    gist: String,
    #[serde(default)]
    decisions: Vec<String>,
    #[serde(default)]
    open_threads: Vec<String>,
    #[serde(default)]
    user_notes: Vec<String>,
}

/// Parse the model's (possibly fenced) JSON into a `SessionSummary`. A malformed
/// reply never fails the pipeline — it falls back to a minimal summary.
pub(crate) fn parse_summary_json(
    raw: &str,
    conversation_id: &str,
    channel: &str,
    timestamp: DateTime<Utc>,
) -> SessionSummary {
    let cleaned = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    match serde_json::from_str::<RawSummary>(cleaned) {
        Ok(r) if !r.title.is_empty() || !r.gist.is_empty() => SessionSummary {
            conversation_id: conversation_id.to_string(),
            channel: channel.to_string(),
            timestamp,
            title: if r.title.is_empty() {
                "Conversation".to_string()
            } else {
                r.title
            },
            gist: r.gist,
            decisions: r.decisions,
            open_threads: r.open_threads,
            user_notes: r.user_notes,
        },
        _ => SessionSummary {
            conversation_id: conversation_id.to_string(),
            channel: channel.to_string(),
            timestamp,
            title: "Conversation".to_string(),
            gist: raw.chars().take(280).collect(),
            decisions: vec![],
            open_threads: vec![],
            user_notes: vec![],
        },
    }
}

/// Distills a conversation's turns into a structured [`SessionSummary`] via the LLM.
pub struct SessionSummarizer {
    llm: std::sync::Arc<dyn ironclaw_llm::LlmProvider>,
}

impl SessionSummarizer {
    pub fn new(llm: std::sync::Arc<dyn ironclaw_llm::LlmProvider>) -> Self {
        Self { llm }
    }

    /// Summarize a conversation. `turns` is `(user_input, assistant_response?)`.
    pub async fn summarize(
        &self,
        conversation_id: &str,
        channel: &str,
        timestamp: DateTime<Utc>,
        turns: &[(String, Option<String>)],
    ) -> Result<SessionSummary, crate::error::Error> {
        use ironclaw_llm::ChatMessage;
        let mut convo = String::new();
        for (u, a) in turns {
            convo.push_str(&format!("User: {u}\n"));
            if let Some(a) = a {
                convo.push_str(&format!("Assistant: {a}\n"));
            }
        }
        let sys = ChatMessage::system(
            "Summarize this conversation as STRICT JSON with keys: title (short), \
             gist (1-3 sentences), decisions (string[]), open_threads (string[] of \
             unfinished items or next steps), user_notes (string[] of notable user \
             context). Output ONLY the JSON object, no prose.",
        );
        let user = ChatMessage::user(convo);
        let raw = self.complete(vec![sys, user]).await?;
        Ok(parse_summary_json(&raw, conversation_id, channel, timestamp))
    }

    /// Run the messages through the LLM (mirrors `ContextCompactor::generate_summary`).
    async fn complete(
        &self,
        messages: Vec<ironclaw_llm::ChatMessage>,
    ) -> Result<String, crate::error::Error> {
        use ironclaw_llm::{CompletionRequest, Reasoning};
        let request = CompletionRequest::new(messages)
            .with_max_tokens(1024)
            .with_temperature(0.3);
        let reasoning =
            Reasoning::new(self.llm.clone()).with_model_name(self.llm.active_model_name());
        let (text, _) = reasoning.complete(request).await?;
        Ok(text)
    }
}

/// Coordinator: distills a conversation and writes the per-session file +
/// `recent.md` digest. Fail-soft — every error is logged, never propagated
/// into a conversation turn.
pub struct SessionMemory {
    summarizer: SessionSummarizer,
    workspace: std::sync::Arc<crate::workspace::Workspace>,
}

impl SessionMemory {
    pub fn new(
        llm: std::sync::Arc<dyn ironclaw_llm::LlmProvider>,
        workspace: std::sync::Arc<crate::workspace::Workspace>,
    ) -> Self {
        Self {
            summarizer: SessionSummarizer::new(llm),
            workspace,
        }
    }

    /// Summarize a conversation and durably store it. Skips trivial
    /// conversations (no substantive user turns) and is idempotent on
    /// `conversation_id` (a pre-existing per-session file means "already
    /// summarized"). All failures are logged; none propagate.
    pub async fn summarize_and_store(
        &self,
        conversation_id: &str,
        channel: &str,
        timestamp: DateTime<Utc>,
        turns: &[(String, Option<String>)],
    ) {
        // Skip trivial: nothing worth remembering if every user turn is empty.
        if turns.iter().all(|(u, _)| u.trim().is_empty()) {
            return;
        }
        let stem = format!("{}-{}", timestamp.format("%Y-%m-%d"), conversation_id);
        let path = sessions_path(&stem);
        // Idempotency: a file already existing for this conversation means we
        // already summarized it — skip (guards session-end vs backstop races).
        if self.file_exists(&path).await {
            return;
        }
        let summary = match self
            .summarizer
            .summarize(conversation_id, channel, timestamp, turns)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("session summary failed for {conversation_id}: {e}");
                return;
            }
        };
        if let Err(e) = self.workspace.write(&path, &summary.to_markdown()).await {
            tracing::warn!("write session file failed for {conversation_id}: {e}");
            return;
        }
        let existing = self.read_or_empty(RECENT_PATH).await;
        let recent = build_recent(&summary.digest_entry(), &existing, 5, 6000);
        if let Err(e) = self.workspace.write(RECENT_PATH, &recent).await {
            tracing::warn!("update recent.md failed for {conversation_id}: {e}");
        }
    }

    /// True if a document already exists at `path` (any read error other than
    /// "found" is treated as absent — the backstop sweep will retry).
    async fn file_exists(&self, path: &str) -> bool {
        self.workspace.read(path).await.is_ok()
    }

    /// Read a document's content, or the empty string if it is absent.
    async fn read_or_empty(&self, path: &str) -> String {
        self.workspace
            .read(path)
            .await
            .map(|d| d.content)
            .unwrap_or_default() // silent-ok: recent.md may not exist yet; empty = fresh digest
    }
}

#[cfg(test)]
impl SessionMemory {
    /// Test-only: read a workspace document's content, or empty on absence.
    async fn workspace_read(&self, path: &str) -> String {
        self.read_or_empty(path).await
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

    #[test]
    fn build_recent_prepends_and_caps_count() {
        let e1 = "### 2026-07-01 — A\ngist a\n";
        let e2 = "### 2026-07-02 — B\ngist b\n";
        let e3 = "### 2026-07-03 — C\ngist c\n";
        let r1 = build_recent(e1, "", 2, 6000);
        assert!(r1.starts_with("# Recent conversations"));
        let r2 = build_recent(e2, &r1, 2, 6000);
        let r3 = build_recent(e3, &r2, 2, 6000);
        // newest first, only 2 kept
        let pos_c = r3.find("— C").unwrap();
        let pos_b = r3.find("— B").unwrap();
        assert!(pos_c < pos_b, "newest first");
        assert!(!r3.contains("— A"), "oldest dropped by count cap");
    }

    #[test]
    fn build_recent_size_cap_drops_wrapped_before_open() {
        let open = "### 2026-07-01 — Open\ngist\n  \n  _open:_ finish X\n";
        let wrapped = "### 2026-07-02 — Wrapped\ngist\n";
        let acc = build_recent(open, "", 5, 6000);
        let acc = build_recent(wrapped, &acc, 5, 60); // tiny cap forces a drop
        assert!(
            acc.contains("— Open"),
            "open-thread entry retained under size pressure"
        );
    }

    #[tokio::test]
    async fn summarize_parses_structured_fields() {
        use std::sync::Arc;
        let json = r#"```json
        {"title":"Gateway fix","gist":"Fixed the profile bug.","decisions":["ship one-liner"],
         "open_threads":["watch reinstall"],"user_notes":["values durability"]}
        ```"#;
        let llm = Arc::new(crate::testing::StubLlm::new(json));
        let s = SessionSummarizer::new(llm);
        let ts = chrono::Utc::now();
        let out = s
            .summarize(
                "c1",
                "gateway",
                ts,
                &[("what broke?".into(), Some("the profile".into()))],
            )
            .await
            .unwrap();
        assert_eq!(out.title, "Gateway fix");
        assert_eq!(out.decisions, vec!["ship one-liner".to_string()]);
        assert_eq!(out.open_threads, vec!["watch reinstall".to_string()]);
        assert_eq!(out.conversation_id, "c1");
    }

    #[test]
    fn parse_summary_json_falls_back_on_garbage() {
        let ts = chrono::Utc::now();
        let out = parse_summary_json("not json at all", "c2", "cli", ts);
        assert_eq!(out.title, "Conversation");
        assert!(out.gist.contains("not json"));
    }

    #[cfg(feature = "libsql")]
    async fn test_workspace() -> (std::sync::Arc<crate::workspace::Workspace>, tempfile::TempDir) {
        use crate::db::libsql::LibSqlBackend;
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("session_memory_test.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("LibSqlBackend");
        <LibSqlBackend as crate::db::Database>::run_migrations(&backend)
            .await
            .expect("migrations");
        let db: std::sync::Arc<dyn crate::db::Database> = std::sync::Arc::new(backend);
        let ws = crate::workspace::Workspace::new_with_db("test_session_memory", db);
        (std::sync::Arc::new(ws), temp_dir)
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn store_writes_session_file_and_updates_recent() {
        let (ws, _dir) = test_workspace().await;
        let llm = std::sync::Arc::new(crate::testing::StubLlm::new(
            r#"{"title":"T","gist":"g","open_threads":["x"]}"#,
        ));
        let mem = SessionMemory::new(llm, ws);
        let ts = chrono::Utc::now();
        mem.summarize_and_store("conv9", "cli", ts, &[("hi".into(), Some("hello".into()))])
            .await;

        let stem = format!("{}-conv9", ts.format("%Y-%m-%d"));
        let file = mem.workspace_read(&sessions_path(&stem)).await;
        assert!(file.contains("title: T"), "session file has frontmatter title");
        let recent = mem.workspace_read(RECENT_PATH).await;
        assert!(recent.contains("— T"), "recent.md has the digest entry");

        // idempotent: a second call does not double-append to recent.md
        mem.summarize_and_store("conv9", "cli", ts, &[("hi".into(), Some("hello".into()))])
            .await;
        let recent2 = mem.workspace_read(RECENT_PATH).await;
        assert_eq!(
            recent2.matches("— T").count(),
            1,
            "idempotent: no double-append"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn store_skips_trivial_conversation() {
        let (ws, _dir) = test_workspace().await;
        let llm = std::sync::Arc::new(crate::testing::StubLlm::new(
            r#"{"title":"T","gist":"g"}"#,
        ));
        let mem = SessionMemory::new(llm, ws);
        let ts = chrono::Utc::now();
        // Only empty user turns -> skip; no file written.
        mem.summarize_and_store("trivial", "cli", ts, &[("   ".into(), Some("hi".into()))])
            .await;
        let stem = format!("{}-trivial", ts.format("%Y-%m-%d"));
        assert!(
            !mem.file_exists(&sessions_path(&stem)).await,
            "trivial conversation must not write a session file"
        );
    }
}
