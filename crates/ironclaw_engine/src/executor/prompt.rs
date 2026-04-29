//! System prompt construction for the execution loop.
//!
//! Builds the v2 execution-loop system prompt.
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

/// Temporary structured-tool-only prompt for demo/abound testing.
const STRUCTURED_TOOL_PREAMBLE: &str = r#"You are IronClaw, a personal AI assistant.

## Execution mode

Use the provider's structured tool_calls interface for every action.
Do not emit Python, repl, py, or other executable fenced code blocks.
Do not call tools as Python functions.
Do not write tool invocations in assistant text. Never output `[[call_tool ...]]`, `<tool_call>`, `<function_call>`, JSON tool-call blobs, or function-style calls such as `tool_name(...)`.
Only the provider-level `tool_calls` field invokes tools. If you need a tool, return a structured tool call instead of describing or printing the call.
When no action is needed, answer in plain text.
"#;

const STRUCTURED_TOOL_POSTAMBLE: &str = r#"
## Strategy

Use structured tool calls when you need data, persistence, external effects, or system state.
After tool results are available, continue with another structured tool call or return the final plain-text answer.
Some integrations use literal UI blocks such as `[[choice_set]]...[[/choice_set]]` in final user-facing text. These are UI markup only; do not invent other bracketed control blocks, especially `[[call_tool ...]]`.
"#;

/// Well-known title for the CodeAct preamble overlay.
pub const PREAMBLE_OVERLAY_TITLE: &str = "prompt:codeact_preamble";

/// Well-known tag for prompt overlay docs.
pub const PROMPT_OVERLAY_TAG: &str = "prompt_overlay";

/// Maximum size for a prompt overlay document (in chars).
const MAX_PROMPT_OVERLAY_CHARS: usize = 4000;

/// Build the system prompt for v2 execution.
///
/// The prompt instructs the LLM to use provider-level structured tool calls,
/// not CodeAct, Python, or text-encoded tool-call syntax.
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
    let mut prompt = if let Ok(custom) = std::env::var("AGENT_PREAMBLE") {
        // Replace the default identity line but keep the structured-tool-only instructions.
        let rest = STRUCTURED_TOOL_PREAMBLE
            .find("\n\n")
            .map(|i| &STRUCTURED_TOOL_PREAMBLE[i..])
            .unwrap_or("");
        format!("{custom}{rest}")
    } else {
        String::from(STRUCTURED_TOOL_PREAMBLE)
    };

    // Inject platform identity and runtime metadata
    if let Some(info) = platform {
        prompt.push_str(&info.to_prompt_section());
    }

    // Append runtime prompt overlay if available
    if let Some(overlay) = overlay {
        prompt.push_str("\n\n## Learned Rules (from self-improvement)\n\n");
        prompt.push_str(overlay);
    }

    // Add tool documentation
    if !actions.is_empty() {
        prompt.push_str("\n## Available tools\n\n");
        for action in actions {
            prompt.push_str(&format!("- `{}(", action.name));
            // Extract parameter names from JSON schema
            if let Some(props) = action.parameters_schema.get("properties")
                && let Some(obj) = props.as_object()
            {
                let params: Vec<&str> = obj.keys().map(String::as_str).collect();
                prompt.push_str(&params.join(", "));
            }
            prompt.push_str(&format!(")` — {}\n", action.description));
        }
    }

    prompt.push_str(STRUCTURED_TOOL_POSTAMBLE);
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
        assert!(prompt.contains("structured tool_calls"));
        assert!(prompt.contains("Do not emit Python"));
        assert!(prompt.contains("Never output `[[call_tool ...]]`"));
        assert!(prompt.contains("Only the provider-level `tool_calls` field invokes tools"));
        assert!(prompt.contains("do not invent other bracketed control blocks"));
        assert!(prompt.contains("Strategy"));
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
