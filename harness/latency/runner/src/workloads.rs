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
    approval_requests: Arc<dyn ApprovalRequestStorePort>,
    secret_store: Arc<dyn SecretStorePort>,
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
        invocation_id: ironclaw_host_api::ids::InvocationId::new(),
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

    let lifecycle_scope = turn_lifecycle_scope(&key, sample, "process")?;
    let submitted = store
        .submit_turn(
            turn_lifecycle_submit_request(
                lifecycle_scope.clone(),
                actor.clone(),
                &key,
                "process",
                payload_len,
            )?,
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await?;
    let (_turn_id, run_id, submit_status) = accepted_run(&submitted);
    let queued = store
        .get_run_state(GetRunStateRequest {
            scope: lifecycle_scope.clone(),
            run_id,
        })
        .await?;
    ensure_status(queued.status, TurnStatus::Queued, "queued readback")?;
    let cancel_requested = store
        .request_cancel(CancelRunRequest {
            scope: lifecycle_scope.clone(),
            actor,
            run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new(format!("idem-{key}-cancel"))?,
        })
        .await?;
    if !matches!(
        cancel_requested.status,
        TurnStatus::CancelRequested | TurnStatus::Cancelled
    ) {
        return Err(format!(
            "request_cancel returned unexpected status {:?}",
            cancel_requested.status
        )
        .into());
    }
    let readback = store
        .get_run_state(GetRunStateRequest {
            scope: lifecycle_scope,
            run_id,
        })
        .await?;
    Ok(status_code(submit_status)
        ^ (status_code(queued.status) << 8)
        ^ (status_code(cancel_requested.status) << 16)
        ^ (status_code(readback.status) << 24)
        ^ u64::try_from(payload_len).unwrap_or(u64::MAX))
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
subagent_activation_provenance: None,
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
