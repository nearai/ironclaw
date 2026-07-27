use super::*;

pub(super) struct RegistryPersistentApprovalGranteeResolver {
    registry: Arc<ExtensionRegistry>,
    outbound_delivery_target_set_provider: ExtensionId,
}

impl PersistentApprovalGranteeResolver for RegistryPersistentApprovalGranteeResolver {
    fn persistent_approval_grantee(&self, capability_id: &CapabilityId) -> Option<Principal> {
        if let Some(descriptor) = self.registry.get_capability(capability_id) {
            return Some(Principal::Extension(descriptor.provider.clone()));
        }
        if capability_id.as_str() == OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID {
            return Some(Principal::Extension(
                self.outbound_delivery_target_set_provider.clone(),
            ));
        }
        None
    }
}

impl RegistryPersistentApprovalGranteeResolver {
    pub(super) fn new(registry: Arc<ExtensionRegistry>) -> Result<Self, RebornRuntimeError> {
        let outbound_delivery_target_set_provider = outbound_delivery_synthetic_provider()
            .map_err(|error| RebornRuntimeError::InvalidArgument {
                reason: format!("outbound delivery synthetic provider id is invalid: {error}"),
            })?;
        Ok(Self {
            registry,
            outbound_delivery_target_set_provider,
        })
    }
}

/// Builds the production approval interaction service.
pub(crate) fn build_approval_interaction_service(
    runtime: &RebornRuntimeStores,
    builtin_capability_policy: Arc<BuiltinCapabilityPolicy>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    audit_sink: Option<Arc<dyn ironclaw_events::AuditSink>>,
) -> Result<Arc<dyn ApprovalInteractionService>, RebornRuntimeError> {
    build_approval_interaction_service_with_turn_run_source(
        runtime,
        builtin_capability_policy,
        turn_coordinator,
        audit_sink,
        Arc::clone(&runtime.turn_state) as Arc<dyn TurnRunSnapshotSource>,
    )
}

/// Testable assembly seam with an injected turn-run snapshot source.
pub(crate) fn build_approval_interaction_service_with_turn_run_source(
    runtime: &RebornRuntimeStores,
    builtin_capability_policy: Arc<BuiltinCapabilityPolicy>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    audit_sink: Option<Arc<dyn ironclaw_events::AuditSink>>,
    turn_run_source: Arc<dyn TurnRunSnapshotSource>,
) -> Result<Arc<dyn ApprovalInteractionService>, RebornRuntimeError> {
    let approval_requests = &runtime.approval_requests;
    let capability_leases = &runtime.capability_leases;
    let extension_registry = &runtime.extension_registry;
    let workspace_mounts = &runtime.workspace_mounts;
    let skill_mounts = &runtime.skill_mounts;
    let memory_mounts = &runtime.memory_mounts;
    let system_extensions_lifecycle_mounts = &runtime.system_extensions_lifecycle_mounts;
    let persistent_approval_policies = &runtime.persistent_approval_policies;
    let tool_permission_overrides = &runtime.tool_permission_overrides;
    let approval_turn_runs = Arc::new(SnapshotApprovalTurnRunLocator::new(turn_run_source));
    let approval_read_model = Arc::new(RunStateApprovalInteractionReadModel::new(
        approval_requests.clone(),
        approval_turn_runs,
    ));
    let mut approval_resolver =
        ApprovalResolverPort::new(approval_requests.clone(), capability_leases.clone());
    if let Some(audit_sink) = audit_sink {
        approval_resolver = approval_resolver.with_audit_sink(audit_sink);
    }
    let approval_resolver = Arc::new(approval_resolver);

    Ok(Arc::new(
        DefaultApprovalInteractionService::new(
            approval_read_model,
            Arc::new(approval::PolicyApprovalLeaseTermsProvider::new(
                builtin_capability_policy,
                Arc::clone(extension_registry),
                workspace_mounts.clone(),
                skill_mounts.clone(),
                memory_mounts.clone(),
                system_extensions_lifecycle_mounts.clone(),
                ironclaw_extension_host::capability_surface::ExtensionCapabilitySurfaceSource::new(
                    Some(Arc::clone(&runtime.extension_management)),
                ),
            )),
            approval_resolver,
            turn_coordinator,
        )
        .with_persistent_policy_store(persistent_approval_policies.clone())
        .with_persistent_grantee_resolver(Arc::new(RegistryPersistentApprovalGranteeResolver::new(
            Arc::clone(extension_registry),
        )?))
        .with_tool_permission_override_store(tool_permission_overrides.clone()),
    ))
}

pub(super) struct SnapshotApprovalTurnRunLocator {
    turn_state: Arc<dyn TurnRunSnapshotSource>,
}

impl SnapshotApprovalTurnRunLocator {
    pub(super) fn new(turn_state: Arc<dyn TurnRunSnapshotSource>) -> Self {
        Self { turn_state }
    }

    async fn snapshot(
        &self,
    ) -> Result<TurnPersistenceSnapshot, ironclaw_product::ProductSurfaceFailure> {
        self.turn_state.turn_run_snapshot().await.map_err(|error| {
            tracing::debug!(
                %error,
                "approval turn-run locator could not read turn persistence snapshot"
            );
            approval_turn_locator_unavailable()
        })
    }
}

pub(super) struct ApprovalRequestGateEvidence {
    pub(super) approval_requests: Arc<dyn ironclaw_run_state::ApprovalRequestStorePort>,
}

/// Test-only constructor mirroring the production loop-exit evidence store.
#[cfg(feature = "test-support")]
pub(crate) fn build_approval_gate_evidence_for_test(
    approval_requests: std::sync::Arc<dyn ironclaw_run_state::ApprovalRequestStorePort>,
) -> std::sync::Arc<dyn ironclaw_runner::loop_exit_applier::ApprovalGateEvidenceStore> {
    std::sync::Arc::new(ApprovalRequestGateEvidence { approval_requests })
}

#[async_trait::async_trait]
impl ApprovalGateEvidenceStore for ApprovalRequestGateEvidence {
    async fn pending_approval_gate(
        &self,
        scope: &TurnScope,
        gate_ref: &LoopGateRef,
    ) -> Result<bool, TurnError> {
        let Some(request_id) = approval_request_id_from_gate_ref(gate_ref) else {
            return Ok(false);
        };
        let record = self
            .approval_requests
            .get(&scope.to_resource_scope(), request_id)
            .await
            .map_err(|error| TurnError::Unavailable {
                reason: format!("approval request evidence lookup failed: {error}"),
            })?;
        Ok(record
            .map(|record| record.status == ironclaw_run_state::ApprovalStatus::Pending)
            .unwrap_or(false))
    }
}

fn approval_request_id_from_gate_ref(gate_ref: &LoopGateRef) -> Option<ApprovalRequestId> {
    gate_ref
        .as_str()
        .strip_prefix("gate:approval-")
        .and_then(|value| ApprovalRequestId::parse(value).ok())
}

#[async_trait::async_trait]
impl ApprovalTurnRunLocator for SnapshotApprovalTurnRunLocator {
    async fn blocked_approval_runs(
        &self,
        scope: &ApprovalInteractionScope,
    ) -> Result<Vec<ApprovalBlockedTurnRun>, ironclaw_product::ProductSurfaceFailure> {
        let turn_scope = TurnScope::new(
            scope.tenant_id.clone(),
            scope.agent_id.clone(),
            scope.project_id.clone(),
            scope.thread_id.clone(),
        );
        let actor = TurnActor::new(scope.user_id.clone());
        let snapshot = self.snapshot().await?;
        let mut runs = snapshot
            .runs
            .iter()
            .filter(|run| {
                run.scope.same_thread(&turn_scope)
                    && run.status == TurnStatus::BlockedApproval
                    && run.gate_ref.is_some()
                    && snapshot_run_actor_matches(&snapshot, run, &actor)
            })
            .filter_map(|run| {
                run.gate_ref.clone().map(|gate_ref| ApprovalBlockedTurnRun {
                    run_id: run.run_id,
                    gate_ref,
                })
            })
            .collect::<Vec<_>>();
        runs.sort_by_key(|run| run.run_id.as_uuid());
        Ok(runs)
    }

    async fn approval_run_for_gate(
        &self,
        scope: &ApprovalInteractionScope,
        gate_ref: &ironclaw_turns::GateRef,
    ) -> Result<Option<TurnRunId>, ironclaw_product::ProductSurfaceFailure> {
        let turn_scope = TurnScope::new(
            scope.tenant_id.clone(),
            scope.agent_id.clone(),
            scope.project_id.clone(),
            scope.thread_id.clone(),
        );
        let actor = TurnActor::new(scope.user_id.clone());
        let snapshot = self.snapshot().await?;
        let active = snapshot
            .runs
            .iter()
            .find(|run| {
                run.scope.same_thread(&turn_scope)
                    && run.status == TurnStatus::BlockedApproval
                    && run.gate_ref.as_ref() == Some(gate_ref)
                    && snapshot_run_actor_matches(&snapshot, run, &actor)
            })
            .map(|run| run.run_id);
        if active.is_some() {
            return Ok(active);
        }

        let mut historical = snapshot
            .checkpoints
            .iter()
            .filter(|checkpoint| {
                checkpoint.status == TurnStatus::BlockedApproval
                    && &checkpoint.gate_ref == gate_ref
                    && checkpoint
                        .scope
                        .as_ref()
                        .is_none_or(|stored| stored.same_thread(&turn_scope))
            })
            .filter_map(|checkpoint| {
                snapshot
                    .runs
                    .iter()
                    .find(|run| {
                        run.run_id == checkpoint.run_id
                            && run.scope.same_thread(&turn_scope)
                            && snapshot_run_actor_matches(&snapshot, run, &actor)
                    })
                    .map(|run| run.run_id)
            })
            .collect::<Vec<_>>();
        historical.sort_by_key(|run_id| run_id.as_uuid());
        historical.dedup();
        Ok(historical.into_iter().next())
    }
}

fn snapshot_run_actor_matches(
    snapshot: &TurnPersistenceSnapshot,
    run: &TurnRunRecord,
    actor: &TurnActor,
) -> bool {
    snapshot.turns.iter().any(|turn| {
        turn.turn_id == run.turn_id && turn.scope.same_thread(&run.scope) && turn.actor == *actor
    })
}

fn approval_turn_locator_unavailable() -> ironclaw_product::ProductSurfaceFailure {
    ironclaw_product::ProductSurfaceFailure::Transient {
        reason: "approval turn-run locator unavailable".to_string(),
    }
}
