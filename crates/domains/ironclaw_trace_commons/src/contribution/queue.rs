//! The on-disk contribution queue: submission records, holds, telemetry and
//! diagnostics types, the per-scope directory layout, and policy and
//! credential resolution.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::OwnedMutexGuard;
use uuid::Uuid;

use ironclaw_host_api::ids::{TenantId, UserId};

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeTraceSubmissionRecord {
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    pub status: NodeTraceSubmissionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<DateTime<Utc>>,
    pub privacy_risk: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub redaction_counts: BTreeMap<String, u32>,
    #[serde(default)]
    pub credit_points_pending: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_points_final: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credit_explanation: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credit_events: Vec<TraceCreditEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<NodeTraceSubmissionHistoryEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_credit_notice_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "TraceCreditNoticeState::is_empty")]
    pub credit_notice_state: TraceCreditNoticeState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeTraceSubmissionHistoryEvent {
    pub event_id: Uuid,
    pub kind: NodeTraceSubmissionHistoryKind,
    pub occurred_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_status: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub credit_delta: f32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub delayed_credit_explanation_count: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeTraceSubmissionHistoryKind {
    StatusSync,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceCreditNoticeState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_presented_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

impl TraceCreditNoticeState {
    pub fn is_empty(&self) -> bool {
        self.last_presented_at.is_none()
            && self.acknowledged_at.is_none()
            && self.snoozed_until.is_none()
            && self.fingerprint.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceCreditNoticeOutboxItem {
    pub notice_id: String,
    pub fingerprint: String,
    pub summary: CreditSummary,
    pub message: String,
    pub status: TraceCreditNoticeOutboxStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub attempt_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delivery_attempts: Vec<TraceCreditNoticeDeliveryAttempt>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceCreditNoticeOutboxStatus {
    Pending,
    Delivered,
    Acknowledged,
    Snoozed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceCreditNoticeDeliveryAttempt {
    pub channel: String,
    pub attempted_at: DateTime<Utc>,
    pub succeeded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<TraceQueueTelemetryFailureKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeTraceSubmissionStatus {
    Submitted,
    Revoked,
    Expired,
    Purged,
}

impl NodeTraceSubmissionStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
            Self::Purged => "purged",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceSubmissionReceipt {
    #[serde(default = "default_submission_status")]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_points_pending: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_points_final: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceSubmissionStatusRequest {
    pub submission_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceSubmissionStatusUpdate {
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub status: String,
    pub credit_points_pending: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_points_final: Option<f32>,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub credit_points_ledger: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_points_total: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delayed_credit_explanations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceQueueHold {
    pub submission_id: Uuid,
    pub kind: TraceQueueHoldKind,
    pub reason: String,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_retry_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceQueueHoldKind {
    PolicyGate,
    ManualReview,
    RetryableSubmissionFailure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TraceQueueHoldSidecar {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) envelope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) held_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<TraceQueueHoldKind>,
    pub(crate) reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) attempts: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) next_retry_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) error_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceQueueFlushReport {
    pub submitted: usize,
    pub held: usize,
    #[serde(default, skip_serializing_if = "TraceQueueCompactionReport::is_empty")]
    pub compaction: TraceQueueCompactionReport,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<TraceQueueHold>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_notice: Option<CreditSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceQueueWorkerReport {
    pub scopes_checked: usize,
    pub submitted: usize,
    pub held: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope_reports: Vec<TraceQueueWorkerScopeReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceQueueWorkerScopeReport {
    pub scope: String,
    pub submitted: usize,
    pub held: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<TraceQueueHold>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_notice: Option<CreditSummary>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceQueueCompactionReport {
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub scanned_count: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub duplicate_envelopes_removed: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub orphan_hold_sidecars_removed: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub malformed_envelopes_quarantined: u32,
}

impl TraceQueueCompactionReport {
    pub fn set_scanned_count(mut self, scanned_count: u32) -> Self {
        self.scanned_count = scanned_count;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.scanned_count == 0
            && self.duplicate_envelopes_removed == 0
            && self.orphan_hold_sidecars_removed == 0
            && self.malformed_envelopes_quarantined == 0
    }

    pub(crate) fn reclaimed_count(&self) -> u32 {
        self.duplicate_envelopes_removed
            .saturating_add(self.orphan_hold_sidecars_removed)
            .saturating_add(self.malformed_envelopes_quarantined)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceQueueTelemetry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_flush_attempt_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_flush_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failed_flush_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub consecutive_flush_failures: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub retryable_submission_failure_count: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub status_sync_failure_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_retryable_submission_failure_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status_sync_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status_sync_failed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_compaction_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_compaction: Option<TraceQueueCompactionReport>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub compaction_reclaimed_items_total: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<TraceQueueTelemetryFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceQueueTelemetryFailure {
    pub kind: TraceQueueTelemetryFailureKind,
    pub reason: String,
    pub error_hash: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceQueueTelemetryFailureKind {
    Policy,
    Endpoint,
    Credential,
    Network,
    NetworkOffline,
    NetworkDns,
    NetworkTimeout,
    NetworkConnectionRefused,
    HttpRejection,
    StatusSync,
    Submission,
    Queue,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceQueueWarning {
    pub kind: TraceQueueWarningKind,
    pub count: u32,
    pub severity: TraceQueueWarningSeverity,
    pub promotion_blocking: bool,
    pub message: String,
    pub recommended_action: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceQueueWarningSeverity {
    Warning,
    Blocking,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceQueueWarningKind {
    SchemaVersionMismatch,
    PolicyVersionMismatch,
    RedactionPipelineMismatch,
    TraceCardRedactionPipelineMismatch,
    MalformedEnvelope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceQueueDiagnostics {
    pub queued_count: u32,
    pub held_count: u32,
    pub submitted_count: u32,
    pub revoked_count: u32,
    pub expired_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_submission_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_credit_sync_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub held_reason_counts: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub retry_scheduled_count: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub manual_review_hold_count: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub policy_hold_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_retry_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub telemetry: TraceQueueTelemetry,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<TraceQueueWarning>,
    pub policy_enabled: bool,
    pub endpoint_configured: bool,
    pub ready_to_flush: bool,
}

pub enum TraceQueueEligibility {
    Submit,
    Hold {
        kind: TraceQueueHoldKind,
        reason: String,
    },
}

pub(crate) fn default_submission_status() -> String {
    "submitted".to_string()
}

pub(crate) fn is_zero_f32(value: &f32) -> bool {
    value.abs() <= f32::EPSILON
}

pub(crate) fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

pub(crate) fn trace_contribution_dir_for_scope_at(
    base: &std::path::Path,
    scope: Option<&str>,
) -> PathBuf {
    let contributions = base.join("trace_contributions");
    match scope {
        Some(scope) if !scope.trim().is_empty() => {
            contributions.join("users").join(scope_hash(scope))
        }
        _ => contributions,
    }
}

pub fn trace_contribution_dir_for_scope(scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope_at(&ironclaw_common::paths::ironclaw_base_dir(), scope)
}

/// Canonical per-scope key for Trace Commons local state (policy, device keys,
/// credits, profile tokens).
///
/// Composite of `tenant_id` + `user_id` so the same user id in two tenants does
/// NOT share local state — Trace Commons state is tenant-scoped, and keying on
/// the user alone would collapse cross-tenant isolation. The returned string is
/// opaque: callers hand it to `trace_contribution_dir_for_scope` /
/// `read_*_for_scope`, which hash it, so only stability and cross-tenant
/// distinctness matter (the `/` separator can't collide because `TenantId`
/// validation forbids it).
pub fn trace_scope_key(tenant_id: &str, user_id: &str) -> String {
    format!("{tenant_id}/{user_id}")
}

pub fn local_pseudonymous_contributor_id(scope: &str) -> String {
    format!("sha256:{}", scope_hash(scope))
}

/// Read (or create on first use) the per-instance random salt used to derive
/// per-user pseudonymous subjects under instance enrollment. Persisted at the
/// instance trace dir (`0600` on Unix). Concurrent first-use races are settled
/// with `create_new`: exactly one writer wins and the loser re-reads.
pub(crate) fn instance_subject_salt_at(base: &std::path::Path) -> anyhow::Result<String> {
    use std::io::Write as _;

    let dir = trace_contribution_dir_for_scope_at(base, None);
    let path = dir.join("subject_salt");
    let read_existing = |path: &std::path::Path| -> anyhow::Result<Option<String>> {
        match std::fs::read_to_string(path) {
            Ok(salt) => {
                let salt = salt.trim().to_string();
                anyhow::ensure!(!salt.is_empty(), "instance subject salt file is empty");
                Ok(Some(salt))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(anyhow::anyhow!("failed to read instance subject salt: {e}")),
        }
    };
    if let Some(salt) = read_existing(&path)? {
        return Ok(salt);
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("failed to create instance trace dir: {e}"))?;
    // 32 random bytes, hex-encoded.
    let salt = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let open_new = || {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        options.open(&path)
    };
    match open_new() {
        Ok(mut file) => {
            file.write_all(salt.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|e| anyhow::anyhow!("failed to write instance subject salt: {e}"))?;
            Ok(salt)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => read_existing(&path)?
            .ok_or_else(|| anyhow::anyhow!("instance subject salt disappeared during creation")),
        Err(e) => Err(anyhow::anyhow!(
            "failed to create instance subject salt: {e}"
        )),
    }
}

/// Per-user pseudonymous subject for instance enrollment, salted with the
/// per-instance random salt. Unlike [`local_pseudonymous_contributor_id`]
/// (an unsalted scope hash used for local state keying and log refs), this
/// value is sent to the Trace Commons server as the claim subject — salting
/// prevents the server (or anyone with ledger access) from dictionary-matching
/// guessable tenant/user identifiers to de-pseudonymize contributors.
pub(crate) fn salted_pseudonymous_contributor_id_at(
    base: &std::path::Path,
    scope: &str,
) -> anyhow::Result<String> {
    let salt = instance_subject_salt_at(base)?;
    let digest = Sha256::digest(format!("{salt}:{scope}").as_bytes());
    // safety: slicing the fixed-size SHA-256 byte array.
    Ok(format!("sha256:{}", hex::encode(&digest[..16])))
}

pub fn local_pseudonymous_tenant_scope_ref(scope: &str) -> String {
    format!("tenant_sha256:{}", scope_hash(scope))
}

pub(crate) static TRACE_SCOPE_MUTATION_LOCKS: LazyLock<
    std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
> = LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

pub(crate) fn trace_scope_mutation_lock_key(scope: Option<&str>) -> String {
    match scope {
        Some(scope) if !scope.trim().is_empty() => format!("scope:{}", scope_hash(scope)),
        _ => "global".to_string(),
    }
}

pub(crate) fn trace_scope_mutation_lock(scope: Option<&str>) -> Arc<tokio::sync::Mutex<()>> {
    let key = trace_scope_mutation_lock_key(scope);
    let mut locks = match TRACE_SCOPE_MUTATION_LOCKS.lock() {
        Ok(locks) => locks,
        Err(poisoned) => poisoned.into_inner(),
    };
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

pub(crate) async fn lock_trace_scope_for_mutation(scope: Option<&str>) -> OwnedMutexGuard<()> {
    trace_scope_mutation_lock(scope).lock_owned().await
}

pub(crate) fn lock_trace_scope_for_mutation_blocking(scope: Option<&str>) -> OwnedMutexGuard<()> {
    let lock = trace_scope_mutation_lock(scope);
    loop {
        if let Ok(guard) = lock.clone().try_lock_owned() {
            return guard;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Read the scoped policy only if its file exists: `Ok(None)` when absent.
/// The presence distinction matters for the instance-enrollment fallback —
/// a scoped policy file that EXISTS with `enabled = false` is an explicit
/// user opt-out (written by `traces opt-out`) and must not be treated like
/// "never configured".
pub(crate) fn read_trace_policy_for_scope_if_present_at(
    base: &std::path::Path,
    scope: Option<&str>,
) -> anyhow::Result<Option<StandingTraceContributionPolicy>> {
    let path = trace_policy_path_at(base, scope);
    // Fail loud on stat/permission errors: `Path::exists()` maps them to
    // `false`, which would silently treat an unreadable policy as
    // missing/default-disabled and flip enrollment/flush behavior. Only a
    // confirmed non-existent path reports absence.
    if !path
        .try_exists()
        .map_err(|e| anyhow::anyhow!("failed to stat trace policy {}: {}", path.display(), e))?
    {
        return Ok(None);
    }
    let body = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("failed to read trace policy {}: {}", path.display(), e))?;
    serde_json::from_str(&body)
        .map(Some)
        .map_err(|e| anyhow::anyhow!("failed to parse trace policy {}: {}", path.display(), e))
}

pub(crate) fn read_trace_policy_for_scope_at(
    base: &std::path::Path,
    scope: Option<&str>,
) -> anyhow::Result<StandingTraceContributionPolicy> {
    Ok(read_trace_policy_for_scope_if_present_at(base, scope)?.unwrap_or_default())
}

pub fn read_trace_policy_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<StandingTraceContributionPolicy> {
    read_trace_policy_for_scope_at(&ironclaw_common::paths::ironclaw_base_dir(), scope)
}

pub(crate) fn write_trace_policy_for_scope_at(
    base: &std::path::Path,
    scope: Option<&str>,
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<()> {
    write_json_file(&trace_policy_path_at(base, scope), policy, "trace policy")
}

pub fn write_trace_policy_for_scope(
    scope: Option<&str>,
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<()> {
    write_trace_policy_for_scope_at(&ironclaw_common::paths::ironclaw_base_dir(), scope, policy)
}

// ── Trace credential resolution (instance enrollment) ────────────────────────
//
// Why credential resolution lives beside the policy/scope-dir helpers: every
// resolver below calls read_trace_policy_for_scope_at /
// trace_contribution_dir_* / DefaultTraceUploadCredentialProvider, so moving
// it out would mean exporting a wide private surface for no gain.

/// Resolved Trace Commons credentials for a (tenant, user): which local-state
/// scope to use and the per-user subject (if any) to send to the server.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceCredentialResolution {
    /// The scope string keying the user's local state (queued envelopes,
    /// records, credits). NOTE: it is NOT always where the device key and
    /// enrollment policy live — under instance enrollment (`subject` is
    /// `Some`) those come from the instance scope (`None`); callers select
    /// the device-key dir based on `subject.is_some()`.
    pub state_scope: String,
    /// Per-user subject to send in upload-claim / login-link requests.
    /// `None` for the personal-invite model (device key already 1:1 with user).
    pub subject: Option<String>,
    /// The resolved enrollment policy.
    pub policy: StandingTraceContributionPolicy,
}

/// Inner implementation that reads policies relative to an explicit base dir.
/// Used by `resolve_trace_credentials` (which supplies the real base) and by
/// tests (which supply an isolated tempdir).
pub(crate) fn resolve_trace_credentials_at(
    base_dir: &std::path::Path,
    tenant_id: &str,
    user_id: &str,
) -> anyhow::Result<Option<TraceCredentialResolution>> {
    let scope = trace_scope_key(tenant_id, user_id);

    match read_trace_policy_for_scope_if_present_at(base_dir, Some(scope.as_str()))
        .map_err(|e| anyhow::anyhow!("failed to read personal trace policy: {e}"))?
    {
        Some(personal) if personal.enabled => {
            return Ok(Some(TraceCredentialResolution {
                state_scope: scope,
                subject: None,
                policy: personal,
            }));
        }
        // A PRESENT scoped policy with enabled=false is an explicit user
        // opt-out (`traces opt-out`); it must win over the instance fallback.
        Some(_) => return Ok(None),
        // No scoped policy was ever written — the instance fallback applies.
        None => {}
    }

    let instance = read_trace_policy_for_scope_at(base_dir, None)
        .map_err(|e| anyhow::anyhow!("failed to read instance trace policy: {e}"))?;
    if instance.enabled {
        return Ok(Some(TraceCredentialResolution {
            subject: Some(salted_pseudonymous_contributor_id_at(base_dir, &scope)?),
            state_scope: scope,
            policy: instance,
        }));
    }

    Ok(None)
}

/// Explicit per-user Trace Commons opt-out: write (or update) the scope's
/// policy with `enabled = false`, which the resolver treats as an explicit
/// opt-out that blocks the instance fallback for this user — WITHOUT touching
/// the instance-level (scope-`None`) policy. This is the primitive a per-user
/// opt-out surface must use: flipping the root policy would disenroll the
/// entire instance.
pub fn opt_out_user_scope_at(base: &std::path::Path, scope: &str) -> anyhow::Result<()> {
    let mut policy =
        read_trace_policy_for_scope_if_present_at(base, Some(scope))?.unwrap_or_default();
    policy.enabled = false;
    write_trace_policy_for_scope_at(base, Some(scope), &policy)
}

/// [`opt_out_user_scope_at`] against the process base dir.
pub fn opt_out_user_scope(scope: &str) -> anyhow::Result<()> {
    opt_out_user_scope_at(&ironclaw_common::paths::ironclaw_base_dir(), scope)
}

/// Pick the user's own (personal-invite) enrollment when present and enabled,
/// else fall back to the admin-provisioned instance enrollment (scope `None`)
/// with a per-user pseudonymous subject. Returns `None` when neither is
/// enabled — and, importantly, when the user's scoped policy exists with
/// `enabled = false` (an explicit `traces opt-out`), which blocks the
/// instance fallback entirely.
pub fn resolve_trace_credentials(
    tenant_id: &TenantId,
    user_id: &UserId,
) -> anyhow::Result<Option<TraceCredentialResolution>> {
    // Typed at the public boundary so callers can't transpose tenant/user;
    // stringify only when handing off to the dir-parameterised core.
    resolve_trace_credentials_at(
        ironclaw_common::paths::ironclaw_base_dir().as_path(),
        tenant_id.as_str(),
        user_id.as_str(),
    )
}

/// The effective enrollment a scope contributes under during an autonomous
/// flush: the policy to gate/submit with, the directory that holds the device
/// key, and the per-user subject (if any) to attribute the upload to.
///
/// This consolidates the flush gate and per-user subject derivation into a
/// single policy-read/path-resolution pass so the two cannot drift — earlier
/// the gate and the subject derivation re-read the same policies independently.
pub(crate) struct EffectiveFlushTarget {
    pub(crate) policy: StandingTraceContributionPolicy,
    pub(crate) device_key_dir: PathBuf,
    pub(crate) subject: Option<String>,
}

/// Inner implementation that reads policies relative to an explicit base dir.
/// Used by `resolve_effective_flush_target` (real base) and by tests (tempdir).
pub(crate) fn resolve_effective_flush_target_at(
    base: &std::path::Path,
    scope: Option<&str>,
) -> anyhow::Result<Option<EffectiveFlushTarget>> {
    // Personal-invite enrollment: the per-scope policy is enabled and its device
    // key is already 1:1 with the user, so no explicit subject is needed.
    match read_trace_policy_for_scope_if_present_at(base, scope)
        .map_err(|e| anyhow::anyhow!("failed to read personal trace policy: {e}"))?
    {
        Some(personal) if personal.enabled => {
            return Ok(Some(EffectiveFlushTarget {
                policy: personal,
                device_key_dir: trace_contribution_dir_for_scope_at(base, scope),
                subject: None,
            }));
        }
        // A PRESENT scoped policy with enabled=false is an explicit user
        // opt-out (`traces opt-out`); capture/flush must NOT fall back to the
        // instance enrollment for this scope.
        Some(_) => return Ok(None),
        // No scoped policy was ever written — the instance fallback applies.
        None => {}
    }

    // Instance enrollment: no enabled per-scope policy, but the admin-provisioned
    // instance policy (scope None) is enabled. The device key lives at the shared
    // instance dir and uploads are attributed via a per-user pseudonymous subject.
    let instance = read_trace_policy_for_scope_at(base, None)
        .map_err(|e| anyhow::anyhow!("failed to read instance trace policy: {e}"))?;
    if instance.enabled {
        return Ok(Some(EffectiveFlushTarget {
            policy: instance,
            device_key_dir: trace_contribution_dir_for_scope_at(base, None),
            subject: scope
                .map(|s| salted_pseudonymous_contributor_id_at(base, s))
                .transpose()?,
        }));
    }

    Ok(None)
}

/// Resolve the enrollment a scope contributes under for the autonomous flush
/// path. See [`resolve_effective_flush_target_at`]. Returns `Ok(None)` when the
/// scope is enrolled in neither a personal-invite nor an instance enrollment.
pub(crate) fn resolve_effective_flush_target(
    scope: Option<&str>,
) -> anyhow::Result<Option<EffectiveFlushTarget>> {
    resolve_effective_flush_target_at(&ironclaw_common::paths::ironclaw_base_dir(), scope)
}

/// The effective trace-contribution policy a scope captures under: its own
/// personal-invite policy when enabled, else the admin-provisioned instance
/// policy (scope `None`). Returns `Ok(None)` when the scope is enrolled in
/// neither — i.e. capture must skip. This is the *capture-side* mirror of the
/// flush gate ([`resolve_effective_flush_target`]) so an instance-only-enrolled
/// user's turns are captured (and later flushed) instead of being dropped
/// because their per-user policy is absent/disabled. The returned policy is
/// always enabled.
pub fn resolve_effective_capture_policy(
    scope: Option<&str>,
) -> anyhow::Result<Option<StandingTraceContributionPolicy>> {
    resolve_effective_capture_policy_at(&ironclaw_common::paths::ironclaw_base_dir(), scope)
}

/// Dir-parameterised core for [`resolve_effective_capture_policy`] so tests can
/// use an isolated tempdir instead of the process-global instance scope.
pub(crate) fn resolve_effective_capture_policy_at(
    base: &std::path::Path,
    scope: Option<&str>,
) -> anyhow::Result<Option<StandingTraceContributionPolicy>> {
    Ok(resolve_effective_flush_target_at(base, scope)?.map(|target| target.policy))
}

pub fn mark_trace_credit_notice_due_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<Option<CreditSummary>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let policy = read_trace_policy_for_scope(scope)?;
    if !policy.enabled || policy.credit_notice_interval_hours == 0 {
        return Ok(None);
    }
    mark_trace_credit_noticed_if_due_at_unlocked(
        scope,
        policy.credit_notice_interval_hours,
        Utc::now(),
    )
}

pub fn trace_credit_notice_due_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<Option<CreditSummary>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let policy = read_trace_policy_for_scope(scope)?;
    if !policy.enabled || policy.credit_notice_interval_hours == 0 {
        return Ok(None);
    }
    trace_credit_notice_due_for_scope_at_unlocked(
        scope,
        policy.credit_notice_interval_hours,
        Utc::now(),
    )
}

pub fn acknowledge_trace_credit_notice_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<Option<CreditSummary>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let policy = read_trace_policy_for_scope(scope)?;
    if !policy.enabled || policy.credit_notice_interval_hours == 0 {
        return Ok(None);
    }
    acknowledge_trace_credit_notice_for_scope_at_unlocked(scope, Utc::now())
}

pub fn snooze_trace_credit_notice_for_scope(
    scope: Option<&str>,
    duration: chrono::Duration,
) -> anyhow::Result<Option<CreditSummary>> {
    let now = Utc::now();
    if duration <= chrono::Duration::zero() {
        anyhow::bail!("trace credit notice snooze duration must be positive");
    }
    if duration > chrono::Duration::hours(i64::from(TRACE_CREDIT_NOTICE_MAX_SNOOZE_HOURS)) {
        anyhow::bail!(
            "trace credit notice snooze duration must be at most {} hours",
            TRACE_CREDIT_NOTICE_MAX_SNOOZE_HOURS
        );
    }
    let snoozed_until = now
        .checked_add_signed(duration)
        .ok_or_else(|| anyhow::anyhow!("trace credit notice snooze deadline is out of range"))?;
    snooze_trace_credit_notice_for_scope_until_at(scope, snoozed_until, now)
}

pub fn snooze_trace_credit_notice_for_scope_until(
    scope: Option<&str>,
    snoozed_until: DateTime<Utc>,
) -> anyhow::Result<Option<CreditSummary>> {
    snooze_trace_credit_notice_for_scope_until_at(scope, snoozed_until, Utc::now())
}

pub fn read_trace_credit_notice_outbox_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<Vec<TraceCreditNoticeOutboxItem>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    read_trace_credit_notice_outbox_for_scope_unlocked(scope)
}

pub fn pending_trace_credit_notice_outbox_items_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<Vec<TraceCreditNoticeOutboxItem>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    pending_trace_credit_notice_outbox_items_for_scope_at_unlocked(scope, Utc::now())
}

pub fn record_trace_credit_notice_delivery_success_for_scope(
    scope: Option<&str>,
    fingerprint: &str,
    channel: &str,
) -> anyhow::Result<Option<TraceCreditNoticeOutboxItem>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    record_trace_credit_notice_delivery_success_for_scope_at_unlocked(
        scope,
        fingerprint,
        channel,
        Utc::now(),
    )
}

pub fn record_trace_credit_notice_delivery_failure_for_scope(
    scope: Option<&str>,
    fingerprint: &str,
    channel: &str,
    error: &str,
) -> anyhow::Result<Option<TraceCreditNoticeOutboxItem>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    record_trace_credit_notice_delivery_failure_for_scope_at_unlocked(
        scope,
        fingerprint,
        channel,
        error,
        Utc::now(),
    )
}

pub(crate) fn snooze_trace_credit_notice_for_scope_until_at(
    scope: Option<&str>,
    snoozed_until: DateTime<Utc>,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<CreditSummary>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let policy = read_trace_policy_for_scope(scope)?;
    if !policy.enabled || policy.credit_notice_interval_hours == 0 {
        return Ok(None);
    }
    snooze_trace_credit_notice_for_scope_until_at_unlocked(scope, snoozed_until, now)
}

pub fn queue_trace_envelope_for_scope(
    scope: Option<&str>,
    envelope: &TraceContributionEnvelope,
) -> anyhow::Result<PathBuf> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    queue_trace_envelope_for_scope_unlocked(scope, envelope)
}

/// Queue an envelope as **held for manual review**: write the envelope into
/// the scope queue and a `ManualReview` hold sidecar carrying `reason`, under
/// a single scope lock. The flush worker skips envelopes that have a hold
/// sidecar, so the trace is durably retained — reviewable and authorizable —
/// without being submitted until the hold is cleared.
pub fn queue_trace_envelope_as_held_for_scope(
    scope: Option<&str>,
    envelope: &TraceContributionEnvelope,
    reason: &str,
) -> anyhow::Result<PathBuf> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let path = queue_trace_envelope_for_scope_unlocked(scope, envelope)?;
    let hold = TraceQueueHold {
        submission_id: envelope.submission_id,
        kind: TraceQueueHoldKind::ManualReview,
        reason: reason.to_string(),
        attempts: 0,
        next_retry_at: None,
    };
    write_trace_queue_hold_sidecar_for_path(&path, &hold)?;
    Ok(path)
}

pub(crate) fn queue_trace_envelope_for_scope_unlocked(
    scope: Option<&str>,
    envelope: &TraceContributionEnvelope,
) -> anyhow::Result<PathBuf> {
    let path = trace_queue_dir(scope).join(format!("{}.json", envelope.submission_id));
    write_json_file(&path, envelope, "queued trace envelope")?;
    Ok(path)
}

pub fn queued_trace_envelope_paths_for_scope(scope: Option<&str>) -> anyhow::Result<Vec<PathBuf>> {
    let dir = trace_queue_dir(scope);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut paths = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .map_err(|e| anyhow::anyhow!("failed to read queue {}: {}", dir.display(), e))?
    {
        let entry = entry.map_err(|e| anyhow::anyhow!("failed to read queue entry: {}", e))?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json")
            && !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".held.json"))
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

pub fn read_trace_queue_holds_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<Vec<TraceQueueHold>> {
    let dir = trace_queue_dir(scope);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut holds = Vec::new();
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
            tracing::debug!(path = %path.display(), "Ignoring Trace Commons queue hold without a valid submission id");
            continue;
        };
        let Ok(body) = std::fs::read_to_string(&path) else {
            tracing::debug!(path = %path.display(), "Ignoring unreadable Trace Commons queue hold");
            continue;
        };
        let Ok(sidecar) = serde_json::from_str::<TraceQueueHoldSidecar>(&body) else {
            tracing::debug!(path = %path.display(), "Ignoring malformed Trace Commons queue hold");
            continue;
        };
        holds.push(trace_queue_hold_from_sidecar(submission_id, &sidecar));
    }
    holds.sort_by_key(|hold| hold.submission_id);
    Ok(holds)
}

/// The subset of queue holds that are awaiting user manual review (e.g. a High
/// residual-PII-risk hold). These are the only holds surfaced for the user to
/// authorize; policy/value gates (`PolicyGate`) and transient retry holds
/// (`RetryableSubmissionFailure`) are intentionally excluded.
pub fn manual_review_holds_for_scope(scope: Option<&str>) -> anyhow::Result<Vec<TraceQueueHold>> {
    Ok(retain_manual_review_holds(
        read_trace_queue_holds_for_scope(scope)?,
    ))
}

pub(crate) fn retain_manual_review_holds(holds: Vec<TraceQueueHold>) -> Vec<TraceQueueHold> {
    holds
        .into_iter()
        .filter(|hold| matches!(hold.kind, TraceQueueHoldKind::ManualReview))
        .collect()
}

/// Authorize a held manual-review trace for submission, promoting it as-is.
///
/// Stamps the queued envelope with `manual_review_authorized` (so
/// [`trace_autonomous_eligibility`] submits it past every gate) and removes
/// its `.held.json` sidecar. The envelope rewrite is the durable consent
/// record and happens BEFORE the sidecar removal, so any failure leaves the
/// trace held (fail closed). Returns `Ok(false)` when the submission has no
/// `ManualReview` hold (nothing to authorize); errors only on IO failure.
pub fn authorize_manual_review_hold_for_scope(
    scope: Option<&str>,
    submission_id: Uuid,
) -> anyhow::Result<bool> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let envelope_path = trace_queue_dir(scope).join(format!("{submission_id}.json"));
    if !envelope_path.exists() {
        return Ok(false);
    }
    let Some(sidecar) = read_trace_queue_hold_sidecar_for_envelope(&envelope_path)? else {
        return Ok(false);
    };
    if trace_queue_hold_from_sidecar(submission_id, &sidecar).kind
        != TraceQueueHoldKind::ManualReview
    {
        return Ok(false);
    }

    let mut envelope = load_trace_envelope(&envelope_path)?;
    envelope.manual_review_authorized = true;
    // Consent record first: persist the authorization before clearing the
    // hold, so a crash between the two leaves the trace held, not submitted.
    write_json_file(&envelope_path, &envelope, "authorized trace envelope")?;

    let hold_path = trace_queue_hold_path_for_envelope_path(&envelope_path);
    std::fs::remove_file(&hold_path).map_err(|error| {
        anyhow::anyhow!(
            "failed to remove trace hold sidecar {}: {}",
            hold_path.display(),
            error
        )
    })?;
    Ok(true)
}

pub fn trace_queue_diagnostics_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<TraceQueueDiagnostics> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let policy = read_trace_policy_for_scope(scope)?;
    let queued_count = queued_trace_envelope_paths_for_scope(scope)?.len() as u32;
    let holds = read_trace_queue_holds_for_scope(scope)?;
    let records = read_local_trace_records_for_scope(scope)?;
    let credit_report = trace_credit_report(&records);
    let telemetry = read_trace_queue_telemetry_for_scope_unlocked(scope)?;
    let warnings = trace_queue_warnings_for_scope_unlocked(scope)?;

    let mut held_reason_counts = BTreeMap::new();
    let mut retry_scheduled_count = 0;
    let mut manual_review_hold_count = 0;
    let mut policy_hold_count = 0;
    let mut next_retry_at = None;
    for hold in &holds {
        *held_reason_counts.entry(hold.reason.clone()).or_insert(0) += 1;
        match hold.kind {
            TraceQueueHoldKind::RetryableSubmissionFailure => {
                retry_scheduled_count += 1;
                if let Some(retry_at) = hold.next_retry_at {
                    next_retry_at = Some(
                        next_retry_at.map_or(retry_at, |current| std::cmp::min(current, retry_at)),
                    );
                }
            }
            TraceQueueHoldKind::ManualReview => manual_review_hold_count += 1,
            TraceQueueHoldKind::PolicyGate => policy_hold_count += 1,
        }
    }

    let endpoint_configured = policy
        .ingestion_endpoint
        .as_deref()
        .is_some_and(|endpoint| !endpoint.trim().is_empty());

    Ok(TraceQueueDiagnostics {
        queued_count,
        held_count: holds.len() as u32,
        submitted_count: credit_report.submissions_submitted,
        revoked_count: credit_report.submissions_revoked,
        expired_count: credit_report.submissions_expired,
        last_submission_at: credit_report.last_submission_at,
        last_credit_sync_at: credit_report.last_credit_sync_at,
        held_reason_counts,
        retry_scheduled_count,
        manual_review_hold_count,
        policy_hold_count,
        next_retry_at,
        telemetry,
        warnings,
        policy_enabled: policy.enabled,
        endpoint_configured,
        ready_to_flush: policy.enabled && endpoint_configured && queued_count > 0,
    })
}

pub fn load_trace_envelope(path: &Path) -> anyhow::Result<TraceContributionEnvelope> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read envelope {}: {}", path.display(), e))?;
    serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("failed to parse envelope {}: {}", path.display(), e))
}

pub(crate) fn load_queued_trace_envelope_or_quarantine(
    scope: Option<&str>,
    path: &Path,
    phase: &str,
) -> anyhow::Result<Option<TraceContributionEnvelope>> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read envelope {}: {}", path.display(), e))?;
    match serde_json::from_str(&body) {
        Ok(envelope) => Ok(Some(envelope)),
        Err(error) => {
            let quarantine_path = quarantine_malformed_trace_queue_envelope(scope, path)?;
            tracing::debug!(
                %error,
                path = %path.display(),
                quarantine_path = %quarantine_path.display(),
                phase,
                "Quarantined malformed Trace Commons queue envelope"
            );
            Ok(None)
        }
    }
}

pub fn apply_credit_estimate_to_envelope(envelope: &mut TraceContributionEnvelope) {
    let estimate = estimate_initial_credit(envelope);
    envelope.value.submission_score = estimate.submission_score;
    envelope.value.credit_points_pending = estimate.credit_points_pending;
    envelope.value.explanation = estimate.explanation;
    envelope.value_card.scorecard = estimate.scorecard;
    envelope.value_card.user_visible_explanation = envelope.value.explanation.clone();
}
