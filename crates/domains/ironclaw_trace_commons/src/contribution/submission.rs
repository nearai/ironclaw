//! Submission orchestration: submit, flush, status sync, revoke, and the
//! scoped credit views built from local records.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::LazyLock;

use chrono::Utc;
use uuid::Uuid;

use super::*;

pub async fn submit_trace_envelope_to_endpoint(
    envelope: &TraceContributionEnvelope,
    endpoint: &str,
    bearer_token_env: &str,
) -> anyhow::Result<TraceSubmissionReceipt> {
    let provider = StaticEnvTraceUploadCredentialProvider { bearer_token_env };
    let policy = StandingTraceContributionPolicy::default().set_bearer_token_env(bearer_token_env);
    submit_trace_envelope_to_endpoint_with_credential_provider(
        envelope, endpoint, &policy, &provider, None, None,
    )
    .await
}

pub async fn submit_trace_envelope_to_endpoint_with_policy(
    envelope: &TraceContributionEnvelope,
    endpoint: &str,
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<TraceSubmissionReceipt> {
    submit_trace_envelope_to_endpoint_with_credential_provider(
        envelope,
        endpoint,
        policy,
        &DefaultTraceUploadCredentialProvider,
        None,
        None,
    )
    .await
}

pub(crate) async fn submit_trace_envelope_to_endpoint_with_credential_provider(
    envelope: &TraceContributionEnvelope,
    endpoint: &str,
    policy: &StandingTraceContributionPolicy,
    provider: &dyn TraceUploadCredentialProvider,
    scope_dir: Option<&Path>,
    subject: Option<String>,
) -> anyhow::Result<TraceSubmissionReceipt> {
    let context = {
        let ctx = TraceUploadClaimContext::for_envelope(envelope);
        let ctx = if let Some(dir) = scope_dir {
            ctx.with_scope_dir(dir.to_path_buf())
        } else {
            ctx
        };
        ctx.with_subject(subject)
    };
    let token = provider.bearer_token(policy, &context, false).await?;
    match submit_trace_envelope_to_endpoint_with_token(envelope, endpoint, &token).await {
        Ok(receipt) => Ok(receipt),
        Err(error) if error.auth_rejection() => {
            let refreshed = provider.bearer_token(policy, &context, true).await?;
            submit_trace_envelope_to_endpoint_with_token(envelope, endpoint, &refreshed)
                .await
                .map_err(anyhow::Error::from)
        }
        Err(error) => Err(anyhow::Error::from(error)),
    }
}

pub(crate) async fn submit_trace_envelope_to_endpoint_with_token(
    envelope: &TraceContributionEnvelope,
    endpoint: &str,
    token: &str,
) -> Result<TraceSubmissionReceipt, TraceRemoteRequestFailure> {
    let response = pinned_trace_remote_http_client(endpoint)
        .await?
        .post(endpoint)
        .bearer_auth(token)
        .header("Idempotency-Key", envelope.submission_id.to_string())
        .json(envelope)
        .send()
        .await
        .map_err(|error| TraceRemoteRequestFailure::request_failed("trace submission", error))?;
    let status = response.status();
    let body = response.text().await;
    if !status.is_success() {
        // The rejection is classified by status; the body is best-effort
        // detail, so a failed read degrades to an empty message, not an
        // error-shape change.
        return Err(TraceRemoteRequestFailure::http_rejection(
            "trace submission",
            status,
            body.unwrap_or_default(),
        ));
    }
    // On a 2xx the body IS the receipt: a stream that dies mid-read is a
    // transport failure and must keep its I/O cause (and its network
    // telemetry kind) rather than collapse into an empty body that the
    // strict parse below would misreport as a server-protocol violation.
    let body = body.map_err(|error| {
        TraceRemoteRequestFailure::request_failed("trace submission response body", error)
    })?;

    // A 2xx whose body does not parse is not a receipt. Synthesizing
    // `status: "submitted"` with a *locally estimated* credit told the user the
    // server had accepted something it may never have seen — and the caller then
    // recorded it as Submitted and deleted the queued envelope, destroying the
    // only retryable copy (#7144). Every field of `TraceSubmissionReceipt` has a
    // serde default, so `{}` already parses; reaching this branch means the body
    // was not a JSON object at all (proxy HTML, a truncated read), which is
    // exactly when inventing success is least defensible.
    parse_trace_submission_receipt(&body).ok_or_else(|| {
        TraceRemoteRequestFailure::response_invalid(
            "trace submission",
            "server returned success with a body that is not a submission receipt",
        )
    })
}

pub fn record_submitted_trace_envelope_for_scope(
    scope: Option<&str>,
    envelope: &TraceContributionEnvelope,
    endpoint: &str,
    receipt: TraceSubmissionReceipt,
) -> anyhow::Result<()> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    record_submitted_trace_envelope_for_scope_unlocked(scope, envelope, endpoint, receipt)
}

pub(crate) fn record_submitted_trace_envelope_for_scope_unlocked(
    scope: Option<&str>,
    envelope: &TraceContributionEnvelope,
    endpoint: &str,
    receipt: TraceSubmissionReceipt,
) -> anyhow::Result<()> {
    let credit_points_pending = receipt
        .credit_points_pending
        .unwrap_or(envelope.value.credit_points_pending);
    let credit_points_final = receipt.credit_points_final;
    let credit_explanation = if receipt.explanation.is_empty() {
        envelope.value.explanation.clone()
    } else {
        receipt.explanation
    };

    upsert_local_trace_record_for_scope(
        scope,
        NodeTraceSubmissionRecord {
            submission_id: envelope.submission_id,
            trace_id: envelope.trace_id,
            endpoint: Some(endpoint.to_string()),
            status: NodeTraceSubmissionStatus::Submitted,
            server_status: Some(receipt.status),
            submitted_at: Some(Utc::now()),
            revoked_at: None,
            privacy_risk: format!("{:?}", envelope.privacy.residual_pii_risk),
            redaction_counts: envelope.privacy.redaction_counts.clone(),
            credit_points_pending,
            credit_points_final,
            credit_explanation,
            credit_events: vec![TraceCreditEvent {
                event_id: Uuid::new_v4(),
                submission_id: envelope.submission_id,
                contributor_pseudonym: envelope
                    .contributor
                    .pseudonymous_contributor_id
                    .clone()
                    .unwrap_or_else(|| "anonymous".to_string()),
                kind: TraceCreditEventKind::Accepted,
                points_delta: credit_points_pending,
                reason: "Accepted for private Trace Commons processing; delayed utility credit may be added later.".to_string(),
                created_at: Utc::now(),
            }],
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: TraceCreditNoticeState::default(),
        },
    )
}

pub async fn flush_trace_contribution_queue_for_scope(
    scope: Option<&str>,
    limit: usize,
) -> anyhow::Result<TraceQueueFlushReport> {
    flush_trace_contribution_queue_for_scope_with_credential_provider(
        scope,
        limit,
        &DefaultTraceUploadCredentialProvider,
    )
    .await
}

pub(crate) async fn flush_trace_contribution_queue_for_scope_with_credential_provider(
    scope: Option<&str>,
    limit: usize,
    provider: &dyn TraceUploadCredentialProvider,
) -> anyhow::Result<TraceQueueFlushReport> {
    let _guard = lock_trace_scope_for_mutation(scope).await;
    let flush_started_at = Utc::now();
    record_trace_queue_flush_attempt_for_scope_unlocked(scope, flush_started_at)?;

    // Resolve which enrollment this scope contributes under in a single
    // policy-read/path pass. A personal-invite enrollment uses the per-scope
    // policy + per-scope device-key dir + no subject; an instance enrollment
    // (no enabled per-scope policy, but the admin-provisioned instance policy at
    // scope None is enabled) uses the instance policy + instance device-key dir
    // + a per-user pseudonymous subject. `Ok(None)` means unenrolled and the
    // flush aborts, exactly as before.
    let target = match resolve_effective_flush_target(scope) {
        Ok(target) => target,
        Err(error) => {
            record_trace_queue_flush_failure_for_scope_unlocked(scope, &error, flush_started_at)?;
            return Err(error);
        }
    };
    let Some(EffectiveFlushTarget {
        policy,
        device_key_dir: scope_dir,
        subject,
    }) = target
    else {
        let error = anyhow::anyhow!("trace contribution opt-in is disabled");
        record_trace_queue_flush_failure_for_scope_unlocked(scope, &error, flush_started_at)?;
        return Err(error);
    };
    let Some(endpoint) = policy.ingestion_endpoint.as_deref() else {
        let error = anyhow::anyhow!("trace contribution endpoint is not configured");
        record_trace_queue_flush_failure_for_scope_unlocked(scope, &error, flush_started_at)?;
        return Err(error);
    };

    let compaction = match compact_trace_queue_for_scope_unlocked(scope) {
        Ok(report) => report,
        Err(error) => {
            record_trace_queue_flush_failure_for_scope_unlocked(scope, &error, flush_started_at)?;
            return Err(error);
        }
    };
    let mut submitted = 0usize;
    let mut holds = Vec::new();
    let mut had_nonfatal_failure = false;
    for path in queued_trace_envelope_paths_for_scope(scope)?
        .into_iter()
        .take(limit)
    {
        let Some(mut envelope) = load_queued_trace_envelope_or_quarantine(scope, &path, "flush")?
        else {
            had_nonfatal_failure = true;
            continue;
        };
        apply_credit_estimate_to_envelope(&mut envelope);

        match trace_autonomous_eligibility(&envelope, &policy) {
            TraceQueueEligibility::Submit => {
                if let Some(hold) = retry_hold_if_not_due(&path, Utc::now())? {
                    holds.push(hold);
                    continue;
                }
                let receipt = match submit_trace_envelope_to_endpoint_with_credential_provider(
                    &envelope,
                    endpoint,
                    &policy,
                    provider,
                    Some(&scope_dir),
                    subject.clone(),
                )
                .await
                {
                    Ok(receipt) => receipt,
                    Err(error) => {
                        record_trace_queue_retryable_submission_failure_for_scope_unlocked(
                            scope,
                            &error,
                            Utc::now(),
                        )?;
                        had_nonfatal_failure = true;
                        let hold = retry_hold_after_submission_failure(
                            &path,
                            envelope.submission_id,
                            &error,
                            Utc::now(),
                        )?;
                        if let Err(hold_error) =
                            write_trace_queue_hold_sidecar_for_path(&path, &hold)
                        {
                            tracing::debug!(
                                error = %hold_error,
                                submission_id = %envelope.submission_id,
                                "Failed to write retry hold reason for trace submission"
                            );
                        }
                        holds.push(hold);
                        continue;
                    }
                };
                record_submitted_trace_envelope_for_scope_unlocked(
                    scope, &envelope, endpoint, receipt,
                )?;
                std::fs::remove_file(&path).map_err(|e| {
                    anyhow::anyhow!("failed to remove queued envelope {}: {}", path.display(), e)
                })?;
                submitted += 1;
            }
            TraceQueueEligibility::Hold { kind, reason } => {
                let hold = TraceQueueHold {
                    submission_id: envelope.submission_id,
                    kind,
                    reason: safe_trace_queue_hold_reason(&reason),
                    attempts: 0,
                    next_retry_at: None,
                };
                write_trace_queue_hold_sidecar_for_path(&path, &hold)?;
                holds.push(hold);
            }
        }
    }

    // Flush keeps the scoped lock through submission and status-sync network calls
    // so another same-scope flush cannot submit or remove the same queue file.
    // Sync with the SAME resolved target (policy, device-key dir, subject) the
    // submissions above used, so instance-enrolled scopes get their final
    // credit status instead of a per-scope re-read that resolves to a disabled
    // personal policy.
    match sync_remote_trace_submission_records_for_scope_unlocked_with_target(
        scope,
        &policy,
        &scope_dir,
        subject.as_deref(),
        provider,
    )
    .await
    {
        Ok(_) => record_trace_queue_status_sync_success_for_scope_unlocked(scope, Utc::now())?,
        Err(error) => {
            record_trace_queue_status_sync_failure_for_scope_unlocked(scope, &error, Utc::now())?;
            had_nonfatal_failure = true;
            tracing::debug!(%error, "Failed to sync remote Trace Commons credit status");
        }
    }

    let credit_notice =
        mark_trace_credit_noticed_if_due_unlocked(scope, policy.credit_notice_interval_hours)?;
    record_trace_queue_flush_success_for_scope_unlocked(scope, Utc::now(), !had_nonfatal_failure)?;
    Ok(TraceQueueFlushReport {
        submitted,
        held: holds.len(),
        compaction,
        holds,
        credit_notice,
    })
}

pub async fn flush_trace_contribution_queue_worker_tick<I, S>(
    scopes: I,
    limit_per_scope: usize,
) -> anyhow::Result<TraceQueueWorkerReport>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut report = TraceQueueWorkerReport {
        scopes_checked: 0,
        submitted: 0,
        held: 0,
        scope_reports: Vec::new(),
    };
    let mut seen = BTreeSet::new();

    for scope in scopes {
        let scope = scope.as_ref().trim();
        if scope.is_empty() || !seen.insert(scope.to_string()) {
            continue;
        }
        report.scopes_checked += 1;
        let scope_report =
            match flush_trace_contribution_queue_for_scope(Some(scope), limit_per_scope).await {
                Ok(flush) => TraceQueueWorkerScopeReport {
                    scope: scope.to_string(),
                    submitted: flush.submitted,
                    held: flush.held,
                    holds: flush.holds,
                    credit_notice: flush.credit_notice,
                },
                Err(error) => {
                    tracing::debug!(
                        %error,
                        scope_hash = %scope_hash(scope),
                        "Trace Commons queue worker skipped scope"
                    );
                    continue;
                }
            };
        report.submitted += scope_report.submitted;
        report.held += scope_report.held;
        report.scope_reports.push(scope_report);
    }

    Ok(report)
}

pub async fn sync_remote_trace_submission_records_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<usize> {
    sync_remote_trace_submission_records_for_scope_with_credential_provider(
        scope,
        &DefaultTraceUploadCredentialProvider,
    )
    .await
}

pub(crate) async fn sync_remote_trace_submission_records_for_scope_with_credential_provider(
    scope: Option<&str>,
    provider: &dyn TraceUploadCredentialProvider,
) -> anyhow::Result<usize> {
    // Resolve the effective enrollment (personal-invite or instance) the same
    // way the flush does, so instance-enrolled scopes sync with the instance
    // policy + device key + per-user subject instead of a disabled per-scope
    // policy read that would silently return Ok(0).
    let Some(EffectiveFlushTarget {
        policy,
        device_key_dir,
        subject,
    }) = resolve_effective_flush_target(scope)?
    else {
        return Ok(0);
    };
    let Some(endpoint) = policy.ingestion_endpoint.as_deref() else {
        return Ok(0);
    };

    let submission_ids = {
        let _guard = lock_trace_scope_for_mutation(scope).await;
        let records = read_local_trace_records_for_scope(scope)?;
        records
            .iter()
            .filter(|record| record.status == NodeTraceSubmissionStatus::Submitted)
            .map(|record| record.submission_id)
            .collect::<Vec<_>>()
    };
    if submission_ids.is_empty() {
        return Ok(0);
    }

    let status_endpoint = trace_submission_status_endpoint(endpoint)?;
    let updates = fetch_trace_submission_statuses_with_credential_provider(
        &status_endpoint,
        &policy,
        provider,
        &submission_ids,
        Some(&device_key_dir),
        subject.as_deref(),
    )
    .await?;
    let _guard = lock_trace_scope_for_mutation(scope).await;
    apply_remote_trace_submission_statuses_for_scope_unlocked(scope, &updates)
}

/// Status-sync core used by the queue flush: syncs the local records of
/// `scope` against the remote, authenticating with the caller-resolved
/// effective flush target (`policy` + `device_key_dir` + `subject`) rather
/// than re-reading the per-scope policy. An instance-enrolled user has no
/// enabled per-scope policy and its device key lives at the instance dir, so
/// re-reading here would silently sync nothing (or with the wrong credential
/// context) right after a successful instance-attributed submission.
pub(crate) async fn sync_remote_trace_submission_records_for_scope_unlocked_with_target(
    scope: Option<&str>,
    policy: &StandingTraceContributionPolicy,
    device_key_dir: &Path,
    subject: Option<&str>,
    provider: &dyn TraceUploadCredentialProvider,
) -> anyhow::Result<usize> {
    if !policy.enabled {
        return Ok(0);
    }
    let Some(endpoint) = policy.ingestion_endpoint.as_deref() else {
        return Ok(0);
    };

    let records = read_local_trace_records_for_scope(scope)?;
    let submission_ids = records
        .iter()
        .filter(|record| record.status == NodeTraceSubmissionStatus::Submitted)
        .map(|record| record.submission_id)
        .collect::<Vec<_>>();
    if submission_ids.is_empty() {
        return Ok(0);
    }

    let status_endpoint = trace_submission_status_endpoint(endpoint)?;
    let updates = fetch_trace_submission_statuses_with_credential_provider(
        &status_endpoint,
        policy,
        provider,
        &submission_ids,
        Some(device_key_dir),
        subject,
    )
    .await?;
    apply_remote_trace_submission_statuses_for_scope_unlocked(scope, &updates)
}

pub fn trace_submission_status_endpoint(submission_endpoint: &str) -> anyhow::Result<String> {
    let mut url = reqwest::Url::parse(submission_endpoint).map_err(|e| {
        anyhow::anyhow!(
            "invalid trace contribution endpoint {}: {}",
            submission_endpoint,
            e
        )
    })?;
    let path = url.path().trim_end_matches('/');
    let replacement = if let Some(prefix) = path.strip_suffix("/v1/traces") {
        format!(
            "{}/v1/contributors/me/submission-status",
            prefix.trim_end_matches('/')
        )
    } else if let Some(prefix) = path.strip_suffix("/traces") {
        format!(
            "{}/contributors/me/submission-status",
            prefix.trim_end_matches('/')
        )
    } else {
        format!(
            "{}/v1/contributors/me/submission-status",
            path.trim_end_matches('/')
        )
    };
    url.set_path(if replacement.starts_with('/') {
        &replacement
    } else {
        "/v1/contributors/me/submission-status"
    });
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

pub async fn fetch_trace_submission_statuses(
    status_endpoint: &str,
    bearer_token_env: &str,
    submission_ids: &[Uuid],
) -> anyhow::Result<Vec<TraceSubmissionStatusUpdate>> {
    let provider = StaticEnvTraceUploadCredentialProvider { bearer_token_env };
    let policy = StandingTraceContributionPolicy::default().set_bearer_token_env(bearer_token_env);
    fetch_trace_submission_statuses_with_credential_provider(
        status_endpoint,
        &policy,
        &provider,
        submission_ids,
        None,
        None,
    )
    .await
}

pub async fn fetch_trace_submission_statuses_with_policy(
    status_endpoint: &str,
    policy: &StandingTraceContributionPolicy,
    submission_ids: &[Uuid],
) -> anyhow::Result<Vec<TraceSubmissionStatusUpdate>> {
    fetch_trace_submission_statuses_with_credential_provider(
        status_endpoint,
        policy,
        &DefaultTraceUploadCredentialProvider,
        submission_ids,
        None,
        None,
    )
    .await
}

pub(crate) async fn fetch_trace_submission_statuses_with_credential_provider(
    status_endpoint: &str,
    policy: &StandingTraceContributionPolicy,
    provider: &dyn TraceUploadCredentialProvider,
    submission_ids: &[Uuid],
    scope_dir: Option<&Path>,
    subject: Option<&str>,
) -> anyhow::Result<Vec<TraceSubmissionStatusUpdate>> {
    let context = {
        let ctx =
            TraceUploadClaimContext::for_status_sync().with_subject(subject.map(str::to_string));
        if let Some(dir) = scope_dir {
            ctx.with_scope_dir(dir.to_path_buf())
        } else {
            ctx
        }
    };
    let mut updates = Vec::new();

    for chunk in submission_ids.chunks(200) {
        let token = provider.bearer_token(policy, &context, false).await?;
        let body =
            match fetch_trace_submission_statuses_chunk_with_token(status_endpoint, chunk, &token)
                .await
            {
                Ok(body) => body,
                Err(error) if error.auth_rejection() => {
                    let refreshed = provider.bearer_token(policy, &context, true).await?;
                    fetch_trace_submission_statuses_chunk_with_token(
                        status_endpoint,
                        chunk,
                        &refreshed,
                    )
                    .await
                    .map_err(anyhow::Error::from)?
                }
                Err(error) => return Err(anyhow::Error::from(error)),
            };
        let mut page: Vec<TraceSubmissionStatusUpdate> = serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("failed to parse trace status sync response: {}", e))?;
        updates.append(&mut page);
    }

    Ok(updates)
}

pub(crate) async fn fetch_trace_submission_statuses_chunk_with_token(
    status_endpoint: &str,
    submission_ids: &[Uuid],
    token: &str,
) -> Result<String, TraceRemoteRequestFailure> {
    let response = pinned_trace_remote_http_client(status_endpoint)
        .await?
        .post(status_endpoint)
        .bearer_auth(token)
        .json(&TraceSubmissionStatusRequest {
            submission_ids: submission_ids.to_vec(),
        })
        .send()
        .await
        .map_err(|error| TraceRemoteRequestFailure::request_failed("trace status sync", error))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(TraceRemoteRequestFailure::http_rejection(
            "trace status sync",
            status,
            body,
        ));
    }
    Ok(body)
}

pub fn apply_remote_trace_submission_statuses_for_scope(
    scope: Option<&str>,
    updates: &[TraceSubmissionStatusUpdate],
) -> anyhow::Result<usize> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    apply_remote_trace_submission_statuses_for_scope_unlocked(scope, updates)
}

pub(crate) fn apply_remote_trace_submission_statuses_for_scope_unlocked(
    scope: Option<&str>,
    updates: &[TraceSubmissionStatusUpdate],
) -> anyhow::Result<usize> {
    if updates.is_empty() {
        return Ok(0);
    }

    let mut records = read_local_trace_records_for_scope(scope)?;
    let mut changed = 0usize;
    let now = Utc::now();
    for update in updates {
        let Some(record) = records
            .iter_mut()
            .find(|record| record.submission_id == update.submission_id)
        else {
            continue;
        };

        let old_effective_credit = record
            .credit_points_final
            .unwrap_or(record.credit_points_pending);
        let new_effective_credit = update
            .credit_points_total
            .or(update.credit_points_final)
            .unwrap_or(update.credit_points_pending);
        let new_stored_final = update.credit_points_total.or(update.credit_points_final);
        let explanation = safe_remote_credit_explanation_lines(update);
        let credit_changed = (old_effective_credit - new_effective_credit).abs() > f32::EPSILON;
        let explanation_changed =
            !explanation.is_empty() && record.credit_explanation != explanation;

        let status_changed = record.server_status.as_deref() != Some(update.status.as_str());
        let credit_delta = new_effective_credit - old_effective_credit;

        record.trace_id = update.trace_id;
        record.server_status = Some(update.status.clone());
        record.credit_points_pending = update.credit_points_pending;
        record.credit_points_final = new_stored_final;
        if !explanation.is_empty() {
            record.credit_explanation = explanation;
        }
        if update.status == "revoked" {
            record.status = NodeTraceSubmissionStatus::Revoked;
            record.revoked_at.get_or_insert(now);
        } else if update.status == "expired" {
            record.status = NodeTraceSubmissionStatus::Expired;
        } else if update.status == "purged" {
            record.status = NodeTraceSubmissionStatus::Purged;
        }

        if status_changed || credit_changed || explanation_changed {
            record.last_credit_notice_at = None;
            record.credit_notice_state = TraceCreditNoticeState::default();
            let sync_reason = if update.credit_points_ledger.abs() > f32::EPSILON {
                format!(
                    "Server status synced as {}; delayed ledger credit now {:+.2}.",
                    update.status, update.credit_points_ledger
                )
            } else {
                format!("Server status synced as {}.", update.status)
            };
            record.credit_events.push(TraceCreditEvent {
                event_id: Uuid::new_v4(),
                submission_id: update.submission_id,
                contributor_pseudonym: "local-sync".to_string(),
                kind: TraceCreditEventKind::CreditSynced,
                points_delta: credit_delta,
                reason: sync_reason,
                created_at: now,
            });
            let history_event = NodeTraceSubmissionHistoryEvent {
                event_id: Uuid::new_v4(),
                kind: NodeTraceSubmissionHistoryKind::StatusSync,
                occurred_at: now,
                server_status: Some(update.status.clone()),
                credit_delta,
                delayed_credit_explanation_count: update
                    .delayed_credit_explanations
                    .len()
                    .try_into()
                    .unwrap_or(u32::MAX),
            };
            if !record.history.iter().any(|event| {
                event.kind == history_event.kind
                    && event.server_status == history_event.server_status
                    && (event.credit_delta - history_event.credit_delta).abs() <= f32::EPSILON
                    && event.delayed_credit_explanation_count
                        == history_event.delayed_credit_explanation_count
            }) {
                record.history.push(history_event);
            }
            changed += 1;
        }
    }

    if changed > 0 {
        write_local_trace_records_for_scope(scope, &records)?;
    }
    Ok(changed)
}

pub(crate) fn safe_remote_credit_explanation_lines(
    update: &TraceSubmissionStatusUpdate,
) -> Vec<String> {
    update
        .explanation
        .iter()
        .chain(update.delayed_credit_explanations.iter())
        .filter_map(|line| {
            let line = safe_remote_credit_explanation_line(line);
            (!line.is_empty()).then_some(line)
        })
        .take(16)
        .collect()
}

pub(crate) fn safe_remote_credit_explanation_line(line: &str) -> String {
    let normalized = line
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if normalized.is_empty() {
        return String::new();
    }
    let (redacted, _) = DeterministicTraceRedactor::default().redact_text(&normalized);
    let redacted = trace_queue_secret_like_reason_regex().replace_all(&redacted, "[REDACTED]");
    let redacted =
        remote_credit_explanation_url_regex().replace_all(&redacted, "[REDACTED:private_url]");
    let redacted = remote_credit_explanation_tenant_ref_regex()
        .replace_all(&redacted, "[REDACTED:tenant_ref]");
    redacted
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .chars()
        .take(240)
        .collect()
}

pub fn read_local_trace_records_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<Vec<NodeTraceSubmissionRecord>> {
    let path = trace_records_path(scope);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let body = std::fs::read_to_string(&path).map_err(|e| {
        anyhow::anyhow!(
            "failed to read local trace submission records {}: {}",
            path.display(),
            e
        )
    })?;
    let records = serde_json::from_str(&body).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse local trace submission records {}: {}",
            path.display(),
            e
        )
    })?;
    Ok(records)
}

/// The credit projection for one scope: the aggregate report plus the
/// manual-review holds awaiting authorization. This is what the WebUI credits
/// surfaces poll for.
#[derive(Debug, Clone)]
pub struct ScopedCreditView {
    pub report: TraceCreditReport,
    pub manual_review_holds: Vec<TraceQueueHold>,
}

/// Cheap change-detection signature of a scope's on-disk credit inputs.
/// `None` for an absent file. Computing the signature is a couple of `stat`s;
/// reading + parsing the full submissions history is what we avoid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CreditViewSignature {
    records: Option<(std::time::SystemTime, u64)>,
    holds: u64,
}

pub(crate) struct CreditViewCacheEntry {
    signature: CreditViewSignature,
    view: ScopedCreditView,
}

/// Per-scope memoization of the computed credit view, keyed by the on-disk
/// signature of the inputs. Bounds polling cost to O(new submissions): when the
/// submissions file and held-trace sidecars are unchanged since the last
/// computation (the steady-state polling case), the request is a couple of
/// `stat`s + a clone, NOT a full-history read/parse/aggregate.
pub(crate) static CREDIT_VIEW_CACHE: LazyLock<
    std::sync::Mutex<HashMap<String, CreditViewCacheEntry>>,
> = LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Hard cap so the cache can't grow one entry per historical caller forever
/// (same bound the trace-queue flush worker observes). Cleared wholesale on
/// overflow — entries are pure memoization and recompute on demand.
pub(crate) const CREDIT_VIEW_CACHE_MAX_SCOPES: usize = 4096;

pub(crate) fn path_change_signature(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    Some((mtime, meta.len()))
}

/// Cheap signature over the scope's `*.held.json` sidecars (manual-review
/// holds): a hash of each sidecar's (name, len, mtime). Scanning the directory
/// entries' metadata is far cheaper than reading + parsing each sidecar.
pub(crate) fn holds_change_signature(scope: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let dir = trace_queue_dir(Some(scope));
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut items: Vec<(String, u64, Option<std::time::SystemTime>)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            if !name.ends_with(".held.json") {
                return None;
            }
            let meta = entry.metadata().ok();
            Some((
                name,
                meta.as_ref().map(|m| m.len()).unwrap_or(0),
                meta.and_then(|m| m.modified().ok()),
            ))
        })
        .collect();
    // Sort so the signature is order-independent across `read_dir` orderings.
    items.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    items.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn current_credit_view_signature(scope: &str) -> CreditViewSignature {
    CreditViewSignature {
        records: path_change_signature(&trace_records_path(Some(scope))),
        holds: holds_change_signature(scope),
    }
}

/// Build the scope's credit view, memoized by the on-disk input signature.
///
/// On a cache hit (inputs unchanged since the last computation) this clones the
/// cached view after a couple of `stat`s. On a miss it reads + aggregates the
/// records and re-scans holds once, then caches the result. Genuine read/parse
/// failures propagate (a missing file is already softened to the zero/empty
/// state inside the underlying readers).
pub fn scoped_credit_view(scope: &str) -> anyhow::Result<ScopedCreditView> {
    let signature = current_credit_view_signature(scope);
    {
        let cache = match CREDIT_VIEW_CACHE.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(entry) = cache.get(scope)
            && entry.signature == signature
        {
            return Ok(entry.view.clone());
        }
    }

    // Miss: recompute once.
    let records = read_local_trace_records_for_scope(Some(scope))?;
    let report = trace_credit_report(&records);
    let manual_review_holds = manual_review_holds_for_scope(Some(scope))?;
    let view = ScopedCreditView {
        report,
        manual_review_holds,
    };

    let mut cache = match CREDIT_VIEW_CACHE.lock() {
        Ok(cache) => cache,
        Err(poisoned) => poisoned.into_inner(),
    };
    if cache.len() >= CREDIT_VIEW_CACHE_MAX_SCOPES && !cache.contains_key(scope) {
        cache.clear();
    }
    cache.insert(
        scope.to_string(),
        CreditViewCacheEntry {
            signature,
            view: view.clone(),
        },
    );
    Ok(view)
}

pub fn trace_credit_summary(records: &[NodeTraceSubmissionRecord]) -> CreditSummary {
    let report = trace_credit_report(records);
    CreditSummary {
        submissions_total: report.submissions_total,
        submissions_submitted: report.submissions_submitted,
        submissions_revoked: report.submissions_revoked,
        submissions_expired: report.submissions_expired,
        pending_credit: report.pending_credit,
        final_credit: report.final_credit,
        delayed_credit_delta: report.delayed_credit_delta,
        credit_events_total: report.credit_events_total,
        recent_explanations: recent_trace_credit_explanations(records, 6),
    }
}

pub fn trace_credit_report(records: &[NodeTraceSubmissionRecord]) -> TraceCreditReport {
    let submissions_submitted = records
        .iter()
        .filter(|record| record.status == NodeTraceSubmissionStatus::Submitted)
        .count() as u32;
    let submissions_revoked = records
        .iter()
        .filter(|record| record.status == NodeTraceSubmissionStatus::Revoked)
        .count() as u32;
    let submissions_expired = records
        .iter()
        .filter(|record| {
            matches!(
                record.status,
                NodeTraceSubmissionStatus::Expired | NodeTraceSubmissionStatus::Purged
            )
        })
        .count() as u32;

    let submissions_accepted = records
        .iter()
        .filter(|record| local_trace_server_status_matches(record, "accepted"))
        .count() as u32;
    let submissions_quarantined = records
        .iter()
        .filter(|record| local_trace_server_status_matches(record, "quarantined"))
        .count() as u32;
    let submissions_rejected = records
        .iter()
        .filter(|record| local_trace_server_status_matches(record, "rejected"))
        .count() as u32;

    let pending_credit = records
        .iter()
        .map(|record| record.credit_points_pending)
        .sum();
    let final_credit = records
        .iter()
        .filter_map(|record| record.credit_points_final)
        .sum();
    let credit_events_total = records
        .iter()
        .map(|record| record.credit_events.len() as u32)
        .sum();
    let delayed_credit_delta = records
        .iter()
        .flat_map(|record| record.credit_events.iter())
        .filter(|event| event.kind != TraceCreditEventKind::Accepted)
        .map(|event| event.points_delta)
        .sum();
    let last_submission_at = records
        .iter()
        .filter_map(|record| record.submitted_at)
        .max();
    let last_credit_sync_at = records
        .iter()
        .flat_map(|record| record.credit_events.iter())
        .filter(|event| event.kind == TraceCreditEventKind::CreditSynced)
        .map(|event| event.created_at)
        .max();

    let explanation_lines = trace_credit_report_explanation_lines(
        records,
        submissions_accepted,
        submissions_quarantined,
        submissions_rejected,
        pending_credit,
        final_credit,
        delayed_credit_delta,
    );

    TraceCreditReport {
        submissions_total: records.len() as u32,
        submissions_submitted,
        submissions_revoked,
        submissions_expired,
        submissions_accepted,
        submissions_quarantined,
        submissions_rejected,
        pending_credit,
        final_credit,
        credit_events_total,
        delayed_credit_delta,
        last_submission_at,
        last_credit_sync_at,
        explanation_lines,
    }
}

pub(crate) fn local_trace_server_status_matches(
    record: &NodeTraceSubmissionRecord,
    expected: &str,
) -> bool {
    record
        .server_status
        .as_deref()
        .map(|status| status.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

pub(crate) fn trace_credit_report_explanation_lines(
    records: &[NodeTraceSubmissionRecord],
    submissions_accepted: u32,
    submissions_quarantined: u32,
    submissions_rejected: u32,
    pending_credit: f32,
    final_credit: f32,
    delayed_credit_delta: f32,
) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!(
        "{} submitted trace(s): {} accepted, {} quarantined, {} rejected.",
        records.len(),
        submissions_accepted,
        submissions_quarantined,
        submissions_rejected
    ));
    lines.push(format!(
        "Credit totals: pending +{:.2}, final confirmed +{:.2}.",
        pending_credit, final_credit
    ));
    if delayed_credit_delta.abs() > f32::EPSILON {
        lines.push(format!(
            "Delayed ledger adjustments currently total {:+.2}.",
            delayed_credit_delta
        ));
    }
    lines.extend(recent_trace_credit_explanations(records, 6));
    lines
}

pub(crate) fn recent_trace_credit_explanations(
    records: &[NodeTraceSubmissionRecord],
    limit: usize,
) -> Vec<String> {
    records
        .iter()
        .rev()
        .flat_map(|record| record.credit_explanation.iter().cloned())
        .take(limit)
        .collect()
}

pub async fn revoke_trace_submission_for_scope(
    scope: Option<&str>,
    submission_id: Uuid,
    endpoint: Option<&str>,
    bearer_token_env: &str,
) -> anyhow::Result<()> {
    let provider = StaticEnvTraceUploadCredentialProvider { bearer_token_env };
    let policy = StandingTraceContributionPolicy::default().set_bearer_token_env(bearer_token_env);
    revoke_trace_submission_for_scope_with_credential_provider(
        scope,
        submission_id,
        endpoint,
        &policy,
        &provider,
    )
    .await
}

pub async fn revoke_trace_submission_for_scope_with_policy(
    scope: Option<&str>,
    submission_id: Uuid,
    endpoint: Option<&str>,
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<()> {
    revoke_trace_submission_for_scope_with_credential_provider(
        scope,
        submission_id,
        endpoint,
        policy,
        &DefaultTraceUploadCredentialProvider,
    )
    .await
}

pub(crate) async fn revoke_trace_submission_for_scope_with_credential_provider(
    scope: Option<&str>,
    submission_id: Uuid,
    endpoint: Option<&str>,
    policy: &StandingTraceContributionPolicy,
    provider: &dyn TraceUploadCredentialProvider,
) -> anyhow::Result<()> {
    if let Some(endpoint) = endpoint {
        // Compute the scope's base directory so DeviceKey auth mode can locate
        // the per-tenant keypair when self-signing the revoke request bearer.
        let scope_dir = trace_contribution_dir_for_scope(scope);
        revoke_trace_submission_at_endpoint_with_credential_provider(
            submission_id,
            endpoint,
            policy,
            provider,
            Some(&scope_dir),
        )
        .await?;
    }

    let _guard = lock_trace_scope_for_mutation(scope).await;
    mark_local_trace_revoked_for_scope_unlocked(scope, submission_id)
}

pub async fn revoke_trace_submission_at_endpoint_with_policy(
    submission_id: Uuid,
    endpoint: &str,
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<()> {
    revoke_trace_submission_at_endpoint_with_credential_provider(
        submission_id,
        endpoint,
        policy,
        &DefaultTraceUploadCredentialProvider,
        None,
    )
    .await
}

pub(crate) async fn revoke_trace_submission_at_endpoint_with_credential_provider(
    submission_id: Uuid,
    endpoint: &str,
    policy: &StandingTraceContributionPolicy,
    provider: &dyn TraceUploadCredentialProvider,
    scope_dir: Option<&Path>,
) -> anyhow::Result<()> {
    let context = {
        let ctx = TraceUploadClaimContext::for_submission_id(submission_id);
        if let Some(dir) = scope_dir {
            ctx.with_scope_dir(dir.to_path_buf())
        } else {
            ctx
        }
    };
    let token = provider.bearer_token(policy, &context, false).await?;
    match revoke_trace_submission_at_endpoint_with_token(submission_id, endpoint, &token).await {
        Ok(()) => Ok(()),
        Err(error) if error.auth_rejection() => {
            let refreshed = provider.bearer_token(policy, &context, true).await?;
            revoke_trace_submission_at_endpoint_with_token(submission_id, endpoint, &refreshed)
                .await
                .map_err(anyhow::Error::from)
        }
        Err(error) => Err(anyhow::Error::from(error)),
    }
}

pub(crate) async fn revoke_trace_submission_at_endpoint_with_token(
    submission_id: Uuid,
    endpoint: &str,
    token: &str,
) -> Result<(), TraceRemoteRequestFailure> {
    let response = pinned_trace_remote_http_client(endpoint)
        .await?
        .delete(endpoint)
        .bearer_auth(token)
        .json(&serde_json::json!({ "submission_id": submission_id }))
        .send()
        .await
        .map_err(|error| TraceRemoteRequestFailure::request_failed("trace revocation", error))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(TraceRemoteRequestFailure::http_rejection(
            "trace revocation",
            status,
            body,
        ));
    }
    Ok(())
}

pub fn trace_autonomous_eligibility(
    envelope: &TraceContributionEnvelope,
    policy: &StandingTraceContributionPolicy,
) -> TraceQueueEligibility {
    // Fail closed before any submit path: an envelope with no trace-content
    // allowed-uses (e.g. a `public_attribution`-only consent scope, which
    // grants `allowed_uses = []`) is not submittable — there is no use the
    // remote side would accept it for. Reject it here rather than relying on
    // the remote to bounce it, even ahead of the manual-review bypass (an
    // authorized hold with no allowed-uses still has nothing to submit).
    if envelope.trace_card.allowed_uses.is_empty() {
        return TraceQueueEligibility::Hold {
            kind: TraceQueueHoldKind::PolicyGate,
            reason: "trace grants no allowed-uses and is not submittable".to_string(),
        };
    }

    // An explicitly user-authorized held trace submits as-is, bypassing every
    // gate (PII manual-review, score, tool-allowlist). The user reviewed the
    // already-redacted trace and accepted its residual risk.
    if envelope.manual_review_authorized {
        return TraceQueueEligibility::Submit;
    }

    if policy.require_manual_approval_when_pii_detected
        && envelope.privacy.residual_pii_risk == ResidualPiiRisk::High
    {
        return TraceQueueEligibility::Hold {
            kind: TraceQueueHoldKind::ManualReview,
            reason: "manual review required because residual privacy risk is high".to_string(),
        };
    }

    if !policy.selected_tools.is_empty()
        && envelope
            .replay
            .required_tools
            .iter()
            .all(|tool| !policy.selected_tools.contains(tool))
    {
        return TraceQueueEligibility::Hold {
            kind: TraceQueueHoldKind::PolicyGate,
            reason: "trace does not use any selected auto-submit tools".to_string(),
        };
    }

    if envelope.value.submission_score < policy.min_submission_score {
        return TraceQueueEligibility::Hold {
            kind: TraceQueueHoldKind::PolicyGate,
            reason: format!(
                "submission score {:.2} is below policy minimum {:.2}",
                envelope.value.submission_score, policy.min_submission_score
            ),
        };
    }

    let failed_trace = matches!(
        envelope.outcome.task_success,
        TaskSuccess::Failure | TaskSuccess::Partial
    );
    if failed_trace && policy.auto_submit_failed_traces {
        return TraceQueueEligibility::Submit;
    }
    if policy.auto_submit_high_value_traces {
        return TraceQueueEligibility::Submit;
    }

    TraceQueueEligibility::Hold {
        kind: TraceQueueHoldKind::PolicyGate,
        reason: "policy does not allow this autonomous submission class".to_string(),
    }
}
