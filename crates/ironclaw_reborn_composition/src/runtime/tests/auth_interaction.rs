use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_auth::InMemoryAuthProductServices;
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_product::{
    AuthInteractionRejectionKind, ListPendingAuthInteractionsRequest, ProductSurfaceFailure,
};
use ironclaw_turns::{TurnActor, TurnScope};

use crate::runtime_input::{RebornRuntimeIdentity, RebornRuntimeInput, TurnRunnerSettings};
use crate::{RebornProductAuthServicePorts, RebornRuntimeProcessBinding};

use super::{RebornRuntime, build_reborn_runtime};

#[derive(Debug)]
struct UnusedModelGateway;

#[async_trait]
impl HostManagedModelGateway for UnusedModelGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Ok(HostManagedModelResponse::assistant_reply(
            "unused auth interaction test reply".to_string(),
        ))
    }
}

#[tokio::test]
async fn local_dev_runtime_auth_interactions_are_unavailable_without_flow_record_source() {
    let auth = Arc::new(InMemoryAuthProductServices::new());
    let ports = RebornProductAuthServicePorts::from_shared(auth);
    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_runtime(
        "auth-read-model-absent",
        root.path().join("local-dev"),
        Some(ports),
    )
    .await
    .expect("runtime builds");
    assert!(
        runtime.product_auth.flow_record_source().is_none(),
        "custom product-auth ports intentionally do not imply a WebUI read projection"
    );
    let conversation = runtime.new_conversation().await.expect("conversation");
    let scope = TurnScope::new(
        runtime.thread_scope.tenant_id.clone(),
        Some(runtime.thread_scope.agent_id.clone()),
        runtime.thread_scope.project_id.clone(),
        conversation.0,
    );

    let error = runtime
        .webui_auth_interaction_service()
        .list_pending(ListPendingAuthInteractionsRequest {
            scope,
            actor: TurnActor::new(runtime.actor_user_id.clone()),
        })
        .await
        .expect_err("auth interaction read model is unavailable");

    assert!(matches!(
        error,
        ProductSurfaceFailure::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::FlowUnavailable
        }
    ));

    runtime.shutdown().await.expect("runtime shutdown");
}

async fn build_runtime(
    owner: &str,
    storage_root: PathBuf,
    product_auth_ports: Option<RebornProductAuthServicePorts>,
) -> Result<RebornRuntime, super::RebornRuntimeError> {
    let mut services = crate::deployment::local_dev_build_input(owner, storage_root)
        .with_runtime_policy(local_dev_runtime_policy())
        .with_runtime_process_binding(RebornRuntimeProcessBinding::None);
    if let Some(ports) = product_auth_ports {
        services = services.with_product_auth_ports(ports);
    }
    let runtime = build_reborn_runtime(
        RebornRuntimeInput::from_build_input(services)
            .with_identity(RebornRuntimeIdentity {
                tenant_id: format!("{owner}-tenant"),
                agent_id: format!("{owner}-agent"),
                source_binding_id: format!("{owner}-source"),
                reply_target_binding_id: format!("{owner}-reply"),
            })
            .with_runner_settings(
                TurnRunnerSettings::default()
                    .set_heartbeat_interval(Duration::from_secs(60))
                    .set_poll_interval(Duration::from_secs(60)),
            )
            .with_model_gateway_override(Arc::new(UnusedModelGateway)),
    )
    .await?;
    runtime.turn_scheduler.stop_for_test().await;
    Ok(runtime)
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
