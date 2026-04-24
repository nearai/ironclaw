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
use crate::types::capability::{
    ActionDef, CapabilityStatus, CapabilitySummary, CapabilitySummaryKind,
};
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

/// The strategy/closing block (after tool listing).
const CODEACT_POSTAMBLE: &str = include_str!("../../prompts/codeact_postamble.md");

/// Optional deploy-time override for the compiled-in preamble, loaded once
/// from `CODEACT_PREAMBLE_PATH`. Same pattern as `AGENTS_SEED` — downstream
/// forks whose agent flow diverges from the upstream prompt's discipline
/// (e.g. a deploy that wants raw tool output in `FINAL()` rather than
/// Markdown reformatting) can point this env var at a markdown file and
/// replace the preamble wholesale. Unset → compiled-in default.
static CODEACT_PREAMBLE_OVERRIDE: std::sync::LazyLock<Option<String>> =
    std::sync::LazyLock::new(|| load_override_file("CODEACT_PREAMBLE_PATH"));

/// Optional deploy-time override for the compiled-in postamble, loaded once
/// from `CODEACT_POSTAMBLE_PATH`. See `CODEACT_PREAMBLE_OVERRIDE`.
static CODEACT_POSTAMBLE_OVERRIDE: std::sync::LazyLock<Option<String>> =
    std::sync::LazyLock::new(|| load_override_file("CODEACT_POSTAMBLE_PATH"));

fn load_override_file(env_var: &str) -> Option<String> {
    let Ok(path) = std::env::var(env_var) else {
        tracing::info!(env_var, "override disabled: env var unset");
        return None;
    };
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            let len = contents.len();
            if contents.trim().is_empty() {
                tracing::warn!(env_var, path, "override file exists but is empty — ignoring");
                None
            } else {
                tracing::info!(env_var, path, len, "override loaded");
                Some(contents)
            }
        }
        Err(e) => {
            tracing::warn!(env_var, path, err = %e, "override file unreadable — falling back to compiled-in default");
            None
        }
    }
}

/// Well-known title for the CodeAct preamble overlay.
pub const PREAMBLE_OVERLAY_TITLE: &str = "prompt:codeact_preamble";

/// Well-known tag for prompt overlay docs.
pub const PROMPT_OVERLAY_TAG: &str = "prompt_overlay";

/// Maximum size for a prompt overlay document (in chars).
const MAX_PROMPT_OVERLAY_CHARS: usize = 4000;

/// Optional deploy-time agent identity, loaded once from `AGENTS_SEED_PATH`.
///
/// Downstream forks (e.g. a domain-specific deploy) can point this env var
/// at a markdown file whose contents are injected into every system prompt
/// as an Identity section. `None` when the var is unset, unreadable, or
/// the file is empty — in which case the compiled-in preamble carries the
/// full identity (generic IronClaw).
///
/// Read once at first access; changes to the file or env var after startup
/// do not take effect until process restart. This matches the rest of the
/// engine's config surface and avoids per-prompt filesystem I/O.
static AGENTS_SEED: std::sync::LazyLock<Option<String>> =
    std::sync::LazyLock::new(|| load_override_file("AGENTS_SEED_PATH").map(|s| s.trim().to_string()));

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
    capabilities: &[CapabilitySummary],
    store: Option<&Arc<dyn Store>>,
    project_id: ProjectId,
    platform: Option<&PlatformInfo>,
) -> String {
    let overlay = if let Some(store) = store {
        load_prompt_overlay(store, project_id).await
    } else {
        None
    };
    build_codeact_system_prompt_inner(actions, capabilities, overlay.as_deref(), platform)
}

/// Build the system prompt using pre-fetched memory docs.
///
/// When the caller already has the `list_memory_docs` result (e.g. because
/// `load_orchestrator` fetched it), pass the docs here to avoid a duplicate
/// Store query.
pub fn build_codeact_system_prompt_with_docs(
    actions: &[ActionDef],
    capabilities: &[CapabilitySummary],
    system_docs: &[crate::types::memory::MemoryDoc],
    platform: Option<&PlatformInfo>,
) -> String {
    let overlay = extract_prompt_overlay(system_docs);
    build_codeact_system_prompt_inner(actions, capabilities, overlay.as_deref(), platform)
}

/// Shared prompt builder used by both the async and pre-fetched-docs variants.
fn build_codeact_system_prompt_inner(
    actions: &[ActionDef],
    capabilities: &[CapabilitySummary],
    overlay: Option<&str>,
    platform: Option<&PlatformInfo>,
) -> String {
    build_codeact_system_prompt_with_overrides(
        actions,
        capabilities,
        overlay,
        platform,
        AGENTS_SEED.as_deref(),
        CODEACT_PREAMBLE_OVERRIDE.as_deref(),
        CODEACT_POSTAMBLE_OVERRIDE.as_deref(),
    )
}

/// Testable core that accepts explicit override strings instead of reading
/// env vars. The env-var path is exercised in
/// `build_codeact_system_prompt_inner`; tests call this helper directly so
/// they don't race on process-wide `LazyLock` state.
fn build_codeact_system_prompt_with_overrides(
    actions: &[ActionDef],
    capabilities: &[CapabilitySummary],
    overlay: Option<&str>,
    platform: Option<&PlatformInfo>,
    identity: Option<&str>,
    preamble_override: Option<&str>,
    postamble_override: Option<&str>,
) -> String {
    let preamble = preamble_override.unwrap_or(CODEACT_PREAMBLE);
    let postamble = postamble_override.unwrap_or(CODEACT_POSTAMBLE);
    let mut prompt = String::from(preamble);

    // Inject platform identity and runtime metadata
    if let Some(info) = platform {
        prompt.push_str(&info.to_prompt_section());
    }

    // Inject deploy-time agent identity from AGENTS_SEED_PATH, if configured.
    // This is how a downstream fork swaps the generic IronClaw persona for
    // a domain-specific one without forking prompt templates — engine v1
    // reads this via the workspace seed, engine v2 reads the file directly
    // at process start. Both paths converge on the same file on disk.
    if let Some(identity) = identity {
        prompt.push_str("\n\n## Agent Identity\n\n");
        prompt.push_str(identity);
    }

    // Append runtime prompt overlay if available
    if let Some(overlay) = overlay {
        prompt.push_str("\n\n## Learned Rules (from self-improvement)\n\n");
        prompt.push_str(overlay);
    }

    // Add tool documentation
    if !actions.is_empty() {
        prompt.push_str("\n## Available tools (call as Python functions)\n\n");
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

    if !capabilities.is_empty() {
        prompt.push_str("\n## Available capabilities (background status)\n\n");
        for capability in capabilities {
            prompt.push_str(&format!(
                "- `{}` [{}] — {}",
                capability.name,
                capability_kind_label(capability.kind),
                capability_status_label(capability.status)
            ));
            if let Some(display_name) = &capability.display_name
                && display_name != &capability.name
            {
                prompt.push_str(&format!(" ({display_name})"));
            }
            if let Some(routing_hint) = &capability.routing_hint {
                prompt.push_str(&format!(". {routing_hint}"));
            }
            if let Some(description) = &capability.description {
                prompt.push_str(&format!(". {description}"));
            }
            prompt.push('\n');
        }
    }

    prompt.push_str(postamble);
    prompt
}

const fn capability_status_label(status: CapabilityStatus) -> &'static str {
    match status {
        CapabilityStatus::Ready => "ready",
        CapabilityStatus::ReadyScoped => "ready_scoped",
        CapabilityStatus::NeedsAuth => "needs_auth",
        CapabilityStatus::NeedsSetup => "needs_setup",
        CapabilityStatus::Inactive => "inactive",
        CapabilityStatus::Latent => "latent",
        CapabilityStatus::Error => "error",
        CapabilityStatus::AvailableNotInstalled => "available_not_installed",
    }
}

const fn capability_kind_label(kind: CapabilitySummaryKind) -> &'static str {
    match kind {
        CapabilitySummaryKind::Channel => "channel",
        CapabilitySummaryKind::Provider => "provider",
        CapabilitySummaryKind::Runtime => "runtime",
    }
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
            build_codeact_system_prompt(&[], &[], None, ProjectId(uuid::Uuid::nil()), None).await;
        assert!(prompt.contains("Python REPL environment"));
        assert!(prompt.contains("Strategy"));
        assert!(!prompt.contains("Learned Rules"));
    }

    #[test]
    fn identity_injected_when_agents_seed_set() {
        let prompt = build_codeact_system_prompt_with_overrides(
            &[],
            &[],
            None,
            None,
            Some("You are the Abound remittance assistant."),
            None,
            None,
        );
        assert!(prompt.contains("## Agent Identity"));
        assert!(prompt.contains("You are the Abound remittance assistant."));
    }

    #[test]
    fn identity_absent_when_agents_seed_unset() {
        let prompt = build_codeact_system_prompt_with_overrides(
            &[], &[], None, None, None, None, None,
        );
        assert!(!prompt.contains("## Agent Identity"));
    }

    #[test]
    fn preamble_override_replaces_compiled_in() {
        let prompt = build_codeact_system_prompt_with_overrides(
            &[],
            &[],
            None,
            None,
            None,
            Some("# Custom preamble\n\nRespond with plain text for simple messages."),
            None,
        );
        assert!(prompt.contains("# Custom preamble"));
        assert!(prompt.contains("Respond with plain text for simple messages."));
        // The stock preamble's "Python REPL environment" marker must not leak through.
        assert!(!prompt.contains("Python REPL environment"));
    }

    #[test]
    fn postamble_override_replaces_compiled_in() {
        let prompt = build_codeact_system_prompt_with_overrides(
            &[],
            &[],
            None,
            None,
            None,
            None,
            Some("## Custom postamble\n\nPass raw tool results to FINAL() unchanged."),
        );
        assert!(prompt.contains("## Custom postamble"));
        assert!(prompt.contains("Pass raw tool results to FINAL() unchanged."));
        // The stock postamble's "Strategy" marker must not leak through.
        assert!(!prompt.contains("Strategy"));
    }

    #[test]
    fn overrides_compose_with_identity_and_tools() {
        let prompt = build_codeact_system_prompt_with_overrides(
            &[],
            &[],
            None,
            None,
            Some("Abound assistant."),
            Some("# PRE"),
            Some("# POST"),
        );
        // Order: preamble → identity → (no overlay) → (no tools) → postamble
        let pre_idx = prompt.find("# PRE").expect("preamble present");
        let identity_idx = prompt.find("## Agent Identity").expect("identity present");
        let post_idx = prompt.find("# POST").expect("postamble present");
        assert!(pre_idx < identity_idx);
        assert!(identity_idx < post_idx);
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
        let prompt = build_codeact_system_prompt(
            &[],
            &[],
            Some(&(store as Arc<dyn Store>)),
            project_id,
            None,
        )
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
        let prompt = build_codeact_system_prompt(
            &[],
            &[],
            Some(&(store as Arc<dyn Store>)),
            project_id,
            None,
        )
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
        let prompt = build_codeact_system_prompt(
            &[],
            &[],
            Some(&(store as Arc<dyn Store>)),
            project_id,
            None,
        )
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
            build_codeact_system_prompt(&[], &[], None, ProjectId(uuid::Uuid::nil()), Some(&info))
                .await;
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
            build_codeact_system_prompt(&[], &[], None, ProjectId(uuid::Uuid::nil()), None).await;
        assert!(!prompt.contains("## Platform"));
    }

    #[test]
    fn prompt_with_capabilities_includes_background_statuses() {
        let prompt = build_codeact_system_prompt_with_docs(
            &[],
            &[
                CapabilitySummary {
                    name: "telegram".into(),
                    display_name: Some("Telegram".into()),
                    kind: crate::types::capability::CapabilitySummaryKind::Channel,
                    status: CapabilityStatus::ReadyScoped,
                    description: Some("Telegram notifications".into()),
                    routing_hint: Some("Usable through message".into()),
                },
                CapabilitySummary {
                    name: "slack".into(),
                    display_name: None,
                    kind: crate::types::capability::CapabilitySummaryKind::Provider,
                    status: CapabilityStatus::NeedsAuth,
                    description: Some("Slack workspace integration".into()),
                    routing_hint: None,
                },
            ],
            &[],
            None,
        );

        assert!(prompt.contains("## Available capabilities (background status)"));
        assert!(prompt.contains("`telegram` [channel]"));
        assert!(prompt.contains("ready_scoped"));
        assert!(prompt.contains("Usable through message"));
        assert!(prompt.contains("`slack` [provider]"));
        assert!(prompt.contains("needs_auth"));
    }
}
