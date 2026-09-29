use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelMessageRole,
    HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_turns::TurnStatus;

use crate::runtime_input::{PollSettings, RebornRuntimeIdentity, RebornRuntimeInput};

use super::{RebornRuntimeError, build_reborn_runtime};

#[derive(Debug)]
struct RecordingGateway {
    requests: Arc<StdMutex<Vec<HostManagedModelRequest>>>,
}

#[async_trait]
impl HostManagedModelGateway for RecordingGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests
            .lock()
            .expect("recording gateway requests lock poisoned")
            .push(request);
        Ok(HostManagedModelResponse::assistant_reply(
            "prompt observed".to_string(),
        ))
    }
}

#[tokio::test]
async fn standalone_runtime_injects_default_system_prompt_into_model_request() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("standalone");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let input = runtime_input(storage_root.clone(), Arc::clone(&requests));

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");

    assert_eq!(reply.status, TurnStatus::Completed);
    assert!(
        storage_root
            .join("system/prompts/default-system.md")
            .exists(),
        "standalone runtime should seed an editable prompt file under storage"
    );
    let recorded_requests = recorded_requests(&requests);
    assert_eq!(recorded_requests.len(), 1);
    assert!(
        recorded_requests[0].messages.iter().any(|message| {
            message.role == HostManagedModelMessageRole::System
                && message
                    .content
                    .contains("When a tool result is partial, truncated, failed")
        }),
        "standalone runtime should send the editable default system prompt to the model gateway"
    );
    // Disclosure is explicitly enabled by `runtime_input`: the system prompt
    // must teach the model the tool_search/tool_describe/tool_call protocol, or
    // a weak model never reaches the deferred long tail.
    assert!(
        recorded_requests[0].messages.iter().any(|message| {
            message.role == HostManagedModelMessageRole::System
                && message.content.contains("Tool Discovery")
                && message.content.contains("tool_search")
                && message.content.contains("tool_describe")
                && message.content.contains("tool_call")
                && message
                    .content
                    .contains("When `extension_search` and `extension_install` are present")
                && message
                    .content
                    .contains("`extension_register_hosted_mcp` is also visible")
        }),
        "bridged disclosure should inject a visibility-conditional discovery protocol"
    );
    // The standalone runtime binds the native memory provider, so the model's
    // surface carries `ironclaw.memory.*`. Pinned at the caller because the
    // gate is composed here (`memory_protocol_active` in `runtime.rs`) from
    // whether a provider actually resolved — the unit tests below only prove
    // the flag is honored once someone sets it, not that it tracks the real
    // binding. A protocol naming concrete tools must not outlive those tools.
    assert!(
        recorded_requests[0].messages.iter().any(|message| {
            message.role == HostManagedModelMessageRole::System
                && message.content.contains("Persistent Memory")
                && message.content.contains("ironclaw.memory.write")
        }),
        "a runtime with a bound memory provider should inject the persistent-memory protocol"
    );
    assert!(
        recorded_requests[0].messages.iter().any(|message| {
            message.role == HostManagedModelMessageRole::User && message.content == "ping"
        }),
        "test should observe the real model request for the submitted user turn"
    );
    assert!(
        !recorded_requests[0].messages.iter().any(|message| {
            message.role == HostManagedModelMessageRole::System
                && message.content.contains("Background-run notifications:")
        }),
        "plain WebUI chat should not resolve an unrelated notification-channel slice"
    );
    assert!(
        recorded_requests[0].messages.iter().any(|message| {
            message.role == HostManagedModelMessageRole::System
                && message.content.contains("Run origin: CLI chat")
        }),
        "standalone runtime send_user_message should tag CLI source-channel origin in runtime context"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

/// With turn-start selection on, a run's prompt follows its frozen `tools`
/// array through the real runtime build: the selection section renders (the
/// selection advertises the bridges), while the memory guidance, the
/// extension-lifecycle paragraphs and the runtime-context tool mentions,
/// whose tools a greeting does not select, are withheld. Their
/// ordinary-surface rendering is pinned by the test above, so their absence
/// here is the selection's doing (#7836).
#[tokio::test]
async fn standalone_runtime_prompt_names_only_the_selected_tools() {
    let root = tempfile::tempdir().expect("tempdir");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let settings = ironclaw_config::ToolPrefetchSettings::resolve(
        None,
        &|name: &str| {
            Ok((name == ironclaw_config::REBORN_TOOL_PREFETCH_ENV).then(|| "lexical".to_string()))
        },
        ironclaw_config::ToolRetrievalMode::Native,
    )
    .expect("settings resolve")
    .expect("selection on");
    let input = runtime_input(root.path().join("standalone"), Arc::clone(&requests))
        .with_tool_prefetch(settings)
        .expect("valid selection");

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");
    assert_eq!(reply.status, TurnStatus::Completed);
    runtime.shutdown().await.expect("runtime shutdown");

    let recorded_requests = recorded_requests(&requests);
    let request = recorded_requests.first().expect("one model request");
    let system: String = request
        .messages
        .iter()
        .filter(|message| message.role == HostManagedModelMessageRole::System)
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        system.contains("Tools Chosen for This Conversation"),
        "{system}"
    );
    assert!(system.contains("Tool Discovery"), "{system}");
    // The `tool_search` description lists deferred tools by name on purpose
    // (they are reachable on demand), so the checks target the gated blocks'
    // own wording rather than bare tool names.
    for withheld in [
        "Persistent Memory",
        "ironclaw.memory.",
        "When `extension_search` and `extension_install` are present",
        "`extension_register_hosted_mcp` is also visible",
        "profile_set capability",
        "time capability",
    ] {
        assert!(
            !system.contains(withheld),
            "the prompt names {withheld:?}, which the run does not advertise: {system}"
        );
    }
}

/// A ranker whose fit always fails, so semantic turn-start selection cannot
/// choose and the run falls back to the ordinary surface.
#[derive(Debug)]
struct FailingRetrieval;

#[async_trait]
impl ironclaw_loop_contracts::ToolRetrievalProvider for FailingRetrieval {
    fn ranker_version(&self) -> &str {
        "failing-composition-test-v1"
    }

    async fn fit(
        &self,
        _definitions: &[ironclaw_loop_contracts::ProviderToolDefinition],
    ) -> Result<
        Arc<dyn ironclaw_loop_contracts::ToolRetrievalIndex>,
        ironclaw_loop_contracts::ToolRetrievalError,
    > {
        Err(
            ironclaw_loop_contracts::ToolRetrievalError::CorpusTooLarge {
                definitions: 0,
                limit: 0,
            },
        )
    }
}

/// The identity (`SYSTEM.md` plus appended sections) message of the first
/// recorded model request.
fn identity_prompt(requests: &Arc<StdMutex<Vec<HostManagedModelRequest>>>) -> String {
    recorded_requests(requests)
        .first()
        .expect("one model request")
        .messages
        .iter()
        .find(|message| {
            message.role == HostManagedModelMessageRole::System
                && message.content.starts_with("You are IronClaw Agent")
        })
        .expect("identity prompt")
        .content
        .clone()
}

/// A run whose turn-start selection fell back to the ordinary surface (here
/// the selection's ranker fails) renders the prompt a runtime with selection
/// off renders, byte for byte: no selection section, and every tool-naming
/// section the ordinary surface always carried.
#[tokio::test]
async fn standalone_runtime_fallback_run_keeps_the_selection_off_prompt() {
    async fn opening_identity_prompt(selection: bool) -> String {
        let root = tempfile::tempdir().expect("tempdir");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let mut input = runtime_input(root.path().join("standalone"), Arc::clone(&requests))
            .with_tool_retrieval_provider(Arc::new(FailingRetrieval));
        if selection {
            let settings = ironclaw_config::ToolPrefetchSettings::resolve(
                None,
                &|name: &str| {
                    Ok((name == ironclaw_config::REBORN_TOOL_PREFETCH_ENV)
                        .then(|| "semantic".to_string()))
                },
                ironclaw_config::ToolRetrievalMode::Dense,
            )
            .expect("settings resolve")
            .expect("selection on");
            input = input.with_tool_prefetch(settings).expect("valid selection");
        }
        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            Duration::from_secs(3),
            runtime.send_user_message(&conversation, "ping"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");
        assert_eq!(reply.status, TurnStatus::Completed);
        runtime.shutdown().await.expect("runtime shutdown");
        identity_prompt(&requests)
    }

    let selection_off = opening_identity_prompt(false).await;
    let fallback = opening_identity_prompt(true).await;
    assert_eq!(fallback, selection_off);
    assert!(!fallback.contains("Tools Chosen for This Conversation"));
    assert!(fallback.contains("`extension_register_hosted_mcp` is also visible"));
    assert!(fallback.contains("Persistent Memory"));
}

#[tokio::test]
async fn standalone_runtime_uses_existing_edited_default_system_prompt() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("standalone");
    let prompt_path = storage_root.join("system/prompts/default-system.md");
    std::fs::create_dir_all(prompt_path.parent().expect("prompt parent")).expect("prompt parent");
    std::fs::write(&prompt_path, "custom edited runtime prompt").expect("edited prompt");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let input = runtime_input(storage_root, Arc::clone(&requests));

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");

    assert_eq!(reply.status, TurnStatus::Completed);
    let recorded_requests = recorded_requests(&requests);
    assert_eq!(recorded_requests.len(), 1);
    assert!(
        recorded_requests[0].messages.iter().any(|message| {
            message.role == HostManagedModelMessageRole::System
                && message.content.starts_with("custom edited runtime prompt")
        }),
        "standalone runtime should preserve and inject the existing edited prompt"
    );
    // Disclosure is explicitly enabled, so the tool-search protocol is appended to the
    // (edited) system prompt and reaches the gateway — without overwriting the
    // user's edited base content.
    assert!(
        recorded_requests[0].messages.iter().any(|message| {
            message.role == HostManagedModelMessageRole::System
                && message.content.starts_with("custom edited runtime prompt")
                && message.content.contains("tool_search")
        }),
        "bridged disclosure should append the tool-search protocol to the edited prompt"
    );
    // Docs grounding is ground knowledge about the runtime (#6734), not a seed
    // default: an install whose SYSTEM.md never contained it must still be told
    // to look IronClaw's own capabilities up in the published docs.
    assert!(
        recorded_requests[0].messages.iter().any(|message| {
            message.role == HostManagedModelMessageRole::System
                && message.content.starts_with("custom edited runtime prompt")
                && message
                    .content
                    .contains("https://docs.ironclaw.com/llms.txt")
        }),
        "self-knowledge docs grounding should reach the model even for a custom system prompt"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn standalone_runtime_rejects_non_file_default_system_prompt() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("standalone");
    let prompt_path = storage_root.join("system/prompts/default-system.md");
    std::fs::create_dir_all(&prompt_path).expect("non-file prompt path");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let input = runtime_input(storage_root, requests);

    let error = match build_reborn_runtime(input).await {
        Ok(runtime) => {
            runtime.shutdown().await.expect("runtime shutdown");
            panic!("runtime should reject non-file default prompt");
        }
        Err(error) => error,
    };

    match error {
        RebornRuntimeError::Build(build_error) => {
            let message = build_error.to_string();
            assert!(message.contains("default system prompt"));
            assert!(message.contains("regular file"));
        }
        other => panic!("expected build error for non-file default prompt, got {other:?}"),
    }
}

/// Ranker that records every corpus it is fitted on and never matches.
#[derive(Debug, Default)]
struct RecordingRetrieval {
    fitted_corpus_sizes: StdMutex<Vec<usize>>,
}

#[derive(Debug)]
struct EmptyIndex;

#[async_trait]
impl ironclaw_loop_contracts::ToolRetrievalIndex for EmptyIndex {
    async fn search(
        &self,
        _query: &str,
        _limit: usize,
    ) -> Result<
        ironclaw_loop_contracts::ToolSearchOutcome,
        ironclaw_loop_contracts::ToolRetrievalError,
    > {
        Ok(ironclaw_loop_contracts::ToolSearchOutcome::no_match())
    }
}

#[async_trait]
impl ironclaw_loop_contracts::ToolRetrievalProvider for RecordingRetrieval {
    fn ranker_version(&self) -> &str {
        "recording-composition-test-v1"
    }

    async fn fit(
        &self,
        definitions: &[ironclaw_loop_contracts::ProviderToolDefinition],
    ) -> Result<
        Arc<dyn ironclaw_loop_contracts::ToolRetrievalIndex>,
        ironclaw_loop_contracts::ToolRetrievalError,
    > {
        self.fitted_corpus_sizes
            .lock()
            .expect("fit recorder lock poisoned")
            .push(definitions.len());
        Ok(Arc::new(EmptyIndex))
    }
}

/// `RebornRuntimeInput::with_tool_retrieval_provider` reaches the
/// tool-disclosure decorator through the real `build_reborn_runtime`: the
/// bound ranker is the one fitted over the run's authorized tool surface.
#[tokio::test]
async fn standalone_runtime_fits_the_bound_tool_retrieval_provider() {
    let root = tempfile::tempdir().expect("tempdir");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let provider = Arc::new(RecordingRetrieval::default());
    let input = runtime_input(root.path().join("standalone"), Arc::clone(&requests))
        .with_tool_retrieval_provider(
            Arc::clone(&provider) as Arc<dyn ironclaw_loop_contracts::ToolRetrievalProvider>
        );

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");
    assert_eq!(reply.status, TurnStatus::Completed);
    runtime.shutdown().await.expect("runtime shutdown");

    let fitted = provider
        .fitted_corpus_sizes
        .lock()
        .expect("fit recorder lock poisoned")
        .clone();
    assert!(
        fitted.iter().any(|size| *size > 0),
        "the bound ranker must be fitted over the run's authorized tools, got {fitted:?}"
    );
}

fn runtime_input(
    storage_root: std::path::PathBuf,
    requests: Arc<StdMutex<Vec<HostManagedModelRequest>>>,
) -> RebornRuntimeInput {
    let gateway = Arc::new(RecordingGateway { requests });
    RebornRuntimeInput::from_build_input(
        crate::deployment::local_filesystem_build_input(
            "runtime-system-prompt-owner",
            storage_root,
        )
        .with_runtime_policy(standalone_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-system-prompt-tenant".to_string(),
        agent_id: "runtime-system-prompt-agent".to_string(),
        source_binding_id: "runtime-system-prompt-source".to_string(),
        reply_target_binding_id: "runtime-system-prompt-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_model_gateway_override(gateway)
    // Pin bridged explicitly so the disclosure-protocol assertions do not depend
    // on the production default.
    .with_tool_disclosure(ironclaw_loop_host::ToolDisclosureMode::Bridged)
}

fn recorded_requests(
    requests: &Arc<StdMutex<Vec<HostManagedModelRequest>>>,
) -> Vec<HostManagedModelRequest> {
    requests
        .lock()
        .expect("recording gateway requests lock poisoned")
        .clone()
}

fn standalone_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalHost,
        resolved_profile: RuntimeProfile::LocalHost,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}
