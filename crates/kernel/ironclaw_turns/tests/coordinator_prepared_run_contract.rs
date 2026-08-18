//! Behavior pins for `TurnCoordinator::prepare_turn` reservations.
//!
//! The prepared-run-id path (`prepare_turn` → `submit_turn { requested_run_id }`)
//! had no direct coverage: no production caller invokes `prepare_turn` today,
//! and none of the reservation semantics — consume-once, the cross-scope
//! `Unauthorized` rejection, the child-run exemption, `abort_prepared_turn`,
//! the capacity cap — were pinned. Unbound turns lean on `submit_turn`
//! admission, so these semantics are pinned FIRST, against unmodified code.

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, ThreadId, UserId},
    output::OutputContract,
    prepared_context::{PreparedContextSource, PreparedTurnDeclarations},
    turn::{RunOriginAdapter, RunProfileRequest, TurnOwner},
};
use ironclaw_loop_contracts::{
    AgentLoopDriverDescriptor, CapabilitySurfaceProfileId, CheckpointSchemaId,
    InMemoryRunProfileRegistry, InMemoryRunProfileResolver, LoopDriverId, RunProfileDefinition,
};
use ironclaw_turns::{
    AcceptedMessageRef, DefaultTurnCoordinator, GetRunStateRequest, IdempotencyKey,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCapacityResource, TurnCoordinator,
    TurnError, TurnRunId, TurnScope, test_support::in_memory_agent_turn_runtime,
};
use std::sync::Arc;

struct StaticPreparedContextSource {
    declarations: PreparedTurnDeclarations,
}

struct FailingPreparedContextSource;

#[async_trait]
impl PreparedContextSource for StaticPreparedContextSource {
    async fn read_declarations(
        &self,
        _scope: &TurnScope,
        _actor: &TurnActor,
        _accepted_message_ref: &AcceptedMessageRef,
    ) -> Result<
        Option<PreparedTurnDeclarations>,
        ironclaw_host_api::prepared_context::PreparedContextReadError,
    > {
        Ok(Some(self.declarations.clone()))
    }
}

#[async_trait]
impl PreparedContextSource for FailingPreparedContextSource {
    async fn read_declarations(
        &self,
        _scope: &TurnScope,
        _actor: &TurnActor,
        _accepted_message_ref: &AcceptedMessageRef,
    ) -> Result<
        Option<PreparedTurnDeclarations>,
        ironclaw_host_api::prepared_context::PreparedContextReadError,
    > {
        Err(
            ironclaw_host_api::prepared_context::PreparedContextReadError::Unavailable {
                reason: "test store unavailable".to_string(),
            },
        )
    }
}

fn scope(label: &str, thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new(format!("tenant-{label}")).expect("tenant"),
        Some(AgentId::new(format!("agent-{label}")).expect("agent")),
        Some(ProjectId::new(format!("project-{label}")).expect("project")),
        ThreadId::new(thread).expect("thread"),
    )
}

fn coordinator() -> impl TurnCoordinator {
    DefaultTurnCoordinator::new(Arc::new(in_memory_agent_turn_runtime()))
}

fn submit_request(
    scope: TurnScope,
    requested_run_id: Option<TurnRunId>,
    idempotency_key: &str,
) -> SubmitTurnRequest {
    SubmitTurnRequest {
        scope,
        actor: TurnActor::new(UserId::new("user-prepared-run").expect("user")),
        accepted_message_ref: AcceptedMessageRef::new("accepted-prepared-run").expect("accepted"),
        requested_run_profile: None,
        output_contract: None,
        requested_model: None,
        idempotency_key: IdempotencyKey::new(idempotency_key).expect("idempotency key"),
        received_at: Utc::now(),
        requested_run_id,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    }
}

/// A run id minted by `prepare_turn` submits under the scope it was prepared
/// for, and the accepted run carries exactly that id.
#[tokio::test]
async fn prepared_run_id_submits_under_its_prepared_scope() {
    let coordinator = coordinator();
    let scope = scope("prepared-same", "thread-prepared-same");

    let prepared = coordinator
        .prepare_turn(scope.clone())
        .await
        .expect("prepare_turn mints a run id");
    let SubmitTurnResponse::Accepted { run_id, .. } = coordinator
        .submit_turn(submit_request(scope, Some(prepared), "prepared-same-key"))
        .await
        .expect("prepared id submits in its own scope");

    assert_eq!(run_id, prepared, "the accepted run keeps the prepared id");
}

/// A prepared run id submitted under a DIFFERENT scope (without a parent run)
/// is rejected `Unauthorized` — a prepared id cannot inject lineage into a
/// foreign scope. The reservation is consumed by that first attempt: a repeat
/// submit with the same id falls through to the store's duplicate-bound check
/// (and succeeds here because no process was created).
#[tokio::test]
async fn prepared_run_id_rejects_cross_scope_submit_and_consumes_the_reservation() {
    let coordinator = coordinator();
    let prepared_scope = scope("prepared-a", "thread-prepared-a");
    let foreign_scope = scope("prepared-b", "thread-prepared-b");

    let prepared = coordinator
        .prepare_turn(prepared_scope)
        .await
        .expect("prepare_turn mints a run id");

    let rejected = coordinator
        .submit_turn(submit_request(
            foreign_scope.clone(),
            Some(prepared),
            "prepared-cross-key",
        ))
        .await;
    assert!(
        matches!(rejected, Err(TurnError::Unauthorized)),
        "cross-scope submit of a prepared id must be Unauthorized, got {rejected:?}"
    );

    // Consume-once: the failed attempt spent the reservation, so the same
    // submit now reaches the store unchecked (today's documented behavior).
    let retried = coordinator
        .submit_turn(submit_request(
            foreign_scope,
            Some(prepared),
            "prepared-cross-key",
        ))
        .await;
    assert!(
        matches!(retried, Ok(SubmitTurnResponse::Accepted { .. })),
        "the reservation is consumed on the first attempt, got {retried:?}"
    );
}

/// The cross-scope check exempts child runs: subagent spawn legitimately
/// prepares a run id under the parent scope and submits it under the child
/// scope with `parent_run_id` set.
#[tokio::test]
async fn prepared_run_id_cross_scope_submit_is_exempt_for_child_runs() {
    let coordinator = coordinator();
    let parent_scope = scope("prepared-parent", "thread-prepared-parent");
    let child_scope = scope("prepared-child", "thread-prepared-child");

    let prepared = coordinator
        .prepare_turn(parent_scope)
        .await
        .expect("prepare_turn mints a run id");

    let mut request = submit_request(child_scope, Some(prepared), "prepared-child-key");
    request.parent_run_id = Some(TurnRunId::new());

    let response = coordinator.submit_turn(request).await;
    assert!(
        matches!(response, Ok(SubmitTurnResponse::Accepted { .. })),
        "child-run submits bypass the cross-scope reservation check, got {response:?}"
    );
}

/// `abort_prepared_turn` releases the reservation: a subsequent cross-scope
/// submit of the (no longer reserved) id is not rejected by the coordinator.
#[tokio::test]
async fn abort_prepared_turn_releases_the_reservation() {
    let coordinator = coordinator();
    let prepared_scope = scope("prepared-abort", "thread-prepared-abort");
    let foreign_scope = scope("prepared-abort-b", "thread-prepared-abort-b");

    let prepared = coordinator
        .prepare_turn(prepared_scope)
        .await
        .expect("prepare_turn mints a run id");
    coordinator
        .abort_prepared_turn(prepared)
        .await
        .expect("abort releases the reservation");

    let response = coordinator
        .submit_turn(submit_request(
            foreign_scope,
            Some(prepared),
            "prepared-abort-key",
        ))
        .await;
    assert!(
        matches!(response, Ok(SubmitTurnResponse::Accepted { .. })),
        "an aborted reservation no longer gates scope, got {response:?}"
    );
}

/// The prepared-id cache is bounded: reservation 4097 is refused with a typed
/// capacity error naming the submit-turn resource.
#[tokio::test]
async fn prepare_turn_reservations_are_capacity_bounded() {
    let coordinator = coordinator();
    let scope = scope("prepared-cap", "thread-prepared-cap");

    for _ in 0..4096 {
        coordinator
            .prepare_turn(scope.clone())
            .await
            .expect("reservations under the cap succeed");
    }

    let over_cap = coordinator.prepare_turn(scope).await;
    match over_cap {
        Err(TurnError::CapacityExceeded { resource, cap }) => {
            assert_eq!(resource, TurnCapacityResource::SubmitTurn);
            assert_eq!(cap, 4096);
        }
        other => panic!("reservation over the cap must be CapacityExceeded, got {other:?}"),
    }
}

#[tokio::test]
async fn admission_persists_prepared_output_contract_for_unbound_submission() {
    let runtime = Arc::new(in_memory_agent_turn_runtime());
    let checkpoint_schema_id = CheckpointSchemaId::from_trusted_static("prepared_checkpoint_v1");
    let unbound_profile = RunProfileDefinition::interactive_like(
        ironclaw_host_api::turn::RunProfileId::unbound_default(),
        AgentLoopDriverDescriptor {
            id: LoopDriverId::from_trusted_static("prepared_agent_loop"),
            version: ironclaw_host_api::turn::RunProfileVersion::new(1),
            checkpoint_schema_id: Some(checkpoint_schema_id.clone()),
            checkpoint_schema_version: Some(ironclaw_host_api::turn::RunProfileVersion::new(1)),
        },
        checkpoint_schema_id,
        ironclaw_host_api::turn::RunProfileVersion::new(1),
        CapabilitySurfaceProfileId::from_trusted_static("unbound_default"),
    );
    let mut registry = InMemoryRunProfileRegistry::with_builtin_profiles();
    registry
        .register(unbound_profile)
        .expect("register unbound profile");
    let coordinator = DefaultTurnCoordinator::new(runtime)
        .with_run_profile_resolver(Arc::new(InMemoryRunProfileResolver::new(registry)))
        .with_prepared_context_source(Arc::new(StaticPreparedContextSource {
            declarations: PreparedTurnDeclarations {
                output: OutputContract::JsonSchema {
                    name: "prepared_v1".to_string(),
                    schema: serde_json::json!({"type": "object", "required": ["answer"]}),
                },
                ..PreparedTurnDeclarations::default()
            },
        }));
    let scope = scope("prepared-output", "thread-prepared-output");
    let request = submit_request(scope.clone(), None, "prepared-output-key");
    let response = coordinator
        .submit_turn(request)
        .await
        .expect("prepared output submission");
    let SubmitTurnResponse::Accepted { run_id, .. } = response;
    let state = coordinator
        .get_run_state(GetRunStateRequest { scope, run_id })
        .await
        .expect("accepted run state");
    assert_eq!(
        state.output_contract,
        OutputContract::JsonSchema {
            name: "prepared_v1".to_string(),
            schema: serde_json::json!({"type": "object", "required": ["answer"]}),
        }
    );
}

#[tokio::test]
async fn admission_normalizes_matching_legacy_structured_profile_hint() {
    let runtime = Arc::new(in_memory_agent_turn_runtime());
    let checkpoint_schema_id = CheckpointSchemaId::from_trusted_static("planned_checkpoint_v1");
    let legacy_profile = RunProfileDefinition::interactive_like(
        ironclaw_host_api::turn::RunProfileId::unbound_structured(),
        AgentLoopDriverDescriptor {
            id: LoopDriverId::from_trusted_static("planned_agent_loop"),
            version: ironclaw_host_api::turn::RunProfileVersion::new(1),
            checkpoint_schema_id: Some(checkpoint_schema_id.clone()),
            checkpoint_schema_version: Some(ironclaw_host_api::turn::RunProfileVersion::new(1)),
        },
        checkpoint_schema_id.clone(),
        ironclaw_host_api::turn::RunProfileVersion::new(1),
        CapabilitySurfaceProfileId::from_trusted_static("unbound_default"),
    );
    let mut registry = InMemoryRunProfileRegistry::with_builtin_profiles();
    registry
        .register(legacy_profile)
        .expect("register legacy structured profile");
    registry
        .register(RunProfileDefinition::interactive_like(
            ironclaw_host_api::turn::RunProfileId::unbound_default(),
            AgentLoopDriverDescriptor {
                id: LoopDriverId::from_trusted_static("planned_agent_loop"),
                version: ironclaw_host_api::turn::RunProfileVersion::new(1),
                checkpoint_schema_id: Some(checkpoint_schema_id.clone()),
                checkpoint_schema_version: Some(ironclaw_host_api::turn::RunProfileVersion::new(1)),
            },
            checkpoint_schema_id,
            ironclaw_host_api::turn::RunProfileVersion::new(1),
            CapabilitySurfaceProfileId::from_trusted_static("unbound_default"),
        ))
        .expect("register ordinary unbound profile");
    let coordinator = DefaultTurnCoordinator::new(runtime)
        .with_run_profile_resolver(Arc::new(InMemoryRunProfileResolver::new(registry)))
        .with_prepared_context_source(Arc::new(StaticPreparedContextSource {
            declarations: PreparedTurnDeclarations {
                output: OutputContract::JsonSchema {
                    name: "prepared_v1".to_string(),
                    schema: serde_json::json!({"type": "object"}),
                },
                ..PreparedTurnDeclarations::default()
            },
        }));
    let scope = scope("prepared-legacy", "thread-prepared-legacy");
    let mut request = submit_request(scope.clone(), None, "prepared-legacy-key");
    request.requested_run_profile = Some(
        ironclaw_host_api::turn::RunProfileRequest::new("unbound_structured")
            .expect("legacy profile request"),
    );

    let SubmitTurnResponse::Accepted { run_id, .. } = coordinator
        .submit_turn(request)
        .await
        .expect("matching legacy structured hint remains compatible through normalization");
    let state = coordinator
        .get_run_state(GetRunStateRequest { scope, run_id })
        .await
        .expect("accepted run state");
    assert_eq!(
        state.resolved_run_profile_id,
        ironclaw_host_api::turn::RunProfileId::unbound_default()
    );
    assert!(state.output_contract.is_json_schema());
}

#[tokio::test]
async fn admission_rejects_legacy_structured_hint_for_assistant_message_declaration() {
    let runtime = Arc::new(in_memory_agent_turn_runtime());
    let coordinator = DefaultTurnCoordinator::new(runtime).with_prepared_context_source(Arc::new(
        StaticPreparedContextSource {
            declarations: PreparedTurnDeclarations::default(),
        },
    ));
    let scope = scope(
        "prepared-legacy-assistant",
        "thread-prepared-legacy-assistant",
    );
    let mut request = submit_request(scope, None, "prepared-legacy-assistant-key");
    request.requested_run_profile = Some(
        ironclaw_host_api::turn::RunProfileRequest::new("unbound_structured")
            .expect("legacy profile request"),
    );

    assert!(
        matches!(
            coordinator.submit_turn(request).await,
            Err(TurnError::AdmissionRejected(rejection))
                if rejection.reason
                    == ironclaw_turns::AdmissionRejectionReason::ProfileRejected
        ),
        "the legacy structured hint must not override an assistant-message declaration"
    );
}

#[tokio::test]
async fn admission_rejects_conflicting_caller_output_contract() {
    let runtime = Arc::new(in_memory_agent_turn_runtime());
    let coordinator = DefaultTurnCoordinator::new(runtime).with_prepared_context_source(Arc::new(
        StaticPreparedContextSource {
            declarations: PreparedTurnDeclarations {
                output: OutputContract::JsonSchema {
                    name: "prepared_v1".to_string(),
                    schema: serde_json::json!({"type": "object"}),
                },
                ..PreparedTurnDeclarations::default()
            },
        },
    ));
    let scope = scope(
        "prepared-output-conflict",
        "thread-prepared-output-conflict",
    );
    let mut request = submit_request(scope, None, "prepared-output-conflict-key");
    request.output_contract = Some(OutputContract::AssistantMessage);
    assert!(
        matches!(
            coordinator.submit_turn(request).await,
            Err(TurnError::AdmissionRejected(_))
        ),
        "a caller cannot replace the accepted context's immutable output contract"
    );
}

#[tokio::test]
async fn explicit_ordinary_profile_does_not_depend_on_prepared_context_store() {
    let runtime = Arc::new(in_memory_agent_turn_runtime());
    let coordinator = DefaultTurnCoordinator::new(runtime)
        .with_prepared_context_source(Arc::new(FailingPreparedContextSource));
    let scope = scope(
        "ordinary-profile-store-error",
        "thread-ordinary-profile-store-error",
    );
    let mut request = submit_request(scope.clone(), None, "ordinary-profile-store-error-key");
    request.requested_run_profile =
        Some(RunProfileRequest::new("default").expect("default profile request"));
    request.output_contract = Some(OutputContract::JsonSchema {
        name: "ordinary_v1".to_string(),
        schema: serde_json::json!({"type": "object"}),
    });

    let SubmitTurnResponse::Accepted { run_id, .. } =
        coordinator.submit_turn(request).await.expect(
            "an explicit ordinary profile must not be blocked by prepared-context store failure",
        );
    let state = coordinator
        .get_run_state(GetRunStateRequest { scope, run_id })
        .await
        .expect("accepted run state");
    assert_eq!(
        state.output_contract,
        OutputContract::JsonSchema {
            name: "ordinary_v1".to_string(),
            schema: serde_json::json!({"type": "object"}),
        },
        "prepared-context read failure must preserve the caller's output contract"
    );
}

#[tokio::test]
async fn scheduled_trigger_does_not_depend_on_prepared_context_store() {
    let runtime = Arc::new(in_memory_agent_turn_runtime());
    let coordinator = DefaultTurnCoordinator::new(runtime)
        .with_prepared_context_source(Arc::new(FailingPreparedContextSource));
    let mut request = submit_request(
        scope(
            "scheduled-trigger-store-error",
            "thread-scheduled-trigger-store-error",
        ),
        None,
        "scheduled-trigger-store-error-key",
    );
    request.product_context = Some(ironclaw_turns::product_context::resolve_inbound(
        ironclaw_turns::product_context::InboundClassification::TrustedTrigger,
        RunOriginAdapter::new("scheduler").expect("scheduler adapter"),
        None,
        TurnOwner::Personal {
            user: UserId::new("user-prepared-run").expect("user"),
        },
    ));

    let response = coordinator.submit_turn(request).await;
    assert!(
        matches!(response, Ok(SubmitTurnResponse::Accepted { .. })),
        "scheduled triggers must not be blocked by prepared-context store failure: {response:?}"
    );
}

#[tokio::test]
async fn hintless_scheduled_trigger_preserves_prepared_output_contract() {
    let runtime = Arc::new(in_memory_agent_turn_runtime());
    let coordinator = DefaultTurnCoordinator::new(runtime).with_prepared_context_source(Arc::new(
        StaticPreparedContextSource {
            declarations: PreparedTurnDeclarations {
                output: OutputContract::JsonSchema {
                    name: "scheduled_v1".to_string(),
                    schema: serde_json::json!({"type": "object", "required": ["answer"]}),
                },
                ..PreparedTurnDeclarations::default()
            },
        },
    ));
    let scope = scope(
        "scheduled-trigger-prepared-output",
        "thread-scheduled-trigger-prepared-output",
    );
    let mut request = submit_request(scope.clone(), None, "scheduled-trigger-prepared-output-key");
    request.product_context = Some(ironclaw_turns::product_context::resolve_inbound(
        ironclaw_turns::product_context::InboundClassification::TrustedTrigger,
        RunOriginAdapter::new("scheduler").expect("scheduler adapter"),
        None,
        TurnOwner::Personal {
            user: UserId::new("user-prepared-run").expect("user"),
        },
    ));

    let SubmitTurnResponse::Accepted { run_id, .. } = coordinator
        .submit_turn(request)
        .await
        .expect("scheduled trigger with prepared output should be accepted");
    let state = coordinator
        .get_run_state(GetRunStateRequest { scope, run_id })
        .await
        .expect("accepted run state");
    assert_eq!(
        state.output_contract,
        OutputContract::JsonSchema {
            name: "scheduled_v1".to_string(),
            schema: serde_json::json!({"type": "object", "required": ["answer"]}),
        }
    );
}

#[tokio::test]
async fn explicit_profile_still_validates_caller_output_contract_without_prepared_lookup() {
    let runtime = Arc::new(in_memory_agent_turn_runtime());
    let coordinator = DefaultTurnCoordinator::new(runtime)
        .with_prepared_context_source(Arc::new(FailingPreparedContextSource));
    let mut request = submit_request(
        scope(
            "ordinary-invalid-contract",
            "thread-ordinary-invalid-contract",
        ),
        None,
        "ordinary-invalid-contract-key",
    );
    request.requested_run_profile =
        Some(RunProfileRequest::new("default").expect("default profile request"));
    request.output_contract = Some(OutputContract::JsonSchema {
        name: "invalid/name".to_string(),
        schema: serde_json::json!({"type": "object"}),
    });

    assert!(
        matches!(
            coordinator.submit_turn(request).await,
            Err(TurnError::InvalidRequest { .. })
        ),
        "caller output contract validation must run before the prepared-context bypass"
    );
}
