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
        }),
        "bridged disclosure should inject a visibility-conditional discovery protocol"
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
                && message.content.contains("Outbound delivery target:")
        }),
        "plain WebUI chat should not resolve an unrelated outbound delivery target"
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
    .with_tool_disclosure(ironclaw_runner::runtime::ToolDisclosureMode::Bridged)
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
