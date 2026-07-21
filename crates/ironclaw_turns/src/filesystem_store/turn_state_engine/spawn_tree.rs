//! `TurnSpawnTreeStateStore` trait implementation for `TurnStateEngine`.
use super::*;
use async_trait::async_trait;

#[async_trait]
impl TurnSpawnTreeStateStore for TurnStateEngine {
    async fn submit_child_turn(
        &self,
        request: SubmitChildRunRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let idempotency_key = SubmitIdempotencyKey {
            scope: request.child_scope.clone(),
            key: request.idempotency_key.clone(),
        };
        if let Some(result) = self
            .wait_for_or_claim_submit_idempotency(&idempotency_key)
            .await?
        {
            return result;
        }
        let _in_flight_guard = SubmitInFlightGuard::new(
            &self.inner,
            &self.submit_idempotency_ready,
            idempotency_key.clone(),
        );

        let submit_template = {
            let mut inner = self.lock_inner()?;
            if let Some(result) = inner.submit_idempotency.get(&idempotency_key).cloned() {
                return result;
            }
            let Some(parent) = inner
                .records
                .get(&request.parent_run_id)
                .filter(|record| record.scope == request.parent_scope)
            else {
                let response = Err(TurnError::ScopeNotFound);
                inner.remember_submit_idempotency(
                    idempotency_key.clone(),
                    response.clone(),
                    request.received_at,
                );
                return response;
            };
            if !same_scope_envelope(&parent.scope, &request.child_scope) {
                let response = Err(TurnError::Unauthorized);
                inner.remember_submit_idempotency(
                    idempotency_key.clone(),
                    response.clone(),
                    request.received_at,
                );
                return response;
            }
            if parent.subagent_depth == u32::MAX {
                let response = Err(invalid_lineage("subagent depth would overflow"));
                inner.remember_submit_idempotency(
                    idempotency_key.clone(),
                    response.clone(),
                    request.received_at,
                );
                return response;
            }
            SubmitTurnRequest {
                requested_model: None,
                scope: request.child_scope.clone(),
                actor: request.actor.clone(),
                accepted_message_ref: request.accepted_message_ref.clone(),
                source_binding_ref: request.source_binding_ref.clone(),
                reply_target_binding_ref: request.reply_target_binding_ref.clone(),
                requested_run_profile: request.requested_run_profile.clone(),
                idempotency_key: request.idempotency_key.clone(),
                received_at: request.received_at,
                requested_run_id: request.requested_run_id,
                parent_run_id: Some(parent.run_id),
                subagent_depth: parent.subagent_depth + 1,
                spawn_tree_root_run_id: Some(
                    parent.spawn_tree_root_run_id.unwrap_or(parent.run_id),
                ),
                product_context: parent.product_context.clone(),
            }
        };

        let admission_result = admission_policy.check_submit(&submit_template);
        {
            let mut inner = self.lock_inner()?;
            if let Some(result) = inner.submit_idempotency.get(&idempotency_key).cloned() {
                return result;
            }
            if let Err(rejection) = admission_result {
                let response = Err(TurnError::AdmissionRejected(rejection));
                inner.remember_submit_idempotency(
                    idempotency_key.clone(),
                    response.clone(),
                    request.received_at,
                );
                return response;
            }
        }

        let profile_resolution = run_profile_resolver
            .resolve_run_profile(RunProfileResolutionRequest {
                requested_run_profile: submit_template.requested_run_profile.clone(),
                ..RunProfileResolutionRequest::interactive_default()
            })
            .await;

        let mut inner = self.lock_inner()?;
        if let Some(result) = inner.submit_idempotency.get(&idempotency_key).cloned() {
            return result;
        }
        let profile = match profile_resolution {
            Ok(resolved) => TurnRunProfile::from_resolved(resolved),
            Err(error) => {
                let response = Err(profile_resolution_error_to_turn_error(error));
                inner.remember_submit_idempotency(
                    idempotency_key.clone(),
                    response.clone(),
                    request.received_at,
                );
                return response;
            }
        };

        let Some(parent) = inner
            .records
            .get(&request.parent_run_id)
            .filter(|record| record.scope == request.parent_scope)
        else {
            let response = Err(TurnError::ScopeNotFound);
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            return response;
        };
        if !same_scope_envelope(&parent.scope, &request.child_scope) {
            let response = Err(TurnError::Unauthorized);
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            return response;
        }
        if parent.subagent_depth == u32::MAX {
            let response = Err(invalid_lineage("subagent depth would overflow"));
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            return response;
        }
        let parent_run_id = parent.run_id;
        let subagent_depth = parent.subagent_depth + 1;
        let root_run_id = parent.spawn_tree_root_run_id.unwrap_or(parent.run_id);
        let parent_product_context = parent.product_context.clone();
        let Some(root) = inner.records.get(&root_run_id) else {
            let response = Err(TurnError::ScopeNotFound);
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            return response;
        };
        if !same_scope_envelope(&root.scope, &request.child_scope) {
            let response = Err(TurnError::Unauthorized);
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            return response;
        }
        if root.spawn_tree_root_run_id.unwrap_or(root.run_id) != root.run_id {
            let response = Err(invalid_lineage(
                "root_run_id must identify the spawn tree root",
            ));
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            return response;
        }

        let lock_key = TurnActiveLockKey::from(&request.child_scope);
        if let Some(response) = inner.thread_busy(&lock_key) {
            return Err(TurnError::ThreadBusy(response));
        }
        let run_id = request.requested_run_id.unwrap_or_else(fresh_turn_run_id);
        if inner.records.contains_key(&run_id) {
            let response = Err(TurnError::Conflict {
                reason: "requested_run_id already bound".to_string(),
            });
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            return response;
        }

        let reservation_key = SpawnTreeReservationKey::new(&request.child_scope, root_run_id);
        let previous_state = inner
            .tree_reservations
            .get(&reservation_key)
            .cloned()
            .unwrap_or_default();
        let previous_tree_count = previous_state.count;
        let next_tree_count = previous_tree_count.checked_add(1).ok_or_else(|| {
            TurnError::capacity_exceeded(
                TurnCapacityResource::SpawnTreeDescendants,
                u64::from(request.spawn_tree_descendant_cap),
            )
        });
        let next_tree_count = match next_tree_count {
            Ok(next) if next <= u64::from(request.spawn_tree_descendant_cap) => next,
            _ => {
                let response = Err(TurnError::capacity_exceeded(
                    TurnCapacityResource::SpawnTreeDescendants,
                    u64::from(request.spawn_tree_descendant_cap),
                ));
                inner.remember_submit_idempotency(
                    idempotency_key.clone(),
                    response.clone(),
                    request.received_at,
                );
                return response;
            }
        };
        inner.tree_reservations.insert(
            reservation_key.clone(),
            TreeReservationState {
                count: next_tree_count,
                released_children: previous_state.released_children.clone(),
            },
        );

        let admission_class = profile.admission_class.clone();
        if let Err(rejection) = inner.reserve_admission(
            run_id,
            admission_class.clone(),
            &request.child_scope,
            &request.actor,
            self.admission_limit_provider.as_ref(),
        ) {
            if previous_tree_count == 0 {
                inner.tree_reservations.remove(&reservation_key);
            } else {
                inner
                    .tree_reservations
                    .insert(reservation_key, previous_state);
            }
            let response = Err(TurnError::AdmissionRejected(rejection));
            inner.remember_submit_idempotency(
                idempotency_key.clone(),
                response.clone(),
                request.received_at,
            );
            return response;
        }

        let turn_id = crate::TurnId::new();
        let cursor = inner.next_cursor();
        let turn_record = TurnRecord {
            turn_id,
            scope: request.child_scope.clone(),
            actor: request.actor.clone(),
            accepted_message_ref: request.accepted_message_ref.clone(),
            source_binding_ref: request.source_binding_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
            created_at: request.received_at,
        };
        let mut record = RunRecord::queued(QueuedRunFields {
            scope: request.child_scope.clone(),
            actor: request.actor,
            turn_id,
            run_id,
            profile: profile.clone(),
            accepted_message_ref: request.accepted_message_ref.clone(),
            source_binding_ref: request.source_binding_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
            event_cursor: cursor,
            received_at: request.received_at,
        });
        record.parent_run_id = Some(parent_run_id);
        record.subagent_depth = subagent_depth;
        record.spawn_tree_root_run_id = Some(root_run_id);
        record.product_context = parent_product_context;
        inner.turns.insert(turn_id, turn_record);
        inner.active_locks.insert(
            lock_key.clone(),
            TurnActiveLockRecord {
                key: lock_key,
                run_id,
                status: TurnStatus::Queued,
                lock_version: TurnLockVersion::new(1),
                acquired_at: request.received_at,
                updated_at: request.received_at,
            },
        );
        inner.queued_runs.push_back(run_id);
        inner.records.insert(run_id, record.clone());
        inner.push_event(&record, TurnEventKind::Submitted, None, None);

        let response = Ok(SubmitTurnResponse::Accepted {
            turn_id,
            run_id,
            status: TurnStatus::Queued,
            resolved_run_profile_id: profile.id,
            resolved_run_profile_version: profile.version,
            event_cursor: cursor,
            accepted_message_ref: request.accepted_message_ref,
            reply_target_binding_ref: request.reply_target_binding_ref,
        });
        inner.remember_submit_idempotency(
            idempotency_key.clone(),
            response.clone(),
            record.received_at,
        );
        response
    }

    async fn children_of(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        let inner = self.lock_inner()?;
        let Some(parent) = inner.records.get(&run_id) else {
            return Ok(Vec::new());
        };
        if parent.scope != *scope {
            return Ok(Vec::new());
        }
        let mut children = inner
            .records
            .values()
            .filter(|record| {
                same_scope_envelope(&record.scope, scope) && record.parent_run_id == Some(run_id)
            })
            .map(RunRecord::persistence_record)
            .collect::<Vec<_>>();
        children.sort_by_key(|record| record.received_at);
        Ok(children)
    }

    async fn get_run_record(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError> {
        let inner = self.lock_inner()?;
        Ok(inner
            .records
            .get(&run_id)
            .filter(|record| record.scope == *scope)
            .map(RunRecord::persistence_record))
    }

    async fn reserve_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        cap: u32,
    ) -> Result<SpawnTreeReservation, TurnError> {
        if delta == 0 {
            return Err(TurnError::InvalidRequest {
                reason: "reservation delta must be greater than zero".to_string(),
            });
        }
        let mut inner = self.lock_inner()?;
        let Some(root) = inner.records.get(&root_run_id) else {
            return Err(TurnError::ScopeNotFound);
        };
        if !same_scope_envelope(&root.scope, scope) {
            return Err(TurnError::Unauthorized);
        }
        let canonical_root_run_id = root.spawn_tree_root_run_id.unwrap_or(root.run_id);
        if canonical_root_run_id != root.run_id {
            return Err(TurnError::InvalidRequest {
                reason: "root_run_id must identify the spawn tree root".to_string(),
            });
        }
        let key = SpawnTreeReservationKey::new(scope, canonical_root_run_id);
        let current = inner
            .tree_reservations
            .get(&key)
            .map(|state| state.count)
            .unwrap_or(0);
        let next = current.checked_add(u64::from(delta)).ok_or_else(|| {
            TurnError::capacity_exceeded(TurnCapacityResource::SpawnTreeDescendants, u64::from(cap))
        })?;
        if next > u64::from(cap) {
            return Err(TurnError::capacity_exceeded(
                TurnCapacityResource::SpawnTreeDescendants,
                u64::from(cap),
            ));
        }
        let released_children = inner
            .tree_reservations
            .get(&key)
            .map(|state| state.released_children.clone())
            .unwrap_or_default();
        inner.tree_reservations.insert(
            key,
            TreeReservationState {
                count: next,
                released_children: released_children.clone(),
            },
        );
        Ok(SpawnTreeReservation {
            scope: scope.clone(),
            root_run_id: canonical_root_run_id,
            descendant_count: next,
            released_children,
        })
    }

    async fn release_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        idempotency_key: TurnRunId,
    ) -> Result<(), TurnError> {
        let mut inner = self.lock_inner()?;
        let Some(root) = inner.records.get(&root_run_id) else {
            return Err(TurnError::ScopeNotFound);
        };
        if !same_scope_envelope(&root.scope, scope) {
            return Err(TurnError::Unauthorized);
        }
        let canonical_root_run_id = root.spawn_tree_root_run_id.unwrap_or(root.run_id);
        if canonical_root_run_id != root.run_id {
            return Err(TurnError::InvalidRequest {
                reason: "root_run_id must identify the spawn tree root".to_string(),
            });
        }
        let key = SpawnTreeReservationKey::new(scope, canonical_root_run_id);
        let mut released_reservation = false;
        if let Some(state) = inner.tree_reservations.get_mut(&key) {
            // §5.5 round-5/6 idempotency: a release already recorded for
            // this child is a no-op, not a second decrement — this is what
            // makes a retried release call (recovery re-driving an edge
            // stuck at `Claimed`) safe to repeat unconditionally.
            if !state.released_children.insert(idempotency_key) {
                return Ok(());
            }
            let previous = state.count;
            if previous < u64::from(delta) {
                // Reject over-release loudly so callers can diagnose
                // double-release bugs instead of silently zeroing the
                // reservation and uncapping the spawn tree. Undo the just-
                // inserted dedup marker so a legitimate retry after fixing
                // the caller bug isn't permanently swallowed as a no-op.
                state.released_children.remove(&idempotency_key);
                return Err(TurnError::InvalidRequest {
                    reason: "release delta exceeds current reservation count".to_string(),
                });
            }
            state.count = previous - u64::from(delta);
            if state.count == 0 {
                inner.tree_reservations.remove(&key);
                released_reservation = true;
            }
        }
        if released_reservation
            && inner
                .records
                .get(&canonical_root_run_id)
                .is_some_and(|record| record.status.get().is_terminal())
            && !inner.terminal_runs.contains(&canonical_root_run_id)
        {
            if inner.terminal_runs.len() >= inner.limits.max_terminal_records {
                inner.records.remove(&canonical_root_run_id);
                inner.admission_reservations.remove(&canonical_root_run_id);
                return Ok(());
            }
            inner.terminal_runs.push_back(canonical_root_run_id);
            inner.prune_terminal_records();
        }
        Ok(())
    }

    async fn prune_released_child(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        let mut inner = self.lock_inner()?;
        let Some(root) = inner.records.get(&root_run_id) else {
            // The reservation (and its tree) may already be fully released
            // and gone — benign, matches `release_tree_descendants`'s own
            // missing-root handling being the only hard error case there.
            return Ok(());
        };
        if !same_scope_envelope(&root.scope, scope) {
            return Err(TurnError::Unauthorized);
        }
        let canonical_root_run_id = root.spawn_tree_root_run_id.unwrap_or(root.run_id);
        let key = SpawnTreeReservationKey::new(scope, canonical_root_run_id);
        if let Some(state) = inner.tree_reservations.get_mut(&key) {
            state.released_children.remove(&child_run_id);
        }
        Ok(())
    }
}
