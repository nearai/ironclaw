//! Persistence-snapshot rehydration and capture for `Inner`.
use super::idempotency::{debug_malformed_idempotency_record, persisted_key_for_record};
use super::run_record::slot_info_for;
use super::*;

impl Inner {
    pub(super) fn from_persistence_snapshot(
        snapshot: TurnPersistenceSnapshot,
        limits: TurnStateStoreLimits,
    ) -> Result<Self, TurnError> {
        let mut cursor = 0;
        let turns = snapshot
            .turns
            .into_iter()
            .map(|record| (record.turn_id, record))
            .collect::<HashMap<_, _>>();
        let mut records = HashMap::new();
        let mut queued_runs = VecDeque::new();
        let mut terminal_runs = VecDeque::new();
        let mut active_locks = HashMap::new();
        for lock in snapshot.active_locks {
            active_locks.insert(lock.key.clone(), lock);
        }
        for run in snapshot.runs {
            cursor = cursor.max(run.event_cursor.0);
            let actor = turns
                .get(&run.turn_id)
                .map(|turn| turn.actor.clone())
                .ok_or_else(|| TurnError::Unavailable {
                    reason: "turn run references missing turn record".to_string(),
                })?;
            let has_non_queued_active_lock = active_locks
                .values()
                .any(|lock| lock.run_id == run.run_id && lock.status != TurnStatus::Queued);
            if run.status == TurnStatus::Queued && !has_non_queued_active_lock {
                queued_runs.push_back(run.run_id);
            }
            if run.status.is_terminal() {
                terminal_runs.push_back(run.run_id);
            }
            records.insert(
                run.run_id,
                RunRecord {
                    scope: run.scope,
                    actor,
                    turn_id: run.turn_id,
                    run_id: run.run_id,
                    status: RunStatusCell::new(run.status),
                    profile: run.profile,
                    resolved_model_route: run.resolved_model_route,
                    model_usage: run.model_usage,
                    accepted_message_ref: run.accepted_message_ref,
                    source_binding_ref: run.source_binding_ref,
                    reply_target_binding_ref: run.reply_target_binding_ref,
                    checkpoint_id: run.checkpoint_id,
                    gate_ref: run.gate_ref,
                    blocked_activity_id: run.blocked_activity_id,
                    credential_requirements: run.credential_requirements,
                    failure: run.failure,
                    event_cursor: run.event_cursor,
                    runner_id: run.runner_id,
                    lease_token: run.lease_token,
                    lease_expires_at: run.lease_expires_at,
                    last_heartbeat_at: run.last_heartbeat_at,
                    claim_count: run.claim_count,
                    received_at: run.received_at,
                    parent_run_id: run.parent_run_id,
                    subagent_depth: run.subagent_depth,
                    spawn_tree_root_run_id: run.spawn_tree_root_run_id,
                    product_context: run.product_context,
                    resume_disposition: run.resume_disposition,
                },
            );
        }

        let mut submit_idempotency = HashMap::new();
        let mut resume_idempotency = HashMap::new();
        let mut retry_idempotency = HashMap::new();
        let mut cancel_idempotency = HashMap::new();
        let mut idempotency_records = HashMap::new();
        let mut submit_idempotency_order = VecDeque::new();
        let mut resume_idempotency_order = VecDeque::new();
        let mut retry_idempotency_order = VecDeque::new();
        let mut cancel_idempotency_order = VecDeque::new();
        let mut idempotency_record_order = VecDeque::new();
        let mut ordered_idempotency_records = snapshot.idempotency_records;
        ordered_idempotency_records.sort_by_key(|record| record.created_at);
        for record in ordered_idempotency_records {
            let persisted_key = persisted_key_for_record(&record);
            idempotency_record_order.push_back(persisted_key.clone());
            idempotency_records.insert(persisted_key, record.clone());
            match record.operation {
                TurnIdempotencyOperationKind::Submit => {
                    if let Some(replay) = record.replay_submit() {
                        let key = SubmitIdempotencyKey {
                            scope: record.scope.clone(),
                            key: record.key.clone(),
                        };
                        submit_idempotency_order.push_back(key.clone());
                        submit_idempotency.insert(key, replay);
                    }
                }
                TurnIdempotencyOperationKind::Resume => {
                    if let (Some(run_id), Some(replay)) = (record.run_id, record.replay_resume()) {
                        let key = RunIdempotencyKey {
                            scope: record.scope.clone(),
                            run_id,
                            key: record.key.clone(),
                        };
                        resume_idempotency_order.push_back(key.clone());
                        resume_idempotency.insert(key, replay);
                    } else {
                        debug_malformed_idempotency_record(&record);
                    }
                }
                TurnIdempotencyOperationKind::Retry => {
                    if let (Some(run_id), Some(replay)) = (record.run_id, record.replay_retry()) {
                        let key = RunIdempotencyKey {
                            scope: record.scope.clone(),
                            run_id,
                            key: record.key.clone(),
                        };
                        retry_idempotency_order.push_back(key.clone());
                        retry_idempotency.insert(key, replay);
                    } else if record.run_id.is_some()
                        && matches!(record.replay, TurnIdempotencyReplay::RetryThreadBusy(_))
                    {
                        // Retry ThreadBusy records are retained in the durable snapshot for
                        // auditability, but are intentionally not replayable.
                    } else {
                        debug_malformed_idempotency_record(&record);
                    }
                }
                TurnIdempotencyOperationKind::Cancel => {
                    if let (Some(run_id), Some(replay)) = (record.run_id, record.replay_cancel()) {
                        let key = RunIdempotencyKey {
                            scope: record.scope.clone(),
                            run_id,
                            key: record.key.clone(),
                        };
                        cancel_idempotency_order.push_back(key.clone());
                        cancel_idempotency.insert(key, replay);
                    } else {
                        debug_malformed_idempotency_record(&record);
                    }
                }
            }
        }

        let loop_checkpoints = snapshot
            .loop_checkpoints
            .into_iter()
            .map(|record| (record.checkpoint_id, record))
            .collect::<HashMap<_, _>>();

        let events = snapshot.events;
        cursor = cursor.max(events.iter().map(|event| event.cursor.0).max().unwrap_or(0));
        cursor = cursor.max(snapshot.event_retention_floor.0);
        let mut admission_reservations = HashMap::new();
        for mut reservation in snapshot.admission_reservations {
            let Some(record) = records.get(&reservation.run_id) else {
                continue;
            };
            if record.status.get().is_terminal() {
                reservation.released = true;
            }
            admission_reservations.insert(reservation.run_id, reservation);
        }
        for record in records.values() {
            if record.status.get().keeps_active_lock() {
                let admission_class = record.profile.admission_class.clone();
                let buckets = admission_buckets(&record.scope, &record.actor, &admission_class);
                let needs_canonical_reservation = admission_reservations
                    .get(&record.run_id)
                    .is_none_or(|reservation| {
                        reservation.released
                            || reservation.admission_class != admission_class
                            || reservation.buckets != buckets
                    });
                if needs_canonical_reservation {
                    admission_reservations.insert(
                        record.run_id,
                        TurnAdmissionReservationRecord {
                            run_id: record.run_id,
                            admission_class,
                            buckets,
                            released: false,
                        },
                    );
                }
            }
        }

        let mut tree_reservations = HashMap::new();
        for reservation in snapshot.spawn_tree_reservations {
            tree_reservations.insert(
                SpawnTreeReservationKey::new(&reservation.scope, reservation.root_run_id),
                TreeReservationState {
                    count: reservation.descendant_count,
                    released_children: reservation.released_children,
                },
            );
        }

        let concurrency_limits = ConcurrencyLimits {
            max_concurrent_runs_per_user: limits.max_concurrent_runs_per_user,
            max_concurrent_trigger_runs: limits.max_concurrent_trigger_runs,
            max_concurrent_conversation_runs: limits.max_concurrent_conversation_runs,
        };
        let concurrency = ConcurrencyLimiter::rebuild_from(
            concurrency_limits,
            records
                .values()
                .filter(|record| holds_running_slot(record.status.get()))
                .map(|record| slot_info_for(record)),
        );

        Ok(Self {
            cursor,
            turns,
            records,
            queued_runs,
            terminal_runs,
            active_locks,
            checkpoints: snapshot.checkpoints,
            loop_checkpoints,
            submit_idempotency,
            submit_idempotency_in_flight: HashSet::new(),
            resume_idempotency,
            retry_idempotency,
            cancel_idempotency,
            idempotency_records,
            submit_idempotency_order,
            resume_idempotency_order,
            retry_idempotency_order,
            cancel_idempotency_order,
            idempotency_record_order,
            events,
            event_retention_floor: snapshot.event_retention_floor,
            admission_reservations,
            tree_reservations,
            limits,
            concurrency,
        })
    }

    pub(super) fn persistence_snapshot(&self) -> TurnPersistenceSnapshot {
        let mut turns = self.turns.values().cloned().collect::<Vec<_>>();
        turns.sort_by_key(|record| record.created_at);
        let mut runs = self
            .records
            .values()
            .map(RunRecord::persistence_record)
            .collect::<Vec<_>>();
        runs.sort_by_key(|record| record.event_cursor);
        let mut active_locks = self.active_locks.values().cloned().collect::<Vec<_>>();
        active_locks.sort_by_key(|record| record.acquired_at);
        let mut checkpoints = self.checkpoints.clone();
        checkpoints.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.sequence.cmp(&b.sequence))
        });
        let mut loop_checkpoints = self.loop_checkpoints.values().cloned().collect::<Vec<_>>();
        loop_checkpoints.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.checkpoint_id.as_uuid().cmp(&b.checkpoint_id.as_uuid()))
        });
        let mut idempotency_records = self
            .idempotency_records
            .values()
            .cloned()
            .collect::<Vec<_>>();
        idempotency_records.sort_by_key(|record| record.created_at);
        let mut admission_reservations = self
            .admission_reservations
            .values()
            .cloned()
            .collect::<Vec<_>>();
        admission_reservations.sort_by_key(|reservation| reservation.run_id.to_string());
        let mut spawn_tree_reservations = self
            .tree_reservations
            .iter()
            .filter_map(|(key, state)| {
                let root = self.records.get(&key.root_run_id)?;
                Some(SpawnTreeReservation {
                    scope: root.scope.clone(),
                    root_run_id: key.root_run_id,
                    descendant_count: state.count,
                    released_children: state.released_children.clone(),
                })
            })
            .collect::<Vec<_>>();
        spawn_tree_reservations.sort_by_key(|reservation| reservation.root_run_id.to_string());
        TurnPersistenceSnapshot {
            turns,
            runs,
            active_locks,
            checkpoints,
            loop_checkpoints,
            idempotency_records,
            events: self.events.clone(),
            event_retention_floor: self.event_retention_floor,
            admission_reservations,
            spawn_tree_reservations,
        }
    }
}
