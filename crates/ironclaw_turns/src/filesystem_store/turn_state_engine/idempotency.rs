//! Idempotency bookkeeping: replay caches, persisted records, and pruning.
use super::*;

impl Inner {
    pub(super) fn remember_submit_idempotency(
        &mut self,
        key: SubmitIdempotencyKey,
        result: Result<SubmitTurnResponse, TurnError>,
        created_at: crate::TurnTimestamp,
    ) {
        if !self.submit_idempotency.contains_key(&key) {
            self.submit_idempotency_order.push_back(key.clone());
        }
        let record = submit_idempotency_record(&key, &result, created_at);
        self.remember_persisted_idempotency(record);
        self.submit_idempotency.insert(key, result);
        let removed = prune_ordered_map(
            &mut self.submit_idempotency,
            &mut self.submit_idempotency_order,
            self.limits.max_idempotency_records,
        );
        for key in removed {
            self.remove_persisted_submit_idempotency(&key);
        }
        self.prune_idempotency_records();
    }

    pub(super) fn remember_resume_idempotency(
        &mut self,
        key: RunIdempotencyKey,
        result: Result<ResumeTurnResponse, TurnError>,
        created_at: crate::TurnTimestamp,
    ) {
        if !self.resume_idempotency.contains_key(&key) {
            self.resume_idempotency_order.push_back(key.clone());
        }
        let record = resume_idempotency_record(&key, &result, created_at);
        self.remember_persisted_idempotency(record);
        self.resume_idempotency.insert(key, result);
        let removed = prune_ordered_map(
            &mut self.resume_idempotency,
            &mut self.resume_idempotency_order,
            self.limits.max_idempotency_records,
        );
        for key in removed {
            self.remove_persisted_run_idempotency(TurnIdempotencyOperationKind::Resume, &key);
        }
        self.prune_idempotency_records();
    }

    pub(super) fn remember_retry_idempotency(
        &mut self,
        key: RunIdempotencyKey,
        result: Result<RetryTurnResponse, TurnError>,
        created_at: crate::TurnTimestamp,
    ) {
        let replayable = !matches!(
            result,
            Err(TurnError::ThreadBusy(_) | TurnError::AdmissionRejected(_))
        );
        if !matches!(result, Err(TurnError::AdmissionRejected(_))) {
            let record = retry_idempotency_record(&key, &result, created_at);
            self.remember_persisted_idempotency(record);
        }
        if replayable {
            if !self.retry_idempotency.contains_key(&key) {
                self.retry_idempotency_order.push_back(key.clone());
            }
            self.retry_idempotency.insert(key, result);
            let removed = prune_ordered_map(
                &mut self.retry_idempotency,
                &mut self.retry_idempotency_order,
                self.limits.max_idempotency_records,
            );
            for key in removed {
                self.remove_persisted_run_idempotency(TurnIdempotencyOperationKind::Retry, &key);
            }
        }
        self.prune_idempotency_records();
    }

    pub(super) fn remember_cancel_idempotency(
        &mut self,
        key: RunIdempotencyKey,
        result: Result<CancelRunResponse, TurnError>,
        created_at: crate::TurnTimestamp,
    ) {
        if !self.cancel_idempotency.contains_key(&key) {
            self.cancel_idempotency_order.push_back(key.clone());
        }
        let record = cancel_idempotency_record(&key, &result, created_at);
        self.remember_persisted_idempotency(record);
        self.cancel_idempotency.insert(key, result);
        let removed = prune_ordered_map(
            &mut self.cancel_idempotency,
            &mut self.cancel_idempotency_order,
            self.limits.max_idempotency_records,
        );
        for key in removed {
            self.remove_persisted_run_idempotency(TurnIdempotencyOperationKind::Cancel, &key);
        }
        self.prune_idempotency_records();
    }

    fn remember_persisted_idempotency(&mut self, record: TurnIdempotencyRecord) {
        let key = persisted_key_for_record(&record);
        if !self.idempotency_records.contains_key(&key) {
            self.idempotency_record_order.push_back(key.clone());
        }
        self.idempotency_records.insert(key, record);
    }

    fn remove_persisted_submit_idempotency(&mut self, key: &SubmitIdempotencyKey) {
        self.idempotency_records.remove(&persisted_submit_key(key));
    }

    fn remove_persisted_run_idempotency(
        &mut self,
        operation: TurnIdempotencyOperationKind,
        key: &RunIdempotencyKey,
    ) {
        self.idempotency_records
            .remove(&persisted_run_key(operation, key));
    }

    fn prune_idempotency_records(&mut self) {
        let _removed = prune_ordered_map(
            &mut self.idempotency_records,
            &mut self.idempotency_record_order,
            self.limits.max_idempotency_records.saturating_mul(4),
        );
    }
}

pub(super) fn persisted_key_for_record(record: &TurnIdempotencyRecord) -> PersistedIdempotencyKey {
    PersistedIdempotencyKey {
        scope: record.scope.clone(),
        operation: record.operation,
        run_id: match record.operation {
            TurnIdempotencyOperationKind::Submit => None,
            TurnIdempotencyOperationKind::Resume
            | TurnIdempotencyOperationKind::Retry
            | TurnIdempotencyOperationKind::Cancel => record.run_id,
        },
        key: record.key.clone(),
    }
}

/// A persisted record whose replay payload no longer matches its operation
/// kind (or lacks a run id) cannot be rehydrated; the duplicate-request guard
/// for that key is lost until the operation is re-recorded. Surface it instead
/// of dropping silently. Logs metadata only — never the replay payload.
pub(super) fn debug_malformed_idempotency_record(record: &TurnIdempotencyRecord) {
    tracing::debug!(
        operation = ?record.operation,
        run_id = ?record.run_id,
        "skipping malformed idempotency record during snapshot load; replay guard lost for this key"
    );
}

fn persisted_submit_key(key: &SubmitIdempotencyKey) -> PersistedIdempotencyKey {
    PersistedIdempotencyKey {
        scope: key.scope.clone(),
        operation: TurnIdempotencyOperationKind::Submit,
        run_id: None,
        key: key.key.clone(),
    }
}

fn persisted_run_key(
    operation: TurnIdempotencyOperationKind,
    key: &RunIdempotencyKey,
) -> PersistedIdempotencyKey {
    PersistedIdempotencyKey {
        scope: key.scope.clone(),
        operation,
        run_id: Some(key.run_id),
        key: key.key.clone(),
    }
}

fn submit_idempotency_record(
    key: &SubmitIdempotencyKey,
    result: &Result<SubmitTurnResponse, TurnError>,
    created_at: crate::TurnTimestamp,
) -> TurnIdempotencyRecord {
    let (turn_id, run_id, outcome, replay) = match result {
        Ok(
            response @ SubmitTurnResponse::Accepted {
                turn_id, run_id, ..
            },
        ) => (
            Some(*turn_id),
            Some(*run_id),
            TurnIdempotencyOutcomeKind::Accepted,
            TurnIdempotencyReplay::SubmitAccepted(response.clone()),
        ),
        Err(TurnError::ThreadBusy(busy)) => (
            None,
            Some(busy.active_run_id),
            TurnIdempotencyOutcomeKind::ThreadBusy,
            TurnIdempotencyReplay::SubmitThreadBusy(busy.clone()),
        ),
        Err(TurnError::AdmissionRejected(rejection)) => (
            None,
            None,
            TurnIdempotencyOutcomeKind::AdmissionRejected,
            TurnIdempotencyReplay::SubmitAdmissionRejected(rejection.clone()),
        ),
        Err(error) => (
            None,
            None,
            TurnIdempotencyOutcomeKind::from_error(error),
            TurnIdempotencyReplay::Error(TurnIdempotencyErrorReplay::from_error(error)),
        ),
    };
    TurnIdempotencyRecord {
        scope: key.scope.clone(),
        operation: TurnIdempotencyOperationKind::Submit,
        key: key.key.clone(),
        turn_id,
        run_id,
        outcome,
        replay,
        created_at,
        expires_at: None,
    }
}

fn resume_idempotency_record(
    key: &RunIdempotencyKey,
    result: &Result<ResumeTurnResponse, TurnError>,
    created_at: crate::TurnTimestamp,
) -> TurnIdempotencyRecord {
    let (outcome, replay) = match result {
        Ok(response) => (
            TurnIdempotencyOutcomeKind::Resumed,
            TurnIdempotencyReplay::ResumeSucceeded(response.clone()),
        ),
        Err(error) => (
            TurnIdempotencyOutcomeKind::from_error(error),
            TurnIdempotencyReplay::Error(TurnIdempotencyErrorReplay::from_error(error)),
        ),
    };
    TurnIdempotencyRecord {
        scope: key.scope.clone(),
        operation: TurnIdempotencyOperationKind::Resume,
        key: key.key.clone(),
        turn_id: None,
        run_id: Some(key.run_id),
        outcome,
        replay,
        created_at,
        expires_at: None,
    }
}

fn retry_idempotency_record(
    key: &RunIdempotencyKey,
    result: &Result<RetryTurnResponse, TurnError>,
    created_at: crate::TurnTimestamp,
) -> TurnIdempotencyRecord {
    let (outcome, replay) = match result {
        Ok(response) => (
            TurnIdempotencyOutcomeKind::Retried,
            TurnIdempotencyReplay::RetrySucceeded(response.clone()),
        ),
        Err(TurnError::ThreadBusy(busy)) => (
            TurnIdempotencyOutcomeKind::ThreadBusy,
            TurnIdempotencyReplay::RetryThreadBusy(busy.clone()),
        ),
        Err(error) => (
            TurnIdempotencyOutcomeKind::from_error(error),
            TurnIdempotencyReplay::Error(TurnIdempotencyErrorReplay::from_error(error)),
        ),
    };
    TurnIdempotencyRecord {
        scope: key.scope.clone(),
        operation: TurnIdempotencyOperationKind::Retry,
        key: key.key.clone(),
        turn_id: None,
        run_id: Some(key.run_id),
        outcome,
        replay,
        created_at,
        expires_at: None,
    }
}

fn cancel_idempotency_record(
    key: &RunIdempotencyKey,
    result: &Result<CancelRunResponse, TurnError>,
    created_at: crate::TurnTimestamp,
) -> TurnIdempotencyRecord {
    let (outcome, replay) = match result {
        Ok(response) => (
            TurnIdempotencyOutcomeKind::CancelRecorded,
            TurnIdempotencyReplay::CancelRecorded(response.clone()),
        ),
        Err(error) => (
            TurnIdempotencyOutcomeKind::from_error(error),
            TurnIdempotencyReplay::Error(TurnIdempotencyErrorReplay::from_error(error)),
        ),
    };
    TurnIdempotencyRecord {
        scope: key.scope.clone(),
        operation: TurnIdempotencyOperationKind::Cancel,
        key: key.key.clone(),
        turn_id: None,
        run_id: Some(key.run_id),
        outcome,
        replay,
        created_at,
        expires_at: None,
    }
}

fn prune_ordered_map<K, V>(
    map: &mut HashMap<K, V>,
    order: &mut VecDeque<K>,
    max_len: usize,
) -> Vec<K>
where
    K: Eq + Hash,
{
    let mut removed = Vec::new();
    while map.len() > max_len {
        let Some(key) = order.pop_front() else {
            break;
        };
        if map.remove(&key).is_some() {
            removed.push(key);
        }
    }

    while order.front().is_some_and(|key| !map.contains_key(key)) {
        order.pop_front();
    }
    removed
}
