//! Path layout for the filesystem-backed engine store.
//!
//! All engine state lives under the `/engine` mount alias on a
//! [`ScopedFilesystem`](ironclaw_filesystem::ScopedFilesystem). The paths
//! below are alias-relative [`ScopedPath`] strings, not raw
//! [`VirtualPath`]s — at every op the [`ScopedFilesystem`] resolves the
//! alias against its [`MountView`](ironclaw_host_api::MountView) and
//! enforces per-grant ACL before any backend dispatch.
//!
//! ```text
//! /engine/threads/<thread_id>.json            — alias-relative
//! /engine/steps/<thread_id>/<step_id>.json
//! /engine/events/<thread_id>/<event_id>.json
//! /engine/projects/<project_id>.json
//! /engine/conversations/<conversation_id>.json
//! /engine/memory/<project_id>/<doc_id>.json
//! /engine/leases/<thread_id>/<lease_id>.json
//! /engine/missions/<project_id>/<mission_id>.json
//! ```
//!
//! These are [`ScopedPath`] strings under the `/engine` mount alias. The
//! [`MountView`](ironclaw_host_api::MountView) granted by composition
//! resolves the alias to a tenant-scoped
//! [`VirtualPath`](ironclaw_host_api::VirtualPath) (e.g.
//! `/engine/tenants/<tenant_id>/users/<user_id>/engine`), so the engine
//! code itself is tenant-agnostic.
//!
//! Indexed projection (rather than path hierarchy) is the queryable
//! surface for `user_id`, `status`, `parent_thread_id`, etc.

use ironclaw_filesystem::{IndexKey, IndexName, IndexValue};
use ironclaw_host_api::{HostApiError, ScopedPath};
use uuid::Uuid;

use crate::types::capability::LeaseId;
use crate::types::conversation::ConversationId;
use crate::types::error::EngineError;
use crate::types::memory::DocId;
use crate::types::mission::MissionId;
use crate::types::project::ProjectId;
use crate::types::step::StepId;
use crate::types::thread::ThreadId;

const THREADS_PREFIX: &str = "/engine/threads";
const STEPS_PREFIX: &str = "/engine/steps";
const EVENTS_PREFIX: &str = "/engine/events";
const PROJECTS_PREFIX: &str = "/engine/projects";
const CONVERSATIONS_PREFIX: &str = "/engine/conversations";
const MEMORY_PREFIX: &str = "/engine/memory";
const LEASES_PREFIX: &str = "/engine/leases";
const MISSIONS_PREFIX: &str = "/engine/missions";

pub(super) fn threads_prefix() -> Result<ScopedPath, EngineError> {
    scoped_path(THREADS_PREFIX)
}

pub(super) fn thread_path(thread_id: ThreadId) -> Result<ScopedPath, EngineError> {
    scoped_path(&format!("{THREADS_PREFIX}/{}.json", thread_id.0))
}

pub(super) fn steps_prefix(thread_id: ThreadId) -> Result<ScopedPath, EngineError> {
    scoped_path(&format!("{STEPS_PREFIX}/{}", thread_id.0))
}

pub(super) fn step_path(thread_id: ThreadId, step_id: StepId) -> Result<ScopedPath, EngineError> {
    scoped_path(&format!(
        "{STEPS_PREFIX}/{}/{}.json",
        thread_id.0, step_id.0
    ))
}

pub(super) fn events_prefix(thread_id: ThreadId) -> Result<ScopedPath, EngineError> {
    scoped_path(&format!("{EVENTS_PREFIX}/{}", thread_id.0))
}

pub(super) fn event_path(thread_id: ThreadId, event_id: Uuid) -> Result<ScopedPath, EngineError> {
    scoped_path(&format!(
        "{EVENTS_PREFIX}/{}/{}.json",
        thread_id.0, event_id
    ))
}

pub(super) fn projects_prefix() -> Result<ScopedPath, EngineError> {
    scoped_path(PROJECTS_PREFIX)
}

pub(super) fn project_path(project_id: ProjectId) -> Result<ScopedPath, EngineError> {
    scoped_path(&format!("{PROJECTS_PREFIX}/{}.json", project_id.0))
}

pub(super) fn conversation_path(
    conversation_id: ConversationId,
) -> Result<ScopedPath, EngineError> {
    scoped_path(&format!(
        "{CONVERSATIONS_PREFIX}/{}.json",
        conversation_id.0
    ))
}

pub(super) fn conversations_prefix() -> Result<ScopedPath, EngineError> {
    scoped_path(CONVERSATIONS_PREFIX)
}

pub(super) fn memory_prefix_for_project(project_id: ProjectId) -> Result<ScopedPath, EngineError> {
    scoped_path(&format!("{MEMORY_PREFIX}/{}", project_id.0))
}

pub(super) fn memory_prefix_all() -> Result<ScopedPath, EngineError> {
    scoped_path(MEMORY_PREFIX)
}

pub(super) fn memory_path(project_id: ProjectId, doc_id: DocId) -> Result<ScopedPath, EngineError> {
    scoped_path(&format!(
        "{MEMORY_PREFIX}/{}/{}.json",
        project_id.0, doc_id.0
    ))
}

pub(super) fn leases_prefix(thread_id: ThreadId) -> Result<ScopedPath, EngineError> {
    scoped_path(&format!("{LEASES_PREFIX}/{}", thread_id.0))
}

pub(super) fn lease_path(
    thread_id: ThreadId,
    lease_id: LeaseId,
) -> Result<ScopedPath, EngineError> {
    scoped_path(&format!(
        "{LEASES_PREFIX}/{}/{}.json",
        thread_id.0, lease_id.0
    ))
}

pub(super) fn leases_root() -> Result<ScopedPath, EngineError> {
    scoped_path(LEASES_PREFIX)
}

pub(super) fn missions_prefix() -> Result<ScopedPath, EngineError> {
    scoped_path(MISSIONS_PREFIX)
}

pub(super) fn missions_prefix_for_project(
    project_id: ProjectId,
) -> Result<ScopedPath, EngineError> {
    scoped_path(&format!("{MISSIONS_PREFIX}/{}", project_id.0))
}

pub(super) fn mission_path(
    project_id: ProjectId,
    mission_id: MissionId,
) -> Result<ScopedPath, EngineError> {
    scoped_path(&format!(
        "{MISSIONS_PREFIX}/{}/{}.json",
        project_id.0, mission_id.0
    ))
}

// ── Indexed-key names ────────────────────────────────────────

pub(super) fn index_key_user_id() -> IndexKey {
    index_key("user_id")
}

pub(super) fn index_key_project_id() -> IndexKey {
    index_key("project_id")
}

pub(super) fn index_key_thread_id() -> IndexKey {
    index_key("thread_id")
}

pub(super) fn index_key_parent_thread_id() -> IndexKey {
    index_key("parent_thread_id")
}

pub(super) fn index_key_status() -> IndexKey {
    index_key("status")
}

pub(super) fn index_key_doc_type() -> IndexKey {
    index_key("doc_type")
}

pub(super) fn index_key_revoked() -> IndexKey {
    index_key("revoked")
}

pub(super) fn index_name(name: &str) -> IndexName {
    // Names in this file are crate constants; if `IndexName::new` ever
    // rejects one the engine test suite will catch it at startup.
    IndexName::new(name).unwrap_or_else(|_| panic!("invalid index name literal: {name}"))
}

pub(super) fn index_value_text(s: impl Into<String>) -> IndexValue {
    IndexValue::Text(s.into())
}

pub(super) fn index_value_bool(b: bool) -> IndexValue {
    IndexValue::Bool(b)
}

// ── Internals ────────────────────────────────────────────────

fn scoped_path(raw: &str) -> Result<ScopedPath, EngineError> {
    ScopedPath::new(raw).map_err(host_api_to_engine_error)
}

fn index_key(key: &str) -> IndexKey {
    // Keys in this file are crate constants; if `IndexKey::new` ever
    // rejects one the engine test suite will catch it at startup.
    IndexKey::new(key).unwrap_or_else(|_| panic!("invalid index key literal: {key}"))
}

pub(super) fn host_api_to_engine_error(error: HostApiError) -> EngineError {
    EngineError::Store {
        reason: format!("filesystem engine store: invalid scoped path: {error}"),
    }
}
