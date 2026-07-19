//! Host-private replay-payload persistence for gate/auth resume (§5.3).
//!
//! When a capability invocation blocks at an approval or auth gate, the raw
//! replay payload needed to re-dispatch on a **later** resume turn — the tool
//! `input` JSON, its [`ResourceEstimate`], the prior-approval identity, the
//! input ref, and the correlation id — currently rides *in-band* through the
//! untrusted loop on [`CapabilityApprovalResume`] /
//! [`CapabilityAuthResume`](ironclaw_turns::run_profile::CapabilityAuthResume)
//! and is stashed in the loop's own serialized checkpoint. The
//! capability-result collapse (arch-simplification §5.3) makes the loop-facing
//! `Resolution` carry only an opaque resume token (equal to the
//! [`InvocationId`]), so the **host** must persist the replay payload itself and
//! reconstitute it on resume.
//!
//! [`ReplayPayload`] is therefore the exact opposite of a
//! [`GateRecord`](ironclaw_host_api::GateRecord): a `GateRecord` is the
//! *model-visible* content a pending gate renders from and carries only a
//! `SafeSummary`; a `ReplayPayload` is **host-private** and carries the raw tool
//! input. It must never be model-visible. Moving it host-side also retires a
//! real exposure — raw tool input no longer round-trips through the loop's
//! serialized checkpoint.
//!
//! This lives in `ironclaw_capabilities` (not `ironclaw_run_state`) because the
//! `ironclaw_run_state` charter forbids persisting raw replay input in run-state
//! records (`CLAUDE.md` line 7), and the `ironclaw_turns` charter forbids
//! persisting raw tool input in turn state or events — whereas
//! `ironclaw_capabilities` owns the caller-facing invoke/resume/spawn workflow
//! this payload exists to serve, and has no such prohibition. The record embeds
//! the resume-payload field types owned by `ironclaw_turns`
//! ([`CapabilityInputRef`], [`AuthResumeApprovalIdentity`]) rather than
//! re-typing them, per `type-placement.md`.
//!
//! The durable store mirrors `ironclaw_run_state`'s `FilesystemGateRecordStore`:
//! a [`ScopedFilesystem`] over any [`RootFilesystem`], the shared lock-free
//! [`cas_update`] lane (fail-closed on non-CAS backends), a `RecordKind` tag so
//! byte-only backends are rejected, and a private [`StoredReplayPayload`] wrapper
//! carrying the scope for a `same_scope_owner` defense-in-depth check.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasApply, CasUpdateError, ContentType, Entry, FilesystemError, RecordKind, RootFilesystem,
    ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    CorrelationId, HostApiError, InvocationId, ResourceEstimate, ResourceScope, ScopedPath,
};
use ironclaw_turns::run_profile::{AuthResumeApprovalIdentity, CapabilityInputRef};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Host-private replay payload for a gate/auth resume, keyed by [`InvocationId`].
///
/// Reuses the exact field types carried by
/// [`CapabilityApprovalResume`](ironclaw_turns::run_profile::CapabilityApprovalResume)
/// / [`CapabilityAuthResume`](ironclaw_turns::run_profile::CapabilityAuthResume)
/// so a later resume-read slice reconstitutes them without any lossy re-typing.
///
/// **Never model-visible.** Unlike a
/// [`GateRecord`](ironclaw_host_api::GateRecord) this deliberately carries no
/// `SafeSummary` — it holds the raw tool `input` and `estimate` and exists only
/// for host-side re-dispatch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayPayload {
    /// Raw runtime input captured when the gate was produced.
    pub input: serde_json::Value,
    /// Resource estimate captured alongside the input.
    pub estimate: ResourceEstimate,
    /// Present when the invocation previously passed a one-shot approval gate;
    /// carries the prior approval identity so auth-resume can claim the matching
    /// fingerprinted lease without a second human approval.
    pub prior_approval: Option<AuthResumeApprovalIdentity>,
    /// Loop-run-scoped input ref the gate was raised against.
    pub input_ref: CapabilityInputRef,
    /// Correlation id restored onto the invocation context on resume.
    pub correlation_id: CorrelationId,
}

/// Replay-payload persistence errors.
#[derive(Debug, Error)]
pub enum ReplayPayloadStoreError {
    /// Write-once violation: a payload already exists for this invocation.
    #[error("replay payload for invocation {invocation_id} already exists")]
    ReplayPayloadAlreadyExists { invocation_id: InvocationId },
    #[error("invalid storage path: {0}")]
    InvalidPath(String),
    #[error("filesystem error: {0}")]
    Filesystem(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("deserialization error: {0}")]
    Deserialization(String),
    #[error("replay payload backend error: {0}")]
    Backend(String),
}

impl From<FilesystemError> for ReplayPayloadStoreError {
    fn from(error: FilesystemError) -> Self {
        Self::Filesystem(error.to_string())
    }
}

/// Durable store for the host-private [`ReplayPayload`] a gate/auth resume
/// reconstitutes from (arch-simplification §5.3).
///
/// This is a dependency-inversion port (`type-placement.md` §"Traits" reason 2 /
/// 4): defined in this kernel crate, implemented by
/// [`FilesystemReplayPayloadStore`] and wired at composition — the same single-
/// production-impl shape as `ironclaw_run_state`'s `GateRecordStore` it mirrors.
///
/// Resource-owner scoped; wrong-scope lookups look unknown (`Ok(None)`). It
/// intentionally exposes no removal method: the replay payload is consumed once
/// on resume, and — like the sibling `GateRecordStore` — there is no scope-safe
/// soft-delete to mirror. Hard deletion of a retained record needs an explicit
/// product/retention contract (`database.md` "Data safety"); a later retention
/// slice can add it.
#[async_trait]
pub trait ReplayPayloadStore: Send + Sync {
    /// Persists the replay payload for `invocation_id` in the exact
    /// resource-owner scope.
    ///
    /// Write-once: an `invocation_id` that already has a payload is a
    /// [`ReplayPayloadStoreError::ReplayPayloadAlreadyExists`]. `InvocationId`s
    /// are freshly minted per invocation, so a collision is a caller-invariant
    /// violation, not an update path.
    async fn save(
        &self,
        scope: ResourceScope,
        invocation_id: InvocationId,
        payload: ReplayPayload,
    ) -> Result<(), ReplayPayloadStoreError>;

    /// Loads the replay payload for `invocation_id`; a wrong-scope lookup must
    /// look unknown (`Ok(None)`), never leak another owner's payload.
    async fn load(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<ReplayPayload>, ReplayPayloadStoreError>;
}

/// `RecordKind` tag written on every replay-payload entry so byte-only backends
/// (e.g. `DiskFilesystem`) are rejected with `Unsupported{WriteFile}` on first
/// put, which `cas_update` maps to `CasUnsupported` (fail-closed).
const REPLAY_PAYLOAD_RECORD_KIND: &str = "replay_payload_record";

/// Durable wrapper carrying the resource-owner scope alongside the
/// [`ReplayPayload`]. `ReplayPayload` has no scope field; persisting the scope
/// beside it lets [`FilesystemReplayPayloadStore::load`] apply the same
/// `same_scope_owner` defense-in-depth check the sibling gate-record store does,
/// so a wrong-scope read looks unknown. The scope is storage metadata only —
/// `load` returns the bare [`ReplayPayload`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredReplayPayload {
    scope: ResourceScope,
    payload: ReplayPayload,
}

/// Filesystem-backed replay-payload store under the `/replay-payloads` mount
/// alias.
///
/// Mirrors `ironclaw_run_state`'s `FilesystemGateRecordStore`: construct with a
/// [`ScopedFilesystem`] over any [`RootFilesystem`]. The [`ScopedFilesystem`]
/// resolves the `/replay-payloads` alias to a tenant/user-scoped
/// [`VirtualPath`](ironclaw_host_api::VirtualPath) per its
/// [`MountView`](ironclaw_host_api::MountView) and enforces per-op ACL before
/// any backend dispatch — so tenant isolation is structural. Within-tenant axes
/// (agent/project/mission/thread) remain in the alias-relative path because they
/// are not covered by the per-tenant `MountAlias`.
pub struct FilesystemReplayPayloadStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemReplayPayloadStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    fn record_entry(record: &StoredReplayPayload) -> Result<Entry, ReplayPayloadStoreError> {
        let body = serialize_pretty(record)?;
        let kind = RecordKind::new(REPLAY_PAYLOAD_RECORD_KIND)
            .map_err(|e| ReplayPayloadStoreError::Backend(e.to_string()))?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        Ok(entry)
    }
}

#[async_trait]
impl<F> ReplayPayloadStore for FilesystemReplayPayloadStore<F>
where
    F: RootFilesystem,
{
    async fn save(
        &self,
        scope: ResourceScope,
        invocation_id: InvocationId,
        payload: ReplayPayload,
    ) -> Result<(), ReplayPayloadStoreError> {
        let path = replay_payload_path(&scope, invocation_id)?;
        let stored = StoredReplayPayload {
            scope: scope.clone(),
            payload,
        };
        cas_update(
            self.filesystem.as_ref(),
            &scope,
            &path,
            |bytes: &[u8]| deserialize::<StoredReplayPayload>(bytes),
            |r: &StoredReplayPayload| Self::record_entry(r),
            |current: Option<StoredReplayPayload>| {
                let fresh = stored.clone();
                // Write-once: reject a duplicate invocation rather than clobbering
                // the host-private payload a later resume turn still needs.
                let outcome = if current.is_some() {
                    Err(ReplayPayloadStoreError::ReplayPayloadAlreadyExists { invocation_id })
                } else {
                    Ok(CasApply::new(fresh, ()))
                };
                async move { outcome }
            },
        )
        .await
        .map_err(map_cas_error)
    }

    async fn load(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<ReplayPayload>, ReplayPayloadStoreError> {
        let path = replay_payload_path(scope, invocation_id)?;
        let Some(versioned) = self.filesystem.get(scope, &path).await? else {
            return Ok(None);
        };
        let stored = deserialize::<StoredReplayPayload>(&versioned.entry.body)?;
        // Defense-in-depth against a shared-path read; wrong scope looks unknown.
        if same_scope_owner(&stored.scope, scope) {
            Ok(Some(stored.payload))
        } else {
            Ok(None)
        }
    }
}

// Path layout under the `/replay-payloads` mount alias:
//
//     /replay-payloads[/agents/<agent>][/projects/<project>][/missions/<mission>][/threads/<thread>]/<invocation_id>.json
//
// Tenant + user identity moves into the caller's `MountView` per the per-tenant
// `MountAlias` rewriting, so neither prefix is encoded in the path itself.
// Within-tenant sub-scope axes (agent/project/mission/thread) stay in the
// alias-relative path because they are within-tenant scoping not covered by the
// per-tenant `MountAlias`. Mirrors `ironclaw_run_state`'s `/gate-records` layout.

const REPLAY_PAYLOADS_PREFIX: &str = "/replay-payloads";

fn replay_payload_path(
    scope: &ResourceScope,
    invocation_id: InvocationId,
) -> Result<ScopedPath, ReplayPayloadStoreError> {
    scoped_path(&format!(
        "{}/{invocation_id}.json",
        scope_owner_alias_string(REPLAY_PAYLOADS_PREFIX, scope)
    ))
}

/// Build the alias-relative owner prefix for a scope under the given mount
/// alias. Tenant and user are intentionally absent — they live in the
/// `MountView` the caller supplied. Sub-scope axes (agent/project/mission/
/// thread) stay in the path so within-tenant cross-scope isolation still works
/// for stores sharing one alias target. Mirrors the sibling helper in
/// `ironclaw_run_state`.
fn scope_owner_alias_string(prefix: &'static str, scope: &ResourceScope) -> String {
    let mut base = String::from(prefix);
    if let Some(agent_id) = &scope.agent_id {
        base.push_str("/agents/");
        base.push_str(agent_id.as_str());
    }
    if let Some(project_id) = &scope.project_id {
        base.push_str("/projects/");
        base.push_str(project_id.as_str());
    }
    if let Some(mission_id) = &scope.mission_id {
        base.push_str("/missions/");
        base.push_str(mission_id.as_str());
    }
    if let Some(thread_id) = &scope.thread_id {
        base.push_str("/threads/");
        base.push_str(thread_id.as_str());
    }
    base
}

fn scoped_path(raw: &str) -> Result<ScopedPath, ReplayPayloadStoreError> {
    ScopedPath::new(raw).map_err(invalid_path)
}

fn invalid_path(error: HostApiError) -> ReplayPayloadStoreError {
    ReplayPayloadStoreError::InvalidPath(error.to_string())
}

fn same_scope_owner(left: &ResourceScope, right: &ResourceScope) -> bool {
    left.tenant_id == right.tenant_id
        && left.user_id == right.user_id
        && left.agent_id == right.agent_id
        && left.project_id == right.project_id
        && left.mission_id == right.mission_id
        && left.thread_id == right.thread_id
}

fn serialize_pretty<T>(value: &T) -> Result<Vec<u8>, ReplayPayloadStoreError>
where
    T: Serialize,
{
    serde_json::to_vec_pretty(value)
        .map_err(|error| ReplayPayloadStoreError::Serialization(error.to_string()))
}

fn deserialize<T>(bytes: &[u8]) -> Result<T, ReplayPayloadStoreError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes)
        .map_err(|error| ReplayPayloadStoreError::Deserialization(error.to_string()))
}

/// Map the shared CAS helper's [`CasUpdateError`] into a
/// [`ReplayPayloadStoreError`], preserving the caller's own error and failing
/// closed on a backend that cannot honor versioned CAS (mirrors
/// `ironclaw_run_state`'s `map_cas_error`).
fn map_cas_error(error: CasUpdateError<ReplayPayloadStoreError>) -> ReplayPayloadStoreError {
    match error {
        CasUpdateError::Apply(inner) => inner,
        CasUpdateError::Timeout | CasUpdateError::RetriesExhausted => {
            ReplayPayloadStoreError::Backend("filesystem CAS retries exhausted".to_string())
        }
        CasUpdateError::CasUnsupported => ReplayPayloadStoreError::Backend(
            "backend does not support versioned compare-and-swap".to_string(),
        ),
        CasUpdateError::Backend(fs_err) => ReplayPayloadStoreError::Filesystem(fs_err.to_string()),
    }
}
