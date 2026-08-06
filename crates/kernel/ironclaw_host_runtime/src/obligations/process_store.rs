//! The process-obligation store — one of the three chartered owners of the
//! obligation module (PROPOSAL §6.5.9, CHECKLIST WS3).
//!
//! Once a capability process starts, obligation cleanup stops being the
//! handler's business and becomes the process lifecycle's: staged handoffs are
//! discarded and a prepared resource reservation is reconciled or released when
//! the process reaches a terminal state. This module owns that wrapper. It
//! stages nothing itself (see [`super::staged_handoffs`]) and decides nothing
//! about which obligations apply (see [`super::handler`]).

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use ironclaw_event_log::{EventSink, RuntimeEvent};
use ironclaw_host_api::{
    ids::{CapabilityId, ProcessId},
    resource::{ResourceScope, ResourceUsage},
};
use ironclaw_processes::{
    ProcessError, ProcessJournalCommit, ProcessJournalCommitObserver, ProcessJournalKind,
    ProcessKind, ProcessRecord, ProcessRuntimePort, ProcessStart, ProcessSubmissionLifecycle,
    capability_process_record, complete_capability_process, fail_capability_process,
    process_record_from_snapshot, submit_capability_process,
};
use ironclaw_resources::{ResourceError, ResourceGovernor};

use super::staged_handoffs::{NetworkObligationPolicyStore, RuntimeSecretInjectionStore};

/// Process-store wrapper that owns spawn-phase obligation handoffs after
/// process submission succeeds.
///
/// `CapabilityHost` aborts prepared effects when process start fails. Once
/// start succeeds, this wrapper becomes responsible for discarding staged
/// network/secret handoffs and reconciling or releasing a prepared resource
/// reservation when the process reaches a terminal state.
pub struct ProcessObligationLifecycleStore {
    processes: Arc<dyn ProcessRuntimePort>,
    network_policies: Arc<NetworkObligationPolicyStore>,
    secret_injections: Arc<RuntimeSecretInjectionStore>,
    resource_governor: Mutex<Arc<dyn ResourceGovernor>>,
    event_sink: Mutex<Option<Arc<dyn EventSink>>>,
    observer_registered: AtomicBool,
    active_process_handoffs: Mutex<HashMap<ProcessObligationHandoffKey, ProcessId>>,
    cleaned_process_handoffs: Mutex<HashSet<ProcessObligationProcessKey>>,
}

impl ProcessObligationLifecycleStore {
    pub(crate) fn new<S>(
        inner: Arc<S>,
        network_policies: Arc<NetworkObligationPolicyStore>,
        secret_injections: Arc<RuntimeSecretInjectionStore>,
        resource_governor: Arc<dyn ResourceGovernor>,
    ) -> Self
    where
        S: ProcessRuntimePort + 'static,
    {
        let inner: Arc<dyn ProcessRuntimePort> = inner;
        Self::from_dyn(
            inner,
            network_policies,
            secret_injections,
            resource_governor,
        )
    }

    pub(crate) fn from_dyn(
        processes: Arc<dyn ProcessRuntimePort>,
        network_policies: Arc<NetworkObligationPolicyStore>,
        secret_injections: Arc<RuntimeSecretInjectionStore>,
        resource_governor: Arc<dyn ResourceGovernor>,
    ) -> Self {
        Self {
            processes,
            network_policies,
            secret_injections,
            resource_governor: Mutex::new(resource_governor),
            event_sink: Mutex::new(None),
            observer_registered: AtomicBool::new(false),
            active_process_handoffs: Mutex::new(HashMap::new()),
            cleaned_process_handoffs: Mutex::new(HashSet::new()),
        }
    }

    pub(crate) fn set_resource_governor(&self, resource_governor: Arc<dyn ResourceGovernor>) {
        match self.resource_governor.lock() {
            Ok(mut slot) => {
                *slot = resource_governor;
            }
            Err(error) => {
                tracing::error!(error = %error, "process resource governor registry unavailable");
            }
        }
    }

    #[doc(hidden)]
    pub fn register_journal_observer(
        self: &Arc<Self>,
        runtime: &dyn ProcessRuntimePort,
    ) -> Result<(), String> {
        if self
            .observer_registered
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }
        let observer: Arc<dyn ProcessJournalCommitObserver> = self.clone();
        if let Err(error) = runtime.subscribe_process_observer(observer) {
            self.observer_registered.store(false, Ordering::Release);
            return Err(error);
        }
        Ok(())
    }

    /// Attaches a best-effort event sink for process lifecycle transitions.
    pub fn set_event_sink(&self, event_sink: Arc<dyn EventSink>) {
        match self.event_sink.lock() {
            Ok(mut slot) => {
                *slot = Some(event_sink);
            }
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    "process lifecycle event sink registry unavailable"
                );
            }
        }
    }

    async fn emit_process_event(&self, event: RuntimeEvent) {
        let event_sink = match self.event_sink.lock() {
            Ok(slot) => slot.clone(),
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    "process lifecycle event sink registry unavailable"
                );
                None
            }
        };
        if let Some(event_sink) = event_sink
            && let Err(error) = event_sink.emit(event).await
        {
            tracing::debug!(?error, "best-effort process lifecycle event emit failed");
        }
    }

    /// Discards staged obligation handoffs and closes any reservation for an
    /// executor that finished but could not publish its result record.
    pub async fn cleanup_process_obligations(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        reconcile: bool,
    ) -> Result<(), ProcessError> {
        if let Some(record) =
            capability_process_record(self.processes.as_ref(), scope, process_id).await?
        {
            self.cleanup_record_obligations(&record, reconcile)?;
            self.release_active_process_handoff(&record)?;
            self.mark_process_handoff_cleaned(&record)?;
        }
        Ok(())
    }

    fn has_process_obligations(&self, start: &ProcessStart) -> Result<bool, ProcessError> {
        let has_secret_handoff = self
            .secret_injections
            .has_for_capability(&start.scope, &start.capability_id)
            .map_err(|_| ProcessError::InvalidStoredRecord {
                reason: "process obligation handoff lookup failed".to_string(),
            })?;
        Ok(start.resource_reservation_id.is_some()
            || self
                .network_policies
                .contains(&start.scope, &start.capability_id)
            || has_secret_handoff)
    }

    fn claim_active_process_handoff(&self, start: &ProcessStart) -> Result<bool, ProcessError> {
        if !self.has_process_obligations(start)? {
            return Ok(false);
        }

        let key = ProcessObligationHandoffKey::new(&start.scope, &start.capability_id);
        let mut active =
            self.active_process_handoffs
                .lock()
                .map_err(|_| ProcessError::InvalidStoredRecord {
                    reason: "process obligation handoff registry unavailable".to_string(),
                })?;
        if let Some(existing_process_id) = active.get(&key) {
            return Err(ProcessError::InvalidStoredRecord {
                reason: format!(
                    "process obligation handoff already active for scoped capability: {existing_process_id}"
                ),
            });
        }
        active.insert(key, start.process_id);
        Ok(true)
    }

    fn release_claimed_process_handoff(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        process_id: ProcessId,
    ) -> Result<(), ProcessError> {
        let key = ProcessObligationHandoffKey::new(scope, capability_id);
        let mut active =
            self.active_process_handoffs
                .lock()
                .map_err(|_| ProcessError::InvalidStoredRecord {
                    reason: "process obligation handoff registry unavailable".to_string(),
                })?;
        if active.get(&key) == Some(&process_id) {
            active.remove(&key);
        }
        Ok(())
    }

    fn release_active_process_handoff(&self, record: &ProcessRecord) -> Result<(), ProcessError> {
        self.release_claimed_process_handoff(
            &record.scope,
            &record.capability_id,
            record.process_id,
        )
    }

    fn has_active_process_handoff(&self, record: &ProcessRecord) -> Result<bool, ProcessError> {
        let key = ProcessObligationHandoffKey::new(&record.scope, &record.capability_id);
        let active =
            self.active_process_handoffs
                .lock()
                .map_err(|_| ProcessError::InvalidStoredRecord {
                    reason: "process obligation handoff registry unavailable".to_string(),
                })?;
        Ok(active.get(&key) == Some(&record.process_id))
    }

    fn process_handoff_cleaned(&self, record: &ProcessRecord) -> Result<bool, ProcessError> {
        let key = ProcessObligationProcessKey::new(&record.scope, record.process_id);
        let cleaned = self.cleaned_process_handoffs.lock().map_err(|_| {
            ProcessError::InvalidStoredRecord {
                reason: "process obligation cleanup registry unavailable".to_string(),
            }
        })?;
        Ok(cleaned.contains(&key))
    }

    fn mark_process_handoff_cleaned(&self, record: &ProcessRecord) -> Result<(), ProcessError> {
        let key = ProcessObligationProcessKey::new(&record.scope, record.process_id);
        let mut cleaned = self.cleaned_process_handoffs.lock().map_err(|_| {
            ProcessError::InvalidStoredRecord {
                reason: "process obligation cleanup registry unavailable".to_string(),
            }
        })?;
        cleaned.insert(key);
        Ok(())
    }

    fn has_staged_handoffs(&self, record: &ProcessRecord) -> Result<bool, ProcessError> {
        let has_secret_handoff = self
            .secret_injections
            .has_for_capability(&record.scope, &record.capability_id)
            .map_err(|_| ProcessError::InvalidStoredRecord {
                reason: "process obligation handoff lookup failed".to_string(),
            })?;
        Ok(self
            .network_policies
            .contains(&record.scope, &record.capability_id)
            || has_secret_handoff)
    }

    fn cleanup_terminal(
        &self,
        record: &ProcessRecord,
        reconcile: bool,
    ) -> Result<(), ProcessError> {
        if let Err(error) = self.cleanup_record_obligations(record, reconcile) {
            // `debug!`, not `warn!`: `cleanup_terminal` runs from
            // `observe_process_commit`, a background journal callback, and
            // `warn!`/`info!` from a background task corrupt the REPL/TUI
            // display. The error is not swallowed — it is returned on the next
            // line, so the caller still sees the failure.
            tracing::debug!(
                process_id = %record.process_id,
                tenant_id = %record.scope.tenant_id,
                user_id = %record.scope.user_id,
                reconcile,
                error = %error,
                "process obligation cleanup failed after terminal transition"
            );
            return Err(error);
        }
        self.release_active_process_handoff(record)?;
        self.mark_process_handoff_cleaned(record)?;
        Ok(())
    }

    fn cleanup_record_obligations(
        &self,
        record: &ProcessRecord,
        reconcile: bool,
    ) -> Result<(), ProcessError> {
        if self.process_handoff_cleaned(record)? {
            return Ok(());
        }
        let should_cleanup_handoffs = self.has_active_process_handoff(record)?
            || record.resource_reservation_id.is_some()
            || self.has_staged_handoffs(record)?;
        if should_cleanup_handoffs {
            self.network_policies
                .discard_for_capability(&record.scope, &record.capability_id);
            self.secret_injections
                .discard_for_capability(&record.scope, &record.capability_id)
                .map_err(|_| ProcessError::InvalidStoredRecord {
                    reason: "process obligation handoff cleanup failed".to_string(),
                })?;
        }
        if let Some(reservation_id) = record.resource_reservation_id {
            let governor =
                self.resource_governor
                    .lock()
                    .map_err(|_| ProcessError::InvalidStoredRecord {
                        reason: "process resource governor registry unavailable".to_string(),
                    })?;
            if reconcile {
                close_reservation_once(
                    governor.reconcile(reservation_id, ResourceUsage::default()),
                )?;
            } else {
                close_reservation_once(governor.release(reservation_id))?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl ProcessJournalCommitObserver for ProcessObligationLifecycleStore {
    fn process_observer_id(&self) -> &'static str {
        "process-obligation-lifecycle-v1"
    }

    async fn observe_process_commit(&self, commit: ProcessJournalCommit) -> Result<(), String> {
        if commit.state.process_kind != ProcessKind::CapabilityInvocation {
            return Ok(());
        }
        let record =
            process_record_from_snapshot(commit.state).map_err(|error| error.to_string())?;
        match commit.kind {
            ProcessJournalKind::Completed => {
                self.emit_process_event(RuntimeEvent::process_completed(
                    record.scope.clone(),
                    record.capability_id.clone(),
                    record.extension_id.clone(),
                    record.runtime,
                    record.process_id,
                ))
                .await;
                self.cleanup_terminal(&record, true)
                    .map_err(|error| error.to_string())?;
            }
            ProcessJournalKind::Failed => {
                self.emit_process_event(RuntimeEvent::process_failed(
                    record.scope.clone(),
                    record.capability_id.clone(),
                    record.extension_id.clone(),
                    record.runtime,
                    record.process_id,
                    record
                        .error_kind
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string()),
                ))
                .await;
                self.cleanup_terminal(&record, false)
                    .map_err(|error| error.to_string())?;
            }
            ProcessJournalKind::Stopped
            | ProcessJournalKind::Cancelled
            | ProcessJournalKind::Killed
            | ProcessJournalKind::RecoveryRequired => {
                self.emit_process_event(RuntimeEvent::process_killed(
                    record.scope.clone(),
                    record.capability_id.clone(),
                    record.extension_id.clone(),
                    record.runtime,
                    record.process_id,
                ))
                .await;
                self.cleanup_terminal(&record, false)
                    .map_err(|error| error.to_string())?;
            }
            _ => {}
        }
        Ok(())
    }
}

#[async_trait]
impl ProcessSubmissionLifecycle for ProcessObligationLifecycleStore {
    async fn before_submit(&self, start: &ProcessStart) -> Result<(), ProcessError> {
        self.claim_active_process_handoff(start).map(|_| ())
    }

    async fn submit_failed(&self, start: &ProcessStart) -> Result<(), ProcessError> {
        self.release_claimed_process_handoff(&start.scope, &start.capability_id, start.process_id)
    }

    async fn submitted(&self, record: &ProcessRecord) -> Result<(), ProcessError> {
        self.emit_process_event(RuntimeEvent::process_started(
            record.scope.clone(),
            record.capability_id.clone(),
            record.extension_id.clone(),
            record.runtime,
            record.process_id,
        ))
        .await;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProcessObligationHandoffKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    invocation_id: String,
    capability_id: String,
}

impl ProcessObligationHandoffKey {
    fn new(scope: &ResourceScope, capability_id: &CapabilityId) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
            mission_id: scope.mission_id.as_ref().map(|id| id.as_str().to_string()),
            thread_id: scope.thread_id.as_ref().map(|id| id.as_str().to_string()),
            invocation_id: scope.invocation_id.to_string(),
            capability_id: capability_id.as_str().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProcessObligationProcessKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    process_id: ProcessId,
}

impl ProcessObligationProcessKey {
    fn new(scope: &ResourceScope, process_id: ProcessId) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
            mission_id: scope.mission_id.as_ref().map(|id| id.as_str().to_string()),
            thread_id: scope.thread_id.as_ref().map(|id| id.as_str().to_string()),
            process_id,
        }
    }
}

impl ProcessObligationLifecycleStore {
    pub fn process_runtime(&self) -> Arc<dyn ProcessRuntimePort> {
        Arc::clone(&self.processes)
    }

    pub async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        let claimed = self.claim_active_process_handoff(&start)?;
        let process_id = start.process_id;
        let scope = start.scope.clone();
        let capability_id = start.capability_id.clone();
        match submit_capability_process(self.processes.as_ref(), start).await {
            Ok(record) => {
                self.emit_process_event(RuntimeEvent::process_started(
                    record.scope.clone(),
                    record.capability_id.clone(),
                    record.extension_id.clone(),
                    record.runtime,
                    record.process_id,
                ))
                .await;
                Ok(record)
            }
            Err(error) => {
                if claimed {
                    self.release_claimed_process_handoff(&scope, &capability_id, process_id)?;
                }
                Err(error)
            }
        }
    }

    pub async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let record =
            complete_capability_process(self.processes.as_ref(), scope, process_id).await?;
        self.cleanup_terminal(&record, true)?;
        Ok(record)
    }

    pub async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        let record =
            fail_capability_process(self.processes.as_ref(), scope, process_id, error_kind).await?;
        self.cleanup_terminal(&record, false)?;
        Ok(record)
    }

    pub async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let result = self
            .processes
            .kill_process(ironclaw_processes::KillProcessRequest {
                scope: scope.clone(),
                process_id,
                operation_id: None,
                reason: None,
            })
            .await
            .map_err(|error| ProcessError::InvalidStoredRecord {
                reason: error.to_string(),
            })?;
        let record = process_record_from_snapshot(result.state)?;
        self.cleanup_terminal(&record, false)?;
        Ok(record)
    }

    pub async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        capability_process_record(self.processes.as_ref(), scope, process_id).await
    }

    pub async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        self.processes
            .process_snapshots(scope)
            .await
            .map_err(|error| ProcessError::InvalidStoredRecord {
                reason: error.to_string(),
            })?
            .into_iter()
            .filter(|snapshot| snapshot.process_kind == ProcessKind::CapabilityInvocation)
            .map(process_record_from_snapshot)
            .collect()
    }
}

fn close_reservation_once<T>(result: Result<T, ResourceError>) -> Result<(), ProcessError> {
    match result {
        Ok(_) => Ok(()),
        Err(ResourceError::ReservationClosed { .. }) => Ok(()),
        Err(ResourceError::UnknownReservation { .. }) => Ok(()),
        Err(error) => Err(error.into()),
    }
}
