use super::*;

pub(super) async fn put_get(
    fs: Arc<dyn RootFilesystem>,
    prefix: &VirtualPath,
    sample: usize,
    payload_len: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let path = child(prefix, "entry")?;
    let path = child(&path, &format!("sample-{sample}"))?;
    let payload = payload(sample, payload_len);
    let version = fs
        .put(&path, Entry::bytes(payload.clone()), CasExpectation::Any)
        .await?;
    let read = fs.get(&path).await?.ok_or("missing put_get readback")?;
    Ok(version.get() ^ read.version.get() ^ read.entry.body.len() as u64)
}

pub(super) async fn query_exact(
    fs: Arc<dyn RootFilesystem>,
    prefix: &VirtualPath,
    sample: usize,
    _payload_len: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let key = IndexKey::new("bucket")?;
    let bucket = format!("b{}", sample % 8);
    let sample_prefix = child(prefix, &format!("sample-{sample}"))?;
    let rows = fs
        .query(
            &sample_prefix,
            &Filter::Eq {
                key,
                value: IndexValue::Text(bucket),
            },
            Page::first(16),
        )
        .await?;
    Ok(rows.len() as u64)
}

pub(super) async fn seed_query_exact_records(
    fs: Arc<dyn RootFilesystem>,
    prefix: &VirtualPath,
    sample: usize,
    payload_bytes: &[usize],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let key = IndexKey::new("bucket")?;
    let kind = ironclaw_filesystem::RecordKind::new("latency_record")?;
    let bucket = format!("b{}", sample % 8);
    let payload_len = payload_bytes[sample % payload_bytes.len()].max(1);
    for i in 0..8 {
        let path = child(prefix, &format!("sample-{sample}/record-{i}"))?;
        let entry = Entry::record(
            kind.clone(),
            &serde_json::json!({"sample": sample, "row": i, "backend": "storage"}),
        )?
        .with_indexed(
            key.clone(),
            IndexValue::Text(if i == 0 {
                bucket.clone()
            } else {
                format!("other-{i}")
            }),
        )
        .with_indexed(IndexKey::new("size")?, IndexValue::I64(payload_len as i64));
        fs.put(&path, entry, CasExpectation::Any).await?;
    }
    Ok(())
}

pub(super) async fn append_tail(
    fs: Arc<dyn RootFilesystem>,
    prefix: &VirtualPath,
    sample: usize,
    payload_len: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let path = child(prefix, "events")?;
    let path = child(&path, &format!("sample-{sample}"))?;
    let payloads = (0..8)
        .map(|i| payload(sample + i, payload_len))
        .collect::<Vec<_>>();
    let seqs = fs.append_batch(&path, payloads).await?;
    let events = fs.tail_bounded(&path, SeqNo::ZERO, 16).await?;
    let payload_bytes = events
        .iter()
        .map(|event| event.payload.len() as u64)
        .sum::<u64>();
    Ok((seqs.len() as u64) ^ (events.len() as u64) ^ payload_bytes)
}

pub(super) async fn reserve_sequence(
    fs: Arc<dyn RootFilesystem>,
    prefix: &VirtualPath,
    sample: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let path = child(prefix, "sequence")?;
    let path = child(&path, &format!("sample-{sample}"))?;
    let first = fs.reserve_sequence(&path).await?;
    let second = fs.reserve_sequence(&path).await?;
    Ok(first.get() ^ second.get())
}

pub(super) async fn trigger_seed_list(
    repository: Arc<dyn TriggerRepository>,
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    run_id: &str,
    sample: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let pool_label = postgres_pool_size
        .map(|pool_size| format!("pool-{pool_size}"))
        .unwrap_or_else(|| "baseline".to_string());
    let scope = format!("{}-{pool_label}-{run_id}-{sample}", backend.as_str());
    let tenant_id = TenantId::new(format!("latency-trigger-tenant-{scope}"))?;
    let creator_user_id = UserId::new(format!("latency-trigger-user-{scope}"))?;
    let agent_id = AgentId::new(format!("latency-trigger-agent-{scope}"))?;
    let project_id = ProjectId::new(format!("latency-trigger-project-{scope}"))?;
    let record = trigger_record(
        sample,
        tenant_id.clone(),
        creator_user_id.clone(),
        agent_id.clone(),
        project_id.clone(),
    )?;
    repository.upsert_trigger(record).await?;
    let tenant_rows = repository.list_triggers(tenant_id.clone()).await?;
    let scoped_rows = repository
        .list_scoped_triggers(
            tenant_id,
            creator_user_id,
            Some(agent_id),
            Some(project_id),
            16,
            &[],
        )
        .await?;
    Ok((tenant_rows.len() as u64) ^ ((scoped_rows.len() as u64) << 8))
}

fn trigger_record(
    sample: usize,
    tenant_id: TenantId,
    creator_user_id: UserId,
    agent_id: AgentId,
    project_id: ProjectId,
) -> Result<TriggerRecord, Box<dyn std::error::Error + Send + Sync>> {
    let created_at = timestamp(1_704_067_000 + sample as i64)?;
    let next_run_at = timestamp(1_704_070_600 + sample as i64)?;
    Ok(TriggerRecord {
        trigger_id: TriggerId::new(),
        tenant_id,
        creator_user_id,
        agent_id: Some(agent_id),
        project_id: Some(project_id),
        name: format!("latency trigger {sample}"),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::cron("0 8 * * *")?,
        delivery_target: None,
        prompt: "run the deterministic latency fixture".to_string(),
        state: TriggerState::Scheduled,
        next_run_at,
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at,
    })
}

fn timestamp(seconds: i64) -> Result<DateTime<Utc>, Box<dyn std::error::Error + Send + Sync>> {
    DateTime::from_timestamp(seconds, 0).ok_or_else(|| "invalid trigger timestamp".into())
}

pub(super) async fn control_plane_snapshot(
    approval_requests: Arc<dyn ApprovalRequestStore>,
    secret_store: Arc<dyn SecretStore>,
    resource_governor: Arc<dyn ResourceGovernor>,
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    run_id: &str,
    sample: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let scope = control_plane_scope(backend, postgres_pool_size, run_id, sample)?;

    let request_id = ApprovalRequestId::new();
    let approval = ApprovalRequest {
        id: request_id,
        correlation_id: CorrelationId::new(),
        requested_by: Principal::User(scope.user_id.clone()),
        action: Box::new(Action::ReserveResources {
            estimate: resource_estimate(sample),
        }),
        invocation_fingerprint: None,
        reason: format!("latency control-plane sample {sample}"),
        reusable_scope: None,
    };
    let pending = approval_requests
        .save_pending(scope.clone(), approval)
        .await?;
    let approved = approval_requests.approve(&scope, request_id).await?;
    let approval_rows = approval_requests.records_for_scope(&scope).await?;

    let handle = SecretHandle::new(format!("latency_secret_{sample}"))?;
    secret_store
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from(format!("secret-material-{sample}-{run_id}")),
            None,
        )
        .await?;
    let metadata = secret_store
        .metadata(&scope, &handle)
        .await?
        .ok_or("missing secret metadata")?;
    let metadata_rows = secret_store.metadata_for_scope(&scope).await?;
    let lease = secret_store.lease_once(&scope, &handle).await?;
    let material = secret_store.consume(&scope, lease.id).await?;

    let account = ResourceAccount::project(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope
            .project_id
            .clone()
            .ok_or("control-plane scope missing project id")?,
    );
    let (account_snapshot, receipt_has_actual) =
        resource_governor_round_trip(resource_governor, account, scope.clone(), sample).await?;
    let account_snapshot = account_snapshot.ok_or("missing resource account snapshot")?;

    let approval_state = match (pending.status, approved.status) {
        (ApprovalStatus::Pending, ApprovalStatus::Approved) => 0x11,
        _ => 0xff,
    };
    Ok(approval_state
        ^ ((approval_rows.len() as u64) << 8)
        ^ ((metadata_rows.len() as u64) << 16)
        ^ ((metadata.handle.as_str().len() as u64) << 24)
        ^ ((material.expose_secret().len() as u64) << 32)
        ^ ((receipt_has_actual as u64) << 40)
        ^ (account_snapshot.ledger.spent.output_bytes << 48))
}

async fn resource_governor_round_trip(
    resource_governor: Arc<dyn ResourceGovernor>,
    account: ResourceAccount,
    scope: ResourceScope,
    sample: usize,
) -> Result<
    (Option<ironclaw_resources::AccountSnapshot>, bool),
    Box<dyn std::error::Error + Send + Sync>,
> {
    tokio::task::spawn_blocking(move || {
        resource_governor.set_limit(account.clone(), resource_limits())?;
        let reservation = resource_governor.reserve(scope, resource_estimate(sample))?;
        let receipt = resource_governor.reconcile(reservation.id, resource_usage(sample))?;
        let account_snapshot = resource_governor.account_snapshot(&account)?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>((
            account_snapshot,
            receipt.actual.is_some(),
        ))
    })
    .await?
}

fn control_plane_scope(
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    run_id: &str,
    sample: usize,
) -> Result<ResourceScope, Box<dyn std::error::Error + Send + Sync>> {
    let pool_label = postgres_pool_size
        .map(|pool_size| format!("pool-{pool_size}"))
        .unwrap_or_else(|| "baseline".to_string());
    let scope = format!("{}-{pool_label}-{run_id}-{sample}", backend.as_str());
    Ok(ResourceScope {
        tenant_id: TenantId::new(format!("latency-control-tenant-{scope}"))?,
        user_id: UserId::new(format!("latency-control-user-{scope}"))?,
        agent_id: Some(AgentId::new(format!("latency-control-agent-{scope}"))?),
        project_id: Some(ProjectId::new(format!("latency-control-project-{scope}"))?),
        mission_id: None,
        thread_id: None,
        invocation_id: ironclaw_host_api::InvocationId::new(),
    })
}

fn resource_estimate(sample: usize) -> ResourceEstimate {
    ResourceEstimate {
        input_tokens: Some(64 + sample as u64 % 16),
        output_tokens: Some(32 + sample as u64 % 8),
        wall_clock_ms: Some(250),
        output_bytes: Some(512),
        concurrency_slots: Some(1),
        ..Default::default()
    }
}

fn resource_usage(sample: usize) -> ResourceUsage {
    ResourceUsage {
        input_tokens: 64 + sample as u64 % 16,
        output_tokens: 32 + sample as u64 % 8,
        wall_clock_ms: 125,
        output_bytes: 256,
        network_egress_bytes: 0,
        process_count: 0,
        ..Default::default()
    }
}

fn resource_limits() -> ResourceLimits {
    ResourceLimits {
        max_input_tokens: Some(1_000_000),
        max_output_tokens: Some(1_000_000),
        max_wall_clock_ms: Some(1_000_000),
        max_output_bytes: Some(1_000_000),
        max_concurrency_slots: Some(10_000),
        ..Default::default()
    }
}

pub(super) async fn turn_lifecycle(
    store: Arc<dyn TurnLifecycleStore>,
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    run_id: &str,
    sample: usize,
    payload_len: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let key = turn_lifecycle_key(backend, postgres_pool_size, run_id, sample);
    let actor = turn_lifecycle_actor(sample)?;
    let resolver = InMemoryRunProfileResolver::default();

    let complete_scope = turn_lifecycle_scope(&key, sample, "complete")?;
    let complete_submit = store
        .submit_turn(
            turn_lifecycle_submit_request(
                complete_scope.clone(),
                actor.clone(),
                &key,
                "complete",
                payload_len,
            )?,
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await?;
    let (complete_turn_id, complete_run_id, complete_submit_status) =
        accepted_run(&complete_submit);
    let (runner_id, lease_token, claimed_state) = claim_expected_run(
        Arc::clone(&store),
        Some(complete_scope.clone()),
        complete_run_id,
        "complete first claim",
    )
    .await?;

    let gate_ref = GateRef::new(format!("gate:latency-{key}-approval"))?;
    let complete_checkpoint_id = TurnCheckpointId::new();
    let complete_checkpoint_ref = LoopCheckpointStateRef::new(format!("checkpoint:latency-{key}"))?;
    let blocked = store
        .block_run(BlockRunRequest {
            run_id: complete_run_id,
            runner_id,
            lease_token,
            checkpoint_id: complete_checkpoint_id,
            state_ref: complete_checkpoint_ref.clone(),
            reason: BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            },
        })
        .await?;
    ensure_status(blocked.status, TurnStatus::BlockedApproval, "block_run")?;
    let checkpoint_code = record_turn_lifecycle_checkpoints(
        Arc::clone(&store),
        &complete_scope,
        complete_turn_id,
        complete_run_id,
        complete_checkpoint_ref,
        &key,
        payload_len,
    )
    .await?;

    let resumed = store
        .resume_turn(ResumeTurnRequest {
            scope: complete_scope.clone(),
            actor: actor.clone(),
            run_id: complete_run_id,
            gate_resolution_ref: gate_ref,
            source_binding_ref: SourceBindingRef::new(format!("source-{key}-resume"))?,
            reply_target_binding_ref: ReplyTargetBindingRef::new(format!("reply-{key}-resume"))?,
            idempotency_key: IdempotencyKey::new(format!("idem-{key}-resume"))?,
            precondition: ResumeTurnPrecondition::BlockedApprovalGate,
            resume_disposition: None,
        })
        .await?;
    ensure_status(resumed.status, TurnStatus::Queued, "resume_turn")?;

    let (runner_id, lease_token, reclaimed_state) = claim_expected_run(
        Arc::clone(&store),
        Some(complete_scope.clone()),
        complete_run_id,
        "complete reclaim",
    )
    .await?;
    let completed = store
        .complete_run(CompleteRunRequest {
            run_id: complete_run_id,
            runner_id,
            lease_token,
        })
        .await?;
    ensure_status(completed.status, TurnStatus::Completed, "complete_run")?;
    let completed_readback = store
        .get_run_state(GetRunStateRequest {
            scope: complete_scope,
            run_id: complete_run_id,
        })
        .await?;
    ensure_status(
        completed_readback.status,
        TurnStatus::Completed,
        "complete readback",
    )?;

    let cancel_scope = turn_lifecycle_scope(&key, sample, "cancel")?;
    let cancel_submit = store
        .submit_turn(
            turn_lifecycle_submit_request(
                cancel_scope.clone(),
                actor.clone(),
                &key,
                "cancel",
                payload_len,
            )?,
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await?;
    let (_cancel_turn_id, cancel_run_id, cancel_submit_status) = accepted_run(&cancel_submit);
    let (cancel_runner_id, cancel_lease_token, cancel_claimed_state) = claim_expected_run(
        Arc::clone(&store),
        Some(cancel_scope.clone()),
        cancel_run_id,
        "cancel claim",
    )
    .await?;
    let cancel_requested = store
        .request_cancel(CancelRunRequest {
            scope: cancel_scope.clone(),
            actor,
            run_id: cancel_run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new(format!("idem-{key}-cancel"))?,
        })
        .await?;
    ensure_status(
        cancel_requested.status,
        TurnStatus::CancelRequested,
        "request_cancel",
    )?;
    let cancelled = store
        .cancel_run(CancelRunCompletionRequest {
            run_id: cancel_run_id,
            runner_id: cancel_runner_id,
            lease_token: cancel_lease_token,
        })
        .await?;
    ensure_status(cancelled.status, TurnStatus::Cancelled, "cancel_run")?;
    let cancelled_readback = store
        .get_run_state(GetRunStateRequest {
            scope: cancel_scope,
            run_id: cancel_run_id,
        })
        .await?;
    ensure_status(
        cancelled_readback.status,
        TurnStatus::Cancelled,
        "cancel readback",
    )?;

    Ok(status_code(complete_submit_status)
        ^ (status_code(claimed_state.status) << 4)
        ^ (option_code(claimed_state.checkpoint_id.is_some()) << 8)
        ^ (status_code(blocked.status) << 12)
        ^ (status_code(resumed.status) << 16)
        ^ (status_code(reclaimed_state.status) << 20)
        ^ (option_code(reclaimed_state.checkpoint_id.is_some()) << 24)
        ^ (status_code(completed.status) << 28)
        ^ (status_code(completed_readback.status) << 32)
        ^ (status_code(cancel_submit_status) << 36)
        ^ (status_code(cancel_claimed_state.status) << 40)
        ^ (option_code(cancel_claimed_state.checkpoint_id.is_some()) << 44)
        ^ (status_code(cancel_requested.status) << 48)
        ^ (status_code(cancelled.status) << 52)
        ^ (status_code(cancelled_readback.status) << 56)
        ^ checkpoint_code)
}

async fn record_turn_lifecycle_checkpoints(
    store: Arc<dyn TurnLifecycleStore>,
    scope: &TurnScope,
    turn_id: TurnId,
    run_id: TurnRunId,
    first_state_ref: LoopCheckpointStateRef,
    key: &str,
    payload_len: usize,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let checkpoint_count = turn_lifecycle_checkpoint_count(payload_len);
    let schema_id = CheckpointSchemaId::new("latency_turn_state")?;
    let schema_version = RunProfileVersion::new(1);
    let mut checksum = checkpoint_count as u64;

    for index in 0..checkpoint_count {
        let state_ref = if index == 0 {
            first_state_ref.clone()
        } else {
            LoopCheckpointStateRef::new(format!("checkpoint:latency-{key}:{index}"))?
        };
        let record = store
            .put_loop_checkpoint(PutLoopCheckpointRequest {
                scope: scope.clone(),
                turn_id,
                run_id,
                state_ref,
                schema_id: schema_id.clone(),
                schema_version,
                kind: LoopCheckpointKind::BeforeBlock,
                gate_ref: None,
            })
            .await?;
        let readback = store
            .get_loop_checkpoint(GetLoopCheckpointRequest {
                scope: scope.clone(),
                turn_id,
                run_id,
                checkpoint_id: record.checkpoint_id,
            })
            .await?
            .ok_or_else(|| format!("checkpoint {index} missing after put"))?;
        if readback != record {
            return Err(format!("checkpoint {index} readback did not match put").into());
        }
        checksum ^= ((index as u64 + 1) << (index % 16)) ^ schema_version.as_u64();
    }

    Ok(checksum)
}

fn turn_lifecycle_checkpoint_count(payload_len: usize) -> usize {
    (payload_len / 256).clamp(1, 16)
}

async fn claim_expected_run(
    store: Arc<dyn TurnLifecycleStore>,
    scope_filter: Option<TurnScope>,
    expected_run_id: TurnRunId,
    operation: &'static str,
) -> Result<
    (TurnRunnerId, TurnLeaseToken, ironclaw_turns::TurnRunState),
    Box<dyn std::error::Error + Send + Sync>,
> {
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter,
        })
        .await?
        .ok_or_else(|| format!("{operation} did not claim a run"))?;
    if claimed.state.run_id != expected_run_id {
        return Err(format!(
            "{operation} claimed {}, expected {expected_run_id}",
            claimed.state.run_id
        )
        .into());
    }
    ensure_status(claimed.state.status, TurnStatus::Running, operation)?;
    Ok((runner_id, lease_token, claimed.state))
}

fn turn_lifecycle_key(
    backend: BackendName,
    postgres_pool_size: Option<usize>,
    run_id: &str,
    sample: usize,
) -> String {
    let pool_label = postgres_pool_size
        .map(|pool_size| format!("p{pool_size}"))
        .unwrap_or_else(|| "base".to_string());
    format!("{}-{pool_label}-{run_id}-{sample}", backend.as_str())
}

fn turn_lifecycle_scope(
    key: &str,
    sample: usize,
    lane: &str,
) -> Result<TurnScope, Box<dyn std::error::Error + Send + Sync>> {
    let owner = turn_lifecycle_user(sample)?;
    Ok(TurnScope::new_with_owner(
        TenantId::new(format!("latency-turn-tenant-{lane}"))?,
        Some(AgentId::new(format!("latency-turn-agent-{lane}"))?),
        Some(ProjectId::new(format!("latency-turn-project-{lane}"))?),
        ThreadId::new(format!("latency-turn-{lane}-{key}"))?,
        Some(owner),
    ))
}

fn turn_lifecycle_actor(
    sample: usize,
) -> Result<TurnActor, Box<dyn std::error::Error + Send + Sync>> {
    Ok(TurnActor::new(turn_lifecycle_user(sample)?))
}

fn turn_lifecycle_user(sample: usize) -> Result<UserId, Box<dyn std::error::Error + Send + Sync>> {
    Ok(UserId::new(format!("latency-turn-user-{}", sample % 8))?)
}

fn turn_lifecycle_submit_request(
    scope: TurnScope,
    actor: TurnActor,
    key: &str,
    lane: &str,
    payload_len: usize,
) -> Result<SubmitTurnRequest, Box<dyn std::error::Error + Send + Sync>> {
    let pad_len = payload_len.min(96);
    let pad = "x".repeat(pad_len);
    Ok(SubmitTurnRequest {
        scope,
        actor,
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{lane}-{key}-{pad}"))?,
        source_binding_ref: SourceBindingRef::new(format!("source-{lane}-{key}"))?,
        reply_target_binding_ref: ReplyTargetBindingRef::new(format!("reply-{lane}-{key}"))?,
        requested_run_profile: Some(RunProfileRequest::new("default")?),
        requested_model: None,
        idempotency_key: IdempotencyKey::new(format!("idem-{lane}-{key}"))?,
        received_at: Utc.with_ymd_and_hms(2026, 7, 5, 0, 0, 0).unwrap(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    })
}

fn accepted_run(response: &SubmitTurnResponse) -> (TurnId, TurnRunId, TurnStatus) {
    let SubmitTurnResponse::Accepted {
        turn_id,
        run_id,
        status,
        ..
    } = response;
    (*turn_id, *run_id, *status)
}

fn ensure_status(
    actual: TurnStatus,
    expected: TurnStatus,
    operation: &'static str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if actual == expected {
        return Ok(());
    }
    Err(format!("{operation} returned {actual:?}, expected {expected:?}").into())
}

fn status_code(status: TurnStatus) -> u64 {
    match status {
        TurnStatus::Queued => 1,
        TurnStatus::Running => 2,
        TurnStatus::BlockedApproval => 3,
        TurnStatus::BlockedAuth => 4,
        TurnStatus::BlockedResource => 5,
        TurnStatus::BlockedDependentRun => 6,
        TurnStatus::BlockedExternalTool => 7,
        TurnStatus::CancelRequested => 8,
        TurnStatus::Cancelled => 9,
        TurnStatus::Completed => 10,
        TurnStatus::Failed => 11,
        TurnStatus::RecoveryRequired => 12,
    }
}

pub(super) fn option_code(present: bool) -> u64 {
    if present { 1 } else { 0 }
}
