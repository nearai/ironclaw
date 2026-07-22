use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    Authorized, CapabilityAuthorizer, CapabilityId, Invocation, ProcessId, Timestamp,
};
use ironclaw_processes::{ProcessError, ProcessExecutionRequest, ProcessStatus, ProcessStore};
use thiserror::Error;

/// Kernel-owned re-minting surface for detached process continuations.
///
/// This does not authorize. It re-seals only the durable spawn decision that
/// `spawn_json` persisted with the process record, after reloading the record
/// and validating that the executor request still matches those persisted
/// facts.
#[async_trait]
pub trait ProcessAuthorizationRemintPort: Send + Sync {
    async fn remint(
        &self,
        request: &ProcessExecutionRequest,
    ) -> Result<Authorized, ProcessAuthorizationRemintError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProcessAuthorizationRemintError {
    #[error("capability {capability} has no sealed process authorization")]
    MissingProcessAuthorization { capability: CapabilityId },
    #[error("failed to load process {process_id}: {reason}")]
    ProcessLookup {
        process_id: ProcessId,
        reason: String,
    },
    #[error("unknown process {process_id}")]
    UnknownProcess { process_id: ProcessId },
    #[error("process {process_id} is not running")]
    ProcessNotRunning { process_id: ProcessId },
    #[error("process authorization record mismatch for {field}")]
    RecordMismatch { field: &'static str },
}

impl ProcessAuthorizationRemintError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::MissingProcessAuthorization { .. } => "missing_process_authorization",
            Self::ProcessLookup { .. } => "process_authorization_lookup_failed",
            Self::UnknownProcess { .. } => "unknown_process",
            Self::ProcessNotRunning { .. } => "process_not_running",
            Self::RecordMismatch { .. } => "process_authorization_mismatch",
        }
    }
}

pub fn process_authorization_remint_port(
    process_store: Arc<dyn ProcessStore>,
) -> Arc<dyn ProcessAuthorizationRemintPort> {
    Arc::new(StoreBackedProcessAuthorizationReminter { process_store })
}

#[derive(Clone)]
struct StoreBackedProcessAuthorizationReminter {
    process_store: Arc<dyn ProcessStore>,
}

impl CapabilityAuthorizer for StoreBackedProcessAuthorizationReminter {}

#[async_trait]
impl ProcessAuthorizationRemintPort for StoreBackedProcessAuthorizationReminter {
    async fn remint(
        &self,
        request: &ProcessExecutionRequest,
    ) -> Result<Authorized, ProcessAuthorizationRemintError> {
        let continuation = request.authorized_continuation.as_ref().ok_or_else(|| {
            ProcessAuthorizationRemintError::MissingProcessAuthorization {
                capability: request.capability_id.clone(),
            }
        })?;
        let record = self
            .process_store
            .get(&request.scope, request.process_id)
            .await
            .map_err(|error| process_lookup_error(request.process_id, error))?
            .ok_or(ProcessAuthorizationRemintError::UnknownProcess {
                process_id: request.process_id,
            })?;

        if record.status != ProcessStatus::Running {
            return Err(ProcessAuthorizationRemintError::ProcessNotRunning {
                process_id: request.process_id,
            });
        }
        ensure_eq(&record.process_id, &request.process_id, "process_id")?;
        ensure_eq(
            &record.invocation_id,
            &request.invocation_id,
            "invocation_id",
        )?;
        ensure_eq(&record.scope, &request.scope, "scope")?;
        ensure_eq(
            &record.authenticated_actor_user_id,
            &request.authenticated_actor_user_id,
            "authenticated_actor_user_id",
        )?;
        ensure_eq(&record.extension_id, &request.extension_id, "extension_id")?;
        ensure_eq(
            &record.capability_id,
            &request.capability_id,
            "capability_id",
        )?;
        ensure_eq(&record.runtime, &request.runtime, "runtime")?;
        ensure_eq(
            &record.estimated_resources,
            &request.estimate,
            "estimated_resources",
        )?;
        ensure_eq(&record.mounts, &request.mounts, "mounts")?;
        ensure_eq(
            &record.resource_reservation_id,
            &request
                .resource_reservation
                .as_ref()
                .map(|reservation| reservation.id),
            "resource_reservation_id",
        )?;
        ensure_eq(
            &record.authorized_continuation.as_ref(),
            &Some(continuation),
            "authorized_continuation",
        )?;
        ensure_eq(
            &continuation.invocation.process_id,
            &request.process_id,
            "continuation.process_id",
        )?;
        ensure_eq(
            &continuation.invocation.capability,
            &request.capability_id,
            "continuation.capability",
        )?;
        ensure_eq(
            &continuation.invocation.scope,
            &request.scope,
            "continuation.scope",
        )?;
        ensure_eq(
            &continuation.invocation.estimate,
            &request.estimate,
            "continuation.estimate",
        )?;
        ensure_eq(
            &continuation.resource_reservation,
            &request.resource_reservation,
            "continuation.resource_reservation",
        )?;
        if let Some(mounts) = &continuation.mounts {
            ensure_eq(mounts, &request.mounts, "continuation.mounts")?;
        }

        let continuation = continuation.clone();
        let ironclaw_host_api::ProcessAuthorizedContinuation {
            invocation,
            lane,
            mounts,
            resource_reservation,
        } = continuation;
        let invocation = Invocation {
            activity_id: invocation.activity_id,
            capability: invocation.capability,
            input: request.input.clone(),
            scope: invocation.scope,
            actor: invocation.actor,
            origin: invocation.origin,
            estimate: invocation.estimate,
            correlation_id: invocation.correlation_id,
            process_id: Some(invocation.process_id),
            parent_process_id: invocation.parent_process_id,
        };
        Ok(Authorized::seal(
            self.authorization_grant(),
            invocation,
            lane,
            mounts,
            resource_reservation,
            Timestamp::MAX_UTC,
        ))
    }
}

fn process_lookup_error(
    process_id: ProcessId,
    error: ProcessError,
) -> ProcessAuthorizationRemintError {
    ProcessAuthorizationRemintError::ProcessLookup {
        process_id,
        reason: error.to_string(),
    }
}

fn ensure_eq<T: PartialEq>(
    actual: &T,
    expected: &T,
    field: &'static str,
) -> Result<(), ProcessAuthorizationRemintError> {
    if actual == expected {
        Ok(())
    } else {
        Err(ProcessAuthorizationRemintError::RecordMismatch { field })
    }
}
