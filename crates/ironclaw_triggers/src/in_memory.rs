use super::*;

#[derive(Debug, Default)]
pub(super) struct InMemoryTriggerRepositoryState {
    records: HashMap<TriggerRepositoryKey, TriggerRecord>,
    runs: HashMap<TriggerRunRepositoryKey, TriggerRunRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TriggerRepositoryKey {
    tenant_id: TenantId,
    trigger_id: TriggerId,
}

impl TriggerRepositoryKey {
    fn new(tenant_id: &TenantId, trigger_id: TriggerId) -> Self {
        Self {
            tenant_id: tenant_id.clone(),
            trigger_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TriggerRunRepositoryKey {
    tenant_id: TenantId,
    trigger_id: TriggerId,
    fire_slot: Timestamp,
}

impl TriggerRunRepositoryKey {
    fn new(tenant_id: &TenantId, trigger_id: TriggerId, fire_slot: Timestamp) -> Self {
        Self {
            tenant_id: tenant_id.clone(),
            trigger_id,
            fire_slot,
        }
    }
}

#[async_trait]
impl TriggerRepository for InMemoryTriggerRepository {
    async fn upsert_trigger(&self, record: TriggerRecord) -> Result<(), TriggerError> {
        record.validate()?;
        let mut state = self.lock_state()?;
        state.records.insert(
            TriggerRepositoryKey::new(&record.tenant_id, record.trigger_id),
            record,
        );
        Ok(())
    }

    async fn get_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Ok(self
            .lock_state()?
            .records
            .get(&TriggerRepositoryKey::new(&tenant_id, trigger_id))
            .cloned())
    }

    async fn list_triggers(&self, tenant_id: TenantId) -> Result<Vec<TriggerRecord>, TriggerError> {
        let state = self.lock_state()?;
        let mut records = state
            .records
            .values()
            .filter(|record| record.tenant_id == tenant_id)
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by_key(|record| (record.created_at, record.trigger_id));
        Ok(records)
    }

    async fn list_scoped_triggers(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        limit: usize,
        excluded_states: &[TriggerState],
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(MAX_TRIGGER_LIST_LIMIT);
        let state = self.lock_state()?;
        let mut records = state
            .records
            .values()
            .filter(|record| {
                record.tenant_id == tenant_id
                    && record.creator_user_id == creator_user_id
                    && record.agent_id == agent_id
                    && record.project_id == project_id
                    && !excluded_states.contains(&record.state)
            })
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by_key(|record| (record.created_at, record.trigger_id));
        records.truncate(limit);
        Ok(records)
    }

    async fn remove_trigger(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Ok(self
            .lock_state()?
            .records
            .remove(&TriggerRepositoryKey::new(&tenant_id, trigger_id)))
    }

    async fn remove_scoped_trigger(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let mut state = self.lock_state()?;
        let key = TriggerRepositoryKey::new(&tenant_id, trigger_id);
        let Some(record) = state.records.get(&key) else {
            return Ok(None);
        };
        if record.creator_user_id != creator_user_id
            || record.agent_id != agent_id
            || record.project_id != project_id
        {
            return Ok(None);
        }
        Ok(state.records.remove(&key))
    }

    async fn set_scoped_trigger_state(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        trigger_id: TriggerId,
        new_state: TriggerState,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        validate_user_settable_trigger_state(new_state)?;
        let mut state = self.lock_state()?;
        let key = TriggerRepositoryKey::new(&tenant_id, trigger_id);
        let Some(record) = state.records.get_mut(&key) else {
            return Ok(None);
        };
        if record.creator_user_id != creator_user_id
            || record.agent_id != agent_id
            || record.project_id != project_id
            || record.state == TriggerState::Completed
        {
            return Ok(None);
        }
        record.state = new_state;
        Ok(Some(record.clone()))
    }

    async fn rename_scoped_trigger(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        trigger_id: TriggerId,
        name: AutomationName,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let mut state = self.lock_state()?;
        let key = TriggerRepositoryKey::new(&tenant_id, trigger_id);
        let Some(record) = state.records.get_mut(&key) else {
            return Ok(None);
        };
        if record.creator_user_id != creator_user_id
            || record.agent_id != agent_id
            || record.project_id != project_id
        {
            return Ok(None);
        }
        record.name = name.into_inner();
        Ok(Some(record.clone()))
    }

    async fn list_due_triggers(
        &self,
        now: Timestamp,
        limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(MAX_DUE_TRIGGER_POLL_LIMIT);
        let state = self.lock_state()?;
        let mut selected_keys = state
            .records
            .iter()
            .filter(|(_, record)| {
                record.state == TriggerState::Scheduled
                    && record.is_due_at(now)
                    && !record.has_active_fire()
            })
            .map(|(key, record)| {
                (
                    record.next_run_at,
                    record.tenant_id.clone(),
                    record.trigger_id,
                    key.clone(),
                )
            })
            .collect::<Vec<_>>();
        selected_keys.sort_by_key(|(next_run_at, tenant_id, trigger_id, _)| {
            (*next_run_at, tenant_id.clone(), *trigger_id)
        });
        selected_keys.truncate(limit);
        Ok(selected_keys
            .into_iter()
            .filter_map(|(_, _, _, key)| state.records.get(&key).cloned())
            .collect())
    }

    async fn list_active_triggers(&self, limit: usize) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.list_active_triggers_after(None, limit).await
    }

    async fn list_active_triggers_after(
        &self,
        after: Option<ActiveTriggerScanCursor>,
        limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(MAX_DUE_TRIGGER_POLL_LIMIT);
        let mut selected_records = {
            let state = self.lock_state()?;
            state
                .records
                .values()
                .filter_map(|record| {
                    let active_fire_slot = record.active_fire_slot?;
                    Some((
                        active_fire_slot,
                        record.tenant_id.clone(),
                        record.trigger_id,
                        record.clone(),
                    ))
                })
                .filter(
                    |(active_fire_slot, tenant_id, trigger_id, _record)| match after.as_ref() {
                        Some(cursor) => {
                            (*active_fire_slot, tenant_id, *trigger_id)
                                > (
                                    cursor.active_fire_slot(),
                                    cursor.tenant_id(),
                                    cursor.trigger_id(),
                                )
                        }
                        None => true,
                    },
                )
                .collect::<Vec<_>>()
        };
        selected_records.sort_by_key(|(active_fire_slot, tenant_id, trigger_id, _record)| {
            (*active_fire_slot, tenant_id.clone(), *trigger_id)
        });
        selected_records.truncate(limit);
        Ok(selected_records
            .into_iter()
            .map(|(_, _, _, record)| record)
            .collect())
    }

    async fn claim_due_fire(
        &self,
        request: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError> {
        let mut state = self.lock_state()?;
        let key = TriggerRepositoryKey::new(&request.tenant_id, request.trigger_id);
        let Some(record) = state.records.get_mut(&key) else {
            return Ok(ClaimDueFireOutcome::NotFound);
        };

        if record.state != TriggerState::Scheduled
            || record.next_run_at != request.fire_slot
            || request.fire_slot > request.now
        {
            return Ok(ClaimDueFireOutcome::NotDue {
                record: record.clone(),
            });
        }

        if record.has_active_fire() {
            return Ok(ClaimDueFireOutcome::AlreadyActive {
                active_fire_slot: record.active_fire_slot,
                active_run_ref: record.active_run_ref,
            });
        }

        record.active_fire_slot = Some(request.fire_slot);
        record.active_run_ref = None;
        let record = record.clone();
        state.runs.insert(
            TriggerRunRepositoryKey::new(&request.tenant_id, request.trigger_id, request.fire_slot),
            TriggerRunRecord::running(
                request.tenant_id,
                request.trigger_id,
                request.fire_slot,
                None,
                request.now,
            ),
        );
        prune_run_history_locked(&mut state, &record.tenant_id, record.trigger_id);
        Ok(ClaimDueFireOutcome::Claimed(ClaimedTriggerFire {
            record,
            fire_slot: request.fire_slot,
        }))
    }

    async fn mark_fire_accepted(
        &self,
        request: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let Some(record) = self.update_claimed_fire(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            |record| {
                if let Some(active_run_ref) = record.active_run_ref {
                    reject_run_ref_rewrite(active_run_ref, request.run_id)?;
                    return Ok(());
                }
                if let Some(nra) = record.schedule.next_slot_after(request.fire_slot)? {
                    reject_non_future_next_run_at(request.fire_slot, nra)?;
                    record.next_run_at = nra;
                }
                record.last_run_at = Some(request.submitted_at);
                record.last_fired_slot = Some(request.fire_slot);
                record.last_status = Some(TriggerRunStatus::Ok);
                record.active_fire_slot = Some(request.fire_slot);
                record.active_run_ref = Some(request.run_id);
                Ok(())
            },
        )?
        else {
            return Ok(None);
        };
        self.upsert_running_run_history(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            request.run_id,
            Some(request.thread_id),
            record.last_run_at.unwrap_or(request.submitted_at),
        )?;
        Ok(Some(record))
    }

    async fn mark_fire_replayed(
        &self,
        request: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let Some(record) = self.update_claimed_fire(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            |record| {
                if let Some(active_run_ref) = record.active_run_ref {
                    reject_run_ref_rewrite(active_run_ref, request.original_run_id)?;
                    return Ok(());
                }
                if let Some(nra) = record.schedule.next_slot_after(request.fire_slot)? {
                    reject_non_future_next_run_at(request.fire_slot, nra)?;
                    record.next_run_at = nra;
                }
                record.last_run_at = Some(request.replayed_at);
                record.last_fired_slot = Some(request.fire_slot);
                record.last_status = Some(TriggerRunStatus::Ok);
                record.active_fire_slot = Some(request.fire_slot);
                record.active_run_ref = Some(request.original_run_id);
                Ok(())
            },
        )?
        else {
            return Ok(None);
        };
        self.upsert_running_run_history(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            request.original_run_id,
            request.thread_id,
            record.last_run_at.unwrap_or(request.replayed_at),
        )?;
        Ok(Some(record))
    }

    async fn mark_fire_retryable_failed(
        &self,
        request: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let Some(record) = self.update_claimed_fire(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            |record| {
                reject_failed_result_after_active_run(record.active_run_ref)?;
                if matches!(record.schedule, TriggerSchedule::Cron { .. })
                    && record.next_run_at > request.fire_slot
                {
                    return Err(TriggerError::InvalidRecord {
                        kind: TriggerRecordValidationKind::Other,
                        reason: "retryable fire failure must leave next_run_at at or before the failed fire slot"
                            .to_string(),
                    });
                }
                record.last_status = Some(TriggerRunStatus::Error);
                record.active_fire_slot = None;
                record.active_run_ref = None;
                Ok(())
            },
        )?
        else {
            return Ok(None);
        };
        self.complete_run_history(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            None,
            TriggerRunHistoryStatus::Error,
            Utc::now(),
        )?;
        Ok(Some(record))
    }

    async fn mark_fire_permanently_failed(
        &self,
        request: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let Some(record) = self.update_claimed_fire(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            |record| {
                reject_failed_result_after_active_run(record.active_run_ref)?;
                reject_non_future_next_run_at(request.fire_slot, request.next_run_at)?;
                record.last_status = Some(TriggerRunStatus::Error);
                record.next_run_at = request.next_run_at;
                record.active_fire_slot = None;
                record.active_run_ref = None;
                Ok(())
            },
        )?
        else {
            return Ok(None);
        };
        self.complete_run_history(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            None,
            TriggerRunHistoryStatus::Error,
            Utc::now(),
        )?;
        Ok(Some(record))
    }

    async fn mark_fire_terminally_failed(
        &self,
        request: FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let Some(record) = self.update_claimed_fire(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            |record| {
                reject_failed_result_after_active_run(record.active_run_ref)?;
                record.state = TriggerState::Completed;
                record.last_status = Some(TriggerRunStatus::Error);
                record.active_fire_slot = None;
                record.active_run_ref = None;
                Ok(())
            },
        )?
        else {
            return Ok(None);
        };
        self.complete_run_history(
            &request.tenant_id,
            request.trigger_id,
            request.fire_slot,
            None,
            TriggerRunHistoryStatus::Error,
            Utc::now(),
        )?;
        Ok(Some(record))
    }

    async fn clear_active_fire(
        &self,
        request: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let mut state = self.lock_state()?;
        let key = TriggerRepositoryKey::new(&request.tenant_id, request.trigger_id);
        let Some(record) = state.records.get_mut(&key) else {
            return Ok(None);
        };
        if record.active_fire_slot != Some(request.fire_slot)
            || record.active_run_ref != Some(request.run_id)
        {
            return Ok(None);
        }
        let next = record.schedule.next_slot_after(request.fire_slot)?;
        record.active_fire_slot = None;
        record.active_run_ref = None;
        if let Some(t) = next {
            record.next_run_at = t;
        }
        record.state = if next.is_some() {
            record.state
        } else {
            TriggerState::Completed
        };
        let record = record.clone();
        let completed_at = Utc::now();
        state
            .runs
            .entry(TriggerRunRepositoryKey::new(
                &request.tenant_id,
                request.trigger_id,
                request.fire_slot,
            ))
            .and_modify(|run| {
                run.run_id = Some(request.run_id);
                run.status = request.status;
                run.completed_at = Some(completed_at);
            })
            .or_insert_with(|| {
                let mut run = TriggerRunRecord::running(
                    request.tenant_id.clone(),
                    request.trigger_id,
                    request.fire_slot,
                    Some(request.run_id),
                    completed_at,
                );
                run.status = request.status;
                run.completed_at = Some(completed_at);
                run
            });
        prune_run_history_locked(&mut state, &request.tenant_id, request.trigger_id);
        Ok(Some(record))
    }

    async fn find_trigger_run_by_thread_id(
        &self,
        tenant_id: TenantId,
        thread_id: &ThreadId,
    ) -> Result<Option<(TriggerRecord, TriggerRunRecord)>, TriggerError> {
        let state = self.lock_state()?;
        let Some(run) = state.runs.values().find(|run| {
            run.tenant_id == tenant_id
                && run.thread_id.as_ref().map(|t| t.as_str()) == Some(thread_id.as_str())
        }) else {
            return Ok(None);
        };
        let run = run.clone();
        let trigger = state
            .records
            .get(&TriggerRepositoryKey::new(&tenant_id, run.trigger_id))
            .cloned();
        Ok(trigger.map(|t| (t, run)))
    }

    async fn list_trigger_run_history(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
        limit: usize,
    ) -> Result<Vec<TriggerRunRecord>, TriggerError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(MAX_TRIGGER_RUN_HISTORY_LIMIT);
        let state = self.lock_state()?;
        let mut runs = state
            .runs
            .values()
            .filter(|run| run.tenant_id == tenant_id && run.trigger_id == trigger_id)
            .cloned()
            .collect::<Vec<_>>();
        runs.sort_by_key(|run| std::cmp::Reverse(run.fire_slot));
        runs.truncate(limit);
        Ok(runs)
    }

    async fn list_trigger_run_history_batch(
        &self,
        tenant_id: TenantId,
        trigger_ids: &[TriggerId],
        limit: usize,
    ) -> Result<HashMap<TriggerId, Vec<TriggerRunRecord>>, TriggerError> {
        let mut runs_by_trigger = HashMap::with_capacity(trigger_ids.len());
        if limit == 0 || trigger_ids.is_empty() {
            return Ok(runs_by_trigger);
        }
        let limit = limit.min(MAX_TRIGGER_RUN_HISTORY_LIMIT);
        let state = self.lock_state()?;
        let trigger_id_set = trigger_ids.iter().copied().collect::<HashSet<_>>();
        for trigger_id in trigger_ids {
            runs_by_trigger.insert(*trigger_id, Vec::new());
        }
        for run in state
            .runs
            .values()
            .filter(|run| run.tenant_id == tenant_id && trigger_id_set.contains(&run.trigger_id))
        {
            runs_by_trigger
                .entry(run.trigger_id)
                .or_default()
                .push(run.clone());
        }
        for runs in runs_by_trigger.values_mut() {
            runs.sort_by_key(|run| std::cmp::Reverse(run.fire_slot));
            runs.truncate(limit);
        }
        Ok(runs_by_trigger)
    }
}

impl InMemoryTriggerRepository {
    pub(crate) fn lock_state(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, InMemoryTriggerRepositoryState>, TriggerError> {
        self.state.lock().map_err(|_| TriggerError::Backend {
            reason: "trigger repository mutex poisoned".to_string(),
        })
    }

    fn update_claimed_fire(
        &self,
        tenant_id: &TenantId,
        trigger_id: TriggerId,
        fire_slot: Timestamp,
        update: impl FnOnce(&mut TriggerRecord) -> Result<(), TriggerError>,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        let mut state = self.lock_state()?;
        let key = TriggerRepositoryKey::new(tenant_id, trigger_id);
        let Some(record) = state.records.get_mut(&key) else {
            return Ok(None);
        };
        if record.active_fire_slot != Some(fire_slot) {
            return Ok(None);
        }
        update(record)?;
        Ok(Some(record.clone()))
    }

    pub(crate) fn upsert_running_run_history(
        &self,
        tenant_id: &TenantId,
        trigger_id: TriggerId,
        fire_slot: Timestamp,
        run_id: TurnRunId,
        thread_id: Option<ThreadId>,
        submitted_at: Timestamp,
    ) -> Result<(), TriggerError> {
        let mut state = self.lock_state()?;
        let key = TriggerRunRepositoryKey::new(tenant_id, trigger_id, fire_slot);
        let existing = state.runs.get(&key);
        if existing.is_some_and(|run| run.completed_at.is_some()) {
            return Ok(());
        }
        // A replay without a resolved scope must not clobber an already
        // persisted canonical thread id back to None.
        let preserved_thread_id =
            thread_id.or_else(|| existing.and_then(|run| run.thread_id.clone()));
        let mut run = TriggerRunRecord::running(
            tenant_id.clone(),
            trigger_id,
            fire_slot,
            Some(run_id),
            submitted_at,
        );
        run.thread_id = preserved_thread_id;
        state.runs.insert(key, run);
        prune_run_history_locked(&mut state, tenant_id, trigger_id);
        Ok(())
    }

    pub(crate) fn complete_run_history(
        &self,
        tenant_id: &TenantId,
        trigger_id: TriggerId,
        fire_slot: Timestamp,
        run_id: Option<TurnRunId>,
        status: TriggerRunHistoryStatus,
        completed_at: Timestamp,
    ) -> Result<(), TriggerError> {
        let mut state = self.lock_state()?;
        state
            .runs
            .entry(TriggerRunRepositoryKey::new(
                tenant_id, trigger_id, fire_slot,
            ))
            .and_modify(|run| {
                if run.run_id.is_none() {
                    run.run_id = run_id;
                }
                run.status = status;
                run.completed_at = Some(completed_at);
            })
            .or_insert_with(|| {
                let mut run = TriggerRunRecord::running(
                    tenant_id.clone(),
                    trigger_id,
                    fire_slot,
                    run_id,
                    completed_at,
                );
                run.status = status;
                run.completed_at = Some(completed_at);
                run
            });
        prune_run_history_locked(&mut state, tenant_id, trigger_id);
        Ok(())
    }
}

fn prune_run_history_locked(
    state: &mut InMemoryTriggerRepositoryState,
    tenant_id: &TenantId,
    trigger_id: TriggerId,
) {
    let mut keys = state
        .runs
        .keys()
        .filter(|key| key.tenant_id == *tenant_id && key.trigger_id == trigger_id)
        .cloned()
        .collect::<Vec<_>>();
    keys.sort_by_key(|key| std::cmp::Reverse(key.fire_slot));
    for key in keys.into_iter().skip(MAX_TRIGGER_RUN_HISTORY_RETAINED) {
        state.runs.remove(&key);
    }
}
