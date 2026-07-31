use std::collections::BTreeSet;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use ironclaw_host_api::decision::RuntimeCredentialAuthRequirement;

use crate::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, CapabilityActivityId, GateRef,
    GetRunStateRequest, ReplyTargetBindingRef, ResumeTurnRequest, ResumeTurnResponse,
    RetryTurnRequest, RetryTurnResponse, RunProfileResolver, SourceBindingRef,
    SubmitChildRunRequest, SubmitTurnRequest, SubmitTurnResponse, TurnActiveRunRefState,
    TurnAdmissionPolicy, TurnCheckpointId, TurnError, TurnId, TurnLeaseToken, TurnRunId,
    TurnRunProfile, TurnRunState, TurnRunnerId, TurnScope, TurnStatus, TurnTimestamp,
    events::EventCursor, run_profile::LoopModelRouteSnapshot,
};

#[async_trait]
pub trait AgentTurnRuntimePort: Send + Sync {
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError>;

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError>;

    async fn retry_turn(&self, request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError>;

    async fn request_cancel(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError>;

    /// Return the run state when the run exists in the supplied exact scope.
    ///
    /// Missing runs and runs outside the supplied scope must both return
    /// [`TurnError::ScopeNotFound`]. This keeps scoped lookups non-enumerating
    /// and gives higher-level helpers one canonical missing-run shape.
    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError>;
}

/// Classify an active run reference through the shared turn-state lookup.
///
/// `None` and missing records both map to [`TurnActiveRunRefState::Missing`];
/// only looked-up terminal records map to [`TurnActiveRunRefState::Terminal`].
pub async fn active_run_ref_state<S>(
    store: &S,
    scope: TurnScope,
    active_run_ref: Option<TurnRunId>,
) -> Result<TurnActiveRunRefState, TurnError>
where
    S: AgentTurnRuntimePort + ?Sized,
{
    let Some(run_id) = active_run_ref else {
        return Ok(TurnActiveRunRefState::Missing);
    };
    match store
        .get_run_state(GetRunStateRequest { scope, run_id })
        .await
    {
        Ok(state) if state.status.is_terminal() => Ok(TurnActiveRunRefState::Terminal),
        Ok(_) => Ok(TurnActiveRunRefState::Nonterminal),
        Err(TurnError::ScopeNotFound) => Ok(TurnActiveRunRefState::Missing),
        Err(error) => Err(error),
    }
}

#[async_trait]
pub trait AgentTurnSpawnTreeRuntimePort: AgentTurnRuntimePort {
    /// Spawn-tree operations are only needed by child-run orchestration.
    /// General turn submission should stay behind `AgentTurnRuntimePort`.
    async fn submit_child_turn(
        &self,
        request: SubmitChildRunRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError>;
    ///
    /// List child runs only when the parent is visible in the supplied scope.
    ///
    /// Implementations must not leak whether a run exists in another tenant,
    /// agent, project, or thread scope; missing and unauthorized parents should
    /// both produce an empty child list.
    async fn children_of(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError>;

    /// Return a run record only when it belongs to the supplied exact scope.
    async fn get_run_record(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError>;

    /// Reserve descendant capacity for a root run after validating root scope.
    ///
    /// Missing roots must return not found and cross-scope roots must return
    /// unauthorized rather than mutating reservation state.
    async fn reserve_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        cap: u32,
    ) -> Result<SpawnTreeReservation, TurnError>;

    /// Release descendant capacity for a root run after validating root scope.
    ///
    /// This compatibility operation is idempotent by child run id. Process
    /// dependencies release their reservation atomically on consume/abandon
    /// and do not call this operation.
    async fn release_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        idempotency_key: TurnRunId,
    ) -> Result<(), TurnError>;

    /// Remove one child's dedup entry from `SpawnTreeReservation.released_children`
    /// once its await-edge is about to be deleted (§5.5 round-7) — called
    /// strictly *before* the edge's `delete_if_version`, never after, so a
    /// crash between prune and delete just re-derives the same prune on the
    /// next recovery pass (idempotent: pruning an absent entry is a no-op).
    /// Without this, `released_children` grows for the tree's entire
    /// cumulative lifetime instead of staying bounded by the live descendant
    /// cap. Missing root is benign here (the tree may already be fully
    /// released and its reservation record deleted) — only real backend
    /// failures return `Err`.
    async fn prune_released_child(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), TurnError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRunRecord {
    pub run_id: TurnRunId,
    pub turn_id: TurnId,
    pub scope: TurnScope,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub status: TurnStatus,
    pub profile: TurnRunProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    /// Cumulative provider-reported token usage for this run's model calls,
    /// captured at loop exit. Rides the JSON-blob snapshot like
    /// `resolved_model_route`; `None` when no usage was reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_usage: Option<crate::run_profile::LoopModelUsage>,
    pub checkpoint_id: Option<TurnCheckpointId>,
    pub gate_ref: Option<GateRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_activity_id: Option<CapabilityActivityId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    pub failure: Option<crate::SanitizedFailure>,
    pub event_cursor: EventCursor,
    pub runner_id: Option<TurnRunnerId>,
    pub lease_token: Option<TurnLeaseToken>,
    pub lease_expires_at: Option<TurnTimestamp>,
    pub last_heartbeat_at: Option<TurnTimestamp>,
    pub claim_count: u64,
    pub received_at: TurnTimestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<TurnRunId>,
    #[serde(default)]
    pub subagent_depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_tree_root_run_id: Option<TurnRunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_context: Option<crate::ProductTurnContext>,
    #[serde(
        rename = "auth_resume_disposition",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub resume_disposition: Option<crate::GateResumeDisposition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnTreeReservation {
    pub scope: TurnScope,
    pub root_run_id: TurnRunId,
    pub descendant_count: u64,
    /// Per-child dedup record for `release_tree_descendants`'s idempotency
    /// key (§5.5 round-5/6) — present only for children whose release has
    /// been recorded but whose await-edge hasn't finished closing yet
    /// (pruned in lockstep with edge deletion, so this stays bounded by the
    /// live descendant cap, never the tree's cumulative lifetime count).
    #[serde(default)]
    pub released_children: BTreeSet<TurnRunId>,
}

#[cfg(test)]
mod tests {
    use crate::{
        AcceptedMessageRef, EventCursor, GateResumeDisposition, ReplyTargetBindingRef,
        SourceBindingRef, TurnRunId, TurnRunRecord, TurnScope, TurnStatus,
    };
    use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId};

    fn minimal_turn_run_record() -> TurnRunRecord {
        // Build a TurnRunRecord by serializing a struct-literal then
        // deserializing back so serde fills in all optional defaults.
        // We construct the profile via the same JSON shortcut used elsewhere
        // in test helpers (no ResolvedRunProfile needed).
        let scope = TurnScope::new(
            TenantId::new("tenant-store-test").unwrap(),
            Some(AgentId::new("agent-store-test").unwrap()),
            Some(ProjectId::new("project-store-test").unwrap()),
            ThreadId::new("thread-store-test").unwrap(),
        );
        let profile: crate::TurnRunProfile = serde_json::from_value(serde_json::json!({
            "id": "default",
            "version": 1,
            "allow_steering": false,
            "auto_queue_followups": false,
        }))
        .expect("profile deserialization");
        TurnRunRecord {
            run_id: TurnRunId::new(),
            turn_id: crate::TurnId::new(),
            scope,
            accepted_message_ref: AcceptedMessageRef::new("accepted-store-test").unwrap(),
            source_binding_ref: SourceBindingRef::new("source-store-test").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-store-test").unwrap(),
            status: TurnStatus::Completed,
            profile,
            resolved_model_route: None,
            model_usage: None,
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: vec![],
            failure: None,
            event_cursor: EventCursor(0),
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 0,
            received_at: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
            resume_disposition: None,
        }
    }

    #[test]
    fn turn_run_record_resume_disposition_defaults_to_none_when_absent() {
        // (a) Deserialize a real TurnRunRecord JSON with auth_resume_disposition key ABSENT.
        // This proves #[serde(default)] is in place on the field.
        let record = minimal_turn_run_record();
        let mut json_val =
            serde_json::to_value(&record).expect("serialize TurnRunRecord with None disposition");

        // The key must already be absent due to skip_serializing_if = "Option::is_none".
        let obj = json_val
            .as_object_mut()
            .expect("TurnRunRecord must serialize to JSON object");
        assert!(
            !obj.contains_key("auth_resume_disposition"),
            "auth_resume_disposition must be absent when resume_disposition is None"
        );

        // Belt-and-suspenders: forcibly remove the key then deserialize.
        obj.remove("auth_resume_disposition");
        let deserialized: TurnRunRecord =
            serde_json::from_value(json_val).expect("deserialize TurnRunRecord missing key");
        assert_eq!(
            deserialized.resume_disposition, None,
            "resume_disposition must default to None when the JSON key is absent"
        );

        // (b) Deserialize a real TurnRunRecord JSON carrying the LEGACY key
        // "auth_resume_disposition": "denied". This proves the serde rename/back-compat.
        let record2 = minimal_turn_run_record();
        let mut json_val2 =
            serde_json::to_value(&record2).expect("serialize TurnRunRecord for legacy key test");
        let obj2 = json_val2
            .as_object_mut()
            .expect("TurnRunRecord must serialize to JSON object");
        obj2.insert(
            "auth_resume_disposition".to_string(),
            serde_json::json!("denied"),
        );

        let deserialized2: TurnRunRecord =
            serde_json::from_value(json_val2).expect("deserialize TurnRunRecord with legacy key");
        assert_eq!(
            deserialized2.resume_disposition,
            Some(GateResumeDisposition::Denied),
            "resume_disposition must be Some(Denied) when legacy key auth_resume_disposition is present"
        );
    }
}
