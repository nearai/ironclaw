//! Tests for the composition root's default system-prompt identity source.
//!
//! A sibling file rather than an inline `#[cfg(test)]` module: the composition
//! budget gate (`scripts/ci/check-composition-budget.sh`) counts production
//! LOC and cannot parse an inline test module out of an otherwise-production
//! file, but it does exclude `tests.rs` siblings. Splitting keeps composition's
//! measured mass honest — this is assembly-layer test code, not assembly.

use ironclaw_host_api::{
    ids::{TenantId, ThreadId, UserId},
    turn::{ProductTurnContext, TurnOriginKind, TurnOwner},
};
use ironclaw_loop_contracts::{
    InMemoryRunProfileResolver, LoopRunContext, RunProfileResolutionRequest, RunProfileResolver,
};
use ironclaw_loop_host::{
    EXTENSION_LIFECYCLE_PROTOCOL_PROMPT, EXTENSION_LIFECYCLE_PROTOCOL_TOOLS,
    HOSTED_MCP_REGISTRATION_PROTOCOL_PROMPT, HOSTED_MCP_REGISTRATION_PROTOCOL_TOOLS,
    TOOL_DISCLOSURE_PROTOCOL_PROMPT, TOOL_PREFETCH_PROTOCOL_PROMPT,
};
use ironclaw_turns::{TurnId, TurnRunId, TurnScope};

use super::*;

async fn test_run_context() -> LoopRunContext {
    let profile = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .expect("profile resolves");
    let scope = TurnScope::new(
        TenantId::new("tenant-default-system-prompt").expect("valid"),
        None,
        None,
        ThreadId::new("thread-default-system-prompt").expect("valid"),
    );
    LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), profile)
}

async fn run_context_with_origin(origin: TurnOriginKind) -> LoopRunContext {
    test_run_context()
        .await
        .with_product_context(ProductTurnContext::new(
            origin,
            None,
            None,
            TurnOwner::Personal {
                user: UserId::new("prompt-test-owner").expect("valid user id"),
            },
        ))
}

/// Guidance a bound memory provider ships. Composition appends the
/// provider's text verbatim and owns none of it, so the tests use their own
/// fixture rather than any real provider's asset — if this were pinned to
/// memory-native's file, the test would be asserting that one provider's
/// wording instead of the composition rule.
const PROVIDER_GUIDANCE: &str = "## Persistent Memory\n\nprovider-shipped memory guidance.";

fn protocols_with_memory_guidance() -> SystemPromptProtocols {
    SystemPromptProtocols {
        memory_guidance: Some(PROVIDER_GUIDANCE.to_string()),
        ..SystemPromptProtocols::default()
    }
}

#[tokio::test]
async fn default_system_prompt_loads_and_resolves_as_identity_message() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().canonicalize().expect("canonical root");
    let prompt_path = storage_root.join("system/prompts/default-system.md");
    seed_default_system_prompt(&storage_root, &prompt_path).expect("prompt seeds");
    let source = DefaultSystemPromptIdentitySource::try_new(
        storage_root,
        prompt_path.clone(),
        protocols_with_memory_guidance(),
    )
    .expect("prompt loads");
    let context = test_run_context().await;

    let candidates = source
        .load_identity_candidates(&context, PromptMode::TextOnly, &AdvertisedTools::Ordinary)
        .await
        .expect("load candidates");

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].name.as_str(), DEFAULT_SYSTEM_PROMPT_NAME);
    assert!(
        prompt_path.exists(),
        "source should seed the editable standalone prompt file"
    );

    let content = source
        .resolve_identity_message_content(
            &context,
            candidates[0]
                .message_ref
                .as_ref()
                .expect("trusted identity has ref"),
        )
        .await
        .expect("resolve content")
        .expect("content exists");

    assert!(
        content
            .content
            .contains("When a tool result is partial, truncated, failed")
    );
    // Self-knowledge must be grounded in the published docs site rather than
    // recalled from training data (#6734): the prompt has to name the
    // llms.txt index and the `.md` raw-markdown suffix, or the model has no
    // way to look its own capabilities up. The guidance is ground knowledge
    // about the runtime, so it is appended in memory rather than seeded into
    // the user-editable file — otherwise only fresh installs would get it.
    assert!(
        !std::fs::read_to_string(&prompt_path)
            .expect("seeded prompt reads")
            .contains("docs.ironclaw.com"),
        "docs grounding must not be seeded into the user-editable prompt file"
    );
    // Same for the persistent-memory protocol (#7185): appended in memory so
    // existing installs get it, never written into the user's file.
    assert!(
        !std::fs::read_to_string(&prompt_path)
            .expect("seeded prompt reads")
            .contains("## Persistent Memory"),
        "memory protocol must not be seeded into the user-editable prompt file"
    );
    assert!(
        content.content.contains("## Persistent Memory"),
        "resolved prompt must carry the persistent-memory protocol"
    );
    assert!(
        content
            .content
            .contains("https://docs.ironclaw.com/llms.txt"),
        "prompt must point capability questions at the docs index"
    );
    assert!(
        content.content.contains(".md"),
        "prompt must teach the raw-markdown `.md` suffix for docs pages"
    );
    assert!(
        !content.content.contains("tool_search"),
        "disclosure-off prompt must not mention the bridge tools"
    );
}

#[tokio::test]
async fn disclosure_active_appends_tool_search_protocol_to_system_prompt() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().canonicalize().expect("canonical root");
    let prompt_path = storage_root.join("system/prompts/default-system.md");
    seed_default_system_prompt(&storage_root, &prompt_path).expect("prompt seeds");

    let off = DefaultSystemPromptIdentitySource::try_new(
        storage_root.clone(),
        prompt_path.clone(),
        SystemPromptProtocols::default(),
    )
    .expect("off source loads");
    let on = DefaultSystemPromptIdentitySource::try_new(
        storage_root,
        prompt_path,
        SystemPromptProtocols {
            disclosure: true,
            ..SystemPromptProtocols::default()
        },
    )
    .expect("on source loads");
    let context = test_run_context().await;

    async fn resolve_content(
        source: &DefaultSystemPromptIdentitySource,
        context: &LoopRunContext,
    ) -> String {
        let candidates = source
            .load_identity_candidates(context, PromptMode::TextOnly, &AdvertisedTools::Ordinary)
            .await
            .expect("candidates load");
        source
            .resolve_identity_message_content(
                context,
                candidates[0]
                    .message_ref
                    .as_ref()
                    .expect("trusted identity has ref"),
            )
            .await
            .expect("resolve content")
            .expect("content exists")
            .content
    }

    let off_content = resolve_content(&off, &context).await;
    let on_content = resolve_content(&on, &context).await;

    // The base prompt is preserved verbatim, and only the active source teaches
    // the search/describe/call protocol — so the model is actually told the
    // deferred long tail exists and how to reach it.
    assert!(on_content.starts_with(off_content.trim_end()));
    assert!(!off_content.contains("tool_search"));
    assert!(on_content.contains("tool_search"));
    assert!(on_content.contains("tool_describe"));
    assert!(on_content.contains("tool_call"));
    assert!(on_content.contains("Tool Discovery"));
    assert!(
        on_content.contains("When `tool_search` is present"),
        "bridged-mode guidance must be conditional on the outgoing surface actually advertising tool_search"
    );
    assert!(
        on_content.contains("When `tool_search` is absent"),
        "below-threshold guidance must direct the model to use the complete direct surface"
    );
}

/// The resolved prompt of `source` for one run advertising `tools`.
async fn resolve_for(
    source: &DefaultSystemPromptIdentitySource,
    context: &LoopRunContext,
    tools: &AdvertisedTools,
) -> String {
    let candidates = source
        .load_identity_candidates(context, PromptMode::TextOnly, tools)
        .await
        .expect("candidates load");
    source
        .resolve_identity_message_content(
            context,
            candidates[0]
                .message_ref
                .as_ref()
                .expect("trusted identity has ref"),
        )
        .await
        .expect("resolve content")
        .expect("content exists")
        .content
}

fn selected(ids: &[&str]) -> AdvertisedTools {
    AdvertisedTools::Selected(
        ids.iter()
            .map(|id| ironclaw_host_api::ids::CapabilityId::new(*id).expect("valid capability id"))
            .collect(),
    )
}

const BRIDGES: [&str; 3] = [
    "ironclaw.tool_search",
    "ironclaw.tool_describe",
    "ironclaw.tool_call",
];

/// Build a source over a freshly seeded `SYSTEM.md`.
fn seeded_source(
    root: &tempfile::TempDir,
    protocols: SystemPromptProtocols,
) -> DefaultSystemPromptIdentitySource {
    let storage_root = root.path().canonicalize().expect("canonical root");
    let prompt_path = storage_root.join("system/prompts/default-system.md");
    seed_default_system_prompt(&storage_root, &prompt_path).expect("prompt seeds");
    DefaultSystemPromptIdentitySource::try_new(storage_root, prompt_path, protocols)
        .expect("source loads")
}

/// On an ordinary surface the tool-discovery section is the one it always
/// was: the discovery text followed by both extension-lifecycle paragraphs,
/// blank-line separated, at the end of the prompt. The lifecycle paragraphs
/// became separate assets so a selection can withhold them, and this pins
/// that splitting them changed no byte of a prompt with selection off.
#[tokio::test]
async fn an_ordinary_surface_keeps_the_whole_tool_discovery_section() {
    let root = tempfile::tempdir().expect("tempdir");
    let source = seeded_source(
        &root,
        SystemPromptProtocols {
            disclosure: true,
            ..SystemPromptProtocols::default()
        },
    );
    let context = test_run_context().await;
    let content = resolve_for(&source, &context, &AdvertisedTools::Ordinary).await;
    let whole_section = format!(
        "\n\n{TOOL_DISCLOSURE_PROTOCOL_PROMPT}\n{EXTENSION_LIFECYCLE_PROTOCOL_PROMPT}\n\
         {HOSTED_MCP_REGISTRATION_PROTOCOL_PROMPT}"
    );
    assert!(content.ends_with(&whole_section), "{content}");
    assert!(
        whole_section
            .contains("required.\n\nWhen `extension_search` and `extension_install` are present")
    );
    assert!(whole_section.contains("Never call a lifecycle tool that is absent.\n"));
}

/// Turn-start selection adds its section after the tool-discovery section,
/// and only on a run that advertises a selection. A run the selection fell
/// back on (no accepted user text, a classifier failure, an unreadable
/// history) keeps the ordinary surface, so its prompt is byte-identical to
/// the prompt with selection off.
#[tokio::test]
async fn the_selection_section_renders_only_on_a_run_that_advertises_a_selection() {
    let root = tempfile::tempdir().expect("tempdir");
    let context = test_run_context().await;
    let selection_off = resolve_for(
        &seeded_source(
            &root,
            SystemPromptProtocols {
                disclosure: true,
                ..SystemPromptProtocols::default()
            },
        ),
        &context,
        &AdvertisedTools::Ordinary,
    )
    .await;
    let selection_on = seeded_source(
        &root,
        SystemPromptProtocols {
            disclosure: true,
            tool_prefetch: true,
            ..SystemPromptProtocols::default()
        },
    );

    let fallback = resolve_for(&selection_on, &context, &AdvertisedTools::Ordinary).await;
    assert_eq!(fallback, selection_off);
    assert!(!fallback.contains("Tools Chosen for This Conversation"));

    let mut advertised: Vec<&str> = BRIDGES.to_vec();
    advertised.extend(HOSTED_MCP_REGISTRATION_PROTOCOL_TOOLS);
    let selected_run = resolve_for(&selection_on, &context, &selected(&advertised)).await;
    assert_eq!(
        selected_run,
        format!("{selection_off}\n{TOOL_PREFETCH_PROTOCOL_PROMPT}"),
        "a selection advertising every named tool adds only the selection section"
    );
    assert!(selected_run.contains("start a new conversation"));
}

/// On a selected run each tool-naming section renders only while the
/// selection advertises every tool it names.
#[tokio::test]
async fn a_selected_run_withholds_sections_naming_tools_it_does_not_advertise() {
    let root = tempfile::tempdir().expect("tempdir");
    let context = test_run_context().await;
    const NAMING_GUIDANCE: &str = "## Persistent Memory\n\nSearch with `ironclaw.memory.search` before saying you do not know.";
    let source = seeded_source(
        &root,
        SystemPromptProtocols {
            disclosure: true,
            tool_prefetch: true,
            memory_guidance: Some(NAMING_GUIDANCE.to_string()),
            ..SystemPromptProtocols::default()
        },
    );

    let bridges_only = resolve_for(&source, &context, &selected(&BRIDGES)).await;
    assert!(bridges_only.contains(TOOL_DISCLOSURE_PROTOCOL_PROMPT));
    assert!(bridges_only.ends_with(TOOL_PREFETCH_PROTOCOL_PROMPT));
    for withheld in [
        "extension_search",
        "extension_install",
        "extension_register_hosted_mcp",
        "ironclaw.memory.search",
        "## Persistent Memory",
    ] {
        assert!(
            !bridges_only.contains(withheld),
            "{withheld:?} names or introduces a tool the run does not advertise: {bridges_only}"
        );
    }

    let mut advertised: Vec<&str> = BRIDGES.to_vec();
    advertised.push("ironclaw.memory.search");
    advertised.extend(EXTENSION_LIFECYCLE_PROTOCOL_TOOLS);
    let with_tools = resolve_for(&source, &context, &selected(&advertised)).await;
    assert!(with_tools.contains(NAMING_GUIDANCE));
    assert!(with_tools.contains(EXTENSION_LIFECYCLE_PROTOCOL_PROMPT));
    assert!(
        !with_tools.contains(HOSTED_MCP_REGISTRATION_PROTOCOL_PROMPT),
        "the hosted-MCP paragraph also needs extension_register_hosted_mcp"
    );

    let no_bridges = resolve_for(&source, &context, &selected(&["ironclaw.memory.search"])).await;
    assert!(no_bridges.contains(NAMING_GUIDANCE));
    assert!(
        !no_bridges.contains("tool_search"),
        "without the bridges no discovery or selection text renders: {no_bridges}"
    );
}

/// The loop tier gates the lifecycle paragraphs on capability ids it cannot
/// import; pin them against the lifecycle capabilities' owner.
#[test]
fn lifecycle_paragraph_gates_name_the_owner_capability_ids() {
    use ironclaw_extension_manager::extension_lifecycle_capabilities::{
        EXTENSION_INSTALL_CAPABILITY_ID, EXTENSION_REGISTER_HOSTED_MCP_CAPABILITY_ID,
    };
    use ironclaw_host_api::capability::EXTENSION_SEARCH_CAPABILITY_ID;
    assert_eq!(
        EXTENSION_LIFECYCLE_PROTOCOL_TOOLS,
        [
            EXTENSION_SEARCH_CAPABILITY_ID,
            EXTENSION_INSTALL_CAPABILITY_ID
        ]
    );
    assert_eq!(
        HOSTED_MCP_REGISTRATION_PROTOCOL_TOOLS,
        [
            EXTENSION_SEARCH_CAPABILITY_ID,
            EXTENSION_INSTALL_CAPABILITY_ID,
            EXTENSION_REGISTER_HOSTED_MCP_CAPABILITY_ID,
        ]
    );
}

#[tokio::test]
async fn benchmarking_mode_active_appends_no_human_protocol_to_system_prompt() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().canonicalize().expect("canonical root");
    let prompt_path = storage_root.join("system/prompts/default-system.md");
    seed_default_system_prompt(&storage_root, &prompt_path).expect("prompt seeds");

    let off = DefaultSystemPromptIdentitySource::try_new(
        storage_root.clone(),
        prompt_path.clone(),
        SystemPromptProtocols::default(),
    )
    .expect("off source loads");
    let on = DefaultSystemPromptIdentitySource::try_new(
        storage_root,
        prompt_path,
        SystemPromptProtocols {
            benchmarking_mode: true,
            ..SystemPromptProtocols::default()
        },
    )
    .expect("on source loads");
    let context = test_run_context().await;

    async fn resolve_content(
        source: &DefaultSystemPromptIdentitySource,
        context: &LoopRunContext,
    ) -> String {
        let candidates = source
            .load_identity_candidates(context, PromptMode::TextOnly, &AdvertisedTools::Ordinary)
            .await
            .expect("candidates load");
        source
            .resolve_identity_message_content(
                context,
                candidates[0]
                    .message_ref
                    .as_ref()
                    .expect("trusted identity has ref"),
            )
            .await
            .expect("resolve content")
            .expect("content exists")
            .content
    }

    let off_content = resolve_content(&off, &context).await;
    let on_content = resolve_content(&on, &context).await;

    // The base prompt is preserved verbatim, and only the active source
    // adds the no-human protocol — real product usage (mode off) is
    // byte-identical to today's prompt.
    assert!(on_content.starts_with(off_content.trim_end()));
    assert!(!off_content.contains("Automated Evaluation Mode"));
    assert!(on_content.contains("Automated Evaluation Mode"));
    assert!(on_content.contains("no one to answer a clarifying question"));
}

/// A `Disabled` memory binding registers no memory package, so the model's
/// surface carries no `ironclaw.memory.*` tools (pinned end-to-end by
/// `group_memory/scenario_disabled_binding_offers_no_memory_tools.rs`).
/// The resolved prompt must not claim persistent memory exists or name a
/// tool the model cannot call — that is a false capability claim that
/// produces unusable tool calls.
#[tokio::test]
async fn memory_protocol_is_absent_without_a_bound_memory_provider() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().canonicalize().expect("canonical root");
    let prompt_path = storage_root.join("system/prompts/default-system.md");
    seed_default_system_prompt(&storage_root, &prompt_path).expect("prompt seeds");

    let unbound = DefaultSystemPromptIdentitySource::try_new(
        storage_root.clone(),
        prompt_path.clone(),
        SystemPromptProtocols::default(),
    )
    .expect("unbound source loads");
    let bound = DefaultSystemPromptIdentitySource::try_new(
        storage_root,
        prompt_path,
        protocols_with_memory_guidance(),
    )
    .expect("bound source loads");
    let context = test_run_context().await;

    async fn resolve_content(
        source: &DefaultSystemPromptIdentitySource,
        context: &LoopRunContext,
    ) -> String {
        let candidates = source
            .load_identity_candidates(context, PromptMode::TextOnly, &AdvertisedTools::Ordinary)
            .await
            .expect("candidates load");
        source
            .resolve_identity_message_content(
                context,
                candidates[0]
                    .message_ref
                    .as_ref()
                    .expect("trusted identity has ref"),
            )
            .await
            .expect("resolve content")
            .expect("content exists")
            .content
    }

    let unbound_content = resolve_content(&unbound, &context).await;
    let bound_content = resolve_content(&bound, &context).await;

    assert!(
        !unbound_content.contains("## Persistent Memory"),
        "a deployment with no bound memory provider must not be told memory exists"
    );
    assert!(
        !unbound_content.contains("ironclaw.memory."),
        "the unbound prompt must not name a memory tool the model cannot call"
    );
    // The bound arm guards the assertions above against passing vacuously
    // (e.g. if the guidance stopped being appended at all). Asserted
    // against the fixture, not against any provider's wording: what
    // composition owes is "append what the bound provider declared", and
    // the native text's own content is pinned where it lives
    // (`memory_native_extension` + the memory-native package tests).
    assert!(bound_content.contains(PROVIDER_GUIDANCE));
    // Everything else about the prompt is identical: gating adds a section,
    // it does not rewrite the rest.
    assert!(bound_content.starts_with(unbound_content.trim_end()));
}

#[tokio::test]
async fn scheduled_trigger_origin_appends_unattended_protocol_only_to_triggered_runs() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().canonicalize().expect("canonical root");
    let prompt_path = storage_root.join("system/prompts/default-system.md");
    seed_default_system_prompt(&storage_root, &prompt_path).expect("prompt seeds");
    let source = DefaultSystemPromptIdentitySource::try_new(
        storage_root,
        prompt_path,
        SystemPromptProtocols::default(),
    )
    .expect("prompt loads");
    let interactive_context = run_context_with_origin(TurnOriginKind::Inbound).await;
    let scheduled_context = run_context_with_origin(TurnOriginKind::ScheduledTrigger).await;

    async fn resolve_content(
        source: &DefaultSystemPromptIdentitySource,
        context: &LoopRunContext,
    ) -> String {
        let candidates = source
            .load_identity_candidates(context, PromptMode::TextOnly, &AdvertisedTools::Ordinary)
            .await
            .expect("candidates load");
        source
            .resolve_identity_message_content(
                context,
                candidates[0]
                    .message_ref
                    .as_ref()
                    .expect("trusted identity has ref"),
            )
            .await
            .expect("resolve content")
            .expect("content exists")
            .content
    }

    let interactive_content = resolve_content(&source, &interactive_context).await;
    let scheduled_content = resolve_content(&source, &scheduled_context).await;

    assert!(
        !interactive_content.contains("Unattended Scheduled Run"),
        "interactive runs must retain the ordinary ask-the-user escape valve"
    );
    assert!(scheduled_content.contains("Unattended Scheduled Run"));
    assert!(scheduled_content.contains("There is no human present"));
    assert!(scheduled_content.contains("Never end the run with a question"));
    assert!(scheduled_content.contains("do not enumerate an entire large collection"));
    assert!(
        scheduled_content.contains("perform the requested action before further investigation")
    );
    assert!(scheduled_content.contains("never claim the side effect happened"));
    assert!(scheduled_content.contains("final reply is the run's recorded output"));
}

#[tokio::test]
async fn default_system_prompt_reloads_edited_prompt_for_new_candidates() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().canonicalize().expect("canonical root");
    let prompt_path = storage_root.join("system/prompts/default-system.md");
    seed_default_system_prompt(&storage_root, &prompt_path).expect("prompt seeds");
    let source = DefaultSystemPromptIdentitySource::try_new(
        storage_root.clone(),
        prompt_path.clone(),
        protocols_with_memory_guidance(),
    )
    .expect("prompt loads");
    let context = test_run_context().await;
    let first_candidates = source
        .load_identity_candidates(&context, PromptMode::TextOnly, &AdvertisedTools::Ordinary)
        .await
        .expect("first candidates load");

    std::fs::write(&prompt_path, "edited standalone prompt").expect("prompt edits");
    let edited_candidates = source
        .load_identity_candidates(&context, PromptMode::TextOnly, &AdvertisedTools::Ordinary)
        .await
        .expect("edited candidates load");

    assert_ne!(
        first_candidates[0].message_ref,
        edited_candidates[0].message_ref
    );
    let content = source
        .resolve_identity_message_content(
            &context,
            edited_candidates[0]
                .message_ref
                .as_ref()
                .expect("trusted identity has ref"),
        )
        .await
        .expect("resolve edited content")
        .expect("edited content exists");

    // The user's edited base is preserved verbatim and stays first, but the
    // docs-grounding self-knowledge section is ground knowledge about the
    // runtime (#6734): it is appended unconditionally, so an install whose
    // SYSTEM.md predates the guidance (or was edited to drop it) still tells
    // the model to look its own capabilities up instead of guessing.
    assert!(content.content.starts_with("edited standalone prompt"));
    assert!(
        content.content.contains("## Self-Knowledge"),
        "self-knowledge guidance must be appended even when SYSTEM.md omits it"
    );
    // Same reasoning for the bound provider's memory guidance (#7185):
    // without it the model is never told to save durable user facts, so
    // nothing ever reaches the next conversation. It must survive a
    // SYSTEM.md that predates the guidance, and must not be baked into the
    // user's file.
    assert!(
        content.content.contains(PROVIDER_GUIDANCE),
        "provider guidance must be appended even when SYSTEM.md omits it"
    );
    assert!(
        content
            .content
            .contains("https://docs.ironclaw.com/llms.txt"),
        "appended guidance must point capability questions at the docs index"
    );
    assert!(
        content.content.contains(".md"),
        "appended guidance must teach the raw-markdown `.md` suffix for docs pages"
    );
    assert!(
        !content.content.contains("tool_search"),
        "disclosure-off prompt must not mention the bridge tools"
    );
}

#[cfg(unix)]
#[test]
fn default_system_prompt_rejects_symlink() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().canonicalize().expect("canonical root");
    let prompt_path = storage_root.join("system/prompts/default-system.md");
    std::fs::create_dir_all(prompt_path.parent().expect("parent")).expect("prompt parent");
    let target = storage_root.join("target.md");
    std::fs::write(&target, "linked prompt").expect("target prompt");
    std::os::unix::fs::symlink(&target, &prompt_path).expect("prompt symlink");

    let error = seed_default_system_prompt(&storage_root, &prompt_path)
        .expect_err("symlink should be rejected");

    assert!(error.to_string().contains("must not be a symlink"));
}
