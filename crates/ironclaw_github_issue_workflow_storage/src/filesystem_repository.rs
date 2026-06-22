//! Filesystem-backed GitHub issue workflow repository storage adapters.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, Filter, IndexKey, IndexValue, Page, RecordKind,
    RecordVersion, RootFilesystem, ScopedFilesystem, VersionedEntry,
};
use ironclaw_github_issue_workflow::{
    AcceptStageResultInput, AcceptStageResultOutcome, AdvanceWorkflowRunInput,
    BlockWorkflowRunInput, BlockWorkflowRunOutcome, ClaimProviderActionInput,
    ClaimProviderActionOutcome, ClaimRunnableWorkflowRunsInput, CompleteProviderActionInput,
    CompleteProviderActionOutcome, CompleteWorkflowStepInput, CompleteWorkflowStepOutcome,
    CreateOrGetProviderActionInput, CreateOrGetWorkflowRunInput, CreateOrGetWorkflowRunOutcome,
    CreateOrGetWorkflowStepInput, CreateOrGetWorkflowStepOutcome, CreateStageRunInput,
    CreateStageRunOutcome, FindLatestWorkflowEventForProviderInput, GithubIssueProviderActionId,
    GithubIssueProviderActionRecord, GithubIssueProviderBinding, GithubIssueProviderBindingId,
    GithubIssueStage, GithubIssueStageRunId, GithubIssueWorkflowError, GithubIssueWorkflowEvent,
    GithubIssueWorkflowEventId, GithubIssueWorkflowMode, GithubIssueWorkflowRepository,
    GithubIssueWorkflowRun, GithubIssueWorkflowRunId, GithubIssueWorkflowRunKey,
    GithubIssueWorkflowRunStatus, GithubIssueWorkflowState, LeaseReleaseOutcome,
    LeaseRenewalOutcome, ListWorkflowEventsAfterInput, ProviderActionStatus,
    RecordWorkflowEventInput, RecordWorkflowEventOutcome, ReleaseWorkflowRunLeaseInput,
    RenewWorkflowRunLeaseInput, TransitionOutcome, UpsertProviderBindingInput,
    WorkflowIdempotencyKey, WorkflowStepRun, WorkflowStepRunId, WorkflowStepStatus,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::{AgentId, InvocationId, ProjectId, TenantId, UserId};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
use ironclaw_host_api::{ResourceScope, ScopedPath};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

mod path;

use path::{
    default_scoped_repository_root, event_key_path, event_path, event_sequence_path,
    events_run_root, provider_action_id_path, provider_action_path, provider_binding_id_path,
    provider_binding_path, run_key_path, run_path, runs_root, scoped_repository_root_for_scope,
    stage_path, step_path, workflow_step_id_path,
};

const RUN_RECORD_KIND: &str = "github_issue_workflow_run";
const RUN_KEY_RECORD_KIND: &str = "github_issue_workflow_run_key";
const EVENT_RECORD_KIND: &str = "github_issue_workflow_event";
const EVENT_KEY_RECORD_KIND: &str = "github_issue_workflow_event_key";
const EVENT_SEQUENCE_RECORD_KIND: &str = "github_issue_workflow_event_sequence";
const STAGE_RECORD_KIND: &str = "github_issue_workflow_stage_run";
const WORKFLOW_STEP_RECORD_KIND: &str = "github_issue_workflow_step";
const WORKFLOW_STEP_ID_RECORD_KIND: &str = "github_issue_workflow_step_id";
const PROVIDER_ACTION_RECORD_KIND: &str = "github_issue_workflow_provider_action";
const PROVIDER_ACTION_ID_RECORD_KIND: &str = "github_issue_workflow_provider_action_id";
const PROVIDER_BINDING_RECORD_KIND: &str = "github_issue_workflow_provider_binding";
const PROVIDER_BINDING_ID_RECORD_KIND: &str = "github_issue_workflow_provider_binding_id";

struct FilesystemGithubIssueWorkflowRepository<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    scope: ResourceScope,
    root: ScopedPath,
}

impl<F> FilesystemGithubIssueWorkflowRepository<F>
where
    F: RootFilesystem + 'static,
{
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    fn new_root(filesystem: Arc<F>) -> Self {
        let root = default_scoped_repository_root();
        Self {
            filesystem: root_scoped_filesystem(filesystem, &root),
            scope: root_scope(),
            root,
        }
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    fn with_root(filesystem: Arc<F>, root: VirtualPath) -> Self {
        let root = ScopedPath::new(root.as_str()).expect("virtual root is a valid scoped path"); // safety: both path types use the same absolute path grammar.
        Self {
            filesystem: root_scoped_filesystem(filesystem, &root),
            scope: root_scope(),
            root,
        }
    }

    fn new_scoped(filesystem: Arc<ScopedFilesystem<F>>, scope: ResourceScope) -> Self {
        let root = scoped_repository_root_for_scope(default_scoped_repository_root(), &scope);
        Self {
            filesystem,
            scope,
            root,
        }
    }

    fn with_scoped_root(
        filesystem: Arc<ScopedFilesystem<F>>,
        scope: ResourceScope,
        root: ScopedPath,
    ) -> Self {
        let root = scoped_repository_root_for_scope(root, &scope);
        Self {
            filesystem,
            scope,
            root,
        }
    }

    async fn create_or_get_workflow_run(
        &self,
        input: CreateOrGetWorkflowRunInput,
    ) -> Result<CreateOrGetWorkflowRunOutcome, GithubIssueWorkflowError> {
        let workflow_run_key = GithubIssueWorkflowRunKey::for_issue(&input.issue_ref)?;
        let key_path = run_key_path(&self.root, &input.tenant_id, &workflow_run_key)?;

        loop {
            if let Some(record) = self.load_run_key(&key_path).await? {
                let run = self.load_run_for_key_record(record).await?;
                return Ok(CreateOrGetWorkflowRunOutcome::Existing { run });
            }

            let workflow_run_id = GithubIssueWorkflowRunId::new();
            let run = GithubIssueWorkflowRun {
                workflow_run_id: workflow_run_id.clone(),
                workflow_run_key: workflow_run_key.clone(),
                tenant_id: input.tenant_id.clone(),
                creator_user_id: input.creator_user_id.clone(),
                agent_id: input.agent_id.clone(),
                project_id: input.project_id.clone(),
                issue_ref: input.issue_ref.clone(),
                workflow_policy_key: input.workflow_policy_key.clone(),
                workflow_policy_version: input.workflow_policy_version.clone(),
                status: GithubIssueWorkflowRunStatus::Active,
                workflow_state: GithubIssueWorkflowState::new(GithubIssueWorkflowMode::New),
                event_cursor: 0,
                workflow_run_version: 0,
                lease_owner: None,
                lease_expires_at: None,
                last_heartbeat_at: None,
                claim_count: 0,
                active_stage_run_id: None,
                workspace_session_id: None,
                created_at: input.now,
                updated_at: input.now,
            };
            let key_record = WorkflowRunKeyRecord {
                workflow_run_id: workflow_run_id.clone(),
                initial_run: run.clone(),
            };

            match self
                .filesystem
                .put(
                    &self.scope,
                    &key_path,
                    entry_for_run_key(&key_record)?,
                    CasExpectation::Absent,
                )
                .await
            {
                Ok(_) => {}
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("reserve workflow run key", error)),
            }

            let path = run_path(&self.root, &run.tenant_id, &workflow_run_id)?;
            match self
                .filesystem
                .put(
                    &self.scope,
                    &path,
                    entry_for_run(&run)?,
                    CasExpectation::Absent,
                )
                .await
            {
                Ok(_) => return Ok(CreateOrGetWorkflowRunOutcome::Created { run }),
                Err(FilesystemError::VersionMismatch { .. }) => {
                    let existing = self
                        .load_required_run(&run.tenant_id, &workflow_run_id)
                        .await?;
                    return Ok(CreateOrGetWorkflowRunOutcome::Existing { run: existing });
                }
                Err(error) => return Err(filesystem_error("create workflow run", error)),
            }
        }
    }

    async fn record_workflow_event(
        &self,
        input: RecordWorkflowEventInput,
    ) -> Result<RecordWorkflowEventOutcome, GithubIssueWorkflowError> {
        let run = self.load_required_run_by_id(&input.workflow_run_id).await?;
        let key_path = event_key_path(
            &self.root,
            &input.workflow_run_id,
            &input.envelope.idempotency_key,
        )?;

        loop {
            if let Some(record) = self.load_event_key(&key_path).await? {
                let existing = self.load_event_for_key_record(record).await?;
                return Ok(RecordWorkflowEventOutcome::Duplicate { existing });
            }

            if let Some(existing) = self.superseding_event(&input).await? {
                return Ok(RecordWorkflowEventOutcome::Superseded { existing });
            }

            let sequence = self
                .claim_next_event_sequence(&input.workflow_run_id)
                .await?;
            let workflow_event_id = GithubIssueWorkflowEventId::new();
            let event = GithubIssueWorkflowEvent {
                workflow_event_id: workflow_event_id.clone(),
                workflow_run_id: input.workflow_run_id.clone(),
                sequence,
                workflow_event_type: input.workflow_event_type.clone(),
                idempotency_key: input.envelope.idempotency_key.clone(),
                source_kind: input.envelope.source_kind.clone(),
                source_delivery_id: input.envelope.source_delivery_id.clone(),
                provider: input.envelope.provider.clone(),
                provider_updated_at: input.envelope.provider_updated_at,
                observed_at: input.envelope.observed_at,
                supersedes_workflow_event_id: None,
                payload_schema: input.envelope.payload_schema.clone(),
                payload: input.envelope.payload.clone(),
                created_at: input.envelope.observed_at,
            };
            let path = event_path(&self.root, &input.workflow_run_id, sequence)?;

            match self
                .filesystem
                .put(
                    &self.scope,
                    &path,
                    entry_for_event(&event)?,
                    CasExpectation::Absent,
                )
                .await
            {
                Ok(_) => {}
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("record workflow event", error)),
            }

            let key_record = WorkflowEventKeyRecord {
                workflow_event_id,
                workflow_run_id: run.workflow_run_id.clone(),
                sequence,
                initial_event: event.clone(),
            };
            match self
                .filesystem
                .put(
                    &self.scope,
                    &key_path,
                    entry_for_event_key(&key_record)?,
                    CasExpectation::Absent,
                )
                .await
            {
                Ok(_) => return Ok(RecordWorkflowEventOutcome::Recorded { event }),
                Err(FilesystemError::VersionMismatch { .. }) => {
                    let existing = self
                        .load_event_key(&key_path)
                        .await?
                        .ok_or_else(|| repository_error("event key disappeared after conflict"))?;
                    let existing = self.load_event_for_key_record(existing).await?;
                    return Ok(RecordWorkflowEventOutcome::Duplicate { existing });
                }
                Err(error) => return Err(filesystem_error("record workflow event key", error)),
            }
        }
    }

    async fn list_workflow_events_after(
        &self,
        input: ListWorkflowEventsAfterInput,
    ) -> Result<Vec<GithubIssueWorkflowEvent>, GithubIssueWorkflowError> {
        if input.limit == 0 {
            return Ok(Vec::new());
        }
        self.load_required_run_by_id(&input.workflow_run_id).await?;
        let mut events = self.load_events_for_run(&input.workflow_run_id).await?;
        events.retain(|event| event.sequence > input.after_sequence);
        events.sort_by_key(|event| event.sequence);
        events.truncate(input.limit);
        Ok(events)
    }

    async fn claim_runnable_workflow_runs(
        &self,
        input: ClaimRunnableWorkflowRunsInput,
    ) -> Result<Vec<GithubIssueWorkflowRun>, GithubIssueWorkflowError> {
        if input.limit == 0 {
            return Ok(Vec::new());
        }

        let mut candidates = self.load_runs_for_tenant(&input.tenant_id).await?;
        candidates.sort_by(|left, right| {
            left.0.created_at.cmp(&right.0.created_at).then_with(|| {
                left.0
                    .workflow_run_id
                    .as_str()
                    .cmp(right.0.workflow_run_id.as_str())
            })
        });

        let mut claimed = Vec::new();
        for (candidate, _) in candidates {
            if claimed.len() >= input.limit {
                break;
            }
            if candidate.status != GithubIssueWorkflowRunStatus::Active
                || !lease_is_claimable(&candidate, input.now)
            {
                continue;
            }
            if let Some(run) = self
                .claim_run_if_current(&candidate.workflow_run_id, &input)
                .await?
            {
                claimed.push(run);
            }
        }

        Ok(claimed)
    }

    async fn renew_workflow_run_lease(
        &self,
        input: RenewWorkflowRunLeaseInput,
    ) -> Result<LeaseRenewalOutcome, GithubIssueWorkflowError> {
        loop {
            let (mut run, version) = self
                .load_required_run_with_version(&input.workflow_run_id)
                .await?;
            if is_terminal(&run.status) {
                return Ok(LeaseRenewalOutcome::Terminal);
            }
            if !lease_is_owned_by(&run, &input.worker_id, input.now) {
                return Ok(LeaseRenewalOutcome::NotLeaseOwner);
            }

            run.lease_expires_at = Some(input.lease_expires_at);
            run.last_heartbeat_at = Some(input.now);
            run.workflow_run_version += 1;
            run.updated_at = input.now;

            match self.put_run_versioned(&run, version).await {
                Ok(()) => return Ok(LeaseRenewalOutcome::Renewed { run }),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("renew workflow run lease", error)),
            }
        }
    }

    async fn release_workflow_run_lease(
        &self,
        input: ReleaseWorkflowRunLeaseInput,
    ) -> Result<LeaseReleaseOutcome, GithubIssueWorkflowError> {
        loop {
            let (mut run, version) = self
                .load_required_run_with_version(&input.workflow_run_id)
                .await?;
            if is_terminal(&run.status) {
                return Ok(LeaseReleaseOutcome::Terminal);
            }
            if run.lease_owner.as_ref() != Some(&input.worker_id) {
                return Ok(LeaseReleaseOutcome::NotLeaseOwner);
            }

            run.lease_owner = None;
            run.lease_expires_at = None;
            run.last_heartbeat_at = Some(input.now);
            run.workflow_run_version += 1;
            run.updated_at = input.now;

            match self.put_run_versioned(&run, version).await {
                Ok(()) => return Ok(LeaseReleaseOutcome::Released { run }),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("release workflow run lease", error)),
            }
        }
    }

    async fn block_workflow_run(
        &self,
        input: BlockWorkflowRunInput,
    ) -> Result<BlockWorkflowRunOutcome, GithubIssueWorkflowError> {
        loop {
            let (mut run, version) = self
                .load_required_run_with_version(&input.workflow_run_id)
                .await?;
            if is_terminal(&run.status) {
                return Ok(BlockWorkflowRunOutcome::Terminal);
            }
            if !lease_is_owned_by(&run, &input.worker_id, input.now) {
                return Ok(BlockWorkflowRunOutcome::NotLeaseOwner);
            }

            run.status = GithubIssueWorkflowRunStatus::Blocked;
            run.workflow_state.active_block = Some(input.active_block.clone());
            run.lease_owner = None;
            run.lease_expires_at = None;
            run.active_stage_run_id = None;
            run.last_heartbeat_at = Some(input.now);
            run.workflow_run_version += 1;
            run.updated_at = input.now;

            match self.put_run_versioned(&run, version).await {
                Ok(()) => return Ok(BlockWorkflowRunOutcome::Blocked { run }),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("block workflow run", error)),
            }
        }
    }

    async fn find_latest_workflow_event_for_provider(
        &self,
        input: FindLatestWorkflowEventForProviderInput,
    ) -> Result<Option<GithubIssueWorkflowEvent>, GithubIssueWorkflowError> {
        self.load_required_run_by_id(&input.workflow_run_id).await?;
        Ok(self
            .load_events_for_run(&input.workflow_run_id)
            .await?
            .into_iter()
            .filter(|event| {
                event.provider == input.provider
                    && input
                        .workflow_event_types
                        .iter()
                        .any(|event_type| event_type == &event.workflow_event_type)
            })
            .max_by(|left, right| {
                left.provider_updated_at
                    .cmp(&right.provider_updated_at)
                    .then_with(|| left.sequence.cmp(&right.sequence))
            }))
    }

    async fn advance_event_cursor_and_transition(
        &self,
        input: AdvanceWorkflowRunInput,
    ) -> Result<TransitionOutcome, GithubIssueWorkflowError> {
        loop {
            let (mut run, version) = self
                .load_required_run_with_version(&input.workflow_run_id)
                .await?;
            if is_terminal(&run.status) {
                return Ok(TransitionOutcome::Terminal);
            }
            if !lease_is_owned_by(&run, &input.worker_id, input.now) {
                return Ok(TransitionOutcome::NotLeaseOwner);
            }
            if run.workflow_run_version != input.expected_workflow_run_version
                || run.event_cursor != input.expected_event_cursor
            {
                return Ok(TransitionOutcome::VersionConflict { current: run });
            }
            if input.next_event_cursor < input.expected_event_cursor {
                return Err(repository_error(
                    "next event cursor must not move backwards",
                ));
            }

            run.event_cursor = input.next_event_cursor;
            if let Some(status) = input.transition.status.clone() {
                run.status = status;
            }
            if let Some(mode) = input.transition.mode.clone() {
                run.workflow_state.mode = mode;
            }
            if input.transition.clear_active_block {
                run.workflow_state.active_block = None;
            }
            if let Some(active_block) = input.transition.active_block.clone() {
                run.workflow_state.active_block = Some(active_block);
            }
            if let Some(workspace_session_id) = input.transition.workspace_session_id.clone() {
                run.workspace_session_id = Some(workspace_session_id.clone());
                run.workflow_state.current_workspace_ref = run
                    .workflow_state
                    .current_workspace_ref
                    .clone()
                    .map(|mut workspace_ref| {
                        workspace_ref.workspace_session_id = Some(workspace_session_id);
                        workspace_ref
                    });
            }

            run.workflow_run_version += 1;
            run.updated_at = input.now;
            if is_terminal(&run.status) {
                run.lease_owner = None;
                run.lease_expires_at = None;
                run.active_stage_run_id = None;
            }

            match self.put_run_versioned(&run, version).await {
                Ok(()) => return Ok(TransitionOutcome::Applied { run }),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("transition workflow run", error)),
            }
        }
    }

    async fn create_stage_run(
        &self,
        input: CreateStageRunInput,
    ) -> Result<CreateStageRunOutcome, GithubIssueWorkflowError> {
        loop {
            let (mut run, version) = self
                .load_required_run_with_version(&input.workflow_run_id)
                .await?;
            if is_terminal(&run.status) {
                return Ok(CreateStageRunOutcome::Terminal);
            }
            if let Some(active_stage_run_id) = run.active_stage_run_id.clone() {
                return Ok(CreateStageRunOutcome::ActiveStageExists {
                    existing_stage_run_id: active_stage_run_id,
                    run,
                });
            }

            let stage_run_id = GithubIssueStageRunId::new();
            run.active_stage_run_id = Some(stage_run_id.clone());
            run.workflow_run_version += 1;
            run.updated_at = input.now;

            match self.put_run_versioned(&run, version).await {
                Ok(()) => {
                    let stage = StoredStageRun {
                        stage_run_id: stage_run_id.clone(),
                        workflow_run_id: input.workflow_run_id,
                        stage: input.stage,
                        result: None,
                        active: true,
                        created_at: input.now,
                        updated_at: input.now,
                    };
                    let path = stage_path(&self.root, &stage.workflow_run_id, &stage_run_id)?;
                    self.filesystem
                        .put(
                            &self.scope,
                            &path,
                            entry_for_stage(&stage)?,
                            CasExpectation::Absent,
                        )
                        .await
                        .map_err(|error| filesystem_error("create stage run", error))?;
                    return Ok(CreateStageRunOutcome::Created { stage_run_id, run });
                }
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("create stage run", error)),
            }
        }
    }

    async fn accept_stage_result(
        &self,
        input: AcceptStageResultInput,
    ) -> Result<AcceptStageResultOutcome, GithubIssueWorkflowError> {
        loop {
            let (mut run, version) = self
                .load_required_run_with_version(&input.workflow_run_id)
                .await?;
            if is_terminal(&run.status) {
                return Ok(AcceptStageResultOutcome::Terminal);
            }
            if run.active_stage_run_id.as_ref() != Some(&input.stage_run_id) {
                return Ok(AcceptStageResultOutcome::NotActiveStage { run });
            }

            let stage_path = stage_path(&self.root, &input.workflow_run_id, &input.stage_run_id)?;
            let Some((mut stage, stage_version)) = load_record::<F, StoredStageRun>(
                &self.filesystem,
                &self.scope,
                &stage_path,
                "load stage run",
            )
            .await?
            else {
                return Ok(AcceptStageResultOutcome::NotActiveStage { run });
            };
            if stage.workflow_run_id != input.workflow_run_id || !stage.active {
                return Ok(AcceptStageResultOutcome::NotActiveStage { run });
            }

            run.active_stage_run_id = None;
            run.workflow_run_version += 1;
            run.updated_at = input.now;
            match self.put_run_versioned(&run, version).await {
                Ok(()) => {
                    stage.result = Some(input.result);
                    stage.active = false;
                    stage.updated_at = input.now;
                    self.filesystem
                        .put(
                            &self.scope,
                            &stage_path,
                            entry_for_stage(&stage)?,
                            CasExpectation::Version(stage_version),
                        )
                        .await
                        .map_err(|error| filesystem_error("accept stage result", error))?;
                    return Ok(AcceptStageResultOutcome::Accepted { run });
                }
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("accept stage result", error)),
            }
        }
    }

    async fn create_or_get_workflow_step(
        &self,
        input: CreateOrGetWorkflowStepInput,
    ) -> Result<CreateOrGetWorkflowStepOutcome, GithubIssueWorkflowError> {
        self.load_required_run_by_id(&input.workflow_run_id).await?;
        let path = step_path(&self.root, &input.workflow_run_id, &input.idempotency_key)?;
        loop {
            if let Some((existing, _)) = load_record::<F, WorkflowStepRun>(
                &self.filesystem,
                &self.scope,
                &path,
                "load workflow step",
            )
            .await?
            {
                if existing.input_hash != input.input_hash {
                    return Err(repository_error(format!(
                        "workflow step `{}` input hash mismatch for idempotency key `{}`",
                        existing.step_name, existing.idempotency_key
                    )));
                }
                return Ok(CreateOrGetWorkflowStepOutcome::Existing { step: existing });
            }

            let step = WorkflowStepRun {
                step_run_id: WorkflowStepRunId::new(),
                workflow_run_id: input.workflow_run_id.clone(),
                step_name: input.step_name.clone(),
                idempotency_key: input.idempotency_key.clone(),
                input_hash: input.input_hash.clone(),
                status: WorkflowStepStatus::Pending,
                result: None,
                error: None,
                started_at: input.now,
                completed_at: None,
                next_attempt_at: None,
            };
            match self
                .filesystem
                .put(
                    &self.scope,
                    &path,
                    entry_for_workflow_step(&step)?,
                    CasExpectation::Absent,
                )
                .await
            {
                Ok(_) => {
                    self.put_workflow_step_id_record(&step).await?;
                    return Ok(CreateOrGetWorkflowStepOutcome::Created { step });
                }
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("create workflow step", error)),
            }
        }
    }

    async fn complete_workflow_step(
        &self,
        input: CompleteWorkflowStepInput,
    ) -> Result<CompleteWorkflowStepOutcome, GithubIssueWorkflowError> {
        loop {
            let (mut step, version) = self.load_required_workflow_step(&input.step_run_id).await?;
            if workflow_step_is_complete(&step.status) {
                return Ok(CompleteWorkflowStepOutcome::AlreadyCompleted { step });
            }

            step.status = input.status.clone();
            step.result = input.result.clone();
            step.error = input.error.clone();
            step.next_attempt_at = input.next_attempt_at;
            step.completed_at = if workflow_step_is_complete(&step.status) {
                Some(input.now)
            } else {
                None
            };

            let path = step_path(&self.root, &step.workflow_run_id, &step.idempotency_key)?;
            match self
                .filesystem
                .put(
                    &self.scope,
                    &path,
                    entry_for_workflow_step(&step)?,
                    CasExpectation::Version(version),
                )
                .await
            {
                Ok(_) => return Ok(CompleteWorkflowStepOutcome::Completed { step }),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("complete workflow step", error)),
            }
        }
    }

    async fn create_or_get_provider_action(
        &self,
        input: CreateOrGetProviderActionInput,
    ) -> Result<GithubIssueProviderActionRecord, GithubIssueWorkflowError> {
        self.load_required_run_by_id(&input.workflow_run_id).await?;
        let path =
            provider_action_path(&self.root, &input.workflow_run_id, &input.idempotency_key)?;
        loop {
            if let Some((existing, _)) = load_record::<F, GithubIssueProviderActionRecord>(
                &self.filesystem,
                &self.scope,
                &path,
                "load provider action",
            )
            .await?
            {
                return Ok(existing);
            }

            let record = GithubIssueProviderActionRecord {
                provider_action_id: GithubIssueProviderActionId::new(),
                workflow_run_id: input.workflow_run_id.clone(),
                stage_run_id: input.stage_run_id.clone(),
                step_run_id: input.step_run_id.clone(),
                name: input.name.clone(),
                kind: input.kind.clone(),
                idempotency_key: input.idempotency_key.clone(),
                input_hash: input.input_hash.clone(),
                status: ProviderActionStatus::Pending,
                provider_ref: None,
                stable_marker: input.stable_marker.clone(),
                reconciliation_strategy: input.reconciliation_strategy.clone(),
                lease_owner: None,
                lease_expires_at: None,
                attempt_count: 0,
                next_attempt_at: None,
                last_reconciled_at: None,
                result: None,
                redacted_failure_kind: None,
                created_at: input.now,
                updated_at: input.now,
            };
            match self
                .filesystem
                .put(
                    &self.scope,
                    &path,
                    entry_for_provider_action(&record)?,
                    CasExpectation::Absent,
                )
                .await
            {
                Ok(_) => {
                    self.put_provider_action_id_record(&record).await?;
                    return Ok(record);
                }
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("create provider action", error)),
            }
        }
    }

    async fn claim_provider_action(
        &self,
        input: ClaimProviderActionInput,
    ) -> Result<ClaimProviderActionOutcome, GithubIssueWorkflowError> {
        loop {
            let (mut action, version) = self
                .load_required_provider_action(&input.provider_action_id)
                .await?;
            if provider_action_is_complete(&action.status) {
                return Ok(ClaimProviderActionOutcome::AlreadyCompleted { action });
            }
            if !provider_action_lease_is_claimable(&action, input.now) {
                return Ok(ClaimProviderActionOutcome::Busy { action });
            }

            action.status = ProviderActionStatus::Running;
            action.lease_owner = Some(input.worker_id.clone());
            action.lease_expires_at = Some(input.lease_expires_at);
            action.attempt_count = action.attempt_count.saturating_add(1);
            action.updated_at = input.now;

            let path =
                provider_action_path(&self.root, &action.workflow_run_id, &action.idempotency_key)?;
            match self
                .filesystem
                .put(
                    &self.scope,
                    &path,
                    entry_for_provider_action(&action)?,
                    CasExpectation::Version(version),
                )
                .await
            {
                Ok(_) => return Ok(ClaimProviderActionOutcome::Claimed { action }),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("claim provider action", error)),
            }
        }
    }

    async fn complete_provider_action(
        &self,
        input: CompleteProviderActionInput,
    ) -> Result<CompleteProviderActionOutcome, GithubIssueWorkflowError> {
        loop {
            let (mut action, version) = self
                .load_required_provider_action(&input.provider_action_id)
                .await?;
            if provider_action_is_complete(&action.status) {
                return Ok(CompleteProviderActionOutcome::AlreadyCompleted { action });
            }
            if action.lease_owner.as_ref() != Some(&input.worker_id) {
                return Ok(CompleteProviderActionOutcome::NotLeaseOwner { action });
            }

            action.status = input.status.clone();
            action.provider_ref = input.provider_ref.clone();
            action.stable_marker = input.stable_marker.clone();
            action.result = input.result.clone();
            action.redacted_failure_kind = input.redacted_failure_kind.clone();
            action.next_attempt_at = None;
            action.last_reconciled_at = match action.status {
                ProviderActionStatus::NeedsReconciliation | ProviderActionStatus::Reconciling => {
                    Some(input.now)
                }
                _ => action.last_reconciled_at,
            };
            action.lease_owner = None;
            action.lease_expires_at = None;
            action.updated_at = input.now;

            let path =
                provider_action_path(&self.root, &action.workflow_run_id, &action.idempotency_key)?;
            match self
                .filesystem
                .put(
                    &self.scope,
                    &path,
                    entry_for_provider_action(&action)?,
                    CasExpectation::Version(version),
                )
                .await
            {
                Ok(_) => return Ok(CompleteProviderActionOutcome::Completed { action }),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("complete provider action", error)),
            }
        }
    }

    async fn upsert_provider_binding(
        &self,
        input: UpsertProviderBindingInput,
    ) -> Result<GithubIssueProviderBinding, GithubIssueWorkflowError> {
        self.load_required_run_by_id(&input.workflow_run_id).await?;
        let path = provider_binding_path(&self.root, &input.provider_ref, &input.role)?;
        loop {
            if let Some((existing, _)) = load_record::<F, GithubIssueProviderBinding>(
                &self.filesystem,
                &self.scope,
                &path,
                "load provider binding",
            )
            .await?
            {
                return Ok(existing);
            }

            let binding = GithubIssueProviderBinding {
                binding_id: GithubIssueProviderBindingId::new(),
                workflow_run_id: input.workflow_run_id.clone(),
                system: input.provider_ref.system.clone(),
                resource_type: input.provider_ref.resource_type.clone(),
                role: input.role.clone(),
                owner: input.provider_ref.owner.clone(),
                repo: input.provider_ref.repo.clone(),
                provider_id: input.provider_ref.provider_id.clone(),
                provider_url: input.provider_ref.provider_url.clone(),
                created_by_provider_action_id: input.created_by_provider_action_id.clone(),
                created_at: input.created_at,
            };
            match self
                .filesystem
                .put(
                    &self.scope,
                    &path,
                    entry_for_provider_binding(&binding)?,
                    CasExpectation::Absent,
                )
                .await
            {
                Ok(_) => {
                    self.put_provider_binding_id_record(&binding).await?;
                    return Ok(binding);
                }
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("upsert provider binding", error)),
            }
        }
    }

    async fn claim_run_if_current(
        &self,
        workflow_run_id: &GithubIssueWorkflowRunId,
        input: &ClaimRunnableWorkflowRunsInput,
    ) -> Result<Option<GithubIssueWorkflowRun>, GithubIssueWorkflowError> {
        loop {
            let (mut run, version) = self.load_required_run_with_version(workflow_run_id).await?;
            if run.tenant_id != input.tenant_id
                || run.status != GithubIssueWorkflowRunStatus::Active
                || !lease_is_claimable(&run, input.now)
            {
                return Ok(None);
            }
            run.lease_owner = Some(input.worker_id.clone());
            run.lease_expires_at = Some(input.lease_expires_at);
            run.last_heartbeat_at = Some(input.now);
            run.claim_count = run.claim_count.saturating_add(1);
            run.workflow_run_version += 1;
            run.updated_at = input.now;

            match self.put_run_versioned(&run, version).await {
                Ok(()) => return Ok(Some(run)),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("claim workflow run", error)),
            }
        }
    }

    async fn load_run_key(
        &self,
        path: &ScopedPath,
    ) -> Result<Option<WorkflowRunKeyRecord>, GithubIssueWorkflowError> {
        load_record::<F, WorkflowRunKeyRecord>(
            &self.filesystem,
            &self.scope,
            path,
            "load workflow run key",
        )
        .await
        .map(|record| record.map(|(record, _)| record))
    }

    async fn load_run_for_key_record(
        &self,
        record: WorkflowRunKeyRecord,
    ) -> Result<GithubIssueWorkflowRun, GithubIssueWorkflowError> {
        match self
            .load_run(&record.initial_run.tenant_id, &record.workflow_run_id)
            .await?
        {
            Some((run, _)) => Ok(run),
            None => {
                let path = run_path(
                    &self.root,
                    &record.initial_run.tenant_id,
                    &record.workflow_run_id,
                )?;
                match self
                    .filesystem
                    .put(
                        &self.scope,
                        &path,
                        entry_for_run(&record.initial_run)?,
                        CasExpectation::Absent,
                    )
                    .await
                {
                    Ok(_) | Err(FilesystemError::VersionMismatch { .. }) => Ok(record.initial_run),
                    Err(error) => Err(filesystem_error("heal workflow run key", error)),
                }
            }
        }
    }

    async fn load_required_run(
        &self,
        tenant_id: &ironclaw_host_api::TenantId,
        workflow_run_id: &GithubIssueWorkflowRunId,
    ) -> Result<GithubIssueWorkflowRun, GithubIssueWorkflowError> {
        self.load_run(tenant_id, workflow_run_id)
            .await?
            .map(|(run, _)| run)
            .ok_or_else(|| {
                repository_error(format!("workflow run `{workflow_run_id}` does not exist"))
            })
    }

    async fn load_required_run_by_id(
        &self,
        workflow_run_id: &GithubIssueWorkflowRunId,
    ) -> Result<GithubIssueWorkflowRun, GithubIssueWorkflowError> {
        self.load_required_run_with_version(workflow_run_id)
            .await
            .map(|(run, _)| run)
    }

    async fn load_required_run_with_version(
        &self,
        workflow_run_id: &GithubIssueWorkflowRunId,
    ) -> Result<(GithubIssueWorkflowRun, RecordVersion), GithubIssueWorkflowError> {
        let filter = Filter::Eq {
            key: index_key("workflow_run_id")?,
            value: text(workflow_run_id.as_str()),
        };
        let entries = self
            .query_records::<GithubIssueWorkflowRun>(
                &runs_root(&self.root)?,
                &filter,
                "query workflow run",
            )
            .await?;
        let Some((run, version)) = entries.into_iter().next() else {
            return Err(repository_error(format!(
                "workflow run `{workflow_run_id}` does not exist"
            )));
        };
        Ok((run, version))
    }

    async fn load_run(
        &self,
        tenant_id: &ironclaw_host_api::TenantId,
        workflow_run_id: &GithubIssueWorkflowRunId,
    ) -> Result<Option<(GithubIssueWorkflowRun, RecordVersion)>, GithubIssueWorkflowError> {
        let path = run_path(&self.root, tenant_id, workflow_run_id)?;
        load_record::<F, GithubIssueWorkflowRun>(
            &self.filesystem,
            &self.scope,
            &path,
            "load workflow run",
        )
        .await
    }

    async fn put_run_versioned(
        &self,
        run: &GithubIssueWorkflowRun,
        version: RecordVersion,
    ) -> Result<(), FilesystemError> {
        let path = run_path(&self.root, &run.tenant_id, &run.workflow_run_id).map_err(|error| {
            FilesystemError::BackendInfrastructure {
                operation: ironclaw_filesystem::FilesystemOperation::WriteFile,
                reason: error.to_string(),
            }
        })?;
        let entry = entry_for_run(run).map_err(|error| FilesystemError::BackendInfrastructure {
            operation: ironclaw_filesystem::FilesystemOperation::WriteFile,
            reason: error.to_string(),
        })?;
        self.filesystem
            .put(&self.scope, &path, entry, CasExpectation::Version(version))
            .await
            .map(|_| ())
    }

    async fn load_runs_for_tenant(
        &self,
        tenant_id: &ironclaw_host_api::TenantId,
    ) -> Result<Vec<(GithubIssueWorkflowRun, RecordVersion)>, GithubIssueWorkflowError> {
        let filter = Filter::Eq {
            key: index_key("tenant_id")?,
            value: text(tenant_id.as_str()),
        };
        self.query_records(&runs_root(&self.root)?, &filter, "query workflow runs")
            .await
    }

    async fn load_event_key(
        &self,
        path: &ScopedPath,
    ) -> Result<Option<WorkflowEventKeyRecord>, GithubIssueWorkflowError> {
        load_record::<F, WorkflowEventKeyRecord>(
            &self.filesystem,
            &self.scope,
            path,
            "load workflow event key",
        )
        .await
        .map(|record| record.map(|(record, _)| record))
    }

    async fn load_event_for_key_record(
        &self,
        record: WorkflowEventKeyRecord,
    ) -> Result<GithubIssueWorkflowEvent, GithubIssueWorkflowError> {
        let path = event_path(&self.root, &record.workflow_run_id, record.sequence)?;
        match load_record::<F, GithubIssueWorkflowEvent>(
            &self.filesystem,
            &self.scope,
            &path,
            "load workflow event",
        )
        .await?
        {
            Some((event, _)) => Ok(event),
            None => {
                match self
                    .filesystem
                    .put(
                        &self.scope,
                        &path,
                        entry_for_event(&record.initial_event)?,
                        CasExpectation::Absent,
                    )
                    .await
                {
                    Ok(_) | Err(FilesystemError::VersionMismatch { .. }) => {
                        Ok(record.initial_event)
                    }
                    Err(error) => Err(filesystem_error("heal workflow event key", error)),
                }
            }
        }
    }

    async fn claim_next_event_sequence(
        &self,
        workflow_run_id: &GithubIssueWorkflowRunId,
    ) -> Result<i64, GithubIssueWorkflowError> {
        let path = event_sequence_path(&self.root, workflow_run_id)?;
        loop {
            match load_record::<F, EventSequenceRecord>(
                &self.filesystem,
                &self.scope,
                &path,
                "load workflow event sequence",
            )
            .await?
            {
                Some((mut record, version)) => {
                    let next = record
                        .last_sequence
                        .checked_add(1)
                        .ok_or_else(|| repository_error("workflow event sequence exhausted"))?;
                    record.last_sequence = next;
                    match self
                        .filesystem
                        .put(
                            &self.scope,
                            &path,
                            entry_for_event_sequence(&record)?,
                            CasExpectation::Version(version),
                        )
                        .await
                    {
                        Ok(_) => return Ok(next),
                        Err(FilesystemError::VersionMismatch { .. }) => continue,
                        Err(error) => {
                            return Err(filesystem_error("claim workflow event sequence", error));
                        }
                    }
                }
                None => {
                    let record = EventSequenceRecord { last_sequence: 1 };
                    match self
                        .filesystem
                        .put(
                            &self.scope,
                            &path,
                            entry_for_event_sequence(&record)?,
                            CasExpectation::Absent,
                        )
                        .await
                    {
                        Ok(_) => return Ok(1),
                        Err(FilesystemError::VersionMismatch { .. }) => continue,
                        Err(error) => {
                            return Err(filesystem_error(
                                "initialize workflow event sequence",
                                error,
                            ));
                        }
                    }
                }
            }
        }
    }

    async fn load_events_for_run(
        &self,
        workflow_run_id: &GithubIssueWorkflowRunId,
    ) -> Result<Vec<GithubIssueWorkflowEvent>, GithubIssueWorkflowError> {
        let mut events = self
            .query_records::<GithubIssueWorkflowEvent>(
                &events_run_root(&self.root, workflow_run_id)?,
                &Filter::All,
                "query workflow events",
            )
            .await?
            .into_iter()
            .map(|(event, _)| event)
            .collect::<Vec<_>>();
        events.sort_by_key(|event| event.sequence);
        Ok(events)
    }

    async fn superseding_event(
        &self,
        input: &RecordWorkflowEventInput,
    ) -> Result<Option<GithubIssueWorkflowEvent>, GithubIssueWorkflowError> {
        let Some(provider_updated_at) = input.envelope.provider_updated_at else {
            return Ok(None);
        };
        Ok(self
            .load_events_for_run(&input.workflow_run_id)
            .await?
            .into_iter()
            .find(|event| {
                event.workflow_event_type == input.workflow_event_type
                    && event.provider == input.envelope.provider
                    && event
                        .provider_updated_at
                        .map(|existing| existing >= provider_updated_at)
                        .unwrap_or(false)
            }))
    }

    async fn put_workflow_step_id_record(
        &self,
        step: &WorkflowStepRun,
    ) -> Result<(), GithubIssueWorkflowError> {
        let path = workflow_step_id_path(&self.root, &step.step_run_id)?;
        let record = WorkflowStepIdRecord {
            step_run_id: step.step_run_id.clone(),
            workflow_run_id: step.workflow_run_id.clone(),
            idempotency_key: step.idempotency_key.clone(),
        };
        match self
            .filesystem
            .put(
                &self.scope,
                &path,
                entry_for_workflow_step_id(&record)?,
                CasExpectation::Absent,
            )
            .await
        {
            Ok(_) | Err(FilesystemError::VersionMismatch { .. }) => Ok(()),
            Err(error) => Err(filesystem_error("create workflow step id record", error)),
        }
    }

    async fn load_required_workflow_step(
        &self,
        step_run_id: &WorkflowStepRunId,
    ) -> Result<(WorkflowStepRun, RecordVersion), GithubIssueWorkflowError> {
        let id_path = workflow_step_id_path(&self.root, step_run_id)?;
        let Some((record, _)) = load_record::<F, WorkflowStepIdRecord>(
            &self.filesystem,
            &self.scope,
            &id_path,
            "load workflow step id",
        )
        .await?
        else {
            return Err(repository_error(format!(
                "workflow step `{step_run_id}` does not exist"
            )));
        };
        let path = step_path(&self.root, &record.workflow_run_id, &record.idempotency_key)?;
        load_record::<F, WorkflowStepRun>(
            &self.filesystem,
            &self.scope,
            &path,
            "load workflow step",
        )
        .await?
        .ok_or_else(|| repository_error(format!("workflow step `{step_run_id}` does not exist")))
    }

    async fn put_provider_action_id_record(
        &self,
        action: &GithubIssueProviderActionRecord,
    ) -> Result<(), GithubIssueWorkflowError> {
        let path = provider_action_id_path(&self.root, &action.provider_action_id)?;
        let record = ProviderActionIdRecord {
            provider_action_id: action.provider_action_id.clone(),
            workflow_run_id: action.workflow_run_id.clone(),
            idempotency_key: action.idempotency_key.clone(),
        };
        match self
            .filesystem
            .put(
                &self.scope,
                &path,
                entry_for_provider_action_id(&record)?,
                CasExpectation::Absent,
            )
            .await
        {
            Ok(_) | Err(FilesystemError::VersionMismatch { .. }) => Ok(()),
            Err(error) => Err(filesystem_error("create provider action id record", error)),
        }
    }

    async fn load_required_provider_action(
        &self,
        provider_action_id: &GithubIssueProviderActionId,
    ) -> Result<(GithubIssueProviderActionRecord, RecordVersion), GithubIssueWorkflowError> {
        let id_path = provider_action_id_path(&self.root, provider_action_id)?;
        let Some((record, _)) = load_record::<F, ProviderActionIdRecord>(
            &self.filesystem,
            &self.scope,
            &id_path,
            "load provider action id",
        )
        .await?
        else {
            return Err(repository_error(format!(
                "provider action `{provider_action_id}` does not exist"
            )));
        };
        let path =
            provider_action_path(&self.root, &record.workflow_run_id, &record.idempotency_key)?;
        load_record::<F, GithubIssueProviderActionRecord>(
            &self.filesystem,
            &self.scope,
            &path,
            "load provider action",
        )
        .await?
        .ok_or_else(|| {
            repository_error(format!(
                "provider action `{provider_action_id}` does not exist"
            ))
        })
    }

    async fn put_provider_binding_id_record(
        &self,
        binding: &GithubIssueProviderBinding,
    ) -> Result<(), GithubIssueWorkflowError> {
        let path = provider_binding_id_path(&self.root, &binding.binding_id)?;
        let record = ProviderBindingIdRecord {
            binding_id: binding.binding_id.clone(),
            binding: binding.clone(),
        };
        match self
            .filesystem
            .put(
                &self.scope,
                &path,
                entry_for_provider_binding_id(&record)?,
                CasExpectation::Absent,
            )
            .await
        {
            Ok(_) | Err(FilesystemError::VersionMismatch { .. }) => Ok(()),
            Err(error) => Err(filesystem_error("create provider binding id record", error)),
        }
    }

    async fn query_records<T>(
        &self,
        prefix: &ScopedPath,
        filter: &Filter,
        operation: &'static str,
    ) -> Result<Vec<(T, RecordVersion)>, GithubIssueWorkflowError>
    where
        T: serde::de::DeserializeOwned,
    {
        let mut records = Vec::new();
        let mut offset = 0;
        loop {
            let entries = self
                .filesystem
                .query(
                    &self.scope,
                    prefix,
                    filter,
                    Page::new(offset, Page::MAX_LIMIT),
                )
                .await
                .map_err(|error| filesystem_error(operation, error))?;
            let received = entries.len();
            for entry in entries {
                let record = parse_entry(&entry, operation)?;
                records.push((record, entry.version));
            }
            if received < Page::MAX_LIMIT as usize {
                return Ok(records);
            }
            offset += u64::from(Page::MAX_LIMIT);
        }
    }
}

/// Scoped-filesystem-backed GitHub issue workflow repository.
pub struct RebornFilesystemGithubIssueWorkflowRepository<F>
where
    F: RootFilesystem,
{
    inner: FilesystemGithubIssueWorkflowRepository<F>,
}

impl<F> RebornFilesystemGithubIssueWorkflowRepository<F>
where
    F: RootFilesystem + 'static,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>, scope: ResourceScope) -> Self {
        Self {
            inner: FilesystemGithubIssueWorkflowRepository::new_scoped(filesystem, scope),
        }
    }

    pub fn with_root(
        filesystem: Arc<ScopedFilesystem<F>>,
        scope: ResourceScope,
        root: ScopedPath,
    ) -> Self {
        Self {
            inner: FilesystemGithubIssueWorkflowRepository::with_scoped_root(
                filesystem, scope, root,
            ),
        }
    }
}

#[async_trait]
impl<F> GithubIssueWorkflowRepository for RebornFilesystemGithubIssueWorkflowRepository<F>
where
    F: RootFilesystem + 'static,
{
    async fn create_or_get_workflow_run(
        &self,
        input: CreateOrGetWorkflowRunInput,
    ) -> Result<CreateOrGetWorkflowRunOutcome, GithubIssueWorkflowError> {
        self.inner.create_or_get_workflow_run(input).await
    }

    async fn record_workflow_event(
        &self,
        input: RecordWorkflowEventInput,
    ) -> Result<RecordWorkflowEventOutcome, GithubIssueWorkflowError> {
        self.inner.record_workflow_event(input).await
    }

    async fn list_workflow_events_after(
        &self,
        input: ListWorkflowEventsAfterInput,
    ) -> Result<Vec<GithubIssueWorkflowEvent>, GithubIssueWorkflowError> {
        self.inner.list_workflow_events_after(input).await
    }

    async fn claim_runnable_workflow_runs(
        &self,
        input: ClaimRunnableWorkflowRunsInput,
    ) -> Result<Vec<GithubIssueWorkflowRun>, GithubIssueWorkflowError> {
        self.inner.claim_runnable_workflow_runs(input).await
    }

    async fn renew_workflow_run_lease(
        &self,
        input: RenewWorkflowRunLeaseInput,
    ) -> Result<LeaseRenewalOutcome, GithubIssueWorkflowError> {
        self.inner.renew_workflow_run_lease(input).await
    }

    async fn release_workflow_run_lease(
        &self,
        input: ReleaseWorkflowRunLeaseInput,
    ) -> Result<LeaseReleaseOutcome, GithubIssueWorkflowError> {
        self.inner.release_workflow_run_lease(input).await
    }

    async fn block_workflow_run(
        &self,
        input: BlockWorkflowRunInput,
    ) -> Result<BlockWorkflowRunOutcome, GithubIssueWorkflowError> {
        self.inner.block_workflow_run(input).await
    }

    async fn find_latest_workflow_event_for_provider(
        &self,
        input: FindLatestWorkflowEventForProviderInput,
    ) -> Result<Option<GithubIssueWorkflowEvent>, GithubIssueWorkflowError> {
        self.inner
            .find_latest_workflow_event_for_provider(input)
            .await
    }

    async fn advance_event_cursor_and_transition(
        &self,
        input: AdvanceWorkflowRunInput,
    ) -> Result<TransitionOutcome, GithubIssueWorkflowError> {
        self.inner.advance_event_cursor_and_transition(input).await
    }

    async fn create_stage_run(
        &self,
        input: CreateStageRunInput,
    ) -> Result<CreateStageRunOutcome, GithubIssueWorkflowError> {
        self.inner.create_stage_run(input).await
    }

    async fn accept_stage_result(
        &self,
        input: AcceptStageResultInput,
    ) -> Result<AcceptStageResultOutcome, GithubIssueWorkflowError> {
        self.inner.accept_stage_result(input).await
    }

    async fn create_or_get_workflow_step(
        &self,
        input: CreateOrGetWorkflowStepInput,
    ) -> Result<CreateOrGetWorkflowStepOutcome, GithubIssueWorkflowError> {
        self.inner.create_or_get_workflow_step(input).await
    }

    async fn complete_workflow_step(
        &self,
        input: CompleteWorkflowStepInput,
    ) -> Result<CompleteWorkflowStepOutcome, GithubIssueWorkflowError> {
        self.inner.complete_workflow_step(input).await
    }

    async fn create_or_get_provider_action(
        &self,
        input: CreateOrGetProviderActionInput,
    ) -> Result<GithubIssueProviderActionRecord, GithubIssueWorkflowError> {
        self.inner.create_or_get_provider_action(input).await
    }

    async fn claim_provider_action(
        &self,
        input: ClaimProviderActionInput,
    ) -> Result<ClaimProviderActionOutcome, GithubIssueWorkflowError> {
        self.inner.claim_provider_action(input).await
    }

    async fn complete_provider_action(
        &self,
        input: CompleteProviderActionInput,
    ) -> Result<CompleteProviderActionOutcome, GithubIssueWorkflowError> {
        self.inner.complete_provider_action(input).await
    }

    async fn upsert_provider_binding(
        &self,
        input: UpsertProviderBindingInput,
    ) -> Result<GithubIssueProviderBinding, GithubIssueWorkflowError> {
        self.inner.upsert_provider_binding(input).await
    }
}

#[cfg(feature = "libsql")]
pub struct RebornLibSqlGithubIssueWorkflowRepository {
    inner: FilesystemGithubIssueWorkflowRepository<LibSqlRootFilesystem>,
}

#[cfg(feature = "libsql")]
impl RebornLibSqlGithubIssueWorkflowRepository {
    pub fn new(filesystem: Arc<LibSqlRootFilesystem>) -> Self {
        Self {
            inner: FilesystemGithubIssueWorkflowRepository::new_root(filesystem),
        }
    }

    pub fn with_root(filesystem: Arc<LibSqlRootFilesystem>, root: VirtualPath) -> Self {
        Self {
            inner: FilesystemGithubIssueWorkflowRepository::with_root(filesystem, root),
        }
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl GithubIssueWorkflowRepository for RebornLibSqlGithubIssueWorkflowRepository {
    async fn create_or_get_workflow_run(
        &self,
        input: CreateOrGetWorkflowRunInput,
    ) -> Result<CreateOrGetWorkflowRunOutcome, GithubIssueWorkflowError> {
        self.inner.create_or_get_workflow_run(input).await
    }

    async fn record_workflow_event(
        &self,
        input: RecordWorkflowEventInput,
    ) -> Result<RecordWorkflowEventOutcome, GithubIssueWorkflowError> {
        self.inner.record_workflow_event(input).await
    }

    async fn list_workflow_events_after(
        &self,
        input: ListWorkflowEventsAfterInput,
    ) -> Result<Vec<GithubIssueWorkflowEvent>, GithubIssueWorkflowError> {
        self.inner.list_workflow_events_after(input).await
    }

    async fn claim_runnable_workflow_runs(
        &self,
        input: ClaimRunnableWorkflowRunsInput,
    ) -> Result<Vec<GithubIssueWorkflowRun>, GithubIssueWorkflowError> {
        self.inner.claim_runnable_workflow_runs(input).await
    }

    async fn renew_workflow_run_lease(
        &self,
        input: RenewWorkflowRunLeaseInput,
    ) -> Result<LeaseRenewalOutcome, GithubIssueWorkflowError> {
        self.inner.renew_workflow_run_lease(input).await
    }

    async fn release_workflow_run_lease(
        &self,
        input: ReleaseWorkflowRunLeaseInput,
    ) -> Result<LeaseReleaseOutcome, GithubIssueWorkflowError> {
        self.inner.release_workflow_run_lease(input).await
    }

    async fn block_workflow_run(
        &self,
        input: BlockWorkflowRunInput,
    ) -> Result<BlockWorkflowRunOutcome, GithubIssueWorkflowError> {
        self.inner.block_workflow_run(input).await
    }

    async fn find_latest_workflow_event_for_provider(
        &self,
        input: FindLatestWorkflowEventForProviderInput,
    ) -> Result<Option<GithubIssueWorkflowEvent>, GithubIssueWorkflowError> {
        self.inner
            .find_latest_workflow_event_for_provider(input)
            .await
    }

    async fn advance_event_cursor_and_transition(
        &self,
        input: AdvanceWorkflowRunInput,
    ) -> Result<TransitionOutcome, GithubIssueWorkflowError> {
        self.inner.advance_event_cursor_and_transition(input).await
    }

    async fn create_stage_run(
        &self,
        input: CreateStageRunInput,
    ) -> Result<CreateStageRunOutcome, GithubIssueWorkflowError> {
        self.inner.create_stage_run(input).await
    }

    async fn accept_stage_result(
        &self,
        input: AcceptStageResultInput,
    ) -> Result<AcceptStageResultOutcome, GithubIssueWorkflowError> {
        self.inner.accept_stage_result(input).await
    }

    async fn create_or_get_workflow_step(
        &self,
        input: CreateOrGetWorkflowStepInput,
    ) -> Result<CreateOrGetWorkflowStepOutcome, GithubIssueWorkflowError> {
        self.inner.create_or_get_workflow_step(input).await
    }

    async fn complete_workflow_step(
        &self,
        input: CompleteWorkflowStepInput,
    ) -> Result<CompleteWorkflowStepOutcome, GithubIssueWorkflowError> {
        self.inner.complete_workflow_step(input).await
    }

    async fn create_or_get_provider_action(
        &self,
        input: CreateOrGetProviderActionInput,
    ) -> Result<GithubIssueProviderActionRecord, GithubIssueWorkflowError> {
        self.inner.create_or_get_provider_action(input).await
    }

    async fn claim_provider_action(
        &self,
        input: ClaimProviderActionInput,
    ) -> Result<ClaimProviderActionOutcome, GithubIssueWorkflowError> {
        self.inner.claim_provider_action(input).await
    }

    async fn complete_provider_action(
        &self,
        input: CompleteProviderActionInput,
    ) -> Result<CompleteProviderActionOutcome, GithubIssueWorkflowError> {
        self.inner.complete_provider_action(input).await
    }

    async fn upsert_provider_binding(
        &self,
        input: UpsertProviderBindingInput,
    ) -> Result<GithubIssueProviderBinding, GithubIssueWorkflowError> {
        self.inner.upsert_provider_binding(input).await
    }
}

#[cfg(feature = "postgres")]
pub struct RebornPostgresGithubIssueWorkflowRepository {
    inner: FilesystemGithubIssueWorkflowRepository<PostgresRootFilesystem>,
}

#[cfg(feature = "postgres")]
impl RebornPostgresGithubIssueWorkflowRepository {
    pub fn new(filesystem: Arc<PostgresRootFilesystem>) -> Self {
        Self {
            inner: FilesystemGithubIssueWorkflowRepository::new_root(filesystem),
        }
    }

    pub fn with_root(filesystem: Arc<PostgresRootFilesystem>, root: VirtualPath) -> Self {
        Self {
            inner: FilesystemGithubIssueWorkflowRepository::with_root(filesystem, root),
        }
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl GithubIssueWorkflowRepository for RebornPostgresGithubIssueWorkflowRepository {
    async fn create_or_get_workflow_run(
        &self,
        input: CreateOrGetWorkflowRunInput,
    ) -> Result<CreateOrGetWorkflowRunOutcome, GithubIssueWorkflowError> {
        self.inner.create_or_get_workflow_run(input).await
    }

    async fn record_workflow_event(
        &self,
        input: RecordWorkflowEventInput,
    ) -> Result<RecordWorkflowEventOutcome, GithubIssueWorkflowError> {
        self.inner.record_workflow_event(input).await
    }

    async fn list_workflow_events_after(
        &self,
        input: ListWorkflowEventsAfterInput,
    ) -> Result<Vec<GithubIssueWorkflowEvent>, GithubIssueWorkflowError> {
        self.inner.list_workflow_events_after(input).await
    }

    async fn claim_runnable_workflow_runs(
        &self,
        input: ClaimRunnableWorkflowRunsInput,
    ) -> Result<Vec<GithubIssueWorkflowRun>, GithubIssueWorkflowError> {
        self.inner.claim_runnable_workflow_runs(input).await
    }

    async fn renew_workflow_run_lease(
        &self,
        input: RenewWorkflowRunLeaseInput,
    ) -> Result<LeaseRenewalOutcome, GithubIssueWorkflowError> {
        self.inner.renew_workflow_run_lease(input).await
    }

    async fn release_workflow_run_lease(
        &self,
        input: ReleaseWorkflowRunLeaseInput,
    ) -> Result<LeaseReleaseOutcome, GithubIssueWorkflowError> {
        self.inner.release_workflow_run_lease(input).await
    }

    async fn block_workflow_run(
        &self,
        input: BlockWorkflowRunInput,
    ) -> Result<BlockWorkflowRunOutcome, GithubIssueWorkflowError> {
        self.inner.block_workflow_run(input).await
    }

    async fn find_latest_workflow_event_for_provider(
        &self,
        input: FindLatestWorkflowEventForProviderInput,
    ) -> Result<Option<GithubIssueWorkflowEvent>, GithubIssueWorkflowError> {
        self.inner
            .find_latest_workflow_event_for_provider(input)
            .await
    }

    async fn advance_event_cursor_and_transition(
        &self,
        input: AdvanceWorkflowRunInput,
    ) -> Result<TransitionOutcome, GithubIssueWorkflowError> {
        self.inner.advance_event_cursor_and_transition(input).await
    }

    async fn create_stage_run(
        &self,
        input: CreateStageRunInput,
    ) -> Result<CreateStageRunOutcome, GithubIssueWorkflowError> {
        self.inner.create_stage_run(input).await
    }

    async fn accept_stage_result(
        &self,
        input: AcceptStageResultInput,
    ) -> Result<AcceptStageResultOutcome, GithubIssueWorkflowError> {
        self.inner.accept_stage_result(input).await
    }

    async fn create_or_get_workflow_step(
        &self,
        input: CreateOrGetWorkflowStepInput,
    ) -> Result<CreateOrGetWorkflowStepOutcome, GithubIssueWorkflowError> {
        self.inner.create_or_get_workflow_step(input).await
    }

    async fn complete_workflow_step(
        &self,
        input: CompleteWorkflowStepInput,
    ) -> Result<CompleteWorkflowStepOutcome, GithubIssueWorkflowError> {
        self.inner.complete_workflow_step(input).await
    }

    async fn create_or_get_provider_action(
        &self,
        input: CreateOrGetProviderActionInput,
    ) -> Result<GithubIssueProviderActionRecord, GithubIssueWorkflowError> {
        self.inner.create_or_get_provider_action(input).await
    }

    async fn claim_provider_action(
        &self,
        input: ClaimProviderActionInput,
    ) -> Result<ClaimProviderActionOutcome, GithubIssueWorkflowError> {
        self.inner.claim_provider_action(input).await
    }

    async fn complete_provider_action(
        &self,
        input: CompleteProviderActionInput,
    ) -> Result<CompleteProviderActionOutcome, GithubIssueWorkflowError> {
        self.inner.complete_provider_action(input).await
    }

    async fn upsert_provider_binding(
        &self,
        input: UpsertProviderBindingInput,
    ) -> Result<GithubIssueProviderBinding, GithubIssueWorkflowError> {
        self.inner.upsert_provider_binding(input).await
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn root_scoped_filesystem<F>(filesystem: Arc<F>, root: &ScopedPath) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem + 'static,
{
    let alias = root_mount_alias(root);
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new(alias.as_str()).expect("root mount alias is valid"), // safety: root_mount_alias returns "/" or one absolute path segment from a valid ScopedPath.
        VirtualPath::new(alias).expect("root mount target is valid"), // safety: root_mount_alias returns an absolute virtual path accepted by VirtualPath.
        MountPermissions::read_write_list_delete(),
    )])
    .expect("root repository mount view is valid"); // safety: the mount view contains one read-write grant with validated alias and target.
    Arc::new(ScopedFilesystem::with_fixed_view(filesystem, mounts))
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn root_mount_alias(root: &ScopedPath) -> String {
    let mut parts = root.as_str().split('/').filter(|part| !part.is_empty());
    let Some(first) = parts.next() else {
        return "/".to_string();
    };
    format!("/{first}")
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn root_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant:github-issue-workflow-storage-root")
            .expect("static tenant id is valid"), // safety: static literal uses the validated tenant id grammar.
        user_id: UserId::new("user:github-issue-workflow-storage-root")
            .expect("static user id is valid"), // safety: static literal uses the validated user id grammar.
        agent_id: Some(
            AgentId::new("agent:github-issue-workflow-storage-root")
                .expect("static agent id is valid"), // safety: static literal uses the validated agent id grammar.
        ),
        project_id: Some(
            ProjectId::new("project:github-issue-workflow-storage-root")
                .expect("static project id is valid"), // safety: static literal uses the validated project id grammar.
        ),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

async fn load_record<F, T>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
    operation: &'static str,
) -> Result<Option<(T, RecordVersion)>, GithubIssueWorkflowError>
where
    F: RootFilesystem,
    T: serde::de::DeserializeOwned,
{
    let Some(entry) = filesystem
        .get(scope, path)
        .await
        .map_err(|error| filesystem_error(operation, error))?
    else {
        return Ok(None);
    };
    let record = parse_entry(&entry, operation)?;
    Ok(Some((record, entry.version)))
}

fn parse_entry<T>(
    entry: &VersionedEntry,
    operation: &'static str,
) -> Result<T, GithubIssueWorkflowError>
where
    T: serde::de::DeserializeOwned,
{
    entry
        .entry
        .parse_json()
        .map_err(|error| durable_error(operation, error))
}

fn entry_for_run(run: &GithubIssueWorkflowRun) -> Result<Entry, GithubIssueWorkflowError> {
    let entry = record_entry(RUN_RECORD_KIND, run)?;
    let entry = with_index(entry, "tenant_id", text(run.tenant_id.as_str()))?;
    let entry = with_index(entry, "workflow_run_id", text(run.workflow_run_id.as_str()))?;
    let entry = with_index(
        entry,
        "workflow_run_key",
        text(run.workflow_run_key.as_str()),
    )?;
    with_index(
        entry,
        "created_at_ms",
        IndexValue::I64(run.created_at.timestamp_millis()),
    )
}

fn entry_for_run_key(record: &WorkflowRunKeyRecord) -> Result<Entry, GithubIssueWorkflowError> {
    let entry = record_entry(RUN_KEY_RECORD_KIND, record)?;
    with_index(
        entry,
        "workflow_run_id",
        text(record.workflow_run_id.as_str()),
    )
}

fn entry_for_event(event: &GithubIssueWorkflowEvent) -> Result<Entry, GithubIssueWorkflowError> {
    let entry = record_entry(EVENT_RECORD_KIND, event)?;
    let entry = with_index(
        entry,
        "workflow_run_id",
        text(event.workflow_run_id.as_str()),
    )?;
    let entry = with_index(entry, "sequence", IndexValue::I64(event.sequence))?;
    with_index(
        entry,
        "idempotency_key",
        text(event.idempotency_key.as_str()),
    )
}

fn entry_for_event_key(record: &WorkflowEventKeyRecord) -> Result<Entry, GithubIssueWorkflowError> {
    let entry = record_entry(EVENT_KEY_RECORD_KIND, record)?;
    let entry = with_index(
        entry,
        "workflow_run_id",
        text(record.workflow_run_id.as_str()),
    )?;
    with_index(entry, "sequence", IndexValue::I64(record.sequence))
}

fn entry_for_event_sequence(
    record: &EventSequenceRecord,
) -> Result<Entry, GithubIssueWorkflowError> {
    let entry = record_entry(EVENT_SEQUENCE_RECORD_KIND, record)?;
    with_index(
        entry,
        "last_sequence",
        IndexValue::I64(record.last_sequence),
    )
}

fn entry_for_stage(stage: &StoredStageRun) -> Result<Entry, GithubIssueWorkflowError> {
    let entry = record_entry(STAGE_RECORD_KIND, stage)?;
    let entry = with_index(
        entry,
        "workflow_run_id",
        text(stage.workflow_run_id.as_str()),
    )?;
    let entry = with_index(entry, "stage_run_id", text(stage.stage_run_id.as_str()))?;
    with_index(entry, "active", IndexValue::Bool(stage.active))
}

fn entry_for_workflow_step(step: &WorkflowStepRun) -> Result<Entry, GithubIssueWorkflowError> {
    let entry = record_entry(WORKFLOW_STEP_RECORD_KIND, step)?;
    let entry = with_index(
        entry,
        "workflow_run_id",
        text(step.workflow_run_id.as_str()),
    )?;
    let entry = with_index(entry, "step_run_id", text(step.step_run_id.as_str()))?;
    with_index(
        entry,
        "idempotency_key",
        text(step.idempotency_key.as_str()),
    )
}

fn entry_for_workflow_step_id(
    record: &WorkflowStepIdRecord,
) -> Result<Entry, GithubIssueWorkflowError> {
    let entry = record_entry(WORKFLOW_STEP_ID_RECORD_KIND, record)?;
    with_index(entry, "step_run_id", text(record.step_run_id.as_str()))
}

fn entry_for_provider_action(
    action: &GithubIssueProviderActionRecord,
) -> Result<Entry, GithubIssueWorkflowError> {
    let entry = record_entry(PROVIDER_ACTION_RECORD_KIND, action)?;
    let entry = with_index(
        entry,
        "workflow_run_id",
        text(action.workflow_run_id.as_str()),
    )?;
    let entry = with_index(
        entry,
        "provider_action_id",
        text(action.provider_action_id.as_str()),
    )?;
    with_index(
        entry,
        "idempotency_key",
        text(action.idempotency_key.as_str()),
    )
}

fn entry_for_provider_action_id(
    record: &ProviderActionIdRecord,
) -> Result<Entry, GithubIssueWorkflowError> {
    let entry = record_entry(PROVIDER_ACTION_ID_RECORD_KIND, record)?;
    with_index(
        entry,
        "provider_action_id",
        text(record.provider_action_id.as_str()),
    )
}

fn entry_for_provider_binding(
    binding: &GithubIssueProviderBinding,
) -> Result<Entry, GithubIssueWorkflowError> {
    let entry = record_entry(PROVIDER_BINDING_RECORD_KIND, binding)?;
    let entry = with_index(entry, "binding_id", text(binding.binding_id.as_str()))?;
    let entry = with_index(
        entry,
        "workflow_run_id",
        text(binding.workflow_run_id.as_str()),
    )?;
    with_index(entry, "provider_id", text(&binding.provider_id))
}

fn entry_for_provider_binding_id(
    record: &ProviderBindingIdRecord,
) -> Result<Entry, GithubIssueWorkflowError> {
    let entry = record_entry(PROVIDER_BINDING_ID_RECORD_KIND, record)?;
    with_index(entry, "binding_id", text(record.binding_id.as_str()))
}

fn record_entry<T>(kind: &'static str, value: &T) -> Result<Entry, GithubIssueWorkflowError>
where
    T: Serialize,
{
    let payload =
        serde_json::to_value(value).map_err(|error| durable_error("serialize record", error))?;
    let kind =
        RecordKind::new(kind).map_err(|error| durable_error("construct record kind", error))?;
    let entry =
        Entry::record(kind, &payload).map_err(|error| durable_error("serialize entry", error))?;
    Ok(entry)
}

fn with_index(
    entry: Entry,
    key: &'static str,
    value: IndexValue,
) -> Result<Entry, GithubIssueWorkflowError> {
    Ok(entry.with_indexed(index_key(key)?, value))
}

fn index_key(value: &'static str) -> Result<IndexKey, GithubIssueWorkflowError> {
    IndexKey::new(value).map_err(|error| durable_error("construct index key", error))
}

fn text(value: &str) -> IndexValue {
    IndexValue::Text(value.to_string())
}

fn durable_error(
    operation: &'static str,
    error: impl std::fmt::Display,
) -> GithubIssueWorkflowError {
    let error_type = std::any::type_name_of_val(&error);
    tracing::error!(
        operation,
        error_type,
        "GitHub issue workflow storage failed"
    );
    repository_error(format!(
        "storage unavailable while attempting to {operation}"
    ))
}

fn filesystem_error(operation: &'static str, error: FilesystemError) -> GithubIssueWorkflowError {
    durable_error(operation, error)
}

fn repository_error(reason: impl Into<String>) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::Repository {
        reason: reason.into(),
    }
}

fn is_terminal(status: &GithubIssueWorkflowRunStatus) -> bool {
    matches!(
        status,
        GithubIssueWorkflowRunStatus::Succeeded
            | GithubIssueWorkflowRunStatus::Failed
            | GithubIssueWorkflowRunStatus::Cancelled
    )
}

fn lease_is_claimable(run: &GithubIssueWorkflowRun, now: DateTime<Utc>) -> bool {
    run.lease_owner.is_none()
        || run
            .lease_expires_at
            .map(|expires_at| expires_at <= now)
            .unwrap_or(true)
}

fn lease_is_owned_by(
    run: &GithubIssueWorkflowRun,
    worker_id: &ironclaw_github_issue_workflow::WorkflowWorkerId,
    now: DateTime<Utc>,
) -> bool {
    run.lease_owner.as_ref() == Some(worker_id)
        && run
            .lease_expires_at
            .map(|expires_at| expires_at > now)
            .unwrap_or(false)
}

fn provider_action_is_complete(status: &ProviderActionStatus) -> bool {
    matches!(
        status,
        ProviderActionStatus::Succeeded
            | ProviderActionStatus::Failed
            | ProviderActionStatus::NeedsReconciliation
    )
}

fn provider_action_lease_is_claimable(
    action: &GithubIssueProviderActionRecord,
    now: DateTime<Utc>,
) -> bool {
    action.lease_owner.is_none()
        || action
            .lease_expires_at
            .map(|expires_at| expires_at <= now)
            .unwrap_or(true)
}

fn workflow_step_is_complete(status: &WorkflowStepStatus) -> bool {
    matches!(
        status,
        WorkflowStepStatus::Succeeded | WorkflowStepStatus::Failed | WorkflowStepStatus::Blocked
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkflowRunKeyRecord {
    workflow_run_id: GithubIssueWorkflowRunId,
    initial_run: GithubIssueWorkflowRun,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkflowEventKeyRecord {
    workflow_event_id: GithubIssueWorkflowEventId,
    workflow_run_id: GithubIssueWorkflowRunId,
    sequence: i64,
    initial_event: GithubIssueWorkflowEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EventSequenceRecord {
    last_sequence: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredStageRun {
    stage_run_id: GithubIssueStageRunId,
    workflow_run_id: GithubIssueWorkflowRunId,
    stage: GithubIssueStage,
    result: Option<JsonValue>,
    active: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkflowStepIdRecord {
    step_run_id: WorkflowStepRunId,
    workflow_run_id: GithubIssueWorkflowRunId,
    idempotency_key: WorkflowIdempotencyKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderActionIdRecord {
    provider_action_id: GithubIssueProviderActionId,
    workflow_run_id: GithubIssueWorkflowRunId,
    idempotency_key: WorkflowIdempotencyKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderBindingIdRecord {
    binding_id: GithubIssueProviderBindingId,
    binding: GithubIssueProviderBinding,
}
