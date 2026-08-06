//! The trace-credit notice state machine and its delivery outbox.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::*;

pub(crate) fn parse_trace_submission_receipt(body: &str) -> Option<TraceSubmissionReceipt> {
    if body.trim().is_empty() {
        return None;
    }
    serde_json::from_str(body).ok()
}

pub(crate) fn upsert_local_trace_record_for_scope(
    scope: Option<&str>,
    record: NodeTraceSubmissionRecord,
) -> anyhow::Result<()> {
    let mut records = read_local_trace_records_for_scope(scope)?;
    if let Some(existing) = records
        .iter_mut()
        .find(|existing| existing.submission_id == record.submission_id)
    {
        *existing = record;
    } else {
        records.push(record);
    }
    write_local_trace_records_for_scope(scope, &records)
}

pub(crate) fn mark_local_trace_revoked_for_scope_unlocked(
    scope: Option<&str>,
    submission_id: Uuid,
) -> anyhow::Result<()> {
    let mut records = read_local_trace_records_for_scope(scope)?;
    let now = Utc::now();
    let mut found = false;
    for record in &mut records {
        if record.submission_id == submission_id {
            record.status = NodeTraceSubmissionStatus::Revoked;
            record.revoked_at = Some(now);
            record.credit_notice_state = TraceCreditNoticeState::default();
            found = true;
        }
    }
    if !found {
        records.push(NodeTraceSubmissionRecord {
            submission_id,
            trace_id: Uuid::nil(),
            endpoint: None,
            status: NodeTraceSubmissionStatus::Revoked,
            server_status: None,
            submitted_at: None,
            revoked_at: Some(now),
            privacy_risk: "unknown".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 0.0,
            credit_points_final: None,
            credit_explanation: Vec::new(),
            credit_events: Vec::new(),
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: TraceCreditNoticeState::default(),
        });
    }
    write_local_trace_records_for_scope(scope, &records)
}

#[cfg(test)]
pub(crate) fn mark_trace_credit_noticed_if_due(
    scope: Option<&str>,
    interval_hours: u32,
) -> anyhow::Result<Option<CreditSummary>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    mark_trace_credit_noticed_if_due_at_unlocked(scope, interval_hours, Utc::now())
}

pub(crate) fn mark_trace_credit_noticed_if_due_unlocked(
    scope: Option<&str>,
    interval_hours: u32,
) -> anyhow::Result<Option<CreditSummary>> {
    mark_trace_credit_noticed_if_due_at_unlocked(scope, interval_hours, Utc::now())
}

#[cfg(test)]
pub(crate) fn trace_credit_notice_due_for_scope_at(
    scope: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<CreditSummary>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let policy = read_trace_policy_for_scope(scope)?;
    if !policy.enabled || policy.credit_notice_interval_hours == 0 {
        return Ok(None);
    }
    trace_credit_notice_due_for_scope_at_unlocked(scope, policy.credit_notice_interval_hours, now)
}

pub(crate) fn trace_credit_notice_due_for_scope_at_unlocked(
    scope: Option<&str>,
    interval_hours: u32,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<CreditSummary>> {
    if interval_hours == 0 {
        return Ok(None);
    }

    let records = read_local_trace_records_for_scope(scope)?;
    Ok(
        trace_credit_notice_due_for_records(&records, interval_hours, now)
            .map(|(summary, _fingerprint)| summary),
    )
}

pub(crate) fn mark_trace_credit_noticed_if_due_at_unlocked(
    scope: Option<&str>,
    interval_hours: u32,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<CreditSummary>> {
    if interval_hours == 0 {
        return Ok(None);
    }

    let mut records = read_local_trace_records_for_scope(scope)?;
    let Some((summary, fingerprint)) =
        trace_credit_notice_due_for_records(&records, interval_hours, now)
    else {
        return Ok(None);
    };
    upsert_trace_credit_notice_outbox_item_unlocked(scope, &summary, &fingerprint, now)?;

    for record in &mut records {
        if trace_record_noticeable(record) {
            record.last_credit_notice_at = Some(now);
            record.credit_notice_state = TraceCreditNoticeState {
                last_presented_at: Some(now),
                acknowledged_at: None,
                snoozed_until: None,
                fingerprint: Some(fingerprint.clone()),
            };
        }
    }
    write_local_trace_records_for_scope(scope, &records)?;
    Ok(Some(summary))
}

pub(crate) fn acknowledge_trace_credit_notice_for_scope_at_unlocked(
    scope: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<CreditSummary>> {
    let mut records = read_local_trace_records_for_scope(scope)?;
    let Some(fingerprint) = trace_credit_notice_fingerprint(&records) else {
        return Ok(None);
    };
    let summary = trace_credit_summary(&records);
    for record in &mut records {
        if trace_record_noticeable(record) {
            record.credit_notice_state = TraceCreditNoticeState {
                last_presented_at: record
                    .credit_notice_state
                    .last_presented_at
                    .or(record.last_credit_notice_at)
                    .or(Some(now)),
                acknowledged_at: Some(now),
                snoozed_until: None,
                fingerprint: Some(fingerprint.clone()),
            };
        }
    }
    mark_trace_credit_notice_outbox_acknowledged_unlocked(scope, &fingerprint, now)?;
    write_local_trace_records_for_scope(scope, &records)?;
    Ok(Some(summary))
}

pub(crate) fn snooze_trace_credit_notice_for_scope_until_at_unlocked(
    scope: Option<&str>,
    snoozed_until: DateTime<Utc>,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<CreditSummary>> {
    let mut records = read_local_trace_records_for_scope(scope)?;
    let Some(fingerprint) = trace_credit_notice_fingerprint(&records) else {
        return Ok(None);
    };
    let summary = trace_credit_summary(&records);
    for record in &mut records {
        if trace_record_noticeable(record) {
            record.credit_notice_state = TraceCreditNoticeState {
                last_presented_at: record
                    .credit_notice_state
                    .last_presented_at
                    .or(record.last_credit_notice_at)
                    .or(Some(now)),
                acknowledged_at: None,
                snoozed_until: Some(snoozed_until),
                fingerprint: Some(fingerprint.clone()),
            };
        }
    }
    mark_trace_credit_notice_outbox_snoozed_unlocked(scope, &fingerprint, snoozed_until, now)?;
    write_local_trace_records_for_scope(scope, &records)?;
    Ok(Some(summary))
}

pub(crate) fn trace_credit_notice_due_for_records(
    records: &[NodeTraceSubmissionRecord],
    interval_hours: u32,
    now: DateTime<Utc>,
) -> Option<(CreditSummary, String)> {
    let fingerprint = trace_credit_notice_fingerprint(records)?;
    let noticeable = records
        .iter()
        .filter(|record| trace_record_noticeable(record))
        .collect::<Vec<_>>();
    if noticeable.is_empty() {
        return None;
    }

    let all_acknowledged = noticeable.iter().all(|record| {
        record.credit_notice_state.fingerprint.as_deref() == Some(fingerprint.as_str())
            && record.credit_notice_state.acknowledged_at.is_some()
    });
    if all_acknowledged {
        return None;
    }

    let all_snoozed = noticeable.iter().all(|record| {
        record.credit_notice_state.fingerprint.as_deref() == Some(fingerprint.as_str())
            && record
                .credit_notice_state
                .snoozed_until
                .is_some_and(|snoozed_until| snoozed_until > now)
    });
    if all_snoozed {
        return None;
    }

    let interval = chrono::Duration::hours(i64::from(interval_hours));
    let notice_due = noticeable.iter().any(|record| {
        if record.credit_notice_state.fingerprint.as_deref() != Some(fingerprint.as_str()) {
            return record
                .last_credit_notice_at
                .map(|last_notice| now.signed_duration_since(last_notice) >= interval)
                .unwrap_or(true);
        }
        if record
            .credit_notice_state
            .snoozed_until
            .is_some_and(|snoozed_until| snoozed_until <= now)
        {
            return true;
        }
        record
            .credit_notice_state
            .last_presented_at
            .or(record.last_credit_notice_at)
            .map(|last_notice| now.signed_duration_since(last_notice) >= interval)
            .unwrap_or(true)
    });

    if notice_due {
        Some((trace_credit_summary(records), fingerprint))
    } else {
        None
    }
}

pub(crate) fn trace_credit_notice_fingerprint(
    records: &[NodeTraceSubmissionRecord],
) -> Option<String> {
    let mut parts = Vec::new();
    for record in records
        .iter()
        .filter(|record| trace_record_noticeable(record))
    {
        let mut events = record
            .credit_events
            .iter()
            .map(|event| {
                format!(
                    "{}:{:?}:{:.6}:{}",
                    event.event_id,
                    event.kind,
                    event.points_delta,
                    event.created_at.timestamp_millis()
                )
            })
            .collect::<Vec<_>>();
        events.sort();
        parts.push(format!(
            "{}|{}|{}|{:.6}|{}|{}",
            record.submission_id,
            record.status.as_str(),
            record.server_status.as_deref().unwrap_or_default(),
            record.credit_points_pending,
            record
                .credit_points_final
                .map(|points| format!("{points:.6}"))
                .unwrap_or_default(),
            events.join(",")
        ));
    }
    if parts.is_empty() {
        return None;
    }
    parts.sort();
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(b"\n");
    }
    Some(format!("sha256:{}", hex::encode(&hasher.finalize()[..16]))) // safety: slicing the fixed-size SHA-256 byte array.
}

pub(crate) fn upsert_trace_credit_notice_outbox_item_unlocked(
    scope: Option<&str>,
    summary: &CreditSummary,
    fingerprint: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let mut outbox = read_trace_credit_notice_outbox_for_scope_unlocked(scope)?;
    let message = trace_credit_notice_message(summary);
    if let Some(item) = outbox
        .iter_mut()
        .find(|item| item.fingerprint == fingerprint)
    {
        item.summary = summary.clone();
        item.message = message;
        item.updated_at = now;
        if item.status != TraceCreditNoticeOutboxStatus::Acknowledged {
            item.status = TraceCreditNoticeOutboxStatus::Pending;
            item.next_attempt_at = None;
            item.snoozed_until = None;
        }
    } else {
        outbox.push(TraceCreditNoticeOutboxItem {
            notice_id: trace_credit_notice_outbox_id(fingerprint),
            fingerprint: fingerprint.to_string(),
            summary: summary.clone(),
            message,
            status: TraceCreditNoticeOutboxStatus::Pending,
            created_at: now,
            updated_at: now,
            last_attempt_at: None,
            delivered_at: None,
            next_attempt_at: None,
            snoozed_until: None,
            attempt_count: 0,
            delivery_attempts: Vec::new(),
        });
    }
    write_trace_credit_notice_outbox_for_scope_unlocked(scope, &outbox)
}

pub(crate) fn mark_trace_credit_notice_outbox_acknowledged_unlocked(
    scope: Option<&str>,
    fingerprint: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    update_trace_credit_notice_outbox_item_unlocked(scope, fingerprint, |item| {
        item.status = TraceCreditNoticeOutboxStatus::Acknowledged;
        item.updated_at = now;
        item.next_attempt_at = None;
        item.snoozed_until = None;
    })
    .map(|_| ())
}

pub(crate) fn mark_trace_credit_notice_outbox_snoozed_unlocked(
    scope: Option<&str>,
    fingerprint: &str,
    snoozed_until: DateTime<Utc>,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    update_trace_credit_notice_outbox_item_unlocked(scope, fingerprint, |item| {
        item.status = TraceCreditNoticeOutboxStatus::Snoozed;
        item.updated_at = now;
        item.next_attempt_at = Some(snoozed_until);
        item.snoozed_until = Some(snoozed_until);
    })
    .map(|_| ())
}

pub(crate) fn read_trace_credit_notice_outbox_for_scope_unlocked(
    scope: Option<&str>,
) -> anyhow::Result<Vec<TraceCreditNoticeOutboxItem>> {
    let path = trace_credit_notice_outbox_path(scope);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let body = std::fs::read_to_string(&path).map_err(|e| {
        anyhow::anyhow!(
            "failed to read trace credit notice outbox {}: {}",
            path.display(),
            e
        )
    })?;
    serde_json::from_str(&body).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse trace credit notice outbox {}: {}",
            path.display(),
            e
        )
    })
}

pub(crate) fn write_trace_credit_notice_outbox_for_scope_unlocked(
    scope: Option<&str>,
    outbox: &[TraceCreditNoticeOutboxItem],
) -> anyhow::Result<()> {
    write_json_file(
        &trace_credit_notice_outbox_path(scope),
        outbox,
        "trace credit notice outbox",
    )
}

#[cfg(test)]
pub(crate) fn pending_trace_credit_notice_outbox_items_for_scope_at(
    scope: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<TraceCreditNoticeOutboxItem>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    pending_trace_credit_notice_outbox_items_for_scope_at_unlocked(scope, now)
}

pub(crate) fn pending_trace_credit_notice_outbox_items_for_scope_at_unlocked(
    scope: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<TraceCreditNoticeOutboxItem>> {
    Ok(read_trace_credit_notice_outbox_for_scope_unlocked(scope)?
        .into_iter()
        .filter(|item| trace_credit_notice_outbox_item_due(item, now))
        .collect())
}

pub(crate) fn trace_credit_notice_outbox_item_due(
    item: &TraceCreditNoticeOutboxItem,
    now: DateTime<Utc>,
) -> bool {
    match item.status {
        TraceCreditNoticeOutboxStatus::Pending => item
            .next_attempt_at
            .map(|next_attempt_at| next_attempt_at <= now)
            .unwrap_or(true),
        TraceCreditNoticeOutboxStatus::Snoozed => item
            .snoozed_until
            .map(|snoozed_until| snoozed_until <= now)
            .unwrap_or_else(|| {
                item.next_attempt_at
                    .map(|next_attempt_at| next_attempt_at <= now)
                    .unwrap_or(true)
            }),
        TraceCreditNoticeOutboxStatus::Delivered | TraceCreditNoticeOutboxStatus::Acknowledged => {
            false
        }
    }
}

pub(crate) fn record_trace_credit_notice_delivery_success_for_scope_at_unlocked(
    scope: Option<&str>,
    fingerprint: &str,
    channel: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<TraceCreditNoticeOutboxItem>> {
    update_trace_credit_notice_outbox_item_unlocked(scope, fingerprint, |item| {
        push_trace_credit_notice_delivery_attempt(
            item,
            TraceCreditNoticeDeliveryAttempt {
                channel: safe_trace_credit_notice_channel(channel),
                attempted_at: now,
                succeeded: true,
                error_kind: None,
                error_hash: None,
            },
        );
        item.status = TraceCreditNoticeOutboxStatus::Delivered;
        item.updated_at = now;
        item.last_attempt_at = Some(now);
        item.delivered_at = Some(now);
        item.next_attempt_at = None;
        item.snoozed_until = None;
        item.attempt_count = item.attempt_count.saturating_add(1);
    })
}

pub(crate) fn record_trace_credit_notice_delivery_failure_for_scope_at_unlocked(
    scope: Option<&str>,
    fingerprint: &str,
    channel: &str,
    error: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<TraceCreditNoticeOutboxItem>> {
    let error_hash = trace_credit_notice_delivery_error_hash(error);
    let error_kind = trace_credit_notice_delivery_error_kind(error);
    update_trace_credit_notice_outbox_item_unlocked(scope, fingerprint, |item| {
        let next_attempt_count = item.attempt_count.saturating_add(1);
        push_trace_credit_notice_delivery_attempt(
            item,
            TraceCreditNoticeDeliveryAttempt {
                channel: safe_trace_credit_notice_channel(channel),
                attempted_at: now,
                succeeded: false,
                error_kind: Some(error_kind),
                error_hash: Some(error_hash.clone()),
            },
        );
        item.status = TraceCreditNoticeOutboxStatus::Pending;
        item.updated_at = now;
        item.last_attempt_at = Some(now);
        item.delivered_at = None;
        item.next_attempt_at = Some(trace_queue_next_retry_at(now, next_attempt_count));
        item.snoozed_until = None;
        item.attempt_count = next_attempt_count;
    })
}

pub(crate) fn update_trace_credit_notice_outbox_item_unlocked(
    scope: Option<&str>,
    fingerprint: &str,
    mut update: impl FnMut(&mut TraceCreditNoticeOutboxItem),
) -> anyhow::Result<Option<TraceCreditNoticeOutboxItem>> {
    let mut outbox = read_trace_credit_notice_outbox_for_scope_unlocked(scope)?;
    let mut updated = None;
    if let Some(item) = outbox
        .iter_mut()
        .find(|item| item.fingerprint == fingerprint)
    {
        update(item);
        updated = Some(item.clone());
    }
    if updated.is_some() {
        write_trace_credit_notice_outbox_for_scope_unlocked(scope, &outbox)?;
    }
    Ok(updated)
}

pub(crate) fn push_trace_credit_notice_delivery_attempt(
    item: &mut TraceCreditNoticeOutboxItem,
    attempt: TraceCreditNoticeDeliveryAttempt,
) {
    item.delivery_attempts.push(attempt);
    let excess = item
        .delivery_attempts
        .len()
        .saturating_sub(TRACE_CREDIT_NOTICE_OUTBOX_MAX_ATTEMPTS_STORED);
    if excess > 0 {
        item.delivery_attempts.drain(0..excess);
    }
}

pub(crate) fn trace_credit_notice_outbox_id(fingerprint: &str) -> String {
    canonical_hash(&format!("trace_credit_notice:{fingerprint}"))
}

pub(crate) fn safe_trace_credit_notice_channel(channel: &str) -> String {
    let sanitized = channel
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(*ch, '-' | '_' | '.'))
        .take(64)
        .collect::<String>();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

pub(crate) fn trace_credit_notice_delivery_error_hash(error: &str) -> String {
    let digest = Sha256::digest(error.as_bytes());
    format!("sha256:{}", hex::encode(&digest[..8])) // safety: slicing the fixed-size SHA-256 byte array.
}

pub(crate) fn trace_credit_notice_delivery_error_kind(
    error: &str,
) -> TraceQueueTelemetryFailureKind {
    let error = anyhow::anyhow!(error.to_string());
    trace_queue_telemetry_failure_kind(&error)
}
