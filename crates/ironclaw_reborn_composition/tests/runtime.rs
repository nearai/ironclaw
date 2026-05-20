use std::time::Duration;

use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_reborn_composition::{
    PollSettings, RebornBuildInput, RebornRuntimeError, RebornRuntimeIdentity, RebornRuntimeInput,
    TurnRunnerSettings, build_reborn_runtime,
};
use ironclaw_turns::TurnStatus;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn runtime_rejects_disabled_profile_before_local_substrate_lookup() {
    let input =
        RebornRuntimeInput::from_services(RebornBuildInput::disabled("runtime-disabled-owner"));

    let error = match build_reborn_runtime(input).await {
        Ok(_) => panic!("disabled profile is not a runnable REPL runtime"),
        Err(error) => error,
    };

    let RebornRuntimeError::InvalidArgument { reason } = error else {
        panic!("expected invalid argument, got {error:?}");
    };
    assert!(reason.contains("profile=disabled is not yet wired end-to-end"));
}

#[tokio::test]
async fn runtime_requires_resolved_runtime_policy_for_local_dev() {
    let root = tempfile::tempdir().unwrap();
    let input = RebornRuntimeInput::from_services(RebornBuildInput::local_dev(
        "runtime-policy-owner",
        root.path().join("local-dev"),
    ));

    let error = match build_reborn_runtime(input).await {
        Ok(_) => panic!("local-dev runtime should require a resolved runtime policy"),
        Err(error) => error,
    };

    let RebornRuntimeError::InvalidArgument { reason } = error else {
        panic!("expected invalid argument, got {error:?}");
    };
    assert!(reason.contains("resolved runtime policy"));
}

#[tokio::test]
async fn stub_gateway_send_cancels_recovery_required_and_releases_conversation() {
    let root = tempfile::tempdir().unwrap();
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev("runtime-test-owner", root.path().join("local-dev"))
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-test-tenant".to_string(),
        agent_id: "runtime-test-agent".to_string(),
        source_binding_id: "runtime-test-source".to_string(),
        reply_target_binding_id: "runtime-test-reply".to_string(),
    })
    .with_runner_settings(TurnRunnerSettings {
        heartbeat_interval: Duration::from_millis(25),
        poll_interval: Duration::from_secs(60),
    });

    let runtime = build_reborn_runtime(input).await.unwrap();
    assert_eq!(runtime.default_run_profile_id(), "reborn-planned-default");

    let conversation = runtime.new_conversation().await.unwrap();
    let reply = tokio::time::timeout(
        Duration::from_secs(2),
        runtime.send_user_message(&conversation, "hello"),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(reply.status, TurnStatus::Cancelled);
    assert_eq!(reply.text, None);

    let second_reply = tokio::time::timeout(
        Duration::from_secs(2),
        runtime.send_user_message(&conversation, "hello again"),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(second_reply.status, TurnStatus::Cancelled);
    assert_eq!(second_reply.text, None);

    runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn send_user_message_with_cancellation_cancels_submitted_run() {
    let root = tempfile::tempdir().unwrap();
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev("runtime-cancel-owner", root.path().join("local-dev"))
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "runtime-cancel-tenant".to_string(),
        agent_id: "runtime-cancel-agent".to_string(),
        source_binding_id: "runtime-cancel-source".to_string(),
        reply_target_binding_id: "runtime-cancel-reply".to_string(),
    })
    .with_runner_settings(TurnRunnerSettings {
        heartbeat_interval: Duration::from_secs(60),
        poll_interval: Duration::from_secs(60),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_secs(60),
        max_total: Duration::from_secs(180),
    });

    let runtime = build_reborn_runtime(input).await.unwrap();
    let conversation = runtime.new_conversation().await.unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let error = tokio::time::timeout(
        Duration::from_secs(2),
        runtime.send_user_message_with_cancellation(&conversation, "cancel me", cancellation),
    )
    .await
    .unwrap()
    .unwrap_err();

    assert!(matches!(error, RebornRuntimeError::OperationCancelled));

    runtime.shutdown().await.unwrap();
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
