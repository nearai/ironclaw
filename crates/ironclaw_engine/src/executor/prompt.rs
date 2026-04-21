//! System prompt construction for the execution loop.
//!
//! Builds a CodeAct/RLM system prompt that instructs the LLM to write
//! Python code in ```repl blocks with tools available as callable functions.
//!
//! Prompt templates live in `crates/ironclaw_engine/prompts/` as plain
//! markdown files for easy inspection and iteration. They are embedded
//! at compile time via `include_str!` and can be extended at runtime with
//! prompt overlays stored as MemoryDocs.

use std::sync::Arc;

use crate::traits::store::Store;
use crate::types::capability::ActionDef;
use crate::types::project::ProjectId;

/// Runtime platform metadata injected into system prompts for self-awareness.
///
/// Provides the agent with knowledge about its own identity and environment
/// so it can answer questions about itself, its capabilities, and its
/// configuration without relying on training data.
#[derive(Debug, Clone, Default)]
pub struct PlatformInfo {
    /// Software version (from CARGO_PKG_VERSION).
    pub version: Option<String>,
    /// LLM backend name (e.g. "nearai", "openai", "anthropic").
    pub llm_backend: Option<String>,
    /// Active model name.
    pub model_name: Option<String>,
    /// Database backend (e.g. "libsql", "postgres").
    pub database_backend: Option<String>,
    /// Active channel names (e.g. ["telegram", "cli"]).
    pub active_channels: Vec<String>,
    /// Owner identifier.
    pub owner_id: Option<String>,
    /// Project repository URL.
    pub repo_url: Option<String>,
}

impl PlatformInfo {
    /// Format as a prompt section. Returns empty string if no info is set.
    pub fn to_prompt_section(&self) -> String {
        let mut lines = Vec::new();

        lines.push("You are **IronClaw**, a secure autonomous AI assistant platform.".into());
        if let Some(ref v) = self.version {
            lines.push(format!("- Version: {v}"));
        }
        if let Some(ref repo) = self.repo_url {
            lines.push(format!("- Repository: {repo}"));
        }
        if let Some(ref owner) = self.owner_id {
            lines.push(format!("- Owner: {owner}"));
        }
        if let Some(ref backend) = self.llm_backend {
            let model = self.model_name.as_deref().unwrap_or("default");
            lines.push(format!("- LLM: {backend} ({model})"));
        }
        if let Some(ref db) = self.database_backend {
            lines.push(format!("- Database: {db}"));
        }
        if !self.active_channels.is_empty() {
            lines.push(format!("- Channels: {}", self.active_channels.join(", ")));
        }

        if lines.len() <= 1 {
            // Only the identity line, no runtime details — still include it
            return format!("\n\n## Platform\n\n{}\n", lines[0]);
        }

        format!("\n\n## Platform\n\n{}\n", lines.join("\n"))
    }
}

/// The main instruction block (before tool listing).
const CODEACT_PREAMBLE: &str = include_str!("../../prompts/codeact_preamble.md");

/// Well-known title for the CodeAct preamble overlay.
pub const PREAMBLE_OVERLAY_TITLE: &str = "prompt:codeact_preamble";

/// Well-known tag for prompt overlay docs.
pub const PROMPT_OVERLAY_TAG: &str = "prompt_overlay";

/// Maximum size for a prompt overlay document (in chars).
const MAX_PROMPT_OVERLAY_CHARS: usize = 4000;

/// Build the system prompt for CodeAct/RLM execution.
///
/// The prompt instructs the LLM to:
/// - Write Python code in ```repl fenced blocks
/// - Call tools as regular Python functions
/// - Use llm_query(prompt, context) for sub-agent calls
/// - Use FINAL(answer) to return the final answer
/// - Access thread context via the `context` variable
///
/// If a Store is provided, checks for a runtime prompt overlay (a MemoryDoc
/// with tag "prompt_overlay" and title "prompt:codeact_preamble") and appends
/// its content after the compiled preamble. This enables the self-improvement
/// mission to evolve the system prompt at runtime.
pub async fn build_codeact_system_prompt(
    actions: &[ActionDef],
    store: Option<&Arc<dyn Store>>,
    project_id: ProjectId,
    platform: Option<&PlatformInfo>,
) -> String {
    let overlay = if let Some(store) = store {
        load_prompt_overlay(store, project_id).await
    } else {
        None
    };
    build_codeact_system_prompt_inner(actions, overlay.as_deref(), platform)
}

/// Build the system prompt using pre-fetched memory docs.
///
/// When the caller already has the `list_memory_docs` result (e.g. because
/// `load_orchestrator` fetched it), pass the docs here to avoid a duplicate
/// Store query.
pub fn build_codeact_system_prompt_with_docs(
    actions: &[ActionDef],
    system_docs: &[crate::types::memory::MemoryDoc],
    platform: Option<&PlatformInfo>,
) -> String {
    let overlay = extract_prompt_overlay(system_docs);
    build_codeact_system_prompt_inner(actions, overlay.as_deref(), platform)
}

/// Shared prompt builder used by both the async and pre-fetched-docs variants.
fn build_codeact_system_prompt_inner(
    actions: &[ActionDef],
    overlay: Option<&str>,
    platform: Option<&PlatformInfo>,
) -> String {
    // Layout for prompt caching: the stable-per-deployment prefix
    // (preamble + platform) comes first so providers can reuse the KV
    // cache across turns. "Stable per deployment" not "per session":
    // PlatformInfo includes version (bumps per release) and model_name
    // (can change if the user switches mid-session), so turns that straddle
    // those events will see a cache miss here — but every normal turn
    // within a single session/deployment hits. Dynamic sections
    // (learned-rules overlay, per-lease tool list) go last so lease changes
    // and overlay edits invalidate only the tail.
    let mut prompt = String::from(CODEACT_PREAMBLE);

    // Platform identity and runtime metadata (stable per deployment).
    if let Some(info) = platform {
        prompt.push_str(&info.to_prompt_section());
    }

    // Runtime prompt overlay (dynamic: self-improvement mission edits this).
    if let Some(overlay) = overlay {
        prompt.push_str("\n\n## Learned Rules (from self-improvement)\n\n");
        prompt.push_str(overlay);
    }

    // Tool list (dynamic: changes with the lease set). Last so lease
    // changes invalidate only the tail, not the whole cacheable prefix.
    if !actions.is_empty() {
        prompt.push_str("\n\n## Available tools (call as Python functions)\n\n");
        for action in actions {
            prompt.push_str(&format!("- `{}(", action.name));
            if let Some(props) = action.parameters_schema.get("properties")
                && let Some(obj) = props.as_object()
            {
                let params: Vec<&str> = obj.keys().map(String::as_str).collect();
                prompt.push_str(&params.join(", "));
            }
            prompt.push_str(&format!(")` — {}\n", action.description));
        }
    }

    prompt
}

/// Load the prompt overlay from the Store, if one exists for this project.
async fn load_prompt_overlay(store: &Arc<dyn Store>, project_id: ProjectId) -> Option<String> {
    let docs = store.list_shared_memory_docs(project_id).await.ok()?;
    extract_prompt_overlay(&docs)
}

/// Extract the prompt overlay from a pre-fetched list of system memory docs.
pub fn extract_prompt_overlay(docs: &[crate::types::memory::MemoryDoc]) -> Option<String> {
    let overlay = docs.iter().find(|d| {
        d.title == PREAMBLE_OVERLAY_TITLE && d.tags.contains(&PROMPT_OVERLAY_TAG.to_string())
    })?;

    let content: String = overlay
        .content
        .chars()
        .take(MAX_PROMPT_OVERLAY_CHARS)
        .collect();
    if content.is_empty() {
        return None;
    }
    Some(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::memory::{DocId, DocType, MemoryDoc};
    use crate::types::shared_owner_id;

    #[tokio::test]
    async fn prompt_without_store_uses_compiled_preamble() {
        let prompt =
            build_codeact_system_prompt(&[], None, ProjectId(uuid::Uuid::nil()), None).await;
        // Structural markers — if the preamble gets accidentally emptied,
        // these fail. Generic words like "Python" or "FINAL" would still
        // match a broken include, so check section headings that exist in
        // exactly one place each.
        assert!(
            prompt.contains("## What each turn does"),
            "preamble include appears to be missing its 'What each turn does' section"
        );
        assert!(
            prompt.contains("## Reminders"),
            "preamble include appears to be missing its trailing 'Reminders' section"
        );
        assert!(!prompt.contains("Learned Rules"));
    }

    #[tokio::test]
    async fn prompt_with_overlay_appends_rules() {
        let project_id = ProjectId(uuid::Uuid::new_v4());
        let overlay = MemoryDoc {
            id: DocId::new(),
            project_id,
            user_id: shared_owner_id().into(),
            doc_type: DocType::Note,
            title: PREAMBLE_OVERLAY_TITLE.into(),
            content: "9. Never call web_fetch — use http() instead.".into(),
            source_thread_id: None,
            tags: vec![PROMPT_OVERLAY_TAG.into()],
            metadata: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let store = Arc::new(crate::tests::InMemoryStore::with_docs(vec![overlay]));
        let prompt =
            build_codeact_system_prompt(&[], Some(&(store as Arc<dyn Store>)), project_id, None)
                .await;
        assert!(prompt.contains("Learned Rules"));
        assert!(prompt.contains("Never call web_fetch"));
    }

    #[tokio::test]
    async fn prompt_overlay_size_is_capped() {
        let project_id = ProjectId(uuid::Uuid::new_v4());
        // Create an overlay that exceeds MAX_PROMPT_OVERLAY_CHARS using a char
        // not found in the compiled preamble/postamble
        let huge_content = "\u{2603}".repeat(MAX_PROMPT_OVERLAY_CHARS + 1000); // snowman
        let overlay = MemoryDoc {
            id: DocId::new(),
            project_id,
            user_id: shared_owner_id().into(),
            doc_type: DocType::Note,
            title: PREAMBLE_OVERLAY_TITLE.into(),
            content: huge_content,
            source_thread_id: None,
            tags: vec![PROMPT_OVERLAY_TAG.into()],
            metadata: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let store = Arc::new(crate::tests::InMemoryStore::with_docs(vec![overlay]));
        let prompt =
            build_codeact_system_prompt(&[], Some(&(store as Arc<dyn Store>)), project_id, None)
                .await;

        let snowman_count = prompt.chars().filter(|c| *c == '\u{2603}').count();
        assert_eq!(snowman_count, MAX_PROMPT_OVERLAY_CHARS);
    }

    #[tokio::test]
    async fn prompt_ignores_wrong_project_overlay() {
        let project_id = ProjectId(uuid::Uuid::new_v4());
        let other_project = ProjectId(uuid::Uuid::new_v4());
        let overlay = MemoryDoc {
            id: DocId::new(),
            project_id: other_project,
            user_id: shared_owner_id().into(),
            doc_type: DocType::Note,
            title: PREAMBLE_OVERLAY_TITLE.into(),
            content: "Should not appear".into(),
            source_thread_id: None,
            tags: vec![PROMPT_OVERLAY_TAG.into()],
            metadata: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let store = Arc::new(crate::tests::InMemoryStore::with_docs(vec![overlay]));
        let prompt =
            build_codeact_system_prompt(&[], Some(&(store as Arc<dyn Store>)), project_id, None)
                .await;
        assert!(!prompt.contains("Should not appear"));
        assert!(!prompt.contains("Learned Rules"));
    }

    #[tokio::test]
    async fn prompt_with_platform_info_injects_identity() {
        let info = PlatformInfo {
            version: Some("1.2.3".into()),
            llm_backend: Some("nearai".into()),
            model_name: Some("qwen3-235b".into()),
            database_backend: Some("libsql".into()),
            active_channels: vec!["telegram".into(), "cli".into()],
            owner_id: Some("alice.near".into()),
            repo_url: Some("https://github.com/nearai/ironclaw".into()),
        };
        let prompt =
            build_codeact_system_prompt(&[], None, ProjectId(uuid::Uuid::nil()), Some(&info)).await;
        assert!(prompt.contains("IronClaw"));
        assert!(prompt.contains("1.2.3"));
        assert!(prompt.contains("nearai"));
        assert!(prompt.contains("qwen3-235b"));
        assert!(prompt.contains("libsql"));
        assert!(prompt.contains("telegram"));
        assert!(prompt.contains("alice.near"));
        assert!(prompt.contains("github.com/nearai/ironclaw"));
    }

    #[tokio::test]
    async fn prompt_without_platform_info_has_no_platform_section() {
        let prompt =
            build_codeact_system_prompt(&[], None, ProjectId(uuid::Uuid::nil()), None).await;
        assert!(!prompt.contains("## Platform"));
    }
}
