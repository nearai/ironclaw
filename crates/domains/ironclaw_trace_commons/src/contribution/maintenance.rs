//! Queue maintenance: compaction, warnings, telemetry accounting, hold
//! sidecars, path helpers, and quarantine of malformed envelopes.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::*;

pub(crate) struct TraceQueueCompactionCandidate {
    path: PathBuf,
    envelope: TraceContributionEnvelope,
    hold: Option<TraceQueueHold>,
}

pub(crate) fn compact_trace_queue_for_scope_unlocked(
    scope: Option<&str>,
) -> anyhow::Result<TraceQueueCompactionReport> {
    let paths = queued_trace_envelope_paths_for_scope(scope)?;
    let mut report = TraceQueueCompactionReport::default().set_scanned_count(paths.len() as u32);
    let mut candidates = Vec::new();
    for path in paths {
        let Some(envelope) = load_queued_trace_envelope_or_quarantine(scope, &path, "compaction")?
        else {
            report.malformed_envelopes_quarantined =
                report.malformed_envelopes_quarantined.saturating_add(1);
            continue;
        };
        let hold = read_trace_queue_hold_sidecar_for_envelope(&path)
            .ok()
            .flatten()
            .and_then(|sidecar| {
                trace_queue_submission_id_from_envelope_path(&path)
                    .map(|submission_id| trace_queue_hold_from_sidecar(submission_id, &sidecar))
            });
        candidates.push(TraceQueueCompactionCandidate {
            path,
            envelope,
            hold,
        });
    }

    let mut by_key: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        by_key
            .entry(trace_queue_dedupe_key(&candidate.envelope))
            .or_default()
            .push(index);
    }

    let mut remove_paths = BTreeSet::new();
    for indexes in by_key.values() {
        if indexes.len() < 2 {
            continue;
        }
        let Some(keep) = indexes
            .iter()
            .copied()
            .max_by_key(|index| trace_queue_compaction_rank(&candidates[*index]))
        else {
            continue;
        };
        for index in indexes.iter().copied() {
            if index != keep {
                remove_paths.insert(candidates[index].path.clone());
            }
        }
    }

    for path in &remove_paths {
        let hold_path = trace_queue_hold_path_for_envelope_path(path);
        if hold_path.exists() {
            std::fs::remove_file(&hold_path).map_err(|e| {
                anyhow::anyhow!(
                    "failed to remove duplicate queue hold {}: {}",
                    hold_path.display(),
                    e
                )
            })?;
        }
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| {
                anyhow::anyhow!(
                    "failed to remove duplicate queue envelope {}: {}",
                    path.display(),
                    e
                )
            })?;
            report.duplicate_envelopes_removed =
                report.duplicate_envelopes_removed.saturating_add(1);
        }
    }

    let dir = trace_queue_dir(scope);
    if dir.exists() {
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| anyhow::anyhow!("failed to read queue {}: {}", dir.display(), e))?
        {
            let entry = entry.map_err(|e| anyhow::anyhow!("failed to read queue entry: {}", e))?;
            let path = entry.path();
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".held.json"))
            {
                continue;
            }
            let Some(submission_id) = trace_queue_hold_submission_id(&path) else {
                continue;
            };
            let envelope_path = dir.join(format!("{submission_id}.json"));
            if !envelope_path.exists() {
                std::fs::remove_file(&path).map_err(|e| {
                    anyhow::anyhow!(
                        "failed to remove orphan queue hold {}: {}",
                        path.display(),
                        e
                    )
                })?;
                report.orphan_hold_sidecars_removed =
                    report.orphan_hold_sidecars_removed.saturating_add(1);
            }
        }
    }

    record_trace_queue_compaction_for_scope_unlocked(scope, &report, Utc::now())?;
    Ok(report)
}

pub(crate) fn trace_queue_dedupe_key(envelope: &TraceContributionEnvelope) -> String {
    let mut value = match serde_json::to_value(envelope) {
        Ok(value) => value,
        Err(_) => {
            return canonical_hash(&format!(
                "unserializable:{}:{}",
                envelope.trace_id, envelope.submission_id
            ));
        }
    };
    if let Value::Object(object) = &mut value {
        object.remove("submission_id");
        object.remove("created_at");
    }
    match serde_json::to_string(&value) {
        Ok(canonical) => canonical_hash(&canonical),
        Err(_) => canonical_hash(&format!(
            "unserializable:{}:{}",
            envelope.trace_id, envelope.submission_id
        )),
    }
}

pub(crate) fn trace_queue_compaction_rank(
    candidate: &TraceQueueCompactionCandidate,
) -> (u8, u32, i64, i64) {
    let hold_rank = candidate.hold.as_ref().map_or(0, |_| 1);
    let attempts = candidate.hold.as_ref().map_or(0, |hold| hold.attempts);
    let next_retry = candidate
        .hold
        .as_ref()
        .and_then(|hold| hold.next_retry_at)
        .map(|at| at.timestamp_millis())
        .unwrap_or(0);
    (
        hold_rank,
        attempts,
        next_retry,
        candidate.envelope.created_at.timestamp_millis(),
    )
}

pub(crate) fn trace_queue_warnings_for_scope_unlocked(
    scope: Option<&str>,
) -> anyhow::Result<Vec<TraceQueueWarning>> {
    let mut counts: BTreeMap<TraceQueueWarningKind, u32> = BTreeMap::new();
    for path in queued_trace_envelope_paths_for_scope(scope)? {
        let envelope = match load_trace_envelope(&path) {
            Ok(envelope) => envelope,
            Err(error) => {
                tracing::debug!(
                    %error,
                    path = %path.display(),
                    "Trace Commons queue diagnostics found malformed envelope"
                );
                *counts
                    .entry(TraceQueueWarningKind::MalformedEnvelope)
                    .or_default() += 1;
                continue;
            }
        };
        if envelope.schema_version != TRACE_CONTRIBUTION_SCHEMA_VERSION {
            *counts
                .entry(TraceQueueWarningKind::SchemaVersionMismatch)
                .or_default() += 1;
        }
        if envelope.consent.policy_version != TRACE_CONTRIBUTION_POLICY_VERSION {
            *counts
                .entry(TraceQueueWarningKind::PolicyVersionMismatch)
                .or_default() += 1;
        }
        if !trace_queue_redaction_pipeline_supported(&envelope.privacy.redaction_pipeline_version) {
            *counts
                .entry(TraceQueueWarningKind::RedactionPipelineMismatch)
                .or_default() += 1;
        }
        if envelope.trace_card.redaction_pipeline_version
            != envelope.privacy.redaction_pipeline_version
        {
            *counts
                .entry(TraceQueueWarningKind::TraceCardRedactionPipelineMismatch)
                .or_default() += 1;
        }
    }
    Ok(counts
        .into_iter()
        .map(|(kind, count)| TraceQueueWarning {
            kind,
            count,
            severity: trace_queue_warning_severity(kind),
            promotion_blocking: trace_queue_warning_promotion_blocking(kind),
            message: trace_queue_warning_message(kind, count),
            recommended_action: trace_queue_warning_recommended_action(kind).to_string(),
        })
        .collect())
}

pub(crate) fn trace_queue_warning_severity(
    kind: TraceQueueWarningKind,
) -> TraceQueueWarningSeverity {
    match kind {
        TraceQueueWarningKind::MalformedEnvelope => TraceQueueWarningSeverity::Blocking,
        TraceQueueWarningKind::SchemaVersionMismatch
        | TraceQueueWarningKind::PolicyVersionMismatch
        | TraceQueueWarningKind::RedactionPipelineMismatch
        | TraceQueueWarningKind::TraceCardRedactionPipelineMismatch => {
            TraceQueueWarningSeverity::Warning
        }
    }
}

pub(crate) fn trace_queue_warning_promotion_blocking(kind: TraceQueueWarningKind) -> bool {
    matches!(
        kind,
        TraceQueueWarningKind::SchemaVersionMismatch
            | TraceQueueWarningKind::PolicyVersionMismatch
            | TraceQueueWarningKind::RedactionPipelineMismatch
            | TraceQueueWarningKind::TraceCardRedactionPipelineMismatch
            | TraceQueueWarningKind::MalformedEnvelope
    )
}

pub(crate) fn trace_queue_warning_message(kind: TraceQueueWarningKind, count: u32) -> String {
    let label = match kind {
        TraceQueueWarningKind::SchemaVersionMismatch => "schema version mismatch",
        TraceQueueWarningKind::PolicyVersionMismatch => "policy version mismatch",
        TraceQueueWarningKind::RedactionPipelineMismatch => "redaction pipeline mismatch",
        TraceQueueWarningKind::TraceCardRedactionPipelineMismatch => {
            "trace card redaction pipeline mismatch"
        }
        TraceQueueWarningKind::MalformedEnvelope => "malformed queued envelope",
    };
    format!("{count} queued trace(s) have {label}")
}

pub(crate) fn trace_queue_warning_recommended_action(kind: TraceQueueWarningKind) -> &'static str {
    match kind {
        TraceQueueWarningKind::SchemaVersionMismatch => {
            "Re-preview or regenerate queued traces with the current contribution schema before production promotion."
        }
        TraceQueueWarningKind::PolicyVersionMismatch => {
            "Refresh user consent for queued traces under the current Trace Commons policy before production promotion."
        }
        TraceQueueWarningKind::RedactionPipelineMismatch => {
            "Re-run local redaction with an approved redaction pipeline before allowing autonomous promotion."
        }
        TraceQueueWarningKind::TraceCardRedactionPipelineMismatch => {
            "Rebuild trace-card metadata so it matches the envelope redaction pipeline before promotion."
        }
        TraceQueueWarningKind::MalformedEnvelope => {
            "Remove, quarantine, or regenerate malformed queue files before enabling production autonomous uploads."
        }
    }
}

pub(crate) fn trace_queue_redaction_pipeline_supported(version: &str) -> bool {
    let parts = version
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    !parts.is_empty()
        && parts.contains(&DETERMINISTIC_REDACTION_PIPELINE_VERSION)
        && parts.iter().all(|part| {
            matches!(
                *part,
                DETERMINISTIC_REDACTION_PIPELINE_VERSION
                    | PRIVACY_FILTER_SIDECAR_PIPELINE_SUFFIX
                    | SERVER_RESCRUB_PIPELINE_SUFFIX
            )
        })
}

pub(crate) fn read_trace_queue_telemetry_for_scope_unlocked(
    scope: Option<&str>,
) -> anyhow::Result<TraceQueueTelemetry> {
    let path = trace_queue_telemetry_path(scope);
    if !path.exists() {
        return Ok(TraceQueueTelemetry::default());
    }
    let body = std::fs::read_to_string(&path).map_err(|e| {
        anyhow::anyhow!(
            "failed to read trace queue telemetry {}: {}",
            path.display(),
            e
        )
    })?;
    serde_json::from_str(&body).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse trace queue telemetry {}: {}",
            path.display(),
            e
        )
    })
}

pub(crate) fn write_trace_queue_telemetry_for_scope_unlocked(
    scope: Option<&str>,
    telemetry: &TraceQueueTelemetry,
) -> anyhow::Result<()> {
    write_json_file(
        &trace_queue_telemetry_path(scope),
        telemetry,
        "trace queue telemetry",
    )
}

pub(crate) fn mutate_trace_queue_telemetry_for_scope_unlocked(
    scope: Option<&str>,
    mut mutate: impl FnMut(&mut TraceQueueTelemetry),
) -> anyhow::Result<()> {
    let mut telemetry = read_trace_queue_telemetry_for_scope_unlocked(scope)?;
    mutate(&mut telemetry);
    write_trace_queue_telemetry_for_scope_unlocked(scope, &telemetry)
}

pub(crate) fn record_trace_queue_flush_attempt_for_scope_unlocked(
    scope: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    mutate_trace_queue_telemetry_for_scope_unlocked(scope, |telemetry| {
        telemetry.last_flush_attempt_at = Some(now);
    })
}

pub(crate) fn record_trace_queue_flush_success_for_scope_unlocked(
    scope: Option<&str>,
    now: DateTime<Utc>,
    clear_failure: bool,
) -> anyhow::Result<()> {
    mutate_trace_queue_telemetry_for_scope_unlocked(scope, |telemetry| {
        telemetry.last_successful_flush_at = Some(now);
        telemetry.consecutive_flush_failures = 0;
        if clear_failure {
            telemetry.last_failure = None;
        }
    })
}

pub(crate) fn record_trace_queue_compaction_for_scope_unlocked(
    scope: Option<&str>,
    report: &TraceQueueCompactionReport,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    mutate_trace_queue_telemetry_for_scope_unlocked(scope, |telemetry| {
        telemetry.last_compaction_at = Some(now);
        telemetry.last_compaction = Some(report.clone());
        telemetry.compaction_reclaimed_items_total = telemetry
            .compaction_reclaimed_items_total
            .saturating_add(report.reclaimed_count());
    })
}

pub(crate) fn record_trace_queue_flush_failure_for_scope_unlocked(
    scope: Option<&str>,
    error: &anyhow::Error,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let failure = trace_queue_telemetry_failure(error, now);
    mutate_trace_queue_telemetry_for_scope_unlocked(scope, |telemetry| {
        telemetry.last_failed_flush_at = Some(now);
        telemetry.consecutive_flush_failures =
            telemetry.consecutive_flush_failures.saturating_add(1);
        telemetry.last_failure = Some(failure.clone());
    })
}

pub(crate) fn record_trace_queue_retryable_submission_failure_for_scope_unlocked(
    scope: Option<&str>,
    error: &anyhow::Error,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let failure =
        trace_queue_telemetry_failure_with_label(error, now, "submission retry scheduled");
    mutate_trace_queue_telemetry_for_scope_unlocked(scope, |telemetry| {
        telemetry.retryable_submission_failure_count = telemetry
            .retryable_submission_failure_count
            .saturating_add(1);
        telemetry.last_retryable_submission_failure_at = Some(now);
        telemetry.last_failure = Some(failure.clone());
    })
}

pub(crate) fn record_trace_queue_status_sync_success_for_scope_unlocked(
    scope: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    mutate_trace_queue_telemetry_for_scope_unlocked(scope, |telemetry| {
        telemetry.last_status_sync_at = Some(now);
    })
}

pub(crate) fn record_trace_queue_status_sync_failure_for_scope_unlocked(
    scope: Option<&str>,
    error: &anyhow::Error,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let kind = match trace_queue_telemetry_failure_kind(error) {
        TraceQueueTelemetryFailureKind::Unknown => TraceQueueTelemetryFailureKind::StatusSync,
        kind => kind,
    };
    let failure = trace_queue_telemetry_failure_with_kind(error, now, kind, "status sync failed");
    mutate_trace_queue_telemetry_for_scope_unlocked(scope, |telemetry| {
        telemetry.status_sync_failure_count = telemetry.status_sync_failure_count.saturating_add(1);
        telemetry.last_status_sync_failed_at = Some(now);
        telemetry.last_failure = Some(failure.clone());
    })
}

pub(crate) fn trace_queue_telemetry_failure(
    error: &anyhow::Error,
    now: DateTime<Utc>,
) -> TraceQueueTelemetryFailure {
    let kind = trace_queue_telemetry_failure_kind(error);
    trace_queue_telemetry_failure_with_kind(error, now, kind, "flush failed")
}

pub(crate) fn trace_queue_telemetry_failure_with_label(
    error: &anyhow::Error,
    now: DateTime<Utc>,
    label: &str,
) -> TraceQueueTelemetryFailure {
    let kind = trace_queue_telemetry_failure_kind(error);
    trace_queue_telemetry_failure_with_kind(error, now, kind, label)
}

pub(crate) fn trace_queue_telemetry_failure_kind(
    error: &anyhow::Error,
) -> TraceQueueTelemetryFailureKind {
    for cause in error.chain() {
        if let Some(remote_failure) = cause.downcast_ref::<TraceRemoteRequestFailure>() {
            return remote_failure.kind;
        }
        if let Some(llm_error) = cause.downcast_ref::<ironclaw_llm::error::LlmError>()
            && let Some(kind) = trace_queue_telemetry_failure_kind_for_llm_error(llm_error)
        {
            return kind;
        }
        if let Some(kind) = trace_queue_telemetry_failure_kind_for_error_source(cause) {
            return kind;
        }
        if let Some(reqwest_error) = cause.downcast_ref::<reqwest::Error>() {
            return trace_remote_request_failure_kind_for_reqwest_error(reqwest_error);
        }
    }
    let message = error
        .chain()
        .map(|cause| cause.to_string())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    if message.contains("endpoint") || message.contains("invalid trace contribution") {
        TraceQueueTelemetryFailureKind::Endpoint
    } else if message.contains("rejected by 401")
        || message.contains("rejected by 403")
        || message.contains("unauthorized")
        || message.contains("forbidden")
    {
        TraceQueueTelemetryFailureKind::Credential
    } else if message.contains("rejected by") {
        TraceQueueTelemetryFailureKind::HttpRejection
    } else if message.contains("not set")
        || message.contains("credentials")
        || message.contains("credential")
        || message.contains("token")
    {
        TraceQueueTelemetryFailureKind::Credential
    } else if message.contains("network is unreachable")
        || message.contains("no route to host")
        || message.contains("offline")
        || message.contains("internet connection appears to be offline")
    {
        TraceQueueTelemetryFailureKind::NetworkOffline
    } else if message.contains("dns")
        || message.contains("failed to lookup")
        || message.contains("failed to resolve")
        || message.contains("name or service not known")
        || message.contains("nodename nor servname")
    {
        TraceQueueTelemetryFailureKind::NetworkDns
    } else if message.contains("timed out")
        || message.contains("timeout")
        || message.contains("deadline elapsed")
    {
        TraceQueueTelemetryFailureKind::NetworkTimeout
    } else if message.contains("connection refused") || message.contains("refused") {
        TraceQueueTelemetryFailureKind::NetworkConnectionRefused
    } else if message.contains("request failed")
        || message.contains("connection")
        || message.contains("tcp")
        || message.contains("error trying to connect")
    {
        TraceQueueTelemetryFailureKind::Network
    } else if message.contains("opt-in") || message.contains("policy") {
        TraceQueueTelemetryFailureKind::Policy
    } else if message.contains("queue") || message.contains("envelope") {
        TraceQueueTelemetryFailureKind::Queue
    } else {
        TraceQueueTelemetryFailureKind::Unknown
    }
}

pub(crate) fn trace_queue_telemetry_failure_kind_for_llm_error(
    error: &ironclaw_llm::error::LlmError,
) -> Option<TraceQueueTelemetryFailureKind> {
    match error {
        ironclaw_llm::error::LlmError::AuthFailed { .. }
        | ironclaw_llm::error::LlmError::SessionExpired { .. }
        | ironclaw_llm::error::LlmError::SessionRenewalFailed { .. } => {
            Some(TraceQueueTelemetryFailureKind::Credential)
        }
        ironclaw_llm::error::LlmError::RateLimited { .. } => {
            Some(TraceQueueTelemetryFailureKind::HttpRejection)
        }
        ironclaw_llm::error::LlmError::RequestFailed { .. } => {
            Some(TraceQueueTelemetryFailureKind::Network)
        }
        _ => None,
    }
}

pub(crate) fn trace_queue_telemetry_failure_with_kind(
    error: &anyhow::Error,
    now: DateTime<Utc>,
    kind: TraceQueueTelemetryFailureKind,
    label: &str,
) -> TraceQueueTelemetryFailure {
    let error_hash = trace_queue_error_hash(error);
    TraceQueueTelemetryFailure {
        kind,
        reason: format!("{label}; error_hash={error_hash}"),
        error_hash,
        at: now,
    }
}

pub(crate) fn trace_queue_error_hash(error: &anyhow::Error) -> String {
    let mut hasher = Sha256::new();
    hasher.update(error.to_string().as_bytes());
    let digest = hasher.finalize();
    format!("sha256:{}", hex::encode(&digest[..8])) // safety: slicing the fixed-size SHA-256 byte array.
}

pub(crate) fn sanitized_trace_submission_failure_reason(error: &anyhow::Error) -> (String, String) {
    let error_hash = trace_queue_error_hash(error);
    (
        format!("submission failed; retained for retry (error_hash={error_hash})"),
        error_hash,
    )
}

pub(crate) fn trace_record_noticeable(record: &NodeTraceSubmissionRecord) -> bool {
    record.status == NodeTraceSubmissionStatus::Submitted || !record.credit_events.is_empty()
}

pub(crate) fn write_local_trace_records_for_scope(
    scope: Option<&str>,
    records: &[NodeTraceSubmissionRecord],
) -> anyhow::Result<()> {
    write_json_file(
        &trace_records_path(scope),
        records,
        "local trace submission records",
    )
}

#[cfg(test)]
pub(crate) fn write_trace_queue_hold_reason(path: &Path, reason: &str) -> anyhow::Result<()> {
    write_trace_queue_hold_sidecar_for_path(
        path,
        &TraceQueueHold {
            submission_id: trace_queue_submission_id_from_envelope_path(path)
                .unwrap_or_else(Uuid::nil),
            kind: trace_queue_hold_kind_for_policy_reason(reason),
            reason: safe_trace_queue_hold_reason(reason),
            attempts: 0,
            next_retry_at: None,
        },
    )
}

pub(crate) fn write_trace_queue_hold_sidecar_for_path(
    path: &Path,
    hold: &TraceQueueHold,
) -> anyhow::Result<()> {
    let hold_path = trace_queue_hold_path_for_envelope_path(path);
    let body = TraceQueueHoldSidecar {
        envelope: path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string),
        held_at: Some(Utc::now()),
        kind: Some(hold.kind),
        reason: Some(safe_trace_queue_hold_reason(&hold.reason)),
        attempts: (hold.attempts > 0).then_some(hold.attempts),
        next_retry_at: hold.next_retry_at,
        error_hash: trace_queue_error_hash_from_reason(&hold.reason),
    };
    write_json_file(&hold_path, &body, "trace queue hold reason")
}

pub(crate) fn retry_hold_if_not_due(
    path: &Path,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<TraceQueueHold>> {
    let Some(sidecar) = read_trace_queue_hold_sidecar_for_envelope(path).unwrap_or_else(|error| {
        tracing::debug!(
            %error,
            path = %path.display(),
            "Ignoring unreadable Trace Commons retry sidecar"
        );
        None
    }) else {
        return Ok(None);
    };
    let Some(submission_id) = trace_queue_submission_id_from_envelope_path(path) else {
        return Ok(None);
    };
    let hold = trace_queue_hold_from_sidecar(submission_id, &sidecar);
    if hold.kind == TraceQueueHoldKind::RetryableSubmissionFailure
        && hold
            .next_retry_at
            .is_some_and(|next_retry_at| next_retry_at > now)
    {
        return Ok(Some(hold));
    }
    Ok(None)
}

pub(crate) fn retry_hold_after_submission_failure(
    path: &Path,
    submission_id: Uuid,
    error: &anyhow::Error,
    now: DateTime<Utc>,
) -> anyhow::Result<TraceQueueHold> {
    let previous = read_trace_queue_hold_sidecar_for_envelope(path).unwrap_or_else(|error| {
        tracing::debug!(
            %error,
            path = %path.display(),
            "Ignoring unreadable Trace Commons retry sidecar before rescheduling"
        );
        None
    });
    let attempts = previous.and_then(|sidecar| sidecar.attempts).unwrap_or(0) + 1;
    let next_retry_at = trace_queue_next_retry_at(now, attempts);
    let (reason, _) = sanitized_trace_submission_failure_reason(error);
    Ok(TraceQueueHold {
        submission_id,
        kind: TraceQueueHoldKind::RetryableSubmissionFailure,
        reason,
        attempts,
        next_retry_at: Some(next_retry_at),
    })
}

pub(crate) fn trace_queue_next_retry_at(now: DateTime<Utc>, attempts: u32) -> DateTime<Utc> {
    let exponent = attempts.saturating_sub(1).min(8);
    let multiplier = 1u64 << exponent;
    let seconds = 300u64.saturating_mul(multiplier).min(86_400);
    now + chrono::Duration::seconds(seconds as i64)
}

pub(crate) fn read_trace_queue_hold_sidecar_for_envelope(
    path: &Path,
) -> anyhow::Result<Option<TraceQueueHoldSidecar>> {
    let hold_path = trace_queue_hold_path_for_envelope_path(path);
    if !hold_path.exists() {
        return Ok(None);
    }
    let body = std::fs::read_to_string(&hold_path).map_err(|e| {
        anyhow::anyhow!(
            "failed to read trace queue hold {}: {}",
            hold_path.display(),
            e
        )
    })?;
    let sidecar = serde_json::from_str::<TraceQueueHoldSidecar>(&body).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse trace queue hold {}: {}",
            hold_path.display(),
            e
        )
    })?;
    Ok(Some(sidecar))
}

pub(crate) fn trace_queue_hold_from_sidecar(
    submission_id: Uuid,
    sidecar: &TraceQueueHoldSidecar,
) -> TraceQueueHold {
    let reason = safe_trace_queue_hold_reason(sidecar.reason.as_deref().unwrap_or("held"));
    TraceQueueHold {
        submission_id,
        kind: sidecar
            .kind
            .unwrap_or_else(|| trace_queue_hold_kind_for_policy_reason(&reason)),
        reason,
        attempts: sidecar.attempts.unwrap_or(0),
        next_retry_at: sidecar.next_retry_at,
    }
}

pub(crate) fn trace_queue_hold_kind_for_policy_reason(reason: &str) -> TraceQueueHoldKind {
    if reason.to_ascii_lowercase().contains("manual review") {
        TraceQueueHoldKind::ManualReview
    } else if reason.to_ascii_lowercase().contains("retained for retry") {
        TraceQueueHoldKind::RetryableSubmissionFailure
    } else {
        TraceQueueHoldKind::PolicyGate
    }
}

pub(crate) fn trace_queue_submission_id_from_envelope_path(path: &Path) -> Option<Uuid> {
    let raw = path.file_stem()?.to_str()?;
    Uuid::parse_str(raw).ok()
}

pub(crate) fn trace_queue_hold_path_for_envelope_path(path: &Path) -> PathBuf {
    path.with_extension("held.json")
}

pub(crate) fn trace_queue_error_hash_from_reason(reason: &str) -> Option<String> {
    reason
        .split("error_hash=")
        .nth(1)
        .map(|suffix| suffix.trim_end_matches(')').trim().to_string())
        .filter(|hash| hash.starts_with("sha256:"))
}

pub(crate) fn trace_queue_hold_submission_id(path: &Path) -> Option<Uuid> {
    let file_name = path.file_name()?.to_str()?;
    let raw = file_name.strip_suffix(".held.json")?;
    Uuid::parse_str(raw).ok()
}

pub(crate) fn safe_trace_queue_hold_reason(reason: &str) -> String {
    let normalized = reason
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if normalized.is_empty() {
        return "held".to_string();
    }
    let (redacted, _) = DeterministicTraceRedactor::default().redact_text(&normalized);
    let redacted = trace_queue_secret_like_reason_regex().replace_all(&redacted, "[REDACTED]");
    let redacted = redacted
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if redacted.is_empty() {
        return "held".to_string();
    }
    redacted.chars().take(240).collect()
}

pub(crate) fn trace_policy_path_at(base: &std::path::Path, scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope_at(base, scope).join("policy.json")
}

pub(crate) fn trace_queue_dir(scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope(scope).join("queue")
}

/// Whether `scope` has any pending (flushable) queue entries on disk.
///
/// Lets the periodic flush worker prune drained scopes from its in-memory
/// observed-scope set instead of retaining one entry per historical caller
/// forever. A `.held.json` sidecar is a manual-review hold (not flushable until
/// authorized) and does not count; only a queued envelope (`<id>.json` with no
/// `.held.json` peer that is awaiting authorization) keeps a scope "pending".
pub fn trace_scope_has_pending_queue(scope: &str) -> bool {
    let dir = trace_queue_dir(Some(scope));
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.ends_with(".json") && !name.ends_with(".held.json"))
    })
}

pub(crate) fn trace_queue_malformed_dir(scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope(scope).join("queue_malformed")
}

pub(crate) fn trace_records_path(scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope(scope).join("submissions.json")
}

pub(crate) fn trace_queue_telemetry_path(scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope(scope).join("queue_telemetry.json")
}

pub(crate) fn trace_credit_notice_outbox_path(scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope(scope).join("credit_notice_outbox.json")
}

/// Atomic, durable, 0o600 JSON write (create_new + uuid temp name + sync_all +
/// best-effort parent-dir sync). Reused by `onboarding::device_key` so the
/// Ed25519 secret is never world-readable at any point and concurrent writers
/// don't race on a fixed temp name.
pub(crate) fn write_json_file<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
    label: &str,
) -> anyhow::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| {
        anyhow::anyhow!(
            "failed to create {} directory {}: {}",
            label,
            parent.display(),
            e
        )
    })?;
    let body = serde_json::to_string_pretty(value)
        .map_err(|e| anyhow::anyhow!("failed to serialize {}: {}", label, e))?;
    let temp_path = parent.join(format!(
        "{}{}.tmp",
        trace_json_temp_prefix(path),
        Uuid::new_v4()
    ));
    let mut temp = {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        // Trace policy files now potentially carry an operator-issued pilot
        // invite code; mirror the CLI's 0o600 stance for atomic policy
        // writes too so the rename-into-place step doesn't widen perms.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&temp_path).map_err(|e| {
            anyhow::anyhow!(
                "failed to create temporary {} {}: {}",
                label,
                temp_path.display(),
                e
            )
        })?
    };
    if let Err(error) = temp.write_all(body.as_bytes()) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(anyhow::anyhow!(
            "failed to write temporary {} for {}: {}",
            label,
            path.display(),
            error
        ));
    }
    if let Err(error) = temp.sync_all() {
        let _ = std::fs::remove_file(&temp_path);
        return Err(anyhow::anyhow!(
            "failed to sync temporary {} for {}: {}",
            label,
            path.display(),
            error
        ));
    }
    drop(temp);
    std::fs::rename(&temp_path, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp_path);
        anyhow::anyhow!("failed to install {} {}: {}", label, path.display(), e)
    })?;
    sync_directory_best_effort(parent, label);
    Ok(())
}

pub(crate) fn trace_json_temp_prefix(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!(".{name}."))
        .unwrap_or_else(|| ".trace-json.".to_string())
}

pub(crate) fn quarantine_malformed_trace_queue_envelope(
    scope: Option<&str>,
    path: &Path,
) -> anyhow::Result<PathBuf> {
    let quarantine_dir = trace_queue_malformed_dir(scope);
    std::fs::create_dir_all(&quarantine_dir).map_err(|e| {
        anyhow::anyhow!(
            "failed to create malformed trace queue directory {}: {}",
            quarantine_dir.display(),
            e
        )
    })?;
    let file_name = path.file_name().ok_or_else(|| {
        anyhow::anyhow!(
            "failed to quarantine malformed trace queue envelope without file name: {}",
            path.display()
        )
    })?;
    let mut quarantine_path = quarantine_dir.join(file_name);
    if quarantine_path.exists() {
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("queued-envelope");
        quarantine_path = quarantine_dir.join(format!("{stem}.{}.json", Uuid::new_v4()));
    }
    std::fs::rename(path, &quarantine_path).map_err(|e| {
        anyhow::anyhow!(
            "failed to quarantine malformed trace queue envelope {} to {}: {}",
            path.display(),
            quarantine_path.display(),
            e
        )
    })?;
    if let Some(active_dir) = path.parent() {
        sync_directory_best_effort(active_dir, "trace queue directory");
    }
    sync_directory_best_effort(&quarantine_dir, "malformed trace queue directory");
    Ok(quarantine_path)
}

pub(crate) fn sync_directory_best_effort(path: &Path, label: &str) {
    match std::fs::File::open(path) {
        Ok(file) => {
            if let Err(error) = file.sync_all() {
                tracing::debug!(
                    %error,
                    path = %path.display(),
                    label,
                    "Directory sync is not supported or failed"
                );
            }
        }
        Err(error) => {
            tracing::debug!(
                %error,
                path = %path.display(),
                label,
                "Directory sync is not supported or failed"
            );
        }
    }
}

pub(crate) fn scope_hash(scope: &str) -> String {
    let digest = Sha256::digest(scope.as_bytes());
    hex::encode(&digest[..16]) // safety: slicing the fixed-size SHA-256 byte array.
}
