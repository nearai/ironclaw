# Episodic Memory Implementation Plan (Sub-project 1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give IronClaw automatic cross-session continuity — distill every conversation into a durable, searchable session summary when it ends, and silently inject a terse digest of the recent ones into every new conversation's system prompt.

**Architecture:** A new `src/agent/session_memory.rs` module owns a `SessionSummary` type, a structured LLM summarizer, and the episodic store (per-session markdown files + a capped `recent.md` digest). Auto-write is wired into the session-manager's idle-prune and a heartbeat/startup backstop; auto-recall is a one-line inclusion of `recent.md` in `Workspace::system_prompt_for_context`.

**Tech Stack:** Rust, tokio, serde/serde_yaml, `ironclaw_llm` (`LlmProvider`, `ChatMessage`), the existing `Workspace` API.

**Spec:** `docs/superpowers/specs/2026-07-09-episodic-memory-design.md`

## Global Constraints

- No `.unwrap()` / `.expect()` in production code (tests fine). Map errors with `?` + context. (`src/agent/CLAUDE.md`)
- Every feature/bugfix commit includes a regression test (commit-msg hook enforces).
- Zero clippy warnings: `cargo clippy --all --benches --tests --examples --all-features`.
- Prefer `crate::` for cross-module imports; `super::` only in tests / intra-module.
- Build/test with `~/.cargo/bin/cargo`.
- **Memory never blocks a conversation:** every auto-write / auto-recall path is fire-and-forget or fail-soft — log and continue, never propagate an error into a turn.
- **Idempotent on `conversation_id`:** a per-session file already existing for a `conversation_id` means "already summarized" — skip.
- **`recent.md` is capped:** last `N` (default 5) entries AND ≤ ~1500 tokens (≈6000 chars), whichever binds first; entries are terse (title + gist + open threads only); entries with non-empty open threads are retained preferentially.
- Storage lives under the workspace at `memory/sessions/<file>.md` and `memory/recent.md` (relative to the workspace root, via `Workspace::read`/`write`).

## File Structure

```
src/agent/session_memory.rs   (NEW) — SessionSummary, SessionSummarizer, SessionMemory (store + digest + backstop)
src/agent/mod.rs              (MODIFY) — `pub mod session_memory;`
src/workspace/mod.rs          (MODIFY) — include memory/recent.md in system_prompt_for_context
src/agent/session_manager.rs  (MODIFY) — call SessionMemory on idle-prune
src/agent/heartbeat.rs        (MODIFY) — backstop sweep on each heartbeat tick
src/agent/agent_loop.rs       (MODIFY) — construct SessionMemory, hand it to session_manager
```

---

### Task 1: `SessionSummary` type + markdown serialization

**Files:**
- Create: `src/agent/session_memory.rs`
- Modify: `src/agent/mod.rs` (add `pub mod session_memory;`)

**Interfaces:**
- Produces:
  - `pub struct SessionSummary { pub conversation_id: String, pub channel: String, pub timestamp: DateTime<Utc>, pub title: String, pub gist: String, pub decisions: Vec<String>, pub open_threads: Vec<String>, pub user_notes: Vec<String> }`
  - `impl SessionSummary { pub fn to_markdown(&self) -> String; pub fn digest_entry(&self) -> String; pub fn file_stem(&self) -> String }`
  - `pub fn sessions_path(stem: &str) -> String` → `format!("memory/sessions/{stem}.md")`

- [ ] **Step 1: Write the failing test**

```rust
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
        assert_eq!(sessions_path(&sample().file_stem()), "memory/sessions/2026-07-09-abc123.md");
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
```

- [ ] **Step 2: Run → fail**

Run: `~/.cargo/bin/cargo test -p ironclaw session_memory::tests`
Expected: FAIL — module/type not found.

- [ ] **Step 3: Implement**

`src/agent/session_memory.rs`:
```rust
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
        format!("{}-{}", self.timestamp.format("%Y-%m-%d"), self.conversation_id)
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
```

Add to `src/agent/mod.rs` (alphabetically near other `pub mod`s): `pub mod session_memory;`

- [ ] **Step 4: Run → pass**

Run: `~/.cargo/bin/cargo test -p ironclaw session_memory::tests`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/agent/session_memory.rs src/agent/mod.rs
git commit -m "feat(memory): SessionSummary type + markdown/digest serialization"
```

---

### Task 2: `recent.md` digest builder (prepend + N-cap + size-cap + open-thread weighting)

**Files:**
- Modify: `src/agent/session_memory.rs`

**Interfaces:**
- Produces: `pub fn build_recent(new_entry: &str, existing: &str, max_entries: usize, max_chars: usize) -> String` — parse `existing` into `### `-delimited entries, prepend `new_entry`, keep at most `max_entries`, drop to fit `max_chars` (dropping fully-wrapped entries — those with no `_open:_` line — before open ones), and return the rebuilt file (with a leading `# Recent conversations\n\n` header).

- [ ] **Step 1: Write the failing test**

```rust
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
    assert!(acc.contains("— Open"), "open-thread entry retained under size pressure");
}
```

- [ ] **Step 2: Run → fail** — `build_recent` not defined.

- [ ] **Step 3: Implement** (append to `session_memory.rs`)

```rust
const RECENT_HEADER: &str = "# Recent conversations\n\n";

fn split_entries(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in body.lines() {
        if line.starts_with("### ") && !cur.is_empty() {
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

pub fn build_recent(new_entry: &str, existing: &str, max_entries: usize, max_chars: usize) -> String {
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
        let idx = entries.iter().rposition(|e| !has_open(e)).unwrap_or(entries.len() - 1);
        entries.remove(idx);
    }
    let mut out = String::from(RECENT_HEADER);
    for e in &entries {
        out.push_str(e.trim_end());
        out.push_str("\n\n");
    }
    out
}
```

- [ ] **Step 4: Run → pass.**

- [ ] **Step 5: Commit**

```bash
git add src/agent/session_memory.rs
git commit -m "feat(memory): recent.md digest builder with count/size caps + open-thread weighting"
```

---

### Task 3: `SessionSummarizer` — turns → structured `SessionSummary`

**Files:**
- Modify: `src/agent/session_memory.rs`

**Interfaces:**
- Consumes: `ironclaw_llm::{LlmProvider, ChatMessage, CompletionRequest}`.
- Produces:
  - `pub struct SessionSummarizer { llm: std::sync::Arc<dyn ironclaw_llm::LlmProvider> }`
  - `impl SessionSummarizer { pub fn new(llm) -> Self; pub async fn summarize(&self, conversation_id: &str, channel: &str, timestamp: DateTime<Utc>, turns: &[(String, Option<String>)]) -> Result<SessionSummary, crate::error::Error> }`
  - `pub(crate) fn parse_summary_json(raw: &str, conversation_id: &str, channel: &str, timestamp: DateTime<Utc>) -> SessionSummary` — tolerant parse of the model's JSON (fenced or bare); on parse failure, fall back to `{title:"Conversation", gist:<raw truncated>, ...}` so a malformed model reply never fails the pipeline.

**Confirm first:** `grep -n "impl.*LlmProvider\|async fn complete\|CompletionRequest" crates/ironclaw_llm/src/lib.rs | head` — confirm the `LlmProvider::complete`/`generate` method name + `CompletionRequest` builder used elsewhere (mirror `ContextCompactor::generate_summary` in `src/agent/compaction.rs:187` for the exact call shape).

- [ ] **Step 1: Write the failing test** (uses `crate::testing::StubLlm` to return canned JSON)

```rust
#[tokio::test]
async fn summarize_parses_structured_fields() {
    use std::sync::Arc;
    let json = r#"```json
    {"title":"Gateway fix","gist":"Fixed the profile bug.","decisions":["ship one-liner"],
     "open_threads":["watch reinstall"],"user_notes":["values durability"]}
    ```"#;
    let llm = Arc::new(crate::testing::StubLlm::with_response(json));
    let s = SessionSummarizer::new(llm);
    let ts = chrono::Utc::now();
    let out = s.summarize("c1", "gateway", ts,
        &[("what broke?".into(), Some("the profile".into()))]).await.unwrap();
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
```

**Confirm first (test helper):** `grep -n "impl StubLlm\|fn with_response\|with_responses" crates/ironclaw_llm/src/testing.rs src/testing/mod.rs` — confirm the stub constructor name; adjust `StubLlm::with_response` to the real helper (it may be `with_responses(vec![...])`).

- [ ] **Step 2: Run → fail.**

- [ ] **Step 3: Implement** (append). The prompt asks for strict JSON with those five fields; `parse_summary_json` strips ``` fences, `serde_json::from_str` into a `#[derive(Deserialize)] struct Raw{...}` with `#[serde(default)]` on every field, and builds the `SessionSummary`; on `Err`, returns the fallback. `summarize` builds `CompletionRequest` from a system prompt + the formatted turns (mirror `compaction.rs::generate_summary`), calls the llm, and delegates to `parse_summary_json`.

```rust
use serde::Deserialize;

#[derive(Deserialize, Default)]
struct RawSummary {
    #[serde(default)] title: String,
    #[serde(default)] gist: String,
    #[serde(default)] decisions: Vec<String>,
    #[serde(default)] open_threads: Vec<String>,
    #[serde(default)] user_notes: Vec<String>,
}

pub(crate) fn parse_summary_json(
    raw: &str, conversation_id: &str, channel: &str, timestamp: DateTime<Utc>,
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
            title: if r.title.is_empty() { "Conversation".into() } else { r.title },
            gist: r.gist,
            decisions: r.decisions,
            open_threads: r.open_threads,
            user_notes: r.user_notes,
        },
        _ => SessionSummary {
            conversation_id: conversation_id.to_string(),
            channel: channel.to_string(),
            timestamp,
            title: "Conversation".into(),
            gist: raw.chars().take(280).collect(),
            decisions: vec![],
            open_threads: vec![],
            user_notes: vec![],
        },
    }
}

pub struct SessionSummarizer {
    llm: std::sync::Arc<dyn ironclaw_llm::LlmProvider>,
}

impl SessionSummarizer {
    pub fn new(llm: std::sync::Arc<dyn ironclaw_llm::LlmProvider>) -> Self {
        Self { llm }
    }

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
             unfinished items/next steps), user_notes (string[] of notable user context). \
             Output ONLY the JSON object.",
        );
        let user = ChatMessage::user(&convo);
        // MIRROR compaction.rs::generate_summary for the exact request build + call:
        let raw = self.complete(vec![sys, user]).await?;
        Ok(parse_summary_json(&raw, conversation_id, channel, timestamp))
    }

    // Implement `complete` by copying the CompletionRequest build + llm call from
    // ContextCompactor::generate_summary (src/agent/compaction.rs:187). Returns the
    // model's text.
    async fn complete(&self, messages: Vec<ironclaw_llm::ChatMessage>) -> Result<String, crate::error::Error> {
        // <fill from compaction.rs generate_summary body — same request shape>
        unimplemented!("copy request build from compaction.rs::generate_summary")
    }
}
```
NOTE for the implementer: replace the `complete` body by copying the `CompletionRequest`/`self.llm.complete(...)` sequence verbatim from `ContextCompactor::generate_summary` (compaction.rs:187+) — it already does exactly this (build request from messages, call the provider, return text). Do not invent a new call shape.

- [ ] **Step 4: Run → pass.**

- [ ] **Step 5: Commit**

```bash
git add src/agent/session_memory.rs
git commit -m "feat(memory): structured session summarizer (turns -> SessionSummary)"
```

---

### Task 4: `SessionMemory::summarize_and_store` (skip-trivial, idempotent, write files)

**Files:**
- Modify: `src/agent/session_memory.rs`

**Interfaces:**
- Consumes: `SessionSummarizer` (Task 3), `SessionSummary`/`build_recent`/`sessions_path`/`RECENT_PATH` (Tasks 1–2), `crate::workspace::Workspace` (`read`/`write`).
- Produces:
  - `pub struct SessionMemory { summarizer: SessionSummarizer, workspace: std::sync::Arc<crate::workspace::Workspace> }`
  - `impl SessionMemory { pub fn new(llm, workspace) -> Self; pub async fn summarize_and_store(&self, conversation_id, channel, timestamp, turns: &[(String, Option<String>)]) }` — returns `()` (fail-soft: logs on error). Skips if `turns` has no user text; skips if the per-session file already exists (idempotent).

- [ ] **Step 1: Write the failing test** (temp workspace + stub llm)

**Confirm first:** `grep -n "pub async fn read\|WorkspaceError::NotFound\|fn new_for_test\|Workspace::" src/workspace/mod.rs | head` — confirm how to (a) build a `Workspace` in a unit test (there is a test constructor / in-memory backend used by other workspace tests — mirror one), and (b) detect "file absent" from `read` (a `WorkspaceError::NotFound` variant vs `Ok`). Wire the idempotency check to whichever `read` returns for a missing path.

```rust
#[tokio::test]
async fn store_writes_session_file_and_updates_recent() {
    let ws = /* build a temp/in-memory Workspace — mirror an existing workspace test */;
    let llm = std::sync::Arc::new(crate::testing::StubLlm::with_response(
        r#"{"title":"T","gist":"g","open_threads":["x"]}"#));
    let mem = SessionMemory::new(llm, std::sync::Arc::new(ws));
    let ts = chrono::Utc::now();
    mem.summarize_and_store("conv9", "cli", ts,
        &[("hi".into(), Some("hello".into()))]).await;

    let stem = format!("{}-conv9", ts.format("%Y-%m-%d"));
    let file = mem.workspace_read(&sessions_path(&stem)).await;
    assert!(file.contains("title: T"));
    let recent = mem.workspace_read(RECENT_PATH).await;
    assert!(recent.contains("— T"));

    // idempotent: a second call does not double-append to recent.md
    mem.summarize_and_store("conv9", "cli", ts,
        &[("hi".into(), Some("hello".into()))]).await;
    let recent2 = mem.workspace_read(RECENT_PATH).await;
    assert_eq!(recent2.matches("— T").count(), 1);
}

#[tokio::test]
async fn store_skips_trivial_conversation() {
    // turns with empty user text -> no file written
}
```
(Add a small `#[cfg(test)] async fn workspace_read` test helper on `SessionMemory`, or read via the workspace handle directly.)

- [ ] **Step 2: Run → fail.**

- [ ] **Step 3: Implement**

```rust
pub struct SessionMemory {
    summarizer: SessionSummarizer,
    workspace: std::sync::Arc<crate::workspace::Workspace>,
}

impl SessionMemory {
    pub fn new(
        llm: std::sync::Arc<dyn ironclaw_llm::LlmProvider>,
        workspace: std::sync::Arc<crate::workspace::Workspace>,
    ) -> Self {
        Self { summarizer: SessionSummarizer::new(llm), workspace }
    }

    pub async fn summarize_and_store(
        &self,
        conversation_id: &str,
        channel: &str,
        timestamp: DateTime<Utc>,
        turns: &[(String, Option<String>)],
    ) {
        // skip trivial
        if turns.iter().all(|(u, _)| u.trim().is_empty()) {
            return;
        }
        let stem = format!("{}-{}", timestamp.format("%Y-%m-%d"), conversation_id);
        let path = sessions_path(&stem);
        // idempotency: if a file already exists, skip (per confirm-first read semantics)
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
            tracing::warn!("write session file failed: {e}");
            return;
        }
        let existing = self.read_or_empty(RECENT_PATH).await;
        let recent = build_recent(&summary.digest_entry(), &existing, 5, 6000);
        if let Err(e) = self.workspace.write(RECENT_PATH, &recent).await {
            tracing::warn!("update recent.md failed: {e}");
        }
    }

    async fn file_exists(&self, path: &str) -> bool {
        self.workspace.read(path).await.is_ok()
    }

    async fn read_or_empty(&self, path: &str) -> String {
        // adjust `.content` accessor to MemoryDocument's real field per confirm-first
        self.workspace.read(path).await.map(|d| d.content).unwrap_or_default()
    }
}
```
**Confirm first:** the `MemoryDocument` content field name (`.content` vs `.text` vs a method) — `grep -n "pub struct MemoryDocument\|pub content\|pub text" src/workspace/*.rs`.

- [ ] **Step 4: Run → pass.**

- [ ] **Step 5: Commit**

```bash
git add src/agent/session_memory.rs
git commit -m "feat(memory): SessionMemory store — skip-trivial, idempotent, writes session file + recent.md"
```

---

### Task 5: Auto-recall — inject `recent.md` into the system prompt

**Files:**
- Modify: `src/workspace/mod.rs` (`system_prompt_for_context`)

**Interfaces:**
- Consumes: `Workspace::read("memory/recent.md")`.
- Produces: the assembled system prompt includes a "Recent conversations" section when `memory/recent.md` exists and is non-empty; unchanged otherwise.

**Confirm first:** `sed -n '1670,1760p' src/workspace/mod.rs` — read `system_prompt_for_context` fully; find where it appends identity/MEMORY sections (the `push_str`/`format!` assembly). Insert the recall section at the end of the assembly (after MEMORY.md), guarded by group-chat exclusion the same way MEMORY.md is (recent-conversation recall is personal context — exclude it in group chats, mirroring the MEMORY.md rule noted at mod.rs:~1670).

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn system_prompt_includes_recent_when_present() {
    let ws = /* temp workspace, mirror existing workspace tests */;
    ws.write("memory/recent.md", "# Recent conversations\n\n### 2026-07-09 — Voice\nwired STT\n").await.unwrap();
    let sp = ws.system_prompt().await.unwrap();
    assert!(sp.contains("Recent conversations"));
    assert!(sp.contains("Voice"));
}

#[tokio::test]
async fn system_prompt_omits_recent_when_absent() {
    let ws = /* temp workspace, no recent.md */;
    let sp = ws.system_prompt().await.unwrap();
    assert!(!sp.contains("Recent conversations"));
}
```

- [ ] **Step 2: Run → fail.**

- [ ] **Step 3: Implement** — in `system_prompt_for_context`, after the existing MEMORY.md handling, add:
```rust
// Episodic recall: include the recent-conversations digest (personal context;
// excluded in group chats, like MEMORY.md).
if !is_group_chat {
    if let Ok(doc) = self.read("memory/recent.md").await {
        let recent = doc.content.trim();   // adjust `.content` per confirm-first
        if !recent.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(recent);
        }
    }
}
```
(Use the actual accumulator variable name from the function — likely `prompt` or `sections`; match it.)

- [ ] **Step 4: Run → pass; then full workspace suite: `cargo test -p ironclaw workspace::`**

- [ ] **Step 5: Commit**

```bash
git add src/workspace/mod.rs
git commit -m "feat(memory): inject recent.md digest into the system prompt (auto-recall)"
```

---

### Task 6: Auto-write wiring — summarize on session idle-prune

**Files:**
- Modify: `src/agent/session_manager.rs`, `src/agent/agent_loop.rs`

**Interfaces:**
- Consumes: `SessionMemory` (Task 4); the in-memory `Session`/`Thread`/`Turn` (`turns: Vec<Turn>`, `Turn { user_input, response }`) available at prune time.
- Produces: `SessionManager` gains `session_memory: Option<Arc<SessionMemory>>` (set via a setter, mirroring how other optional deps are set); on idle-prune, for each stale thread with turns, spawn `session_memory.summarize_and_store(...)`.

**Confirm first:**
- `grep -n "OnSessionEnd\|stale_sessions\|stale_thread\|for.*stale\|fn prune\|fn cleanup\|conversation_id\|thread.id" src/agent/session_manager.rs` — find the prune loop (around line 340) and how a thread maps to a `conversation_id` + channel. Each stale `Thread` has `turns`; build `Vec<(String, Option<String>)>` from `turns.iter().map(|t| (t.user_input.clone(), t.response.clone()))`.
- `grep -n "SessionManager::new\|fn set_\|pub fn with_\|session_manager\b" src/agent/agent_loop.rs` — find where the `SessionManager` is created in `Agent::new` and add the `SessionMemory` (built from `deps.cheap_llm`/`deps.llm` + `deps.workspace`) via a setter, only when both a workspace and llm are present.

- [ ] **Step 1:** Add `session_memory: Option<Arc<SessionMemory>>` to `SessionManager` + `pub fn set_session_memory(&mut self, m: Arc<SessionMemory>)` (init `None` in its constructor).

- [ ] **Step 2:** In the idle-prune loop, before/after firing `OnSessionEnd`, for each stale thread that has ≥1 turn:
```rust
if let Some(mem) = self.session_memory.clone() {
    let conv_id = /* thread's conversation_id as String */;
    let channel = /* session/thread channel */;
    let ts = chrono::Utc::now();
    let turns: Vec<(String, Option<String>)> =
        thread.turns.iter().map(|t| (t.user_input.clone(), t.response.clone())).collect();
    tokio::spawn(async move {
        mem.summarize_and_store(&conv_id, &channel, ts, &turns).await;
    });
}
```

- [ ] **Step 3:** In `Agent::new` (agent_loop.rs), after the session_manager + workspace exist:
```rust
if let (Some(ws), _) = (self.deps.workspace.clone(), ()) {
    let llm = self.deps.cheap_llm.clone().unwrap_or_else(|| self.deps.llm.clone());
    let mem = std::sync::Arc::new(crate::agent::session_memory::SessionMemory::new(llm, ws));
    session_manager_mut.set_session_memory(mem);   // adapt to actual mutability/ownership
}
```
(Adjust to how `session_manager` is owned in `Agent::new` — it may be an `Arc`; if so, set the memory before wrapping in `Arc`, mirroring how the scheduler's setters are called before `Arc::new`.)

- [ ] **Step 4: Build** — `cargo build -p ironclaw`. Expected: compiles. (No new unit test: this is wiring, exercised by Task 8's manual e2e + Task 7's backstop test which shares `summarize_and_store`.) Include `[skip-regression-check]` in the commit if the hook demands a test.

- [ ] **Step 5: Commit**

```bash
git add src/agent/session_manager.rs src/agent/agent_loop.rs
git commit -m "feat(memory): summarize conversations on session idle-prune [skip-regression-check]"
```

---

### Task 7: Backstop sweep — summarize recently-ended conversations with no summary

**Files:**
- Modify: `src/agent/session_memory.rs` (add `sweep`), `src/agent/heartbeat.rs` (call it), `src/agent/agent_loop.rs` (startup call)

**Interfaces:**
- Consumes: `SessionMemory`; a store handle to enumerate recent conversations + load their turns.
- Produces: `impl SessionMemory { pub async fn sweep(&self, store: &SystemScope, since: DateTime<Utc>) -> usize }` — for each conversation with `updated_at >= since` and no per-session file, load its messages, summarize, store; returns count. Idempotent (the per-session-file check in `summarize_and_store` guards double-writes).

**Confirm first:**
- `grep -n "fn list_conversations_with_preview\|struct.*Conversation\|conversation_id\|updated_at\|fn list_messages\|fn get_conversation_messages\|messages" src/history/store.rs src/db/mod.rs` — find (a) how to list recent conversations for the owner and (b) how to load a conversation's messages/turns from the DB (the method that returns the user/assistant message pairs). Map each message pair to `(String, Option<String>)`.
- If no direct "messages for conversation" method exists, use the one the gateway history endpoint uses (`grep -rn "history\|build_turns_from_db_messages" src/channels/web/util.rs`).

- [ ] **Step 1:** Implement `sweep` in `session_memory.rs`:
```rust
pub async fn sweep(&self, store: &crate::tenant::SystemScope, since: DateTime<Utc>) -> usize {
    let convos = match /* store.list_recent_conversations(since) per confirm-first */ {
        Ok(c) => c,
        Err(e) => { tracing::warn!("memory sweep list failed: {e}"); return 0; }
    };
    let mut n = 0;
    for c in convos {
        let stem = format!("{}-{}", c.updated_at.format("%Y-%m-%d"), c.id);
        if self.file_exists(&sessions_path(&stem)).await { continue; }
        let turns = /* load message pairs for c.id per confirm-first */;
        if turns.is_empty() { continue; }
        self.summarize_and_store(&c.id.to_string(), &c.channel, c.updated_at, &turns).await;
        n += 1;
    }
    n
}
```

- [ ] **Step 2: Integration test** (`--features integration`, temp/real store): insert a conversation with a couple of messages and no summary file; run `sweep`; assert a per-session file exists and `recent.md` updated; run `sweep` again → returns 0 new (idempotent). Mirror an existing `tests/*_integration.rs` store-setup harness.

- [ ] **Step 3:** Call `sweep` from the heartbeat tick (heartbeat.rs `run()` loop, once per tick, `since = now - 24h`) and once at startup in `Agent::new`, both guarded on `session_memory` + `store` present, both `tokio::spawn` fire-and-forget.

- [ ] **Step 4: Build + integration test pass.**

- [ ] **Step 5: Commit**

```bash
git add src/agent/session_memory.rs src/agent/heartbeat.rs src/agent/agent_loop.rs
git commit -m "feat(memory): backstop sweep summarizes un-summarized ended conversations"
```

---

### Task 8: Full gate + docs

**Files:**
- Modify: `src/workspace/README.md` (document the `memory/sessions/` + `memory/recent.md` convention)

- [ ] **Step 1: Full lint + test gate**
```bash
~/.cargo/bin/cargo test -p ironclaw session_memory:: && \
~/.cargo/bin/cargo test -p ironclaw workspace:: && \
~/.cargo/bin/cargo clippy -p ironclaw --tests --all-features 2>&1 | grep -E "warning|error" || echo "clippy clean"
grep -nE '\.unwrap\(|\.expect\(' src/agent/session_memory.rs | grep -v test || echo "no prod unwrap/expect"
```
Expected: all pass; clippy clean; no prod panics.

- [ ] **Step 2: Manual e2e (on the box, after deploy):** hold a short conversation via OWUI/TUI, let it go idle past the prune interval (or restart to trigger the startup sweep), confirm a file appears under `memory/sessions/` and `memory/recent.md` updates; start a new conversation and confirm IronClaw silently has the recent context (ask "what were we just doing?").

- [ ] **Step 3: Docs** — add a "Episodic memory" subsection to `src/workspace/README.md`: `memory/sessions/YYYY-MM-DD-<conv>.md` per conversation (full, searchable), `memory/recent.md` terse digest (rides the system prompt), written on idle-prune + backstop sweep.

- [ ] **Step 4: Commit**

```bash
git add src/workspace/README.md
git commit -m "docs(memory): episodic memory workspace convention"
```

---

## Self-Review

**Spec coverage:** auto-write (T4, T6) ✓; per-session searchable files + recent.md digest (T1, T2, T4) ✓; structured summarizer reusing compaction's pattern (T3) ✓; auto-recall injection, adaptive/silent, group-chat-excluded (T5) ✓; backstop sweep, idempotent, durable-from-DB (T7) ✓; two-channel model — push window (T5) + pull search over per-session files (inherent: files are workspace docs, already RRF-searchable) ✓; terse digest + N-cap + size-cap + open-thread weighting (T2) ✓; memory-never-blocks + fail-soft + idempotent-on-conversation_id (T4, T6, T7 constraints) ✓; token budget (T2) ✓.

**Placeholders:** three `complete`/`sweep`/workspace-test spots are marked **confirm-first** with the exact grep to run and the existing code to mirror (`compaction.rs::generate_summary`, the workspace test harness, the DB message-load) — these are 2-minute in-situ lookups, not design gaps, matching the MCP-jobs plan's pattern. Everything else is complete code.

**Type consistency:** `SessionSummary` fields, `sessions_path`/`RECENT_PATH`, `build_recent(new,existing,max_entries,max_chars)`, `SessionSummarizer::summarize(conversation_id,channel,timestamp,turns)`, `SessionMemory::{new,summarize_and_store,sweep}` are used consistently across tasks. Turns are `&[(String, Option<String>)]` everywhere.

**Watch items for execution:** the LLM `complete` call shape (mirror compaction), the `StubLlm` helper name, the `MemoryDocument` content accessor, the workspace test constructor, and the DB "messages for a conversation" method — each has a confirm-first grep. None block the design.
