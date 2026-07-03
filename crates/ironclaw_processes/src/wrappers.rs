//! Cross-cutting [`ProcessStore`] decorators.
//!
//! - [`EventingProcessStore`] wraps any inner store and emits redacted
//!   `RuntimeEvent`s on every lifecycle transition.
//! - [`ResourceManagedProcessStore`] reserves resources before `start`,
//!   tracks ownership, and reconciles or releases on terminal transitions.
//!
//! Both are composable: typical production stacks layer
//! `Eventing(ResourceManaged(InnerStore))`.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use ironclaw_events::{EventSink, RuntimeEvent};
use ironclaw_host_api::{ProcessId, ResourceReservationId, ResourceScope, ResourceUsage};
use ironclaw_resources::{ResourceError, ResourceGovernor};

use crate::types::{ProcessError, ProcessKey, ProcessRecord, ProcessStart, ProcessStore};

/// RAII guard that releases a reservation on `Drop` unless explicitly
/// [`defuse`](Self::defuse)d. Used to ensure a panic inside
/// [`ProcessStore::start`] does not leak the just-acquired reservation.
struct ReservationDropGuard<G>
where
    G: ResourceGovernor + ?Sized,
{
    governor: Option<Arc<G>>,
    reservation_id: ResourceReservationId,
}

impl<G> ReservationDropGuard<G>
where
    G: ResourceGovernor + ?Sized,
{
    fn new(governor: Arc<G>, reservation_id: ResourceReservationId) -> Self {
        Self {
            governor: Some(governor),
            reservation_id,
        }
    }

    fn defuse(mut self) {
        self.governor.take();
    }
}

impl<G> Drop for ReservationDropGuard<G>
where
    G: ResourceGovernor + ?Sized,
{
    fn drop(&mut self) {
        if let Some(governor) = self.governor.take() {
            let _ = governor.release(self.reservation_id);
        }
    }
}

pub struct EventingProcessStore<S>
where
    S: ProcessStore,
{
    inner: S,
    event_sink: Arc<dyn EventSink>,
}

impl<S> EventingProcessStore<S>
where
    S: ProcessStore,
{
    pub fn new(inner: S, event_sink: Arc<dyn EventSink>) -> Self {
        Self { inner, event_sink }
    }

    async fn emit_best_effort(&self, event: RuntimeEvent) {
        let _ = self.event_sink.emit(event).await;
    }
}

#[async_trait]
impl<S> ProcessStore for EventingProcessStore<S>
where
    S: ProcessStore,
{
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        let record = self.inner.start(start).await?;
        self.emit_best_effort(RuntimeEvent::process_started(
            record.scope.clone(),
            record.capability_id.clone(),
            record.extension_id.clone(),
            record.runtime,
            record.process_id,
        ))
        .await;
        Ok(record)
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let record = self.inner.complete(scope, process_id).await?;
        self.emit_best_effort(RuntimeEvent::process_completed(
            record.scope.clone(),
            record.capability_id.clone(),
            record.extension_id.clone(),
            record.runtime,
            record.process_id,
        ))
        .await;
        Ok(record)
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        let record = self.inner.fail(scope, process_id, error_kind).await?;
        self.emit_best_effort(RuntimeEvent::process_failed(
            record.scope.clone(),
            record.capability_id.clone(),
            record.extension_id.clone(),
            record.runtime,
            record.process_id,
            record
                .error_kind
                .clone()
                .unwrap_or_else(|| "Unknown".to_string()),
        ))
        .await;
        Ok(record)
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let record = self.inner.kill(scope, process_id).await?;
        self.emit_best_effort(RuntimeEvent::process_killed(
            record.scope.clone(),
            record.capability_id.clone(),
            record.extension_id.clone(),
            record.runtime,
            record.process_id,
        ))
        .await;
        Ok(record)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        self.inner.get(scope, process_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        self.inner.records_for_scope(scope).await
    }
}

pub struct ResourceManagedProcessStore<S, G>
where
    S: ProcessStore,
    G: ResourceGovernor + ?Sized,
{
    inner: S,
    governor: Arc<G>,
    completion_usage: ResourceUsage,
    owned_reservations: Mutex<HashMap<ProcessKey, ResourceReservationId>>,
}

impl<S, G> ResourceManagedProcessStore<S, G>
where
    S: ProcessStore,
    G: ResourceGovernor + ?Sized,
{
    pub fn new(inner: S, governor: Arc<G>) -> Self {
        Self {
            inner,
            governor,
            completion_usage: ResourceUsage::default(),
            owned_reservations: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_completion_usage(mut self, usage: ResourceUsage) -> Self {
        self.completion_usage = usage;
        self
    }

    fn owned_reservations_guard(
        &self,
    ) -> MutexGuard<'_, HashMap<ProcessKey, ResourceReservationId>> {
        self.owned_reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn record_owned_reservation(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        reservation_id: ResourceReservationId,
    ) {
        self.owned_reservations_guard()
            .insert(ProcessKey::new(scope, process_id), reservation_id);
    }

    fn take_owned_reservation(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        record_reservation_id: Option<ResourceReservationId>,
    ) -> Result<ResourceReservationId, ProcessError> {
        let reservation_id = self
            .owned_reservations_guard()
            .remove(&ProcessKey::new(scope, process_id))
            .ok_or(ProcessError::ResourceReservationNotOwned {
                process_id,
                reservation_id: record_reservation_id,
            })?;
        if Some(reservation_id) != record_reservation_id {
            self.owned_reservations_guard()
                .insert(ProcessKey::new(scope, process_id), reservation_id);
            return Err(ProcessError::ResourceReservationMismatch {
                process_id,
                expected: reservation_id,
                actual: record_reservation_id,
            });
        }
        Ok(reservation_id)
    }

    fn release_reservation(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<(), ResourceError> {
        self.governor.release(reservation_id)?;
        Ok(())
    }

    fn release_reservation_after_error(
        &self,
        reservation_id: ResourceReservationId,
        original: ProcessError,
    ) -> ProcessError {
        match self.release_reservation(reservation_id) {
            Ok(()) => original,
            Err(cleanup) => ProcessError::ResourceCleanupFailed {
                original: Box::new(original),
                cleanup,
            },
        }
    }

    fn reconcile_reservation(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<(), ResourceError> {
        self.governor
            .reconcile(reservation_id, self.completion_usage.clone())?;
        Ok(())
    }

    fn reconcile_reservation_after_error(
        &self,
        reservation_id: ResourceReservationId,
        original: ProcessError,
    ) -> ProcessError {
        match self.reconcile_reservation(reservation_id) {
            Ok(()) => original,
            Err(cleanup) => ProcessError::ResourceCleanupFailed {
                original: Box::new(original),
                cleanup,
            },
        }
    }
}

#[async_trait]
impl<S, G> ProcessStore for ResourceManagedProcessStore<S, G>
where
    S: ProcessStore,
    G: ResourceGovernor + ?Sized,
{
    async fn start(&self, mut start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        if let Some(reservation_id) = start.resource_reservation_id {
            return Err(ProcessError::ResourceReservationAlreadyAssigned {
                process_id: start.process_id,
                reservation_id,
            });
        }

        let reservation = self
            .governor
            .reserve(start.scope.clone(), start.estimated_resources.clone())?;
        start.resource_reservation_id = Some(reservation.id);
        let drop_guard = ReservationDropGuard::new(Arc::clone(&self.governor), reservation.id);
        let inner_result = self.inner.start(start).await;
        drop_guard.defuse();
        match inner_result {
            Ok(record) if record.resource_reservation_id == Some(reservation.id) => {
                self.record_owned_reservation(&record.scope, record.process_id, reservation.id);
                Ok(record)
            }
            Ok(record) => {
                let original = ProcessError::ResourceReservationMismatch {
                    process_id: record.process_id,
                    expected: reservation.id,
                    actual: record.resource_reservation_id,
                };
                Err(self.release_reservation_after_error(reservation.id, original))
            }
            Err(error) => Err(self.release_reservation_after_error(reservation.id, error)),
        }
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let current = self
            .inner
            .get(scope, process_id)
            .await?
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        let reservation_id =
            self.take_owned_reservation(scope, process_id, current.resource_reservation_id)?;
        let record = match self.inner.complete(scope, process_id).await {
            Ok(record) => record,
            Err(error) => {
                self.record_owned_reservation(scope, process_id, reservation_id);
                return Err(error);
            }
        };
        if record.resource_reservation_id != Some(reservation_id) {
            let original = ProcessError::ResourceReservationMismatch {
                process_id: record.process_id,
                expected: reservation_id,
                actual: record.resource_reservation_id,
            };
            return Err(self.reconcile_reservation_after_error(reservation_id, original));
        }
        self.reconcile_reservation(reservation_id)
            .map_err(ProcessError::Resource)?;
        Ok(record)
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        let current = self
            .inner
            .get(scope, process_id)
            .await?
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        let reservation_id =
            self.take_owned_reservation(scope, process_id, current.resource_reservation_id)?;
        let record = match self.inner.fail(scope, process_id, error_kind).await {
            Ok(record) => record,
            Err(error) => {
                self.record_owned_reservation(scope, process_id, reservation_id);
                return Err(error);
            }
        };
        if record.resource_reservation_id != Some(reservation_id) {
            let original = ProcessError::ResourceReservationMismatch {
                process_id: record.process_id,
                expected: reservation_id,
                actual: record.resource_reservation_id,
            };
            return Err(self.release_reservation_after_error(reservation_id, original));
        }
        self.release_reservation(reservation_id)
            .map_err(ProcessError::Resource)?;
        Ok(record)
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let current = self
            .inner
            .get(scope, process_id)
            .await?
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        let reservation_id =
            self.take_owned_reservation(scope, process_id, current.resource_reservation_id)?;
        let record = match self.inner.kill(scope, process_id).await {
            Ok(record) => record,
            Err(error) => {
                self.record_owned_reservation(scope, process_id, reservation_id);
                return Err(error);
            }
        };
        if record.resource_reservation_id != Some(reservation_id) {
            let original = ProcessError::ResourceReservationMismatch {
                process_id: record.process_id,
                expected: reservation_id,
                actual: record.resource_reservation_id,
            };
            return Err(self.release_reservation_after_error(reservation_id, original));
        }
        self.release_reservation(reservation_id)
            .map_err(ProcessError::Resource)?;
        Ok(record)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        self.inner.get(scope, process_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        self.inner.records_for_scope(scope).await
    }
}
