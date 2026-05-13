//! System prompt construction for the execution loop.
//!
//! Builds a CodeAct/RLM system prompt that instructs the LLM to write
//! Python code in ```repl blocks while keeping the model-facing surfaces
//! separated:
//! - callable actions stay in the normal tool inventory
//! - contextual capability background stays in one canonical always-on section
//! - blocked managed integrations are rendered separately under
//!   `Activatable Integrations`
//!
//! Prompt templates live in `crates/ironclaw_engine/prompts/` as plain
//! markdown files for easy inspection and iteration. They are embedded
//! at compile time via `include_str!` and can be extended at runtime with
//! prompt overlays stored as MemoryDocs.

use std::sync::Arc;

use crate::traits::store::Store;
use crate::types::capability::{
    ActionDef, CapabilityStatus, CapabilitySummary, CapabilitySummaryKind, ModelToolSurface,
};
use crate::types::message::{MessageRole, ThreadMessage};
use crate::types::project::ProjectId;

// Runtime platform metadata lives in `ironclaw_common::platform`. Re-exported
// from this module's path for back-compat with prior call sites.
pub use ironclaw_common::platform::PlatformInfo;

/// The main instruction block (before tool listing).
const CODEACT_PREAMBLE: &str = include_str!("../../prompts/codeact_preamble.md");

/// The strategy/closing block appended after the dynamic metadata sections.
const CODEACT_POSTAMBLE: &str = include_str!("../../prompts/codeact_postamble.md");

/// Marker for the engine-owned CodeAct system prompt.
const CODEACT_SYSTEM_PROMPT_MARKER: &str = "<!-- ironclaw:codeact-system-prompt -->\n";
const CODEACT_LEGACY_OPENING: &str = "You are an AI assistant with a Python REPL environment.";
const CODEACT_STRATEGY_HEADING: &str = "\n## Strategy\n";
const CODEACT_CAPABILITIES_HEADING: &str = "\n## Available capabilities (background status)\n";
const CODEACT_BACKGROUND_CAPABILITIES_HEADING: &str = "\n## Capabilities\n";
const CODEACT_ENABLED_TOOLS_HEADING: &str = "\n## Enabled Tools\n";
const CODEACT_ACTIVATABLE_INTEGRATIONS_HEADING: &str = "\n## Activatable Integrations\n";
const PRIOR_KNOWLEDGE_HEADING: &str = "\n\n## Prior Knowledge (from completed threads)\n";
const ACTIVE_SKILLS_HEADING: &str = "\n\n## Active Skills\n";
const MISSING_SKILLS_PREFIX: &str =
    "\n\nThe user explicitly requested slash skill(s) that are not installed or were not found:";

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
    capabilities: &[CapabilitySummary],
    compact_actions: &[ActionDef],
    store: Option<&Arc<dyn Store>>,
    project_id: ProjectId,
    platform: Option<&PlatformInfo>,
) -> String {
    let overlay = if let Some(store) = store {
        load_prompt_overlay(store, project_id).await
    } else {
        None
    };
    build_codeact_system_prompt_inner(capabilities, compact_actions, overlay.as_deref(), platform)
}

/// Build the system prompt using pre-fetched memory docs.
///
/// When the caller already has the `list_memory_docs` result (e.g. because
/// `load_orchestrator` fetched it), pass the docs here to avoid a duplicate
/// Store query.
pub fn build_codeact_system_prompt_with_docs(
    capabilities: &[CapabilitySummary],
    compact_actions: &[ActionDef],
    system_docs: &[crate::types::memory::MemoryDoc],
    platform: Option<&PlatformInfo>,
) -> String {
    let overlay = extract_prompt_overlay(system_docs);
    build_codeact_system_prompt_inner(capabilities, compact_actions, overlay.as_deref(), platform)
}

/// Shared prompt builder used by both the async and pre-fetched-docs variants.
fn build_codeact_system_prompt_inner(
    capabilities: &[CapabilitySummary],
    compact_actions: &[ActionDef],
    overlay: Option<&str>,
    platform: Option<&PlatformInfo>,
) -> String {
    let mut prompt = String::from(CODEACT_SYSTEM_PROMPT_MARKER);
    prompt.push_str(CODEACT_PREAMBLE);

    // Inject platform identity and runtime metadata
    if let Some(info) = platform {
        prompt.push_str(&info.to_prompt_section());
    }

    // Append runtime prompt overlay if available
    if let Some(overlay) = overlay {
        prompt.push_str("\n\n## Learned Rules (from self-improvement)\n\n");
        prompt.push_str(overlay);
    }

    let (activatable_integrations, background_capabilities): (Vec<_>, Vec<_>) = capabilities
        .iter()
        .partition(|capability| is_activatable_integration(capability));

    if !background_capabilities.is_empty() {
        prompt.push_str(CODEACT_BACKGROUND_CAPABILITIES_HEADING);
        prompt.push('\n');
        for capability in background_capabilities {
            prompt.push_str(&render_background_capability(capability));
        }
    }

    let compact_actions: Vec<_> = compact_actions
        .iter()
        .filter(|action| matches!(action.model_tool_surface, ModelToolSurface::CompactToolInfo))
        .collect();

    if !compact_actions.is_empty() {
        prompt.push_str(CODEACT_ENABLED_TOOLS_HEADING);
        prompt.push('\n');
        prompt.push_str(
            "These enabled tools are shown in compact form. Before calling one, always check its schema with `tool_info(name=\"<tool>\", detail=\"schema\")`.\n\n",
        );
        for action in compact_actions {
            prompt.push_str(&render_enabled_tool(action));
        }
    }

    if !activatable_integrations.is_empty() {
        prompt.push_str(CODEACT_ACTIVATABLE_INTEGRATIONS_HEADING);
        prompt.push('\n');
        prompt.push_str(
            "These integrations need user setup before their tools become callable. \
             You cannot enable them yourself — tell the user to install or activate them \
             through the IronClaw UI (or wait for them to do so). \
             If you need parameter details before suggesting one, call \
             `tool_info(name=\"<tool>\", detail=\"summary\")` on a preview tool.\n\n",
        );
        for capability in activatable_integrations {
            prompt.push_str(&render_activatable_integration(capability));
        }
    }

    prompt.push_str(CODEACT_POSTAMBLE);
    prompt
}

pub fn is_codeact_system_prompt(content: &str) -> bool {
    content.starts_with(CODEACT_SYSTEM_PROMPT_MARKER) || is_legacy_codeact_system_prompt(content)
}

pub fn refresh_codeact_system_prompt(existing_content: &str, system_prompt: &str) -> String {
    if !is_codeact_system_prompt(existing_content) {
        return system_prompt.to_string();
    }

    let suffix = codeact_system_prompt_suffix(existing_content).unwrap_or_default();

    if suffix.is_empty() {
        system_prompt.to_string()
    } else {
        let mut refreshed = String::from(system_prompt);
        refreshed.push_str(suffix);
        refreshed
    }
}

pub fn upsert_codeact_system_prompt(
    messages: &mut Vec<ThreadMessage>,
    system_prompt: String,
) -> bool {
    if let Some(message) = messages.iter_mut().find(|message| {
        message.role == MessageRole::System && is_codeact_system_prompt(&message.content)
    }) {
        let refreshed = refresh_codeact_system_prompt(&message.content, &system_prompt);
        if message.content == refreshed {
            return false;
        }
        message.content = refreshed;
        return true;
    }

    if messages
        .iter()
        .any(|message| message.role == MessageRole::System)
    {
        return false;
    }

    messages.insert(0, ThreadMessage::system(system_prompt));
    true
}

fn is_legacy_codeact_system_prompt(content: &str) -> bool {
    content.starts_with(CODEACT_LEGACY_OPENING)
        && content.contains("```repl")
        && (content.contains(CODEACT_STRATEGY_HEADING)
            || content.contains(CODEACT_CAPABILITIES_HEADING))
}

fn codeact_system_prompt_suffix(existing_content: &str) -> Option<&str> {
    let append_markers = [
        PRIOR_KNOWLEDGE_HEADING,
        ACTIVE_SKILLS_HEADING,
        MISSING_SKILLS_PREFIX,
    ];

    let suffix_start = append_markers
        .iter()
        .filter_map(|marker| existing_content.find(marker))
        .min()
        .or_else(|| {
            existing_content
                .rfind(CODEACT_POSTAMBLE)
                .map(|idx| idx + CODEACT_POSTAMBLE.len())
        })?;

    existing_content.get(suffix_start..)
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

fn is_activatable_integration(capability: &CapabilitySummary) -> bool {
    // NeedsAuth is intentionally NOT here: post-#3133, installed-but-unauthed
    // provider tools are direct-callable (the engine's auth preflight raises
    // an Authentication gate at execute time) so they live in the regular
    // action inventory, not in the separate setup-required section.
    matches!(
        capability.kind,
        CapabilitySummaryKind::Provider | CapabilitySummaryKind::Channel
    ) && matches!(
        capability.status,
        CapabilityStatus::NeedsSetup
            | CapabilityStatus::Inactive
            | CapabilityStatus::Latent
            | CapabilityStatus::AvailableNotInstalled
    )
}

fn render_background_capability(capability: &CapabilitySummary) -> String {
    let mut line = format!(
        "- `{}` [{}] — {}",
        capability.name,
        capability_kind_label(capability.kind),
        capability_status_label(capability.status)
    );
    if let Some(display_name) = &capability.display_name
        && display_name != &capability.name
    {
        line.push_str(&format!(" ({display_name})"));
    }
    if let Some(routing_hint) = &capability.routing_hint {
        line.push_str(&format!(". {routing_hint}"));
    }
    if let Some(description) = &capability.description {
        line.push_str(&format!(". {description}"));
    }
    line.push('\n');
    line
}

fn render_enabled_tool(action: &ActionDef) -> String {
    format!(
        "- `{}` — {}\n",
        action.discovery_name(),
        compact_prompt_description(&action.description)
    )
}

fn compact_prompt_description(description: &str) -> String {
    description.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn render_activatable_integration(capability: &CapabilitySummary) -> String {
    let mut line = format!(
        "- `{}` [{}]",
        capability.name,
        capability_kind_label(capability.kind)
    );
    if let Some(display_name) = &capability.display_name
        && display_name != &capability.name
    {
        line.push_str(&format!(" ({display_name})"));
    }
    if let Some(description) = &capability.description {
        line.push_str(&format!(" — {description}"));
    }
    if !capability.action_preview.is_empty() {
        line.push_str(&format!(
            ". Unlocks: {}",
            format_action_preview(&capability.action_preview)
        ));
    }
    line.push('\n');
    line
}

fn format_action_preview(actions: &[String]) -> String {
    const MAX_PREVIEW: usize = 3;

    let mut rendered = actions
        .iter()
        .take(MAX_PREVIEW)
        .map(|action| format!("`{action}`"))
        .collect::<Vec<_>>();
    if actions.len() > MAX_PREVIEW {
        rendered.push(format!("+{} more", actions.len() - MAX_PREVIEW));
    }
    rendered.join(", ")
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
            &[
                CapabilitySummary {
                    name: "telegram".into(),
                    display_name: Some("Telegram".into()),
                    kind: crate::types::capability::CapabilitySummaryKind::Channel,
                    status: CapabilityStatus::ReadyScoped,
                    description: Some("Telegram notifications".into()),
                    action_preview: Vec::new(),
                    routing_hint: Some("Usable through message".into()),
                },
                CapabilitySummary {
                    name: "slack".into(),
                    display_name: None,
                    kind: crate::types::capability::CapabilitySummaryKind::Provider,
                    // NeedsSetup (not NeedsAuth) lands in "Activatable
                    // Integrations". NeedsAuth tools are direct-callable
                    // post-#3133, so they live in the regular action
                    // inventory rather than the setup-required section.
                    status: CapabilityStatus::NeedsSetup,
                    description: Some("Slack workspace integration".into()),
                    action_preview: vec!["slack_send".into(), "slack_history".into()],
                    routing_hint: None,
                },
            ],
            &[],
            &[],
            None,
        );

        assert!(prompt.contains("## Capabilities"));
        assert!(prompt.contains("`telegram` [channel]"));
        assert!(prompt.contains("ready_scoped"));
        assert!(prompt.contains("Usable through message"));
        assert!(prompt.contains("## Activatable Integrations"));
        assert!(prompt.contains("`slack` [provider]"));
        assert!(prompt.contains("need user setup before their tools become callable"));
        assert!(prompt.contains("tool_info(name=\"<tool>\", detail=\"summary\")"));
        assert!(prompt.contains("Unlocks: `slack_send`, `slack_history`"));
    }

    #[test]
    fn prompt_renders_compact_enabled_tools_once_with_schema_instruction() {
        let prompt = build_codeact_system_prompt_with_docs(
            &[CapabilitySummary {
                name: "gmail".into(),
                display_name: Some("Gmail".into()),
                kind: CapabilitySummaryKind::Provider,
                // NeedsSetup keeps gmail in Activatable Integrations.
                // NeedsAuth gmail would render in the regular action
                // inventory instead (post-#3133 direct-callable path).
                status: CapabilityStatus::NeedsSetup,
                description: Some("Gmail integration".into()),
                action_preview: vec!["gmail_send".into()],
                routing_hint: None,
            }],
            &[
                ActionDef {
                    name: "mission_create".into(),
                    description: "Create scheduled or event-driven missions.".into(),
                    parameters_schema: serde_json::json!({"type": "object"}),
                    effects: Vec::new(),
                    requires_approval: false,
                    model_tool_surface: ModelToolSurface::CompactToolInfo,
                    discovery: None,
                },
                ActionDef {
                    name: "http".into(),
                    description: "Make HTTP requests.".into(),
                    parameters_schema: serde_json::json!({"type": "object"}),
                    effects: Vec::new(),
                    requires_approval: false,
                    model_tool_surface: ModelToolSurface::FullSchema,
                    discovery: None,
                },
            ],
            &[],
            None,
        );

        assert!(prompt.contains("## Enabled Tools"));
        assert_eq!(prompt.matches("## Enabled Tools").count(), 1);
        assert!(prompt.contains(
            "Before calling one, always check its schema with `tool_info(name=\"<tool>\", detail=\"schema\")`."
        ));
        assert!(prompt.contains("- `mission_create`"));
        assert!(!prompt.contains("mission_create(name, goal, cadence"));
        assert!(!prompt.contains("- `http`"));
        assert!(prompt.contains("## Activatable Integrations"));
        assert_eq!(prompt.matches("`gmail` [provider]").count(), 1);
    }

    #[test]
    fn needs_auth_capability_is_not_activatable_integration() {
        // Post-#3133: gmail with NeedsAuth status (installed but missing
        // OAuth) is direct-callable. The auth gate raises at execute
        // time, so the capability does NOT belong in the Activatable
        // Integrations section.
        let prompt = build_codeact_system_prompt_with_docs(
            &[CapabilitySummary {
                name: "gmail".into(),
                display_name: Some("Gmail".into()),
                kind: CapabilitySummaryKind::Provider,
                status: CapabilityStatus::NeedsAuth,
                description: Some("Gmail integration".into()),
                action_preview: vec!["gmail_send".into()],
                routing_hint: None,
            }],
            &[],
            &[],
            None,
        );
        assert!(!prompt.contains("## Activatable Integrations"));
    }

    #[test]
    fn prompt_no_longer_duplicates_callable_tool_inventory() {
        let prompt = build_codeact_system_prompt_with_docs(&[], &[], &[], None);

        assert!(!prompt.contains("## Available tools (call as Python functions)"));
        assert!(!prompt.contains("`message(text)`"));
    }

    #[test]
    fn prompt_keeps_callable_tools_out_of_extra_prompt_sections() {
        let prompt = build_codeact_system_prompt_with_docs(&[], &[], &[], None);

        assert!(!prompt.contains("## Lookup-only tools"));
        assert!(!prompt.contains("## Deferred large tools"));
        assert!(!prompt.contains("inspect one on demand"));
        assert!(!prompt.contains("oneOf"));
        assert!(!prompt.contains("\"query\""));
    }

    #[test]
    fn upsert_replaces_engine_owned_system_prompt() {
        let old_prompt = build_codeact_system_prompt_with_docs(&[], &[], &[], None);
        let new_prompt = build_codeact_system_prompt_with_docs(
            &[CapabilitySummary {
                name: "telegram".into(),
                display_name: None,
                kind: CapabilitySummaryKind::Channel,
                status: CapabilityStatus::ReadyScoped,
                description: None,
                action_preview: Vec::new(),
                routing_hint: Some("Usable through message".into()),
            }],
            &[],
            &[],
            None,
        );
        let mut messages = vec![ThreadMessage::system(old_prompt), ThreadMessage::user("hi")];

        assert!(upsert_codeact_system_prompt(
            &mut messages,
            new_prompt.clone()
        ));
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, MessageRole::System);
        assert_eq!(messages[0].content, new_prompt);
    }

    #[test]
    fn refresh_preserves_step_zero_system_appends() {
        let old_prompt = build_codeact_system_prompt_with_docs(&[], &[], &[], None);
        let existing = format!(
            "{old_prompt}\n\n## Prior Knowledge (from completed threads)\n\n### [LESSON] Use http\n\n<skill name=\"github\" version=\"1\">\nGitHub API Skill\n</skill>\n\nThe user explicitly requested slash skill(s) that are not installed."
        );
        let new_prompt = build_codeact_system_prompt_with_docs(
            &[CapabilitySummary {
                name: "slack".into(),
                display_name: None,
                kind: CapabilitySummaryKind::Provider,
                status: CapabilityStatus::NeedsAuth,
                description: None,
                action_preview: vec!["slack_send".into()],
                routing_hint: None,
            }],
            &[],
            &[],
            None,
        );

        let refreshed = refresh_codeact_system_prompt(&existing, &new_prompt);
        assert!(refreshed.starts_with(&new_prompt));
        assert!(refreshed.contains("## Prior Knowledge (from completed threads)"));
        assert!(refreshed.contains("GitHub API Skill"));
        assert!(refreshed.contains("slash skill(s) that are not installed"));
    }

    #[test]
    fn upsert_replaces_legacy_codeact_prompt_revisions() {
        let legacy_prompt = format!(
            "{CODEACT_LEGACY_OPENING}\n\nLegacy prompt body.\n\n```repl\nprint('hi')\n```\n{CODEACT_STRATEGY_HEADING}\nLegacy strategy text.\n"
        );
        let new_prompt = build_codeact_system_prompt_with_docs(&[], &[], &[], None);
        let mut messages = vec![
            ThreadMessage::system(legacy_prompt),
            ThreadMessage::user("resume me"),
        ];

        assert!(upsert_codeact_system_prompt(
            &mut messages,
            new_prompt.clone()
        ));
        assert_eq!(messages[0].content, new_prompt);
    }

    #[test]
    fn refresh_preserves_appends_for_legacy_prompt_revisions() {
        let legacy_prompt = format!(
            "{CODEACT_LEGACY_OPENING}\n\nLegacy prompt body.\n\n```repl\nprint('hi')\n```\n{CODEACT_STRATEGY_HEADING}\nLegacy strategy text.\n{PRIOR_KNOWLEDGE_HEADING}\n### [LESSON] Use http\n\n## Active Skills\n\n<skill name=\"github\" version=\"1\">\nGitHub API Skill\n</skill>\n\nThe user explicitly requested slash skill(s) that are not installed or were not found: /missing."
        );
        let new_prompt = build_codeact_system_prompt_with_docs(
            &[CapabilitySummary {
                name: "slack".into(),
                display_name: None,
                kind: CapabilitySummaryKind::Provider,
                status: CapabilityStatus::NeedsAuth,
                description: None,
                action_preview: vec!["slack_send".into()],
                routing_hint: None,
            }],
            &[],
            &[],
            None,
        );

        let refreshed = refresh_codeact_system_prompt(&legacy_prompt, &new_prompt);
        assert!(refreshed.starts_with(&new_prompt));
        assert!(refreshed.contains("## Prior Knowledge (from completed threads)"));
        assert!(refreshed.contains("GitHub API Skill"));
        assert!(refreshed.contains("/missing"));
    }
}
