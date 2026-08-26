// arch-exempt: large_file, run-start sweep tests share this file's fixtures and doubles with the delivery-chain tests they extend, plan #7788
use super::*;

#[test]
fn benign_already_resumed_set_is_exactly_queued_running_completed() {
    let benign = [
        TurnStatus::Queued,
        TurnStatus::Running,
        TurnStatus::Completed,
    ];
    for from in benign {
        let error = TurnError::InvalidTransition {
            from,
            to: TurnStatus::Queued,
        };
        assert!(
            is_benign_already_resumed(&error),
            "{from:?} must be treated as benign already-resumed"
        );
    }
}

#[test]
fn non_benign_invalid_transition_statuses_surface_as_real_errors() {
    // Every `TurnStatus` NOT in the benign set — including the
    // still-blocked-on-something-else statuses that are the actual data
    // -loss bug this discriminator guards against (a parent stuck on an
    // unrelated approval/auth/resource/external-tool gate must not be
    // silently treated as "already resumed").
    let non_benign = [
        TurnStatus::BlockedApproval,
        TurnStatus::BlockedAuth,
        TurnStatus::BlockedResource,
        TurnStatus::BlockedDependentRun,
        TurnStatus::BlockedExternalTool,
        TurnStatus::CancelRequested,
        TurnStatus::Cancelled,
        TurnStatus::Failed,
        TurnStatus::RecoveryRequired,
    ];
    for from in non_benign {
        let error = TurnError::InvalidTransition {
            from,
            to: TurnStatus::Queued,
        };
        assert!(
            !is_benign_already_resumed(&error),
            "{from:?} must NOT be treated as benign — it indicates the parent \
             never actually moved past BlockedDependentRun for an unrelated reason"
        );
    }
}

#[test]
fn non_invalid_transition_errors_are_never_benign() {
    // A wildcard on the *error variant* (matching `Conflict` or any
    // other kind alongside `InvalidTransition`) is exactly the class of
    // bug this discriminator replaced — pin that only this one error
    // shape, with only this one `from`-set, is ever benign.
    assert!(!is_benign_already_resumed(&TurnError::Conflict {
        reason: "unrelated conflict".to_string()
    }));
    assert!(!is_benign_already_resumed(&TurnError::ScopeNotFound));
    assert!(!is_benign_already_resumed(&TurnError::Unauthorized));
}

// ─── reconstruct_edge (FIX A): pure data transformation off cached
// `SubagentThreadMetadata`, zero `agent_turn_runtime` calls for the
// parent ──────────────────────────────────────────────────────────

struct ReconResultWriter;

#[async_trait::async_trait]
impl ironclaw_loop_host::LoopCapabilityResultWriter for ReconResultWriter {
    async fn write_capability_result(
        &self,
        _write: ironclaw_loop_host::CapabilityResultWrite<'_>,
    ) -> Result<ironclaw_loop_host::CapabilityWriteResult, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            ironclaw_loop_contracts::AgentLoopHostErrorKind::Unavailable,
            "not exercised by reconstruct_edge tests",
        ))
    }
}

fn recon_scoped_fs()
-> Arc<ironclaw_filesystem::ScopedFilesystem<ironclaw_filesystem::InMemoryBackend>> {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/processes").unwrap(),
        VirtualPath::new("/processes").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

fn recon_resolver(
    thread_service: Arc<ironclaw_threads::InMemorySessionThreadService>,
) -> AwaitEdgeResolver<ironclaw_threads::InMemorySessionThreadService> {
    let store = Arc::new(AwaitEdgeStore::new(Arc::new(
        ironclaw_processes::ProcessJournalStore::new(recon_scoped_fs()),
    )));
    let agent_turn_runtime: Arc<dyn AgentTurnSpawnTreeRuntimePort> =
        Arc::new(ironclaw_turns::test_support::in_memory_agent_turn_runtime());
    let result_writer: Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter> =
        Arc::new(ReconResultWriter);
    AwaitEdgeResolver::new_unbound(store, agent_turn_runtime, result_writer, thread_service)
}

fn recon_child_record(
    tenant_id: &ironclaw_host_api::ids::TenantId,
    agent_id: &ironclaw_host_api::ids::AgentId,
    child_thread_id: &ironclaw_host_api::ids::ThreadId,
    child_run_id: TurnRunId,
    parent_run_id: TurnRunId,
    resolved_run_profile: ironclaw_loop_contracts::ResolvedRunProfile,
) -> TurnRunRecord {
    TurnRunRecord {
        subagent_activation_provenance: None,
        run_id: child_run_id,
        turn_id: ironclaw_host_api::turn::TurnId::new(),
        scope: TurnScope::new(
            tenant_id.clone(),
            Some(agent_id.clone()),
            None,
            child_thread_id.clone(),
        ),
        accepted_message_ref: ironclaw_host_api::turn::AcceptedMessageRef::new("msg:child")
            .unwrap(),
        status: TurnStatus::Completed,
        profile: ironclaw_turns::TurnRunProfile::from_resolved(resolved_run_profile),
        output_contract: Default::default(),
        resolved_model_route: None,
        model_usage: None,
        execution_outcome: None,
        checkpoint_id: None,
        gate_ref: None,
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: ironclaw_host_api::turn::EventCursor(1),
        runner_id: None,
        lease_token: None,
        lease_expires_at: None,
        last_heartbeat_at: None,
        claim_count: 0,
        received_at: chrono::Utc::now(),
        parent_run_id: Some(parent_run_id),
        subagent_depth: 1,
        spawn_tree_root_run_id: Some(parent_run_id),
        product_context: None,
        resume_disposition: None,
    }
}

fn recon_event(
    child_run_id: TurnRunId,
    scope: TurnScope,
    owner_user_id: UserId,
) -> TurnLifecycleEvent {
    TurnLifecycleEvent {
        cursor: ironclaw_host_api::turn::EventCursor(1),
        scope,
        occurred_at: None,
        owner_user_id: Some(owner_user_id),
        run_id: child_run_id,
        status: TurnStatus::Completed,
        kind: ironclaw_turns::TurnEventKind::Completed,
        blocked_gate: None,
        sanitized_reason: None,
        retryable: None,
        detail: None,
    }
}

async fn recon_seed_thread(
    thread_service: &ironclaw_threads::InMemorySessionThreadService,
    tenant_id: &ironclaw_host_api::ids::TenantId,
    agent_id: &ironclaw_host_api::ids::AgentId,
    child_thread_id: &ironclaw_host_api::ids::ThreadId,
    owner_user_id: &UserId,
    metadata_json: Option<String>,
) {
    thread_service
        .ensure_thread(ironclaw_threads::EnsureThreadRequest {
            scope: ThreadScope {
                tenant_id: tenant_id.clone(),
                agent_id: agent_id.clone(),
                project_id: None,
                owner_user_id: Some(owner_user_id.clone()),
                mission_id: None,
            },
            thread_id: Some(child_thread_id.clone()),
            created_by_actor_id: "test".to_string(),
            title: None,
            metadata_json,
        })
        .await
        .unwrap();
}

// (T1) well-formed metadata -> correct AwaitEdge with gate_ref +
// parent_run_context sourced from metadata. Mutation: source gate_ref
// from a derived token instead of `metadata.gate_ref` -> RED (the
// shared-batch-gate assertion below fails because a derived token never
// matches the metadata-cached one).
#[tokio::test]
async fn reconstruct_edge_builds_edge_from_cached_metadata() {
    let tenant_id = ironclaw_host_api::ids::TenantId::new("recon-tenant-t1").unwrap();
    let agent_id = ironclaw_host_api::ids::AgentId::new("recon-agent-t1").unwrap();
    let child_thread_id = ironclaw_host_api::ids::ThreadId::new("recon-child-thread-t1").unwrap();
    let parent_thread_id = ironclaw_host_api::ids::ThreadId::new("recon-parent-thread-t1").unwrap();
    let owner_user_id = UserId::new("recon-owner-t1").unwrap();
    let parent_run_id = TurnRunId::new();
    let child_run_id = TurnRunId::new();

    let parent_context = ironclaw_agent_loop::test_support::test_run_context("recon-t1");
    let child_record = recon_child_record(
        &tenant_id,
        &agent_id,
        &child_thread_id,
        child_run_id,
        parent_run_id,
        parent_context.resolved_run_profile.clone(),
    );
    let event = recon_event(
        child_run_id,
        child_record.scope.clone(),
        owner_user_id.clone(),
    );
    // Distinct from the derived `gate:subagent-<child_run_id>` token so
    // the test can tell "sourced from metadata" apart from "recomputed".
    let metadata_gate_ref = TurnGateRef::new("gate:subagent-shared-batch").unwrap();
    let metadata = ironclaw_loop_host::SubagentThreadMetadata {
        kind: ironclaw_loop_host::SubagentThreadKind::Subagent,
        parent_run_id,
        parent_thread_id: parent_thread_id.clone(),
        tree_root_run_id: parent_run_id,
        child_run_id,
        subagent_kind: ironclaw_loop_host::SubagentKindId::new("general").unwrap(),
        mode: ironclaw_loop_host::SpawnSubagentMode::Blocking,
        result_ref: ironclaw_host_api::turn::LoopResultRef::new("result:subagent.recon-t1")
            .unwrap(),
        spawn_provider_call_id: Some("spawn-call-recon-t1".to_string()),
        handoff: None,
        parent_run_context: parent_context.clone(),
        gate_ref: metadata_gate_ref.clone(),
    };

    let thread_service = Arc::new(ironclaw_threads::InMemorySessionThreadService::default());
    recon_seed_thread(
        &thread_service,
        &tenant_id,
        &agent_id,
        &child_thread_id,
        &owner_user_id,
        Some(serde_json::to_string(&metadata).unwrap()),
    )
    .await;
    let resolver = recon_resolver(thread_service);

    let edge = resolver
        .reconstruct_edge(&child_record, parent_run_id, &event)
        .await
        .unwrap()
        .expect("well-formed metadata should reconstruct an edge");

    assert_eq!(edge.gate_ref, metadata_gate_ref);
    assert_eq!(edge.parent_run_context.turn_id, parent_context.turn_id);
    assert_eq!(
        edge.parent_run_context.resolved_run_profile,
        parent_context.resolved_run_profile
    );
    assert_eq!(edge.parent_run_context.run_id, parent_run_id);
    assert_eq!(edge.parent_run_context.thread_id, parent_thread_id);
    assert_eq!(
        edge.parent_run_context.actor,
        Some(TurnActor::new(owner_user_id))
    );
    assert_eq!(edge.parent_thread_id, parent_thread_id);
    assert_eq!(edge.tree_root_run_id, parent_run_id);
    assert_eq!(edge.mode, ironclaw_loop_host::SpawnSubagentMode::Blocking);
}

// (T2) identity mismatch: metadata's own `parent_run_id` disagrees with
// the trusted child record's `parent_run_id` argument -> fail closed to
// `Ok(None)`, never reconstruct against the wrong parent.
#[tokio::test]
async fn reconstruct_edge_fails_closed_on_parent_run_id_mismatch() {
    let tenant_id = ironclaw_host_api::ids::TenantId::new("recon-tenant-t2").unwrap();
    let agent_id = ironclaw_host_api::ids::AgentId::new("recon-agent-t2").unwrap();
    let child_thread_id = ironclaw_host_api::ids::ThreadId::new("recon-child-thread-t2").unwrap();
    let parent_thread_id = ironclaw_host_api::ids::ThreadId::new("recon-parent-thread-t2").unwrap();
    let owner_user_id = UserId::new("recon-owner-t2").unwrap();
    let parent_run_id = TurnRunId::new();
    let wrong_parent_run_id = TurnRunId::new();
    let child_run_id = TurnRunId::new();

    let parent_context = ironclaw_agent_loop::test_support::test_run_context("recon-t2");
    let child_record = recon_child_record(
        &tenant_id,
        &agent_id,
        &child_thread_id,
        child_run_id,
        parent_run_id,
        parent_context.resolved_run_profile.clone(),
    );
    let event = recon_event(
        child_run_id,
        child_record.scope.clone(),
        owner_user_id.clone(),
    );
    let metadata = ironclaw_loop_host::SubagentThreadMetadata {
        kind: ironclaw_loop_host::SubagentThreadKind::Subagent,
        parent_run_id: wrong_parent_run_id,
        parent_thread_id: parent_thread_id.clone(),
        tree_root_run_id: wrong_parent_run_id,
        child_run_id,
        subagent_kind: ironclaw_loop_host::SubagentKindId::new("general").unwrap(),
        mode: ironclaw_loop_host::SpawnSubagentMode::Blocking,
        result_ref: ironclaw_host_api::turn::LoopResultRef::new("result:subagent.recon-t2")
            .unwrap(),
        spawn_provider_call_id: None,
        handoff: None,
        parent_run_context: parent_context,
        gate_ref: TurnGateRef::new("gate:subagent-t2").unwrap(),
    };

    let thread_service = Arc::new(ironclaw_threads::InMemorySessionThreadService::default());
    recon_seed_thread(
        &thread_service,
        &tenant_id,
        &agent_id,
        &child_thread_id,
        &owner_user_id,
        Some(serde_json::to_string(&metadata).unwrap()),
    )
    .await;
    let resolver = recon_resolver(thread_service);

    let result = resolver
        .reconstruct_edge(&child_record, parent_run_id, &event)
        .await
        .unwrap();

    assert!(
        result.is_none(),
        "parent_run_id mismatch must fail closed to None"
    );
}

// (T3) malformed/absent metadata -> `Ok(None)`, never an error and never
// a fabricated edge.
#[tokio::test]
async fn reconstruct_edge_returns_none_for_absent_or_malformed_metadata() {
    let tenant_id = ironclaw_host_api::ids::TenantId::new("recon-tenant-t3").unwrap();
    let agent_id = ironclaw_host_api::ids::AgentId::new("recon-agent-t3").unwrap();
    let child_thread_id = ironclaw_host_api::ids::ThreadId::new("recon-child-thread-t3").unwrap();
    let owner_user_id = UserId::new("recon-owner-t3").unwrap();
    let parent_run_id = TurnRunId::new();
    let child_run_id = TurnRunId::new();
    let parent_context = ironclaw_agent_loop::test_support::test_run_context("recon-t3");
    let child_record = recon_child_record(
        &tenant_id,
        &agent_id,
        &child_thread_id,
        child_run_id,
        parent_run_id,
        parent_context.resolved_run_profile.clone(),
    );
    let event = recon_event(
        child_run_id,
        child_record.scope.clone(),
        owner_user_id.clone(),
    );

    // (a) no metadata at all on the child's thread.
    let thread_service_absent = Arc::new(ironclaw_threads::InMemorySessionThreadService::default());
    recon_seed_thread(
        &thread_service_absent,
        &tenant_id,
        &agent_id,
        &child_thread_id,
        &owner_user_id,
        None,
    )
    .await;
    let resolver_absent = recon_resolver(thread_service_absent);
    let result_absent = resolver_absent
        .reconstruct_edge(&child_record, parent_run_id, &event)
        .await
        .unwrap();
    assert!(result_absent.is_none(), "absent metadata must return None");

    // (b) metadata present but not subagent-kind shaped.
    let thread_service_malformed =
        Arc::new(ironclaw_threads::InMemorySessionThreadService::default());
    recon_seed_thread(
        &thread_service_malformed,
        &tenant_id,
        &agent_id,
        &child_thread_id,
        &owner_user_id,
        Some("{\"kind\":\"not-a-subagent\"}".to_string()),
    )
    .await;
    let resolver_malformed = recon_resolver(thread_service_malformed);
    let result_malformed = resolver_malformed
        .reconstruct_edge(&child_record, parent_run_id, &event)
        .await
        .unwrap();
    assert!(
        result_malformed.is_none(),
        "malformed metadata must return None"
    );
}

// (T4) ANTI-TAMPER PIN: metadata's cached `parent_run_context.scope`
// disagrees with the trusted anchor (different tenant) -> the resulting
// edge uses the anchor's scope/actor, never metadata's. Mutation: trust
// `metadata.parent_run_context` wholesale (skip the anchor override) ->
// RED (the tenant/thread_id assertions below fail against the tampered
// values).
#[tokio::test]
async fn reconstruct_edge_anti_tamper_pin_overrides_metadata_scope_with_trusted_anchor() {
    let tenant_id = ironclaw_host_api::ids::TenantId::new("recon-tenant-t4").unwrap();
    let agent_id = ironclaw_host_api::ids::AgentId::new("recon-agent-t4").unwrap();
    let child_thread_id = ironclaw_host_api::ids::ThreadId::new("recon-child-thread-t4").unwrap();
    let parent_thread_id = ironclaw_host_api::ids::ThreadId::new("recon-parent-thread-t4").unwrap();
    let owner_user_id = UserId::new("recon-owner-t4").unwrap();
    let parent_run_id = TurnRunId::new();
    let child_run_id = TurnRunId::new();

    let mut tampered_context = ironclaw_agent_loop::test_support::test_run_context("recon-t4");
    // Attacker-controlled thread metadata claims a different
    // tenant/thread than the trusted child run record — this must never
    // win.
    let attacker_tenant = ironclaw_host_api::ids::TenantId::new("attacker-tenant-t4").unwrap();
    let attacker_thread = ironclaw_host_api::ids::ThreadId::new("attacker-thread-t4").unwrap();
    tampered_context.scope =
        TurnScope::new(attacker_tenant.clone(), None, None, attacker_thread.clone());

    let child_record = recon_child_record(
        &tenant_id,
        &agent_id,
        &child_thread_id,
        child_run_id,
        parent_run_id,
        tampered_context.resolved_run_profile.clone(),
    );
    let event = recon_event(
        child_run_id,
        child_record.scope.clone(),
        owner_user_id.clone(),
    );
    let metadata = ironclaw_loop_host::SubagentThreadMetadata {
        kind: ironclaw_loop_host::SubagentThreadKind::Subagent,
        parent_run_id,
        parent_thread_id: parent_thread_id.clone(),
        tree_root_run_id: parent_run_id,
        child_run_id,
        subagent_kind: ironclaw_loop_host::SubagentKindId::new("general").unwrap(),
        mode: ironclaw_loop_host::SpawnSubagentMode::Blocking,
        result_ref: ironclaw_host_api::turn::LoopResultRef::new("result:subagent.recon-t4")
            .unwrap(),
        spawn_provider_call_id: None,
        handoff: None,
        parent_run_context: tampered_context,
        gate_ref: TurnGateRef::new("gate:subagent-t4").unwrap(),
    };

    let thread_service = Arc::new(ironclaw_threads::InMemorySessionThreadService::default());
    recon_seed_thread(
        &thread_service,
        &tenant_id,
        &agent_id,
        &child_thread_id,
        &owner_user_id,
        Some(serde_json::to_string(&metadata).unwrap()),
    )
    .await;
    let resolver = recon_resolver(thread_service);

    let edge = resolver
        .reconstruct_edge(&child_record, parent_run_id, &event)
        .await
        .unwrap()
        .expect("tampered-but-parseable metadata should still reconstruct");

    // The anchor (built from the trusted child record + recovered
    // owner) must win — never the attacker-controlled tenant/thread.
    assert_eq!(edge.parent_run_context.scope.tenant_id, tenant_id);
    assert_ne!(edge.parent_run_context.scope.tenant_id, attacker_tenant);
    assert_eq!(edge.parent_run_context.scope.thread_id, parent_thread_id);
    assert_ne!(edge.parent_run_context.scope.thread_id, attacker_thread);
    assert_eq!(edge.parent_run_context.thread_id, parent_thread_id);
    assert_eq!(
        edge.parent_run_context.actor,
        Some(TurnActor::new(owner_user_id))
    );
}

#[derive(Default)]
struct RecordingResumeCoordinator {
    resumes: std::sync::Mutex<Vec<ResumeTurnRequest>>,
    // Task 6 (2c): the recorded `ActivateThreadRequest`s this double saw,
    // and the scripted response `activate()` replays for every call —
    // every background-delivery activation test scripts exactly one
    // outcome, so a single slot (not a per-call queue) is enough.
    activations: std::sync::Mutex<Vec<ActivateThreadRequest>>,
    activation_result:
        std::sync::Mutex<Option<Result<ironclaw_turns::SubmitTurnResponse, TurnError>>>,
}

impl RecordingResumeCoordinator {
    fn resumes(&self) -> Vec<ResumeTurnRequest> {
        self.resumes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn activations(&self) -> Vec<ActivateThreadRequest> {
        self.activations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn with_activation_result(
        self,
        result: Result<ironclaw_turns::SubmitTurnResponse, TurnError>,
    ) -> Self {
        *self
            .activation_result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
        self
    }
}

#[derive(Default)]
struct RecordingUpdateWriter {
    updates: std::sync::Mutex<Vec<serde_json::Value>>,
}

impl RecordingUpdateWriter {
    fn updates(&self) -> Vec<serde_json::Value> {
        self.updates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait::async_trait]
impl ironclaw_loop_host::LoopCapabilityResultWriter for RecordingUpdateWriter {
    async fn write_capability_result(
        &self,
        _write: ironclaw_loop_host::CapabilityResultWrite<'_>,
    ) -> Result<ironclaw_loop_host::CapabilityWriteResult, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            ironclaw_loop_contracts::AgentLoopHostErrorKind::InvalidInvocation,
            "write is not used by await-edge update test",
        ))
    }

    async fn update_capability_result(
        &self,
        _run_context: &LoopRunContext,
        _result_ref: &ironclaw_host_api::turn::LoopResultRef,
        output: serde_json::Value,
    ) -> Result<u64, AgentLoopHostError> {
        let byte_len = serde_json::to_vec(&output)
            .map_err(|error| {
                AgentLoopHostError::new(
                    ironclaw_loop_contracts::AgentLoopHostErrorKind::Unavailable,
                    error.to_string(),
                )
            })?
            .len() as u64;
        self.updates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(output);
        Ok(byte_len)
    }
}

#[async_trait::async_trait]
impl TurnCoordinator for RecordingResumeCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        _request: ironclaw_turns::SubmitTurnRequest,
    ) -> Result<ironclaw_turns::SubmitTurnResponse, TurnError> {
        Err(TurnError::InvalidRequest {
            reason: "submit is not used by await-edge drain test".to_string(),
        })
    }

    async fn activate(
        &self,
        request: ActivateThreadRequest,
    ) -> Result<ironclaw_turns::SubmitTurnResponse, TurnError> {
        self.activations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        self.activation_result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .expect("activation result must be scripted before activate() is called")
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ironclaw_turns::ResumeTurnResponse, TurnError> {
        let run_id = request.run_id;
        self.resumes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        Ok(ironclaw_turns::ResumeTurnResponse {
            run_id,
            status: TurnStatus::Queued,
            event_cursor: ironclaw_host_api::turn::EventCursor(9),
        })
    }

    async fn retry_turn(
        &self,
        request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        Err(TurnError::RunNotRetryable {
            run_id: request.run_id,
        })
    }

    async fn cancel_run(
        &self,
        _request: ironclaw_turns::CancelRunRequest,
    ) -> Result<ironclaw_turns::CancelRunResponse, TurnError> {
        Err(TurnError::InvalidRequest {
            reason: "cancel is not used by await-edge drain test".to_string(),
        })
    }

    async fn get_run_state(
        &self,
        _request: GetRunStateRequest,
    ) -> Result<ironclaw_turns::TurnRunState, TurnError> {
        Err(TurnError::ScopeNotFound)
    }
}

#[tokio::test]
async fn mixed_status_group_updates_each_result_resumes_once_and_consumes_every_edge() {
    use chrono::Utc;
    use ironclaw_host_api::ids::{ProcessId, ProviderToolName};
    use ironclaw_loop_host::{AwaitedChildSetRecord, SpawnSubagentMode, SubagentKindId};
    use ironclaw_processes::{
        ProcessDependencyPort, ProcessDependencySubmission, ProcessJournalStore, ProcessKind,
        ProcessOperationId, ProcessSubmissionPort, SubmitProcessRequest,
    };
    use ironclaw_threads::{
        AppendFinalizedAssistantMessageRequest, AppendToolResultReferenceRequest,
        EnsureThreadRequest, MessageContent, ProviderToolCallReferenceEnvelope,
        SessionThreadService, ThreadHistoryRequest, ThreadScope, ToolResultReferenceEnvelope,
        ToolResultSafeSummary,
    };

    let process_store = Arc::new(ProcessJournalStore::new(recon_scoped_fs()));
    let dependencies = Arc::clone(&process_store)
        as Arc<dyn ProcessDependencyPort<Error = ironclaw_processes::ProcessJournalStoreError>>;
    let edge_store = Arc::new(AwaitEdgeStore::new(dependencies));
    let thread_service = Arc::new(ironclaw_threads::InMemorySessionThreadService::default());
    let runtime: Arc<dyn AgentTurnSpawnTreeRuntimePort> =
        Arc::new(ironclaw_turns::test_support::in_memory_agent_turn_runtime());
    let recording_writer = Arc::new(RecordingUpdateWriter::default());
    let writer: Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter> =
        Arc::clone(&recording_writer) as Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter>;
    let resolver = Arc::new(AwaitEdgeResolver::new_unbound(
        Arc::clone(&edge_store),
        runtime,
        writer,
        Arc::clone(&thread_service),
    ));
    let coordinator = Arc::new(RecordingResumeCoordinator::default());
    resolver
        .bind_coordinator(Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>)
        .expect("bind coordinator");

    let tenant_id = ironclaw_host_api::ids::TenantId::new("drain-tenant").expect("tenant");
    let user_id = UserId::new("drain-user").expect("user");
    let agent_id = ironclaw_host_api::ids::AgentId::new("drain-agent").expect("agent");
    let parent_thread_id =
        ironclaw_host_api::ids::ThreadId::new("drain-parent-thread").expect("parent thread");
    let parent_scope = TurnScope::new_with_owner(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        parent_thread_id.clone(),
        Some(user_id.clone()),
    );
    let parent_run_id = TurnRunId::new();
    let parent_process_id = ProcessId::from_uuid(parent_run_id.as_uuid());
    process_store
        .submit_process(SubmitProcessRequest {
            process_id: parent_process_id,
            process_kind: ProcessKind::AgentTurn,
            scope: parent_scope.to_resource_scope(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit parent process");

    let parent_thread_scope = ThreadScope {
        tenant_id: tenant_id.clone(),
        agent_id: agent_id.clone(),
        project_id: None,
        owner_user_id: Some(user_id.clone()),
        mission_id: None,
    };
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: parent_thread_scope.clone(),
            thread_id: Some(parent_thread_id.clone()),
            created_by_actor_id: user_id.to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("ensure parent thread");

    let mut parent_context = ironclaw_agent_loop::test_support::test_run_context("drain-parent");
    parent_context.scope = parent_scope.clone();
    parent_context.thread_id = parent_thread_id.clone();
    parent_context.run_id = parent_run_id;
    parent_context.actor = Some(TurnActor::new(user_id.clone()));
    let gate_ref = TurnGateRef::new("gate:mixed-status-group").expect("gate");
    let child_cases = [
        (
            "completed",
            EdgeTerminalKind::Completed,
            None,
            "completed child output",
        ),
        (
            "failed",
            EdgeTerminalKind::Failed,
            Some("sanitized child failure".to_string()),
            "failed child output",
        ),
    ];
    let mut children = Vec::new();

    for (label, terminal_kind, terminal_reason, final_text) in child_cases {
        let child_run_id = TurnRunId::new();
        let child_thread_id = ironclaw_host_api::ids::ThreadId::new(format!("drain-child-{label}"))
            .expect("child thread");
        let child_scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            Some(agent_id.clone()),
            None,
            child_thread_id.clone(),
            Some(user_id.clone()),
        );
        let result_ref =
            ironclaw_host_api::turn::LoopResultRef::new(format!("result:drain-{label}"))
                .expect("result ref");
        let spawn_provider_call_id = format!("spawn-call-{label}");
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: parent_thread_scope.clone(),
                thread_id: Some(child_thread_id.clone()),
                created_by_actor_id: user_id.to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("ensure child thread");
        thread_service
            .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
                scope: parent_thread_scope.clone(),
                thread_id: child_thread_id.clone(),
                turn_run_id: child_run_id.to_string(),
                content: MessageContent::text(final_text),
            })
            .await
            .expect("append child final output");
        thread_service
            .append_tool_result_reference(AppendToolResultReferenceRequest {
                intrinsic_outcome: None,
                scope: parent_thread_scope.clone(),
                thread_id: parent_thread_id.clone(),
                turn_run_id: parent_run_id.to_string(),
                result_ref: result_ref.as_str().to_string(),
                safe_summary: ToolResultSafeSummary::new("subagent still running")
                    .expect("initial summary"),
                provider_call: Some(ProviderToolCallReferenceEnvelope {
                    provider_id: "test-provider".to_string(),
                    provider_model_id: "test-model".to_string(),
                    provider_turn_id: "test-turn".to_string(),
                    provider_call_id: spawn_provider_call_id.clone(),
                    provider_tool_name: ProviderToolName::new("spawn_subagent")
                        .expect("provider tool name"),
                    capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
                        .expect("capability"),
                    arguments: serde_json::json!({"task": label}),
                    response_reasoning: None,
                    reasoning: None,
                    signature: None,
                }),
                model_observation: None,
            })
            .await
            .expect("append parent result placeholder");

        let submitted = AwaitedChildSetRecord {
            gate_ref: gate_ref.clone(),
            parent_run_context: parent_context.clone(),
            tree_root_run_id: parent_run_id,
            child_scope: child_scope.clone(),
            child_run_id,
            child_thread_id: child_thread_id.clone(),
            subagent_kind: SubagentKindId::new("general").expect("kind"),
            spawn_capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
                .expect("capability"),
            spawn_provider_call_id: Some(spawn_provider_call_id),
            result_ref: result_ref.clone(),
            mode: SpawnSubagentMode::Blocking,
        };
        process_store
            .submit_process(SubmitProcessRequest {
                process_id: ProcessId::from_uuid(child_run_id.as_uuid()),
                process_kind: ProcessKind::AgentTurn,
                scope: child_scope.to_resource_scope(),
                exclusive_within_scope: false,
                operation_id: Some(ProcessOperationId::from_trusted(format!(
                    "drain-child-{label}"
                ))),
                owner_user_id: Some(user_id.clone()),
                concurrency_class: None,
                parent_process_id: Some(parent_process_id),
                root_process_id: Some(parent_process_id),
                spawn_tree_descendant_cap: Some(2),
                dependency: Some(ProcessDependencySubmission {
                    dependent_process_id: parent_process_id,
                    root_process_id: parent_process_id,
                    group_ref: Some(gate_ref.as_str().to_string()),
                    metadata: serde_json::to_value(submitted).expect("edge metadata"),
                }),
                checkpoint_ref: None,
                input: None,
                created_at: Utc::now(),
                metadata: serde_json::Value::Null,
            })
            .await
            .expect("submit child process");
        if terminal_kind == EdgeTerminalKind::Completed {
            edge_store
                .settle(
                    &child_scope,
                    parent_run_id,
                    child_run_id,
                    terminal_kind,
                    Some(17),
                    terminal_reason.clone(),
                )
                .await
                .expect("settle edge")
                .expect("edge exists");
        }
        children.push((child_scope, child_run_id, result_ref, terminal_kind));
    }

    let group = edge_store
        .list_group(&children[0].0, parent_run_id, &gate_ref)
        .await
        .expect("list settle group");
    assert_eq!(group.len(), 2);
    assert!(
        group
            .iter()
            .any(|(_, edge)| edge.state == AwaitEdgeState::Settled)
    );
    assert!(
        group
            .iter()
            .any(|(_, edge)| edge.state == AwaitEdgeState::Open)
    );
    assert_eq!(
        edge_store
            .list_unclosed_for_scope(&children[0].0)
            .await
            .expect("list unclosed edges")
            .len(),
        2
    );
    let open_edge = edge_store
        .peek(&children[1].0, parent_run_id, children[1].1)
        .await
        .expect("peek open edge")
        .expect("open edge exists");
    assert_eq!(open_edge.state, AwaitEdgeState::Open);
    edge_store
        .close(&children[1].0, parent_run_id, children[1].1)
        .await
        .expect("closing an open edge is a no-op");
    assert!(
        crate::loop_exit_applier::AwaitDependentRunEvidenceStore::has_awaited_child_gate(
            edge_store.as_ref(),
            &children[0].0,
            parent_run_id,
            &ironclaw_host_api::turn::LoopGateRef::new(gate_ref.as_str()).expect("loop gate ref"),
        )
        .await
        .expect("query blocking gate evidence")
    );
    let partial_recovery = crate::subagent::await_edge::boot_recovery::recover_scope(
        &resolver,
        edge_store.as_ref(),
        &children[0].0,
    )
    .await;
    assert_eq!(partial_recovery.failed, 0);
    assert_eq!(partial_recovery.resumed, 0);
    assert_eq!(partial_recovery.drained, 0);
    assert_eq!(
        edge_store
            .peek(&children[0].0, parent_run_id, children[0].1)
            .await
            .expect("peek recovery-settled edge")
            .expect("settled edge remains while sibling is open")
            .state,
        AwaitEdgeState::Settled
    );

    let failed = &children[1];
    let outcome = resolver
        .settle_and_maybe_drain(
            &failed.0,
            parent_run_id,
            failed.1,
            EdgeTerminalKind::Failed,
            &TurnLifecycleEvent {
                cursor: ironclaw_host_api::turn::EventCursor(8),
                scope: failed.0.clone(),
                occurred_at: Some(Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: failed.1,
                status: TurnStatus::Failed,
                kind: ironclaw_turns::TurnEventKind::Failed,
                blocked_gate: None,
                sanitized_reason: Some("sanitized child failure".to_string()),
                retryable: Some(false),
                detail: None,
            },
        )
        .await
        .expect("settle and drain group");
    assert_eq!(outcome, ResolveOutcome::Resumed);
    let updates = recording_writer.updates();
    assert_eq!(updates.len(), 1, "only the open child result is staged");
    assert!(
        updates[0].to_string().contains("\"failed\""),
        "staged terminal payload records the child's failed status: {updates:?}"
    );
    let resumes = coordinator.resumes();
    assert_eq!(resumes.len(), 1);
    assert_eq!(resumes[0].scope, parent_scope);
    assert_eq!(
        resumes[0].precondition,
        ResumeTurnPrecondition::BlockedDependentRunGate
    );
    assert_eq!(resumes[0].gate_resolution_ref, gate_ref);

    for (child_scope, child_run_id, _, _) in &children {
        assert!(
            edge_store
                .peek(child_scope, parent_run_id, *child_run_id)
                .await
                .expect("peek consumed edge")
                .is_none(),
            "every edge must be consumed after one group drain"
        );
        edge_store
            .close(child_scope, parent_run_id, *child_run_id)
            .await
            .expect("closing an already consumed edge is idempotent");
    }
    edge_store
        .abandon(&children[0].0, parent_run_id, children[0].1)
        .await
        .expect("abandon replay is idempotent");
    let recovery = crate::subagent::await_edge::boot_recovery::recover_scope(
        &resolver,
        edge_store.as_ref(),
        &children[0].0,
    )
    .await;
    assert_eq!(recovery.failed, 0);
    assert_eq!(recovery.resumed, 0);
    let recovery_driver = crate::subagent::await_edge::boot_recovery::ScopeRecoveryDriver::new(
        Arc::clone(&resolver),
        Arc::clone(&edge_store),
    );
    ironclaw_loop_host::AwaitEdgeWriter::check_scope_recovered(&recovery_driver, &children[0].0)
        .await
        .expect("scope recovery driver completes");
    ironclaw_loop_host::AwaitEdgeWriter::abandon_awaited_child(
        &recovery_driver,
        &children[0].0,
        parent_run_id,
        children[0].1,
    )
    .await
    .expect("scope recovery driver abandon replay");

    let parent_thread = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: parent_thread_scope,
            thread_id: parent_thread_id,
        })
        .await
        .expect("read parent thread");
    let summaries = parent_thread
        .messages
        .iter()
        .filter_map(|message| message.content.as_deref())
        .filter_map(|content| ToolResultReferenceEnvelope::from_json_str(content).ok())
        .map(|envelope| envelope.safe_summary.as_str().to_string())
        .collect::<Vec<_>>();
    assert_eq!(summaries.len(), 2);
    assert!(
        summaries
            .iter()
            .any(|summary| summary.contains("completed")),
        "completed child keeps its own terminal status: {summaries:?}"
    );
    assert!(
        summaries.iter().any(|summary| summary.contains("failed")),
        "failed child keeps its own terminal status: {summaries:?}"
    );
}

// ─── Task 5 (2b): `deliver_background` — append + live-run enqueue tail ──

#[derive(Debug, Clone, Copy)]
enum EnqueueRefusal {
    RunClosed,
    CapacityExhausted,
}

struct RecordingEnqueue {
    requests: std::sync::Mutex<Vec<EnqueueQueuedMessageRequest>>,
    refusal: Option<EnqueueRefusal>,
}

impl RecordingEnqueue {
    fn accepting() -> Self {
        Self {
            requests: std::sync::Mutex::new(Vec::new()),
            refusal: None,
        }
    }

    fn refusing(refusal: EnqueueRefusal) -> Self {
        Self {
            requests: std::sync::Mutex::new(Vec::new()),
            refusal: Some(refusal),
        }
    }

    fn requests(&self) -> Vec<EnqueueQueuedMessageRequest> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait::async_trait]
impl HostInputEnqueuePort for RecordingEnqueue {
    async fn enqueue_queued_message(
        &self,
        request: EnqueueQueuedMessageRequest,
    ) -> Result<ironclaw_loop_host::HostInputEnvelope, HostInputQueueError> {
        let input = request.input.clone();
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        match self.refusal {
            None => Ok(ironclaw_loop_host::HostInputEnvelope {
                input,
                cursor: ironclaw_loop_contracts::LoopInputCursorToken::origin(),
                ack_token: ironclaw_loop_contracts::LoopInputAckToken::new("input-ack:1")
                    .expect("ack token"),
            }),
            Some(EnqueueRefusal::RunClosed) => Err(HostInputQueueError::RunClosed),
            Some(EnqueueRefusal::CapacityExhausted) => Err(HostInputQueueError::CapacityExhausted),
        }
    }
}

/// Fails one scripted `transition_process_dependency` call per armed
/// target state, and/or one scripted `consume_process_dependency` call —
/// simulating a crash between a durable side effect (thread acceptance,
/// a successful enqueue) and the store CAS that would have recorded it.
/// Every other call passes straight through to `inner`.
struct ScriptedDependencyFailures {
    inner: Arc<
        dyn ironclaw_processes::ProcessDependencyPort<
                Error = ironclaw_processes::ProcessJournalStoreError,
            >,
    >,
    fail_transition_once: std::sync::Mutex<Vec<ironclaw_processes::ProcessDependencyState>>,
    fail_consume_once: std::sync::atomic::AtomicBool,
}

impl ScriptedDependencyFailures {
    fn new(
        inner: Arc<
            dyn ironclaw_processes::ProcessDependencyPort<
                    Error = ironclaw_processes::ProcessJournalStoreError,
                >,
        >,
    ) -> Self {
        Self {
            inner,
            fail_transition_once: std::sync::Mutex::new(Vec::new()),
            fail_consume_once: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn fail_transition_once_for(self, state: ironclaw_processes::ProcessDependencyState) -> Self {
        self.fail_transition_once
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(state);
        self
    }

    fn fail_consume_once(self) -> Self {
        self.fail_consume_once
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self
    }
}

#[async_trait::async_trait]
impl ironclaw_processes::ProcessDependencyPort for ScriptedDependencyFailures {
    type Error = ironclaw_processes::ProcessJournalStoreError;

    async fn open_process_dependency(
        &self,
        request: ironclaw_processes::OpenProcessDependencyRequest,
    ) -> Result<ironclaw_processes::ProcessDependencyRecord, Self::Error> {
        self.inner.open_process_dependency(request).await
    }

    async fn settle_process_dependency(
        &self,
        request: ironclaw_processes::SettleProcessDependencyRequest,
    ) -> Result<Option<ironclaw_processes::ProcessDependencyRecord>, Self::Error> {
        self.inner.settle_process_dependency(request).await
    }

    async fn consume_process_dependency(
        &self,
        request: ironclaw_processes::CloseProcessDependencyRequest,
    ) -> Result<Option<ironclaw_processes::ProcessDependencyRecord>, Self::Error> {
        if self
            .fail_consume_once
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(
                ironclaw_processes::ProcessJournalStoreError::InvalidRequest(
                    "scripted consume failure".to_string(),
                ),
            );
        }
        self.inner.consume_process_dependency(request).await
    }

    async fn abandon_process_dependency(
        &self,
        request: ironclaw_processes::CloseProcessDependencyRequest,
    ) -> Result<Option<ironclaw_processes::ProcessDependencyRecord>, Self::Error> {
        self.inner.abandon_process_dependency(request).await
    }

    async fn transition_process_dependency(
        &self,
        request: ironclaw_processes::TransitionProcessDependencyRequest,
    ) -> Result<Option<ironclaw_processes::ProcessDependencyRecord>, Self::Error> {
        let should_fail = {
            let mut armed = self
                .fail_transition_once
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(index) = armed.iter().position(|state| *state == request.next) {
                armed.remove(index);
                true
            } else {
                false
            }
        };
        if should_fail {
            return Err(
                ironclaw_processes::ProcessJournalStoreError::InvalidRequest(format!(
                    "scripted transition failure for {:?}",
                    request.next
                )),
            );
        }
        self.inner.transition_process_dependency(request).await
    }

    async fn query_process_dependencies(
        &self,
        request: ironclaw_processes::ProcessDependencyQuery,
    ) -> Result<Vec<ironclaw_processes::ProcessDependencyRecord>, Self::Error> {
        self.inner.query_process_dependencies(request).await
    }

    async fn unresolved_process_dependencies(
        &self,
    ) -> Result<Vec<ironclaw_processes::ProcessDependencyRecord>, Self::Error> {
        self.inner.unresolved_process_dependencies().await
    }
}

/// Stub `AgentTurnSpawnTreeRuntimePort`: `get_run_record` always answers
/// with the fixture's child record (`handle_child_terminal_inner`'s
/// lookup), and `recent_runs_for_thread` answers with a configured
/// live-parent window (`deliver_background`'s attend step) — no process
/// journal involved, unlike the child/parent turn records themselves,
/// which the fixture submits through the real journal so the await-edge
/// machinery has something real to settle/append/attend/close.
struct StubBackgroundRuntime {
    child_record: TurnRunRecord,
    recent_runs: Vec<TurnRunRecord>,
}

#[async_trait::async_trait]
impl ironclaw_turns::AgentTurnRuntimePort for StubBackgroundRuntime {
    async fn submit_turn(
        &self,
        _request: ironclaw_turns::SubmitTurnRequest,
        _admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
        _run_profile_resolver: &dyn ironclaw_loop_contracts::RunProfileResolver,
    ) -> Result<ironclaw_turns::SubmitTurnResponse, TurnError> {
        unreachable!("background delivery tests do not submit turns")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ironclaw_turns::ResumeTurnResponse, TurnError> {
        unreachable!("background delivery tests do not resume turns")
    }

    async fn retry_turn(
        &self,
        request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        Err(TurnError::RunNotRetryable {
            run_id: request.run_id,
        })
    }

    async fn request_cancel(
        &self,
        _request: ironclaw_turns::CancelRunRequest,
    ) -> Result<ironclaw_turns::CancelRunResponse, TurnError> {
        unreachable!("background delivery tests do not cancel")
    }

    async fn get_run_state(
        &self,
        _request: GetRunStateRequest,
    ) -> Result<ironclaw_turns::TurnRunState, TurnError> {
        unreachable!("background delivery tests do not get run state")
    }

    async fn recent_runs_for_thread(
        &self,
        _scope: &TurnScope,
        _limit: u32,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        Ok(self.recent_runs.clone())
    }
}

#[async_trait::async_trait]
impl AgentTurnSpawnTreeRuntimePort for StubBackgroundRuntime {
    async fn submit_child_turn(
        &self,
        _request: ironclaw_turns::SubmitChildRunRequest,
        _admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
        _run_profile_resolver: &dyn ironclaw_loop_contracts::RunProfileResolver,
    ) -> Result<ironclaw_turns::SubmitTurnResponse, TurnError> {
        unreachable!("background delivery tests do not submit child turns")
    }

    async fn children_of(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        Ok(Vec::new())
    }

    async fn get_run_record(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError> {
        Ok(Some(self.child_record.clone()))
    }

    async fn reserve_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        _cap: u32,
    ) -> Result<ironclaw_turns::SpawnTreeReservation, TurnError> {
        Ok(ironclaw_turns::SpawnTreeReservation {
            scope: scope.clone(),
            root_run_id,
            descendant_count: u64::from(delta),
            released_children: std::collections::BTreeSet::new(),
        })
    }

    async fn release_tree_descendants(
        &self,
        _scope: &TurnScope,
        _root_run_id: TurnRunId,
        _delta: u32,
        _idempotency_key: TurnRunId,
    ) -> Result<(), TurnError> {
        Ok(())
    }

    async fn prune_released_child(
        &self,
        _scope: &TurnScope,
        _root_run_id: TurnRunId,
        _child_run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        Ok(())
    }
}

struct BgFixture {
    resolver: Arc<AwaitEdgeResolver<ironclaw_threads::InMemorySessionThreadService>>,
    edge_store: Arc<AwaitEdgeStore>,
    thread_service: Arc<ironclaw_threads::InMemorySessionThreadService>,
    tenant_id: ironclaw_host_api::ids::TenantId,
    agent_id: ironclaw_host_api::ids::AgentId,
    owner_user_id: UserId,
    parent_thread_id: ironclaw_host_api::ids::ThreadId,
    child_scope: TurnScope,
    parent_run_id: TurnRunId,
    child_run_id: TurnRunId,
    event: TurnLifecycleEvent,
}

/// Builds one background-mode await-edge (`Open`, real process journal)
/// plus a resolver wired to it: `dependencies` is the (optionally
/// scripted) `ProcessDependencyPort` the edge store's CAS writes go
/// through, `live_run` configures the stub runtime's
/// `recent_runs_for_thread` answer for the parent's thread — `Some` for a
/// live, non-terminal parent run, `None` for no live run at all — and
/// `coordinator` is bound the same way production always binds one
/// (`ironclaw_turn_runner::runtime.rs`), so the Task 6 (2c) parked-parent
/// activation branch has a real callee regardless of which fixture uses
/// it. `profile_id`, when set, overrides the parent's resolved run
/// profile id (default is `RunProfileId::default_profile()`) so
/// activation-shape tests can assert the id an activation request
/// carries is the parent's own, not a hardcoded default.
async fn bg_fixture(
    process_store: Arc<
        ironclaw_processes::ProcessJournalStore<ironclaw_filesystem::InMemoryBackend>,
    >,
    dependencies: Arc<
        dyn ironclaw_processes::ProcessDependencyPort<
                Error = ironclaw_processes::ProcessJournalStoreError,
            >,
    >,
    enqueue: Arc<RecordingEnqueue>,
    live_run: Option<(TurnRunId, ironclaw_host_api::turn::TurnId)>,
    coordinator: Arc<dyn TurnCoordinator>,
    profile_id: Option<ironclaw_host_api::turn::RunProfileId>,
) -> BgFixture {
    use ironclaw_host_api::ids::{AgentId, ProcessId, TenantId, ThreadId};
    use ironclaw_processes::{
        ProcessDependencySubmission, ProcessKind, ProcessOperationId, ProcessSubmissionPort,
        SubmitProcessRequest,
    };
    use ironclaw_threads::{
        AppendFinalizedAssistantMessageRequest, EnsureThreadRequest, MessageContent,
    };

    let tenant_id = TenantId::new("bg-tenant").expect("tenant");
    let agent_id = AgentId::new("bg-agent").expect("agent");
    let owner_user_id = UserId::new("bg-owner").expect("owner");
    let parent_thread_id = ThreadId::new("bg-parent-thread").expect("parent thread");
    let child_thread_id = ThreadId::new("bg-child-thread").expect("child thread");

    let parent_scope = TurnScope::new_with_owner(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        parent_thread_id.clone(),
        Some(owner_user_id.clone()),
    );
    let child_scope = TurnScope::new_with_owner(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        child_thread_id.clone(),
        Some(owner_user_id.clone()),
    );
    let parent_run_id = TurnRunId::new();
    let child_run_id = TurnRunId::new();
    let parent_process_id = ProcessId::from_uuid(parent_run_id.as_uuid());

    process_store
        .submit_process(SubmitProcessRequest {
            process_id: parent_process_id,
            process_kind: ProcessKind::AgentTurn,
            scope: parent_scope.to_resource_scope(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(owner_user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit parent process");

    let thread_service = Arc::new(ironclaw_threads::InMemorySessionThreadService::default());
    let thread_scope = ThreadScope {
        tenant_id: tenant_id.clone(),
        agent_id: agent_id.clone(),
        project_id: None,
        owner_user_id: Some(owner_user_id.clone()),
        mission_id: None,
    };
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(parent_thread_id.clone()),
            created_by_actor_id: owner_user_id.to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("ensure parent thread");
    thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: Some(child_thread_id.clone()),
            created_by_actor_id: owner_user_id.to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("ensure child thread");
    thread_service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: thread_scope.clone(),
            thread_id: child_thread_id.clone(),
            turn_run_id: child_run_id.to_string(),
            content: MessageContent::text("child background output"),
        })
        .await
        .expect("append child final output");

    let mut parent_context = ironclaw_agent_loop::test_support::test_run_context("bg-parent");
    parent_context.scope = parent_scope.clone();
    parent_context.thread_id = parent_thread_id.clone();
    parent_context.run_id = parent_run_id;
    parent_context.actor = Some(TurnActor::new(owner_user_id.clone()));
    if let Some(profile_id) = profile_id {
        parent_context.resolved_run_profile.profile_id = profile_id;
    }

    let gate_ref = TurnGateRef::new(format!("gate:subagent-bg-{child_run_id}")).expect("gate ref");
    let result_ref =
        ironclaw_host_api::turn::LoopResultRef::new("result:bg-subagent").expect("result ref");
    let submitted = ironclaw_loop_host::AwaitedChildSetRecord {
        gate_ref: gate_ref.clone(),
        parent_run_context: parent_context.clone(),
        tree_root_run_id: parent_run_id,
        child_scope: child_scope.clone(),
        child_run_id,
        child_thread_id: child_thread_id.clone(),
        subagent_kind: ironclaw_loop_host::SubagentKindId::new("general").expect("kind"),
        spawn_capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
            .expect("capability"),
        spawn_provider_call_id: Some("spawn-call-bg".to_string()),
        result_ref: result_ref.clone(),
        mode: SpawnSubagentMode::Background,
    };
    process_store
        .submit_process(SubmitProcessRequest {
            process_id: ProcessId::from_uuid(child_run_id.as_uuid()),
            process_kind: ProcessKind::AgentTurn,
            scope: child_scope.to_resource_scope(),
            exclusive_within_scope: false,
            operation_id: Some(ProcessOperationId::from_trusted("bg-child".to_string())),
            owner_user_id: Some(owner_user_id.clone()),
            concurrency_class: None,
            parent_process_id: Some(parent_process_id),
            root_process_id: Some(parent_process_id),
            spawn_tree_descendant_cap: Some(2),
            dependency: Some(ProcessDependencySubmission {
                dependent_process_id: parent_process_id,
                root_process_id: parent_process_id,
                // The exact tag `finish_spawn` writes for
                // `SpawnSubagentMode::Background` (`subagent_spawn_port.rs`),
                // not the per-child `gate_ref` — the run-start sweep queries
                // by this thread-scoped group, not by gate.
                group_ref: Some(format!("bg:{parent_thread_id}")),
                metadata: serde_json::to_value(submitted).expect("edge metadata"),
            }),
            checkpoint_ref: None,
            input: None,
            created_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit child process");

    let edge_store = Arc::new(AwaitEdgeStore::new(dependencies));

    let child_record = TurnRunRecord {
        subagent_activation_provenance: None,
        run_id: child_run_id,
        turn_id: ironclaw_host_api::turn::TurnId::new(),
        scope: child_scope.clone(),
        accepted_message_ref: ironclaw_host_api::turn::AcceptedMessageRef::new("msg:bg-child")
            .expect("accepted message ref"),
        status: TurnStatus::Completed,
        profile: ironclaw_turns::TurnRunProfile::from_resolved(
            parent_context.resolved_run_profile.clone(),
        ),
        output_contract: Default::default(),
        resolved_model_route: None,
        model_usage: None,
        execution_outcome: None,
        checkpoint_id: None,
        gate_ref: None,
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: ironclaw_host_api::turn::EventCursor(1),
        runner_id: None,
        lease_token: None,
        lease_expires_at: None,
        last_heartbeat_at: None,
        claim_count: 0,
        received_at: chrono::Utc::now(),
        parent_run_id: Some(parent_run_id),
        subagent_depth: 1,
        spawn_tree_root_run_id: Some(parent_run_id),
        product_context: None,
        resume_disposition: None,
    };

    let recent_runs = match live_run {
        Some((live_run_id, live_turn_id)) => vec![TurnRunRecord {
            subagent_activation_provenance: None,
            run_id: live_run_id,
            turn_id: live_turn_id,
            scope: parent_scope.clone(),
            accepted_message_ref: ironclaw_host_api::turn::AcceptedMessageRef::new("msg:bg-live")
                .expect("accepted message ref"),
            status: TurnStatus::Running,
            profile: ironclaw_turns::TurnRunProfile::from_resolved(
                parent_context.resolved_run_profile.clone(),
            ),
            output_contract: Default::default(),
            resolved_model_route: None,
            model_usage: None,
            execution_outcome: None,
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: ironclaw_host_api::turn::EventCursor(1),
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 0,
            received_at: chrono::Utc::now(),
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
            resume_disposition: None,
        }],
        None => Vec::new(),
    };
    let runtime = Arc::new(StubBackgroundRuntime {
        child_record,
        recent_runs,
    }) as Arc<dyn AgentTurnSpawnTreeRuntimePort>;

    let result_writer: Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter> =
        Arc::new(RecordingUpdateWriter::default());

    let resolver = Arc::new(AwaitEdgeResolver::new_unbound(
        Arc::clone(&edge_store),
        runtime,
        result_writer,
        Arc::clone(&thread_service),
    ));
    resolver
        .bind_input_enqueue(Arc::clone(&enqueue) as Arc<dyn HostInputEnqueuePort>)
        .expect("bind input enqueue");
    resolver
        .bind_coordinator(coordinator)
        .expect("bind coordinator");

    let event = TurnLifecycleEvent {
        cursor: ironclaw_host_api::turn::EventCursor(2),
        scope: child_scope.clone(),
        occurred_at: Some(chrono::Utc::now()),
        owner_user_id: Some(owner_user_id.clone()),
        run_id: child_run_id,
        status: TurnStatus::Completed,
        kind: ironclaw_turns::TurnEventKind::Completed,
        blocked_gate: None,
        sanitized_reason: None,
        retryable: None,
        detail: None,
    };

    BgFixture {
        resolver,
        edge_store,
        thread_service,
        tenant_id,
        agent_id,
        owner_user_id,
        parent_thread_id,
        child_scope,
        parent_run_id,
        child_run_id,
        event,
    }
}

async fn single_system_message(fixture: &BgFixture) -> ironclaw_threads::ThreadMessageRecord {
    let history = fixture
        .thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: fixture.tenant_id.clone(),
                agent_id: fixture.agent_id.clone(),
                project_id: None,
                owner_user_id: Some(fixture.owner_user_id.clone()),
                mission_id: None,
            },
            thread_id: fixture.parent_thread_id.clone(),
        })
        .await
        .expect("read parent thread");
    let mut system_messages: Vec<_> = history
        .messages
        .into_iter()
        .filter(|message| message.kind == MessageKind::System)
        .collect();
    assert_eq!(
        system_messages.len(),
        1,
        "exactly one background-result row must land on the parent thread"
    );
    system_messages.remove(0)
}

async fn assert_accept_subagent_result_replays(
    fixture: &BgFixture,
    expected_message_id: ThreadMessageId,
) {
    let replay = fixture
        .thread_service
        .accept_subagent_result(AcceptSubagentResultRequest {
            scope: ThreadScope {
                tenant_id: fixture.tenant_id.clone(),
                agent_id: fixture.agent_id.clone(),
                project_id: None,
                owner_user_id: Some(fixture.owner_user_id.clone()),
                mission_id: None,
            },
            thread_id: fixture.parent_thread_id.clone(),
            source_binding_id: format!("subagent-result:{}", fixture.parent_run_id),
            external_event_id: fixture.child_run_id.to_string(),
            content: FramedSubagentText::frame(
                "replay probe — content is irrelevant, identity is what dedupes",
            ),
        })
        .await
        .expect("replay accept_subagent_result");
    assert!(
        replay.idempotent_replay,
        "a second acceptance on the same (scope, source_binding_id, external_event_id) \
         must replay the existing row, proving the identity the resolver used"
    );
    assert_eq!(replay.message_id, expected_message_id);
}

#[tokio::test]
async fn background_delivery_appends_and_enqueues_for_live_parent() {
    let process_store = Arc::new(ironclaw_processes::ProcessJournalStore::new(
        recon_scoped_fs(),
    ));
    let dependencies = Arc::clone(&process_store)
        as Arc<
            dyn ironclaw_processes::ProcessDependencyPort<
                    Error = ironclaw_processes::ProcessJournalStoreError,
                >,
        >;
    let enqueue = Arc::new(RecordingEnqueue::accepting());
    let live_run_id = TurnRunId::new();
    let live_turn_id = ironclaw_host_api::turn::TurnId::new();
    let fixture = bg_fixture(
        process_store,
        dependencies,
        Arc::clone(&enqueue),
        Some((live_run_id, live_turn_id)),
        Arc::new(RecordingResumeCoordinator::default()) as Arc<dyn TurnCoordinator>,
        None,
    )
    .await;

    let outcome = fixture
        .resolver
        .handle_child_terminal(&fixture.event)
        .await
        .expect("background delivery to a live parent succeeds");
    assert_eq!(outcome, ResolveOutcome::Drained);

    let edge = fixture
        .edge_store
        .peek(
            &fixture.child_scope,
            fixture.parent_run_id,
            fixture.child_run_id,
        )
        .await
        .expect("peek edge")
        .expect("the queue ack effect keeps the edge recoverable");
    assert_eq!(edge.state, AwaitEdgeState::ResultAppended);
    assert!(edge.attention_outcome.is_none());

    let row = single_system_message(&fixture).await;
    assert_eq!(row.status, MessageStatus::Finalized);
    let content = row.content.as_deref().expect("row has content");
    let expected_framed = FramedSubagentText::frame("child background output");
    assert_eq!(content, expected_framed.as_str());

    assert_accept_subagent_result_replays(&fixture, row.message_id).await;

    let requests = enqueue.requests();
    assert_eq!(
        requests.len(),
        1,
        "no resume_turn path in background mode: exactly one enqueue"
    );
    let request = &requests[0];
    assert_eq!(request.run_id, live_run_id);
    assert_eq!(request.turn_id, live_turn_id);
    assert_eq!(request.message_id, row.message_id);
    let expected_ref = LoopMessageRef::new(format!("msg:{}", row.message_id)).expect("message ref");
    assert_eq!(
        request.input,
        LoopInput::SubagentSettled {
            child_run_id: fixture.child_run_id,
            message_ref: expected_ref,
        }
    );
    assert!(
        request.ack_effect.is_some(),
        "live background enqueue must carry the deferred edge acknowledgment"
    );
}

#[tokio::test]
async fn background_ack_effect_closes_edge_exactly_once() {
    let process_store = Arc::new(ironclaw_processes::ProcessJournalStore::new(
        recon_scoped_fs(),
    ));
    let dependencies = Arc::clone(&process_store)
        as Arc<
            dyn ironclaw_processes::ProcessDependencyPort<
                    Error = ironclaw_processes::ProcessJournalStoreError,
                >,
        >;
    let enqueue = Arc::new(RecordingEnqueue::accepting());
    let fixture = bg_fixture(
        process_store,
        dependencies,
        Arc::clone(&enqueue),
        Some((TurnRunId::new(), ironclaw_host_api::turn::TurnId::new())),
        Arc::new(RecordingResumeCoordinator::default()) as Arc<dyn TurnCoordinator>,
        None,
    )
    .await;
    fixture
        .resolver
        .handle_child_terminal(&fixture.event)
        .await
        .expect("enqueue leaves the edge awaiting durable input acknowledgment");
    let effect = LoopInputAckEffect {
        child_scope: fixture.child_scope.clone(),
        parent_run_id: fixture.parent_run_id,
        child_run_id: fixture.child_run_id,
    };

    <AwaitEdgeResolver<_> as HostInputAckEffectHandler>::handle_ack_effect(
        &fixture.resolver,
        effect.clone(),
    )
    .await
    .expect("first callback records attention and closes");
    assert!(
        fixture
            .edge_store
            .peek(
                &fixture.child_scope,
                fixture.parent_run_id,
                fixture.child_run_id,
            )
            .await
            .expect("peek after callback")
            .is_none()
    );

    <AwaitEdgeResolver<_> as HostInputAckEffectHandler>::handle_ack_effect(
        &fixture.resolver,
        effect,
    )
    .await
    .expect("duplicate callback is idempotent");
}

#[tokio::test]
async fn background_ack_effect_retries_close_after_attention_commit() {
    let process_store = Arc::new(ironclaw_processes::ProcessJournalStore::new(
        recon_scoped_fs(),
    ));
    let dependencies = Arc::new(
        ScriptedDependencyFailures::new(Arc::clone(&process_store)
            as Arc<
                dyn ironclaw_processes::ProcessDependencyPort<
                        Error = ironclaw_processes::ProcessJournalStoreError,
                    >,
            >)
        .fail_consume_once(),
    )
        as Arc<
            dyn ironclaw_processes::ProcessDependencyPort<
                    Error = ironclaw_processes::ProcessJournalStoreError,
                >,
        >;
    let enqueue = Arc::new(RecordingEnqueue::accepting());
    let fixture = bg_fixture(
        process_store,
        dependencies,
        Arc::clone(&enqueue),
        Some((TurnRunId::new(), ironclaw_host_api::turn::TurnId::new())),
        Arc::new(RecordingResumeCoordinator::default()) as Arc<dyn TurnCoordinator>,
        None,
    )
    .await;
    fixture
        .resolver
        .handle_child_terminal(&fixture.event)
        .await
        .expect("enqueue leaves the edge recoverable");
    let effect = LoopInputAckEffect {
        child_scope: fixture.child_scope.clone(),
        parent_run_id: fixture.parent_run_id,
        child_run_id: fixture.child_run_id,
    };

    let first = <AwaitEdgeResolver<_> as HostInputAckEffectHandler>::handle_ack_effect(
        &fixture.resolver,
        effect.clone(),
    )
    .await;
    assert!(
        first.is_err(),
        "the scripted close failure must be retained"
    );
    assert_eq!(
        fixture
            .edge_store
            .peek(
                &fixture.child_scope,
                fixture.parent_run_id,
                fixture.child_run_id,
            )
            .await
            .expect("peek after failed close")
            .expect("edge remains recoverable")
            .state,
        AwaitEdgeState::AttentionScheduled
    );

    <AwaitEdgeResolver<_> as HostInputAckEffectHandler>::handle_ack_effect(
        &fixture.resolver,
        effect,
    )
    .await
    .expect("retry closes the already-attended edge");
    assert!(
        fixture
            .edge_store
            .peek(
                &fixture.child_scope,
                fixture.parent_run_id,
                fixture.child_run_id,
            )
            .await
            .expect("peek after retry")
            .is_none()
    );
}

#[tokio::test]
async fn background_delivery_replays_idempotently_after_crash_before_result_appended() {
    let process_store = Arc::new(ironclaw_processes::ProcessJournalStore::new(
        recon_scoped_fs(),
    ));
    let dependencies = Arc::new(
        ScriptedDependencyFailures::new(Arc::clone(&process_store)
            as Arc<
                dyn ironclaw_processes::ProcessDependencyPort<
                        Error = ironclaw_processes::ProcessJournalStoreError,
                    >,
            >)
        .fail_transition_once_for(ironclaw_processes::ProcessDependencyState::ResultAppended),
    )
        as Arc<
            dyn ironclaw_processes::ProcessDependencyPort<
                    Error = ironclaw_processes::ProcessJournalStoreError,
                >,
        >;
    let enqueue = Arc::new(RecordingEnqueue::accepting());
    let live_run_id = TurnRunId::new();
    let live_turn_id = ironclaw_host_api::turn::TurnId::new();
    let fixture = bg_fixture(
        process_store,
        dependencies,
        Arc::clone(&enqueue),
        Some((live_run_id, live_turn_id)),
        Arc::new(RecordingResumeCoordinator::default()) as Arc<dyn TurnCoordinator>,
        None,
    )
    .await;

    let first = fixture.resolver.handle_child_terminal(&fixture.event).await;
    assert!(
        first.is_err(),
        "a crash between acceptance and record_result_appended must surface as an error"
    );

    let second = fixture
        .resolver
        .handle_child_terminal(&fixture.event)
        .await
        .expect("re-drive recovers once the store CAS is no longer scripted to fail");
    assert_eq!(second, ResolveOutcome::Drained);

    let row = single_system_message(&fixture).await;
    assert_accept_subagent_result_replays(&fixture, row.message_id).await;

    let requests = enqueue.requests();
    assert_eq!(
        requests.len(),
        1,
        "the append never reached attend on the failed first pass, so only the re-drive enqueues"
    );
    let expected_ref = LoopMessageRef::new(format!("msg:{}", row.message_id)).expect("message ref");
    assert_eq!(
        requests[0].input,
        LoopInput::SubagentSettled {
            child_run_id: fixture.child_run_id,
            message_ref: expected_ref,
        }
    );
}

#[tokio::test]
async fn background_delivery_replays_safely_before_queue_acknowledgment() {
    let process_store = Arc::new(ironclaw_processes::ProcessJournalStore::new(
        recon_scoped_fs(),
    ));
    let dependencies = Arc::new(
        ScriptedDependencyFailures::new(Arc::clone(&process_store)
            as Arc<
                dyn ironclaw_processes::ProcessDependencyPort<
                        Error = ironclaw_processes::ProcessJournalStoreError,
                    >,
            >)
        .fail_transition_once_for(ironclaw_processes::ProcessDependencyState::AttentionScheduled)
        .fail_consume_once(),
    )
        as Arc<
            dyn ironclaw_processes::ProcessDependencyPort<
                    Error = ironclaw_processes::ProcessJournalStoreError,
                >,
        >;
    let enqueue = Arc::new(RecordingEnqueue::accepting());
    let live_run_id = TurnRunId::new();
    let live_turn_id = ironclaw_host_api::turn::TurnId::new();
    let fixture = bg_fixture(
        process_store,
        dependencies,
        Arc::clone(&enqueue),
        Some((live_run_id, live_turn_id)),
        Arc::new(RecordingResumeCoordinator::default()) as Arc<dyn TurnCoordinator>,
        None,
    )
    .await;

    fixture
        .resolver
        .handle_child_terminal(&fixture.event)
        .await
        .expect("enqueue succeeds without closing the await edge");

    fixture
        .resolver
        .handle_child_terminal(&fixture.event)
        .await
        .expect("re-drive remains idempotent before queue acknowledgment");

    let edge = fixture
        .edge_store
        .peek(
            &fixture.child_scope,
            fixture.parent_run_id,
            fixture.child_run_id,
        )
        .await
        .expect("peek edge")
        .expect("edge is not yet closed");
    assert_eq!(edge.state, AwaitEdgeState::ResultAppended);
    assert_eq!(edge.attention_outcome, None);

    assert_eq!(
        enqueue.requests().len(),
        2,
        "the queue double saw a second enqueue attempt, but the durable queue's identity \
         dedupe makes replaying it safe"
    );

    single_system_message(&fixture).await;
}

async fn assert_background_delivery_parks_on_enqueue_refusal(refusal: EnqueueRefusal) {
    let process_store = Arc::new(ironclaw_processes::ProcessJournalStore::new(
        recon_scoped_fs(),
    ));
    let dependencies = Arc::clone(&process_store)
        as Arc<
            dyn ironclaw_processes::ProcessDependencyPort<
                    Error = ironclaw_processes::ProcessJournalStoreError,
                >,
        >;
    let enqueue = Arc::new(RecordingEnqueue::refusing(refusal));
    let live_run_id = TurnRunId::new();
    let live_turn_id = ironclaw_host_api::turn::TurnId::new();
    // The enqueue refusal (Task 5) now falls through into the Task 6
    // parked-parent activation branch; script the coordinator with
    // `ThreadBusy` so this test still exercises (and asserts) the
    // "stays parked, not closed" outcome the refusal itself is about,
    // rather than actually activating the parent.
    let coordinator = Arc::new(
        RecordingResumeCoordinator::default().with_activation_result(Err(TurnError::ThreadBusy(
            ironclaw_turns::ThreadBusy {
                active_run_id: live_run_id,
                status: TurnStatus::Running,
                event_cursor: ironclaw_host_api::turn::EventCursor(1),
            },
        ))),
    );
    let fixture = bg_fixture(
        process_store,
        dependencies,
        Arc::clone(&enqueue),
        Some((live_run_id, live_turn_id)),
        Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
        None,
    )
    .await;

    let outcome = fixture
        .resolver
        .handle_child_terminal(&fixture.event)
        .await
        .expect("a refused enqueue leaves the edge parked, not an error, in this slice");
    assert_eq!(outcome, ResolveOutcome::Drained);

    let edge = fixture
        .edge_store
        .peek(
            &fixture.child_scope,
            fixture.parent_run_id,
            fixture.child_run_id,
        )
        .await
        .expect("peek edge")
        .expect("edge stays parked, not closed");
    assert_eq!(edge.state, AwaitEdgeState::ResultAppended);
    assert!(edge.attention_outcome.is_none());
    assert_eq!(enqueue.requests().len(), 1);
    assert_eq!(
        coordinator.activations().len(),
        1,
        "the refused enqueue must still attempt activation once"
    );
}

#[tokio::test]
async fn background_delivery_parks_edge_on_run_closed_enqueue_refusal() {
    assert_background_delivery_parks_on_enqueue_refusal(EnqueueRefusal::RunClosed).await;
}

#[tokio::test]
async fn background_delivery_parks_edge_on_capacity_exhausted_enqueue_refusal() {
    assert_background_delivery_parks_on_enqueue_refusal(EnqueueRefusal::CapacityExhausted).await;
}

// ─── Task 6 (2c): parked-parent activation + streak-cap deferral ──────

fn activation_accepted(run_id: TurnRunId) -> Result<ironclaw_turns::SubmitTurnResponse, TurnError> {
    Ok(ironclaw_turns::SubmitTurnResponse::Accepted {
        turn_id: ironclaw_host_api::turn::TurnId::new(),
        run_id,
        status: TurnStatus::Queued,
        resolved_run_profile_id: ironclaw_host_api::turn::RunProfileId::default_profile(),
        resolved_run_profile_version: ironclaw_host_api::turn::RunProfileVersion::new(1),
        event_cursor: ironclaw_host_api::turn::EventCursor(1),
        accepted_message_ref: AcceptedMessageRef::new("accepted-activation-probe")
            .expect("accepted message ref"),
    })
}

#[tokio::test]
async fn background_delivery_activates_parked_parent_with_system_provenance_and_preserves_profile()
{
    let process_store = Arc::new(ironclaw_processes::ProcessJournalStore::new(
        recon_scoped_fs(),
    ));
    let dependencies = Arc::clone(&process_store)
        as Arc<
            dyn ironclaw_processes::ProcessDependencyPort<
                    Error = ironclaw_processes::ProcessJournalStoreError,
                >,
        >;
    let enqueue = Arc::new(RecordingEnqueue::accepting());
    let coordinator = Arc::new(
        RecordingResumeCoordinator::default()
            .with_activation_result(activation_accepted(TurnRunId::new())),
    );
    let fixture = bg_fixture(
        process_store,
        dependencies,
        Arc::clone(&enqueue),
        None,
        Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
        Some(ironclaw_host_api::turn::RunProfileId::long_running_mission()),
    )
    .await;

    let outcome = fixture
        .resolver
        .handle_child_terminal(&fixture.event)
        .await
        .expect("parked-parent activation succeeds");
    assert_eq!(outcome, ResolveOutcome::Drained);

    assert!(
        fixture
            .edge_store
            .peek(
                &fixture.child_scope,
                fixture.parent_run_id,
                fixture.child_run_id
            )
            .await
            .expect("peek edge")
            .is_none(),
        "an activated edge must be closed"
    );

    let row = single_system_message(&fixture).await;
    assert_accept_subagent_result_replays(&fixture, row.message_id).await;
    assert!(
        enqueue.requests().is_empty(),
        "no live parent: the parked branch must never enqueue"
    );

    let activations = coordinator.activations();
    assert_eq!(activations.len(), 1, "exactly one activate() call");
    let request = &activations[0];
    assert_eq!(request.provenance, ActivationProvenance::System);
    assert_eq!(
        request.accepted_message_ref,
        AcceptedMessageRef::new(format!("msg:{}", row.message_id)).expect("accepted ref")
    );
    assert_eq!(
        request.idempotency_key,
        IdempotencyKey::new(format!(
            "subagent-activate:{}:{}",
            fixture.parent_run_id, fixture.child_run_id
        ))
        .expect("idempotency key")
    );
    assert_eq!(request.requested_run_profile, None);
    assert_eq!(
        request
            .resolved_run_profile
            .as_ref()
            .expect("profile snapshot")
            .profile_id,
        ironclaw_host_api::turn::RunProfileId::long_running_mission()
    );
    let expected_scope = TurnScope::new_with_owner(
        fixture.tenant_id.clone(),
        Some(fixture.agent_id.clone()),
        None,
        fixture.parent_thread_id.clone(),
        Some(fixture.owner_user_id.clone()),
    );
    assert_eq!(request.scope, expected_scope);
    assert_eq!(request.actor, TurnActor::new(fixture.owner_user_id.clone()));
}

#[tokio::test]
async fn background_delivery_defers_streak_capped_parent_and_excludes_it_from_redrive() {
    let process_store = Arc::new(ironclaw_processes::ProcessJournalStore::new(
        recon_scoped_fs(),
    ));
    let dependencies = Arc::clone(&process_store)
        as Arc<
            dyn ironclaw_processes::ProcessDependencyPort<
                    Error = ironclaw_processes::ProcessJournalStoreError,
                >,
        >;
    let enqueue = Arc::new(RecordingEnqueue::accepting());
    let coordinator = Arc::new(
        RecordingResumeCoordinator::default().with_activation_result(Err(
            TurnError::AdmissionRejected(ironclaw_turns::AdmissionRejection::new(
                AdmissionRejectionReason::SystemWakeStreak,
            )),
        )),
    );
    let fixture = bg_fixture(
        process_store,
        dependencies,
        Arc::clone(&enqueue),
        None,
        Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
        None,
    )
    .await;

    let outcome = fixture
        .resolver
        .handle_child_terminal(&fixture.event)
        .await
        .expect("a streak-capped activation refusal is not a hard error");
    assert_eq!(outcome, ResolveOutcome::Drained);

    let edge = fixture
        .edge_store
        .peek(
            &fixture.child_scope,
            fixture.parent_run_id,
            fixture.child_run_id,
        )
        .await
        .expect("peek edge")
        .expect("a streak-deferred edge stays unclosed");
    assert_eq!(edge.state, AwaitEdgeState::AttentionDeferredStreakCap);

    assert!(
        fixture
            .edge_store
            .list_unclosed_for_scope(&fixture.child_scope)
            .await
            .expect("list unclosed")
            .into_iter()
            .any(|(parent, child, _)| parent == fixture.parent_run_id
                && child == fixture.child_run_id),
        "a streak-deferred edge must still be returned by list_unclosed_for_scope"
    );

    // Re-drive: the edge is no longer `Settled`, so `deliver_background`
    // must skip it rather than attempt a second activation.
    let redrive = fixture
        .resolver
        .handle_child_terminal(&fixture.event)
        .await
        .expect("re-driving a deferred edge is a no-op, not an error");
    assert_eq!(redrive, ResolveOutcome::Drained);
    assert_eq!(
        coordinator.activations().len(),
        1,
        "autonomous retry must not call activate() again on a streak-capped edge"
    );
}

#[tokio::test]
async fn background_delivery_leaves_parked_edge_on_thread_busy_activation_refusal() {
    let process_store = Arc::new(ironclaw_processes::ProcessJournalStore::new(
        recon_scoped_fs(),
    ));
    let dependencies = Arc::clone(&process_store)
        as Arc<
            dyn ironclaw_processes::ProcessDependencyPort<
                    Error = ironclaw_processes::ProcessJournalStoreError,
                >,
        >;
    let enqueue = Arc::new(RecordingEnqueue::accepting());
    let raced_run_id = TurnRunId::new();
    let coordinator = Arc::new(
        RecordingResumeCoordinator::default().with_activation_result(Err(TurnError::ThreadBusy(
            ironclaw_turns::ThreadBusy {
                active_run_id: raced_run_id,
                status: TurnStatus::Running,
                event_cursor: ironclaw_host_api::turn::EventCursor(1),
            },
        ))),
    );
    let fixture = bg_fixture(
        process_store,
        dependencies,
        Arc::clone(&enqueue),
        None,
        Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
        None,
    )
    .await;

    let outcome = fixture
        .resolver
        .handle_child_terminal(&fixture.event)
        .await
        .expect("a ThreadBusy activation refusal is not a hard error");
    assert_eq!(outcome, ResolveOutcome::Drained);

    let edge = fixture
        .edge_store
        .peek(
            &fixture.child_scope,
            fixture.parent_run_id,
            fixture.child_run_id,
        )
        .await
        .expect("peek edge")
        .expect("edge stays parked, not closed");
    assert_eq!(edge.state, AwaitEdgeState::ResultAppended);
    assert!(edge.attention_outcome.is_none());
    assert_eq!(coordinator.activations().len(), 1);
    assert!(enqueue.requests().is_empty());
}

#[tokio::test]
async fn background_delivery_leaves_parked_edge_on_transient_activation_error() {
    let process_store = Arc::new(ironclaw_processes::ProcessJournalStore::new(
        recon_scoped_fs(),
    ));
    let dependencies = Arc::clone(&process_store)
        as Arc<
            dyn ironclaw_processes::ProcessDependencyPort<
                    Error = ironclaw_processes::ProcessJournalStoreError,
                >,
        >;
    let enqueue = Arc::new(RecordingEnqueue::accepting());
    let coordinator = Arc::new(
        RecordingResumeCoordinator::default().with_activation_result(Err(TurnError::Unavailable {
            reason: "activation transiently unavailable".to_string(),
        })),
    );
    let fixture = bg_fixture(
        process_store,
        dependencies,
        Arc::clone(&enqueue),
        None,
        Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
        None,
    )
    .await;

    let outcome = fixture
        .resolver
        .handle_child_terminal(&fixture.event)
        .await
        .expect("a transient activation error is not a hard error");
    assert_eq!(outcome, ResolveOutcome::Drained);

    let edge = fixture
        .edge_store
        .peek(
            &fixture.child_scope,
            fixture.parent_run_id,
            fixture.child_run_id,
        )
        .await
        .expect("peek edge")
        .expect("edge stays parked, not closed");
    assert_eq!(edge.state, AwaitEdgeState::ResultAppended);
    assert!(edge.attention_outcome.is_none());
    assert_eq!(coordinator.activations().len(), 1);
    assert!(enqueue.requests().is_empty());
}

#[tokio::test]
async fn background_delivery_closes_without_second_activate_after_crash_before_close() {
    let process_store = Arc::new(ironclaw_processes::ProcessJournalStore::new(
        recon_scoped_fs(),
    ));
    let dependencies = Arc::new(
        ScriptedDependencyFailures::new(Arc::clone(&process_store)
            as Arc<
                dyn ironclaw_processes::ProcessDependencyPort<
                        Error = ironclaw_processes::ProcessJournalStoreError,
                    >,
            >)
        .fail_consume_once(),
    )
        as Arc<
            dyn ironclaw_processes::ProcessDependencyPort<
                    Error = ironclaw_processes::ProcessJournalStoreError,
                >,
        >;
    let enqueue = Arc::new(RecordingEnqueue::accepting());
    let coordinator = Arc::new(
        RecordingResumeCoordinator::default()
            .with_activation_result(activation_accepted(TurnRunId::new())),
    );
    let fixture = bg_fixture(
        process_store,
        dependencies,
        Arc::clone(&enqueue),
        None,
        Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
        None,
    )
    .await;

    let first = fixture.resolver.handle_child_terminal(&fixture.event).await;
    assert!(
        first.is_err(),
        "a crash between record_attention(Activated) and close must surface as an error"
    );

    let edge = fixture
        .edge_store
        .peek(
            &fixture.child_scope,
            fixture.parent_run_id,
            fixture.child_run_id,
        )
        .await
        .expect("peek edge")
        .expect("edge is not yet closed");
    assert_eq!(edge.state, AwaitEdgeState::AttentionScheduled);
    assert_eq!(
        edge.attention_outcome,
        Some(crate::subagent::await_edge::AttentionOutcome::Activated)
    );

    let second = fixture
        .resolver
        .handle_child_terminal(&fixture.event)
        .await
        .expect("re-drive recovers once the scripted close failure is spent");
    assert_eq!(second, ResolveOutcome::Drained);

    assert!(
        fixture
            .edge_store
            .peek(
                &fixture.child_scope,
                fixture.parent_run_id,
                fixture.child_run_id
            )
            .await
            .expect("peek edge")
            .is_none(),
        "the re-drive must close the edge"
    );

    assert_eq!(
        coordinator.activations().len(),
        1,
        "the re-drive must not call activate() a second time"
    );
}

// ─── Run-start sweep (§4.2, Task 7/2c) ─────────────────────────────────────

/// Minimal harness for `sweep_thread_on_run_start` tests: one parent
/// thread/scope/run, plus a resolver wired the same way production wires
/// one (`ironclaw_turn_runner::runtime.rs`). `open_background_edge` opens
/// one background-mode dependency edge directly against the real process
/// journal (`store/tests.rs`'s `settled_background_edge` pattern, not the
/// reactive `handle_child_terminal` path) so each test drives its edge(s)
/// to an exact target state before sweeping — the sweep must never re-derive
/// or repeat delivery steps a state already carries.
struct SweepFixture {
    resolver: Arc<AwaitEdgeResolver<ironclaw_threads::InMemorySessionThreadService>>,
    edge_store: Arc<AwaitEdgeStore>,
    process_store:
        Arc<ironclaw_processes::ProcessJournalStore<ironclaw_filesystem::InMemoryBackend>>,
    parent_scope: TurnScope,
    parent_run_id: TurnRunId,
    parent_context: LoopRunContext,
}

impl SweepFixture {
    /// Opens one `Open` background-mode dependency edge under this
    /// fixture's parent, returning `(child_run_id, child_scope)` — the pair
    /// every `AwaitEdgeStore` state-transition call needs.
    async fn open_background_edge(&self, suffix: &str) -> (TurnRunId, TurnScope) {
        use ironclaw_host_api::ids::{AgentId, ProcessId, TenantId, ThreadId};
        use ironclaw_processes::{
            ProcessDependencySubmission, ProcessKind, ProcessOperationId, ProcessSubmissionPort,
            SubmitProcessRequest,
        };

        let child_run_id = TurnRunId::new();
        let child_thread_id =
            ThreadId::new(format!("sweep-child-thread-{suffix}")).expect("child thread");
        let tenant_id = TenantId::new("sweep-tenant").expect("tenant");
        let agent_id = AgentId::new("sweep-agent").expect("agent");
        let owner_user_id = UserId::new("sweep-owner").expect("owner");
        let child_scope = TurnScope::new_with_owner(
            tenant_id,
            Some(agent_id),
            None,
            child_thread_id.clone(),
            Some(owner_user_id.clone()),
        );
        let parent_process_id = ProcessId::from_uuid(self.parent_run_id.as_uuid());
        let gate_ref =
            TurnGateRef::new(format!("gate:sweep-{suffix}")).expect("gate ref for sweep edge");
        let result_ref =
            ironclaw_host_api::turn::LoopResultRef::new(format!("result:sweep-{suffix}"))
                .expect("result ref");
        let submitted = ironclaw_loop_host::AwaitedChildSetRecord {
            gate_ref,
            parent_run_context: self.parent_context.clone(),
            tree_root_run_id: self.parent_run_id,
            child_scope: child_scope.clone(),
            child_run_id,
            child_thread_id: child_thread_id.clone(),
            subagent_kind: ironclaw_loop_host::SubagentKindId::new("general").expect("kind"),
            spawn_capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
                .expect("capability"),
            spawn_provider_call_id: Some(format!("spawn-call-sweep-{suffix}")),
            result_ref,
            mode: SpawnSubagentMode::Background,
        };
        self.process_store
            .submit_process(SubmitProcessRequest {
                process_id: ProcessId::from_uuid(child_run_id.as_uuid()),
                process_kind: ProcessKind::AgentTurn,
                scope: child_scope.to_resource_scope(),
                exclusive_within_scope: false,
                operation_id: Some(ProcessOperationId::from_trusted(format!(
                    "sweep-child-{suffix}"
                ))),
                owner_user_id: Some(owner_user_id),
                concurrency_class: None,
                parent_process_id: Some(parent_process_id),
                root_process_id: Some(parent_process_id),
                spawn_tree_descendant_cap: Some(64),
                dependency: Some(ProcessDependencySubmission {
                    dependent_process_id: parent_process_id,
                    root_process_id: parent_process_id,
                    // The exact tag `finish_spawn` writes for
                    // `SpawnSubagentMode::Background` — the run-start sweep
                    // queries by this thread-scoped group.
                    group_ref: Some(format!("bg:{}", self.parent_scope.thread_id)),
                    metadata: serde_json::to_value(submitted).expect("edge metadata"),
                }),
                checkpoint_ref: None,
                input: None,
                created_at: chrono::Utc::now(),
                metadata: serde_json::Value::Null,
            })
            .await
            .expect("submit child process");
        (child_run_id, child_scope)
    }
}

async fn sweep_fixture(
    enqueue: Arc<RecordingEnqueue>,
    live_run: Option<(TurnRunId, ironclaw_host_api::turn::TurnId)>,
    coordinator: Arc<dyn TurnCoordinator>,
) -> SweepFixture {
    use ironclaw_host_api::ids::{AgentId, ProcessId, TenantId, ThreadId};
    use ironclaw_processes::{ProcessKind, ProcessSubmissionPort, SubmitProcessRequest};

    let tenant_id = TenantId::new("sweep-tenant").expect("tenant");
    let agent_id = AgentId::new("sweep-agent").expect("agent");
    let owner_user_id = UserId::new("sweep-owner").expect("owner");
    let parent_thread_id = ThreadId::new("sweep-parent-thread").expect("parent thread");
    let parent_scope = TurnScope::new_with_owner(
        tenant_id,
        Some(agent_id),
        None,
        parent_thread_id,
        Some(owner_user_id.clone()),
    );
    let parent_run_id = TurnRunId::new();
    let parent_process_id = ProcessId::from_uuid(parent_run_id.as_uuid());

    let process_store = Arc::new(ironclaw_processes::ProcessJournalStore::new(
        recon_scoped_fs(),
    ));
    process_store
        .submit_process(SubmitProcessRequest {
            process_id: parent_process_id,
            process_kind: ProcessKind::AgentTurn,
            scope: parent_scope.to_resource_scope(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(owner_user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit parent process");

    let dependencies = Arc::clone(&process_store)
        as Arc<
            dyn ironclaw_processes::ProcessDependencyPort<
                    Error = ironclaw_processes::ProcessJournalStoreError,
                >,
        >;
    let edge_store = Arc::new(AwaitEdgeStore::new(dependencies));

    let mut parent_context = ironclaw_agent_loop::test_support::test_run_context("sweep-parent");
    parent_context.scope = parent_scope.clone();
    parent_context.thread_id = parent_scope.thread_id.clone();
    parent_context.run_id = parent_run_id;
    parent_context.actor = Some(TurnActor::new(owner_user_id));

    // `sweep_thread_on_run_start` never calls `get_run_record` (only the
    // reactive `handle_child_terminal_inner` path does), so this value is
    // never read by these tests — a well-formed placeholder is enough to
    // satisfy `StubBackgroundRuntime`'s field.
    let placeholder_child_record = TurnRunRecord {
        subagent_activation_provenance: None,
        run_id: TurnRunId::new(),
        turn_id: ironclaw_host_api::turn::TurnId::new(),
        scope: parent_context.scope.clone(),
        accepted_message_ref: ironclaw_host_api::turn::AcceptedMessageRef::new(
            "msg:sweep-placeholder",
        )
        .expect("accepted message ref"),
        status: TurnStatus::Completed,
        profile: ironclaw_turns::TurnRunProfile::from_resolved(
            parent_context.resolved_run_profile.clone(),
        ),
        output_contract: Default::default(),
        resolved_model_route: None,
        model_usage: None,
        execution_outcome: None,
        checkpoint_id: None,
        gate_ref: None,
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: ironclaw_host_api::turn::EventCursor(1),
        runner_id: None,
        lease_token: None,
        lease_expires_at: None,
        last_heartbeat_at: None,
        claim_count: 0,
        received_at: chrono::Utc::now(),
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
        resume_disposition: None,
    };
    let recent_runs = match live_run {
        Some((live_run_id, live_turn_id)) => vec![TurnRunRecord {
            subagent_activation_provenance: None,
            run_id: live_run_id,
            turn_id: live_turn_id,
            scope: parent_scope.clone(),
            accepted_message_ref: ironclaw_host_api::turn::AcceptedMessageRef::new(
                "msg:sweep-live",
            )
            .expect("accepted message ref"),
            status: TurnStatus::Running,
            profile: ironclaw_turns::TurnRunProfile::from_resolved(
                parent_context.resolved_run_profile.clone(),
            ),
            output_contract: Default::default(),
            resolved_model_route: None,
            model_usage: None,
            execution_outcome: None,
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: ironclaw_host_api::turn::EventCursor(1),
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 0,
            received_at: chrono::Utc::now(),
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
            resume_disposition: None,
        }],
        None => Vec::new(),
    };
    let runtime = Arc::new(StubBackgroundRuntime {
        child_record: placeholder_child_record,
        recent_runs,
    }) as Arc<dyn AgentTurnSpawnTreeRuntimePort>;

    let result_writer: Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter> =
        Arc::new(RecordingUpdateWriter::default());
    let thread_service = Arc::new(ironclaw_threads::InMemorySessionThreadService::default());
    let resolver = Arc::new(AwaitEdgeResolver::new_unbound(
        Arc::clone(&edge_store),
        runtime,
        result_writer,
        thread_service,
    ));
    resolver
        .bind_input_enqueue(Arc::clone(&enqueue) as Arc<dyn HostInputEnqueuePort>)
        .expect("bind input enqueue");
    resolver
        .bind_coordinator(coordinator)
        .expect("bind coordinator");

    SweepFixture {
        resolver,
        edge_store,
        process_store,
        parent_scope,
        parent_run_id,
        parent_context,
    }
}

/// (a) One `ResultAppended` background edge: the sweep re-attends and enqueues
/// into the just-starting live run, leaving the edge for queue acknowledgment.
#[tokio::test]
async fn sweep_result_appended_edge_enqueues_into_starting_run_and_waits_for_ack() {
    let enqueue = Arc::new(RecordingEnqueue::accepting());
    let live_run_id = TurnRunId::new();
    let live_turn_id = ironclaw_host_api::turn::TurnId::new();
    let coordinator = Arc::new(RecordingResumeCoordinator::default());
    let fixture = sweep_fixture(
        Arc::clone(&enqueue),
        Some((live_run_id, live_turn_id)),
        Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
    )
    .await;
    let (child_run_id, child_scope) = fixture.open_background_edge("a").await;
    fixture
        .edge_store
        .settle(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            EdgeTerminalKind::Completed,
            Some(9),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");
    fixture
        .edge_store
        .record_result_appended(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            LoopMessageRef::new(format!(
                "msg:{}",
                ironclaw_threads::ThreadMessageId::new().as_uuid()
            ))
            .expect("valid ref"),
        )
        .await
        .expect("append")
        .expect("edge exists");

    fixture
        .resolver
        .sweep_thread_on_run_start(&fixture.parent_scope, true)
        .await
        .expect("sweep succeeds");

    let requests = enqueue.requests();
    assert_eq!(requests.len(), 1, "the sweep must enqueue exactly once");
    assert_eq!(requests[0].run_id, live_run_id);
    let edge = fixture
        .edge_store
        .peek(&child_scope, fixture.parent_run_id, child_run_id)
        .await
        .expect("peek edge")
        .expect("the queue acknowledgment still owns closure");
    assert_eq!(edge.state, AwaitEdgeState::ResultAppended);
    assert!(
        coordinator.activations().is_empty(),
        "a live starting run must never be woken by activate()"
    );
}

/// (b) `MAX_QUEUED_INPUTS_PER_RUN + 1` pending `ResultAppended` edges: the
/// sweep's own bounded query drains exactly the cap, leaving the remainder
/// untouched and unclosed.
#[tokio::test]
async fn sweep_caps_at_max_queued_inputs_per_run_leaving_the_remainder_unclosed() {
    let enqueue = Arc::new(RecordingEnqueue::accepting());
    let live_run_id = TurnRunId::new();
    let live_turn_id = ironclaw_host_api::turn::TurnId::new();
    let coordinator = Arc::new(RecordingResumeCoordinator::default());
    let fixture = sweep_fixture(
        Arc::clone(&enqueue),
        Some((live_run_id, live_turn_id)),
        Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
    )
    .await;

    let total = ironclaw_loop_host::MAX_QUEUED_INPUTS_PER_RUN + 1;
    let mut edges = Vec::with_capacity(total);
    for index in 0..total {
        let suffix = format!("cap-{index}");
        let (child_run_id, child_scope) = fixture.open_background_edge(&suffix).await;
        fixture
            .edge_store
            .settle(
                &child_scope,
                fixture.parent_run_id,
                child_run_id,
                EdgeTerminalKind::Completed,
                Some(1),
                None,
            )
            .await
            .expect("settle")
            .expect("edge exists");
        fixture
            .edge_store
            .record_result_appended(
                &child_scope,
                fixture.parent_run_id,
                child_run_id,
                LoopMessageRef::new(format!(
                    "msg:{}",
                    ironclaw_threads::ThreadMessageId::new().as_uuid()
                ))
                .expect("valid ref"),
            )
            .await
            .expect("append")
            .expect("edge exists");
        edges.push((child_run_id, child_scope));
    }

    fixture
        .resolver
        .sweep_thread_on_run_start(&fixture.parent_scope, true)
        .await
        .expect("sweep succeeds");

    let mut closed = 0;
    let mut still_pending = 0;
    for (child_run_id, child_scope) in &edges {
        match fixture
            .edge_store
            .peek(child_scope, fixture.parent_run_id, *child_run_id)
            .await
            .expect("peek edge")
        {
            None => closed += 1,
            Some(edge) => {
                assert_eq!(
                    edge.state,
                    AwaitEdgeState::ResultAppended,
                    "an edge the sweep didn't reach must be untouched"
                );
                still_pending += 1;
            }
        }
    }
    assert_eq!(
        closed, 0,
        "the sweep must not close before queue acknowledgment"
    );
    assert_eq!(
        still_pending, total,
        "all enqueued edges remain recoverable"
    );
    assert_eq!(
        enqueue.requests().len(),
        ironclaw_loop_host::MAX_QUEUED_INPUTS_PER_RUN
    );
}

/// (c) An `AttentionDeferredStreakCap` edge drains only when the starting
/// run's provenance is human/permitted; an autonomous (System/ParentAgent)
/// start must leave it parked.
#[tokio::test]
async fn sweep_drains_a_streak_capped_edge_only_when_human_initiated() {
    let enqueue = Arc::new(RecordingEnqueue::accepting());
    let coordinator = Arc::new(
        RecordingResumeCoordinator::default()
            .with_activation_result(activation_accepted(TurnRunId::new())),
    );
    let fixture = sweep_fixture(
        Arc::clone(&enqueue),
        None,
        Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
    )
    .await;
    let (child_run_id, child_scope) = fixture.open_background_edge("c").await;
    fixture
        .edge_store
        .settle(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            EdgeTerminalKind::Completed,
            Some(4),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");
    fixture
        .edge_store
        .record_result_appended(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            LoopMessageRef::new(format!(
                "msg:{}",
                ironclaw_threads::ThreadMessageId::new().as_uuid()
            ))
            .expect("valid ref"),
        )
        .await
        .expect("append")
        .expect("edge exists");
    fixture
        .edge_store
        .defer_streak_capped(&child_scope, fixture.parent_run_id, child_run_id)
        .await
        .expect("deferral recorded")
        .expect("edge exists");

    // Not human-initiated (System/ParentAgent wake): the sweep must skip it.
    fixture
        .resolver
        .sweep_thread_on_run_start(&fixture.parent_scope, false)
        .await
        .expect("sweep succeeds");
    let still_parked = fixture
        .edge_store
        .peek(&child_scope, fixture.parent_run_id, child_run_id)
        .await
        .expect("peek edge")
        .expect("edge exists");
    assert_eq!(
        still_parked.state,
        AwaitEdgeState::AttentionDeferredStreakCap,
        "an autonomous start must not drain a streak-capped edge"
    );
    assert!(
        coordinator.activations().is_empty(),
        "an autonomous start must never call activate() on a parked edge"
    );

    // Human/permitted start: the sweep must drain it forward and close it.
    fixture
        .resolver
        .sweep_thread_on_run_start(&fixture.parent_scope, true)
        .await
        .expect("sweep succeeds");
    assert!(
        fixture
            .edge_store
            .peek(&child_scope, fixture.parent_run_id, child_run_id)
            .await
            .expect("peek edge")
            .is_none(),
        "a permitted start must drain and close the parked edge"
    );
    assert_eq!(
        coordinator.activations().len(),
        1,
        "a permitted start drains through exactly one activate() call"
    );
}

/// (d) An `AttentionScheduled` edge is closed with no re-enqueue and no
/// re-activation — its attention is already durable.
#[tokio::test]
async fn sweep_closes_attention_scheduled_edge_without_re_enqueue_or_re_activate() {
    let enqueue = Arc::new(RecordingEnqueue::accepting());
    let coordinator = Arc::new(RecordingResumeCoordinator::default());
    let fixture = sweep_fixture(
        Arc::clone(&enqueue),
        None,
        Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
    )
    .await;
    let (child_run_id, child_scope) = fixture.open_background_edge("d").await;
    fixture
        .edge_store
        .settle(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            EdgeTerminalKind::Completed,
            Some(3),
            None,
        )
        .await
        .expect("settle")
        .expect("edge exists");
    fixture
        .edge_store
        .record_result_appended(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            LoopMessageRef::new(format!(
                "msg:{}",
                ironclaw_threads::ThreadMessageId::new().as_uuid()
            ))
            .expect("valid ref"),
        )
        .await
        .expect("append")
        .expect("edge exists");
    fixture
        .edge_store
        .record_attention(
            &child_scope,
            fixture.parent_run_id,
            child_run_id,
            crate::subagent::await_edge::AttentionOutcome::Queued,
        )
        .await
        .expect("attention recorded")
        .expect("edge exists");

    fixture
        .resolver
        .sweep_thread_on_run_start(&fixture.parent_scope, true)
        .await
        .expect("sweep succeeds");

    assert!(
        fixture
            .edge_store
            .peek(&child_scope, fixture.parent_run_id, child_run_id)
            .await
            .expect("peek edge")
            .is_none(),
        "an already-attended edge must close"
    );
    assert!(
        enqueue.requests().is_empty(),
        "attention is already durable — the sweep must not re-enqueue"
    );
    assert!(
        coordinator.activations().is_empty(),
        "attention is already durable — the sweep must not re-activate"
    );
}
