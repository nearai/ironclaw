use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelMessageRole,
    HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_turns::TurnStatus;

use crate::input::RebornBuildInput;
use crate::runtime_input::{PollSettings, RebornRuntimeIdentity, RebornRuntimeInput};

use super::build_reborn_runtime;

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
async fn local_dev_runtime_injects_default_system_prompt_into_model_request() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let gateway = Arc::new(RecordingGateway {
        requests: Arc::clone(&requests),
    });
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev("runtime-system-prompt-owner", storage_root.clone())
            .with_runtime_policy(local_dev_runtime_policy()),
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
    .with_model_gateway_override(gateway);

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
        "local-dev runtime should seed an editable prompt file under storage"
    );
    let recorded_requests = requests
        .lock()
        .expect("recording gateway requests lock poisoned");
    assert_eq!(recorded_requests.len(), 1);
    assert!(
        recorded_requests[0].messages.iter().any(|message| {
            message.role == HostManagedModelMessageRole::System
                && message
                    .content
                    .contains("When a tool result is partial, truncated, failed")
        }),
        "local-dev runtime should send the editable default system prompt to the model gateway"
    );
    assert!(
        recorded_requests[0].messages.iter().any(|message| {
            message.role == HostManagedModelMessageRole::User && message.content == "ping"
        }),
        "test should observe the real model request for the submitted user turn"
    );
    drop(recorded_requests);

    runtime.shutdown().await.expect("runtime shutdown");
}

fn local_dev_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}
