use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthGateRef, AuthProductScope,
    AuthProviderId, AuthSessionId, AuthSurface, InMemoryAuthProductServices, NewAuthFlow,
    OAuthAuthorizationUrl, OpaqueStateHash, PkceVerifierHash, TurnRunRef,
};
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_host_api::{InvocationId, ResourceScope, RuntimeCredentialAuthRequirement, UserId};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_product_workflow::{
    AuthInteractionRejectionKind, ListPendingAuthInteractionsRequest, ProductWorkflowError,
};
use ironclaw_turns::runner::{BlockRunRequest, ClaimRunRequest, TurnRunTransitionPort};
use ironclaw_turns::{
    AcceptedMessageRef, AllowAllTurnAdmissionPolicy, BlockedReason, GateRef, IdempotencyKey,
    InMemoryRunProfileResolver, LoopCheckpointStateRef, ReplyTargetBindingRef, RunProfileRequest,
    SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCheckpointId,
    TurnLeaseToken, TurnRunId, TurnRunnerId, TurnScope, TurnStateStore,
};

use crate::input::RebornBuildInput;
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
async fn local_dev_runtime_auth_interactions_use_flow_record_source() {
    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_runtime(
        "auth-read-model-present",
        root.path().join("local-dev"),
        None,
    )
    .await
    .expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let subject_user_id = UserId::new("team-agent-user").expect("subject user id");
    let scope = TurnScope::new_with_owner(
        runtime.thread_scope.tenant_id.clone(),
        Some(runtime.thread_scope.agent_id.clone()),
        runtime.thread_scope.project_id.clone(),
        conversation.0,
        Some(subject_user_id.clone()),
    );
    let actor = TurnActor::new(runtime.actor_user_id.clone());
    let gate_ref = GateRef::new("gate:runtime-auth-read-model").expect("gate");
    let run_id = submit_and_block_auth_run(
        &runtime,
        scope.clone(),
        actor.clone(),
        &gate_ref,
        Vec::new(),
    )
    .await;
    create_auth_flow(&runtime, &scope, &actor, run_id, &gate_ref).await;

    let pending = runtime
        .webui_auth_interaction_service()
        .list_pending(ListPendingAuthInteractionsRequest {
            scope: scope.clone(),
            actor: actor.clone(),
        })
        .await
        .expect("pending auth interactions");

    assert_eq!(pending.auth_interactions.len(), 1);
    let view = &pending.auth_interactions[0];
    assert_eq!(view.scope.tenant_id, scope.tenant_id);
    assert_eq!(view.scope.user_id, subject_user_id);
    assert_eq!(view.scope.thread_id, scope.thread_id);
    assert_eq!(view.run_id, run_id);
    assert_eq!(view.auth_request_ref, gate_ref);

    runtime.shutdown().await.expect("runtime shutdown");
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
        runtime
            .services
            .product_auth
            .as_ref()
            .expect("product auth")
            .flow_record_source()
            .is_none(),
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
        ProductWorkflowError::AuthInteractionRejected {
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
    let mut services = RebornBuildInput::local_dev(owner, storage_root)
        .with_runtime_policy(local_dev_runtime_policy())
        .with_runtime_process_binding(RebornRuntimeProcessBinding::None);
    if let Some(ports) = product_auth_ports {
        services = services.with_product_auth_ports(ports);
    }
    build_reborn_runtime(
        RebornRuntimeInput::from_services(services)
            .with_identity(RebornRuntimeIdentity {
                tenant_id: format!("{owner}-tenant"),
                agent_id: format!("{owner}-agent"),
                source_binding_id: format!("{owner}-source"),
                reply_target_binding_id: format!("{owner}-reply"),
            })
            .with_runner_settings(TurnRunnerSettings {
                heartbeat_interval: Duration::from_secs(60),
                poll_interval: Duration::from_secs(60),
                ..TurnRunnerSettings::default()
            })
            .with_model_gateway_override(Arc::new(UnusedModelGateway)),
    )
    .await
}

async fn submit_and_block_auth_run(
    runtime: &RebornRuntime,
    scope: TurnScope,
    actor: TurnActor,
    gate_ref: &GateRef,
    credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
) -> TurnRunId {
    let local_runtime = runtime
        .services
        .local_runtime
        .as_ref()
        .expect("local runtime");
    let admission = AllowAllTurnAdmissionPolicy;
    let profiles = InMemoryRunProfileResolver::default();
    let submit = local_runtime
        .turn_state
        .submit_turn(
            SubmitTurnRequest {
                scope: scope.clone(),
                actor,
                accepted_message_ref: AcceptedMessageRef::new("message-runtime-auth-read-model")
                    .expect("message ref"),
                source_binding_ref: SourceBindingRef::new("source-runtime-auth-read-model")
                    .expect("source ref"),
                reply_target_binding_ref: ReplyTargetBindingRef::new(
                    "reply-runtime-auth-read-model",
                )
                .expect("reply ref"),
                requested_run_profile: Some(RunProfileRequest::new("default").expect("profile")),
                idempotency_key: IdempotencyKey::new("submit-runtime-auth-read-model")
                    .expect("idempotency key"),
                received_at: Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
                product_context: None,
            },
            &admission,
            &profiles,
        )
        .await
        .expect("submit turn through local-dev turn state");
    let SubmitTurnResponse::Accepted { run_id, .. } = submit;
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    local_runtime
        .turn_state
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope),
        })
        .await
        .expect("claim run")
        .expect("queued run exists");
    local_runtime
        .turn_state
        .block_run(BlockRunRequest {
            run_id,
            runner_id,
            lease_token,
            checkpoint_id: TurnCheckpointId::new(),
            state_ref: LoopCheckpointStateRef::new("checkpoint:runtime-auth-read-model")
                .expect("checkpoint ref"),
            reason: BlockedReason::Auth {
                gate_ref: gate_ref.clone(),
                credential_requirements,
            },
        })
        .await
        .expect("block auth run");
    run_id
}

async fn create_auth_flow(
    runtime: &RebornRuntime,
    scope: &TurnScope,
    actor: &TurnActor,
    run_id: TurnRunId,
    gate_ref: &GateRef,
) {
    runtime
        .services
        .product_auth
        .as_ref()
        .expect("product auth")
        .flow_manager()
        .create_flow(NewAuthFlow {
            id: None,
            scope: auth_scope_for_turn(scope, actor),
            kind: AuthFlowKind::IntegrationCredential,
            provider: AuthProviderId::new("github").expect("provider"),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                    .expect("authorization url"),
                expires_at: Utc::now() + chrono::Duration::minutes(5),
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(run_id.to_string()).expect("turn run ref"),
                gate_ref: AuthGateRef::new(gate_ref.as_str()).expect("auth gate ref"),
            },
            update_binding: None,
            opaque_state_hash: Some(state_hash()),
            pkce_verifier_hash: Some(pkce_hash()),
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        })
        .await
        .expect("auth flow");
}

fn auth_scope_for_turn(scope: &TurnScope, actor: &TurnActor) -> AuthProductScope {
    AuthProductScope::new(
        ResourceScope {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope
                .explicit_owner_user_id()
                .cloned()
                .unwrap_or_else(|| actor.user_id.clone()),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            mission_id: None,
            thread_id: Some(scope.thread_id.clone()),
            invocation_id: InvocationId::new(),
        },
        AuthSurface::Web,
    )
    .with_session_id(AuthSessionId::new("session-runtime-auth-read-model").expect("session id"))
}

fn state_hash() -> OpaqueStateHash {
    OpaqueStateHash::new(fake_digest("state-hash")).expect("state hash")
}

fn pkce_hash() -> PkceVerifierHash {
    PkceVerifierHash::new(fake_digest("pkce-hash")).expect("pkce hash")
}

fn fake_digest(value: &str) -> String {
    format!(
        "{:064x}",
        value.bytes().fold(0_u64, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(u64::from(byte))
        })
    )
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

/// Caller-level regression for the channel-connection gate resume (V2). A
/// channel-pairing gate carries no `AuthFlowRecord`, so it never surfaces
/// through the OAuth `list_pending` read model; the dedicated resume read model
/// enumerates it from blocked turn state instead. This drives the real
/// composition read model + product-workflow resume service against a live
/// `LocalDevTurnStateStore`, exactly as the Slack pairing-redeem route wires
/// them, and pins scope + channel isolation.
#[cfg(feature = "slack-v2-host-beta")]
mod channel_pairing_resume_tests {
    use std::sync::Arc;

    use ironclaw_host_api::{
        ExtensionId, RuntimeCredentialAccountProviderId, RuntimeCredentialAccountSetup,
        RuntimeCredentialAuthRequirement, ThreadId, UserId,
    };
    use ironclaw_product_workflow::{
        ChannelConnectionResumeReadModel, ChannelConnectionResumeScope,
        ChannelConnectionResumeService, DefaultChannelConnectionResumeService,
        ResumeChannelConnectionRequest,
    };
    use ironclaw_turns::{
        GateRef, GetRunStateRequest, TurnActor, TurnCoordinator, TurnScope, TurnStatus,
    };

    use super::{build_runtime, submit_and_block_auth_run};
    use crate::channel_connection_resume::LocalDevChannelConnectionResumeReadModel;

    fn channel_pairing_requirement(channel: &str) -> RuntimeCredentialAuthRequirement {
        RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new(channel).expect("provider"),
            setup: RuntimeCredentialAccountSetup::ChannelPairing {
                channel: channel.to_string(),
            },
            requester_extension: ExtensionId::new(channel).expect("requester extension"),
            provider_scopes: Vec::new(),
        }
    }

    #[tokio::test]
    async fn channel_pairing_resume_continues_only_the_callers_runs_for_the_paired_channel() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_runtime(
            "channel-pairing-resume",
            root.path().join("local-dev"),
            None,
        )
        .await
        .expect("runtime builds");

        let tenant = runtime.thread_scope.tenant_id.clone();
        let agent = Some(runtime.thread_scope.agent_id.clone());
        let project = runtime.thread_scope.project_id.clone();
        let alice = UserId::new("user:alice-pairing").expect("alice");
        let bob = UserId::new("user:bob-pairing").expect("bob");

        // Run A: alice parked on the Slack connection gate.
        let scope_a = TurnScope::new_with_owner(
            tenant.clone(),
            agent.clone(),
            project.clone(),
            ThreadId::new("thread:pairing-a").expect("thread"),
            Some(alice.clone()),
        );
        let run_a = submit_and_block_auth_run(
            &runtime,
            scope_a.clone(),
            TurnActor::new(alice.clone()),
            &GateRef::new("gate:pairing-alice-slack").expect("gate"),
            vec![channel_pairing_requirement("slack")],
        )
        .await;

        // Run B: a DIFFERENT caller (bob) parked on the Slack gate.
        let scope_b = TurnScope::new_with_owner(
            tenant.clone(),
            agent.clone(),
            project.clone(),
            ThreadId::new("thread:pairing-b").expect("thread"),
            Some(bob.clone()),
        );
        let run_b = submit_and_block_auth_run(
            &runtime,
            scope_b.clone(),
            TurnActor::new(bob.clone()),
            &GateRef::new("gate:pairing-bob-slack").expect("gate"),
            vec![channel_pairing_requirement("slack")],
        )
        .await;

        // Run C: alice parked on a DIFFERENT channel (telegram).
        let scope_c = TurnScope::new_with_owner(
            tenant.clone(),
            agent.clone(),
            project.clone(),
            ThreadId::new("thread:pairing-c").expect("thread"),
            Some(alice.clone()),
        );
        let run_c = submit_and_block_auth_run(
            &runtime,
            scope_c.clone(),
            TurnActor::new(alice.clone()),
            &GateRef::new("gate:pairing-alice-telegram").expect("gate"),
            vec![channel_pairing_requirement("telegram")],
        )
        .await;

        let turn_state = Arc::clone(
            &runtime
                .services
                .local_runtime
                .as_ref()
                .expect("local runtime")
                .turn_state,
        );
        let read_model = Arc::new(LocalDevChannelConnectionResumeReadModel::new(turn_state));

        // Enumeration reads the durable snapshot (runner-independent, race-free):
        // exactly alice's Slack run — not bob's, not alice's telegram run.
        let enumerated = read_model
            .channel_pairing_blocked_runs(
                &ChannelConnectionResumeScope {
                    tenant_id: tenant.clone(),
                    user_id: alice.clone(),
                },
                "slack",
            )
            .await
            .expect("enumerate caller's parked slack runs");
        assert_eq!(
            enumerated.len(),
            1,
            "only alice's slack-pairing run is enumerable for the caller"
        );
        assert_eq!(enumerated[0].run_id, run_a);

        let service = DefaultChannelConnectionResumeService::new(
            read_model,
            runtime.webui_turn_coordinator(),
        );
        let response = service
            .resume_channel_connection(ResumeChannelConnectionRequest {
                scope: ChannelConnectionResumeScope {
                    tenant_id: tenant.clone(),
                    user_id: alice.clone(),
                },
                channel: "slack".to_string(),
            })
            .await
            .expect("resume alice's slack-pairing runs");

        // A successful `BlockedAuthGate` resume proves run A left the gate. The
        // resume re-queues the SAME run id (no new turn/run), so no synthetic
        // "Slack is connected, continue" message is appended.
        assert_eq!(response.resumed_runs, vec![run_a]);

        // Scope + channel isolation: bob's run and alice's telegram run remain
        // parked, untouched by the resume.
        let coordinator = runtime.webui_turn_coordinator();
        let state_b = coordinator
            .get_run_state(GetRunStateRequest {
                scope: scope_b,
                run_id: run_b,
            })
            .await
            .expect("bob run state");
        assert_eq!(
            state_b.status,
            TurnStatus::BlockedAuth,
            "another caller's parked run must not be resumed"
        );
        let state_c = coordinator
            .get_run_state(GetRunStateRequest {
                scope: scope_c,
                run_id: run_c,
            })
            .await
            .expect("telegram run state");
        assert_eq!(
            state_c.status,
            TurnStatus::BlockedAuth,
            "a run parked on a different channel must not be resumed"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }
}
