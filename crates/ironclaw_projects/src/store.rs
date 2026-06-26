//! [`ProjectRepository`] over the Reborn `ScopedFilesystem` substrate.
//!
//! Records are JSON entries under a control-plane mount that the agent cannot
//! reach (the same substrate the reborn-identity store rides). Backend
//! selection — Postgres / libSQL / JSONL / in-memory — is the host's
//! `RootFilesystem` concern, so this is the single backend-agnostic
//! implementation; the crate carries no SQL.
//!
//! Layout (opaque key parts are base64url-encoded into their own segments so a
//! delimiter-like id cannot collide with a path boundary, mirroring
//! `ironclaw_reborn_identity`):
//!
//! ```text
//! /tenant-shared/reborn-projects/<tenant>/records/<project_id>.json
//! /tenant-shared/reborn-projects/<tenant>/members/<project_id>/<user_id>.json
//! ```
//!
//! Tenant isolation is twofold: a per-call [`ResourceScope`] carries the
//! `tenant_id` (so a real mount resolver maps `/tenant-shared` to a per-tenant
//! virtual path) AND the tenant is a path segment (so isolation also holds
//! under a fixed-view resolver, as in tests). `created_at` is immutable across
//! updates. Concurrency uses the substrate's compare-and-swap: create writes
//! with [`CasExpectation::Absent`] (a conflict ⇒ [`ProjectError::AlreadyExists`]);
//! delete is keyed off the record's presence so a losing racer observes `None`.

use std::sync::Arc;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, ScopedPath, TenantId, UserId,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    ProjectError, ProjectMemberRecord, ProjectMemberStatus, ProjectRecord, ProjectRepository,
    ProjectRole,
};

const PROJECTS_ROOT: &str = "/tenant-shared/reborn-projects";

/// [`ProjectRepository`] persisting over a [`ScopedFilesystem`].
pub struct FilesystemProjectRepository<F>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    /// Control-plane caller identity. The `tenant_id` for each operation is
    /// supplied per call (it drives mount resolution and the path); these are
    /// the stable user/agent the control-plane store acts as.
    user_id: UserId,
    agent_id: AgentId,
}

impl<F> FilesystemProjectRepository<F>
where
    F: RootFilesystem + 'static,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>, user_id: UserId, agent_id: AgentId) -> Self {
        Self {
            filesystem,
            user_id,
            agent_id,
        }
    }

    /// Per-operation scope carrying the operation's tenant.
    fn scope_for(&self, tenant_id: &TenantId) -> ResourceScope {
        ResourceScope {
            tenant_id: tenant_id.clone(),
            user_id: self.user_id.clone(),
            agent_id: Some(self.agent_id.clone()),
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    async fn read_record<T>(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Option<T>, ProjectError>
    where
        T: DeserializeOwned,
    {
        let Some(versioned) = self.filesystem.get(scope, path).await.map_err(fs_backend)? else {
            return Ok(None);
        };
        let value = serde_json::from_slice(&versioned.entry.body).map_err(|error| {
            ProjectError::backend(&format!("decode record at {}", path.as_str()), error)
        })?;
        Ok(Some(value))
    }

    async fn write_record<T>(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        value: &T,
        cas: CasExpectation,
    ) -> Result<(), FilesystemError>
    where
        T: Serialize,
    {
        let body =
            serde_json::to_vec(value).map_err(|error| FilesystemError::BackendInfrastructure {
                operation: FilesystemOperation::WriteFile,
                reason: format!(
                    "record at {} could not be serialized: {error}",
                    path.as_str()
                ),
            })?;
        self.filesystem
            .put(
                scope,
                path,
                Entry::bytes(body).with_content_type(ContentType::json()),
                cas,
            )
            .await
            .map(|_version| ())
    }

    /// Scoped paths of the JSON record files directly under `dir`. A missing
    /// directory is an empty listing (a project with no members / a tenant with
    /// no projects).
    async fn child_record_paths(
        &self,
        scope: &ResourceScope,
        dir: &ScopedPath,
    ) -> Result<Vec<ScopedPath>, ProjectError> {
        let entries = match self.filesystem.list_dir(scope, dir).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(fs_backend(error)),
        };
        let mut paths = Vec::with_capacity(entries.len());
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            paths.push(scoped(&format!("{}/{}", dir.as_str(), entry.name))?);
        }
        Ok(paths)
    }
}

#[async_trait]
impl<F> ProjectRepository for FilesystemProjectRepository<F>
where
    F: RootFilesystem + 'static,
{
    async fn create_project(&self, record: ProjectRecord) -> Result<(), ProjectError> {
        record.validate()?;
        let scope = self.scope_for(&record.tenant_id);
        let path = record_path(&record.tenant_id, &record.project_id)?;
        match self
            .write_record(&scope, &path, &record, CasExpectation::Absent)
            .await
        {
            Ok(()) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => Err(ProjectError::AlreadyExists),
            Err(error) => Err(fs_backend(error)),
        }
    }

    async fn get_project(
        &self,
        tenant_id: &TenantId,
        project_id: &ProjectId,
    ) -> Result<Option<ProjectRecord>, ProjectError> {
        let scope = self.scope_for(tenant_id);
        self.read_record(&scope, &record_path(tenant_id, project_id)?)
            .await
    }

    async fn update_project(&self, mut record: ProjectRecord) -> Result<(), ProjectError> {
        record.validate()?;
        let scope = self.scope_for(&record.tenant_id);
        let path = record_path(&record.tenant_id, &record.project_id)?;
        let Some(existing) = self.read_record::<ProjectRecord>(&scope, &path).await? else {
            return Err(ProjectError::NotFound);
        };
        // `created_at` is immutable.
        record.created_at = existing.created_at;
        self.write_record(&scope, &path, &record, CasExpectation::Any)
            .await
            .map_err(fs_backend)
    }

    async fn delete_project(
        &self,
        tenant_id: &TenantId,
        project_id: &ProjectId,
    ) -> Result<Option<ProjectRecord>, ProjectError> {
        let scope = self.scope_for(tenant_id);
        let path = record_path(tenant_id, project_id)?;
        let Some(record) = self.read_record::<ProjectRecord>(&scope, &path).await? else {
            return Ok(None);
        };
        // Delete the record first: a "record gone, members orphaned" window is
        // harmless (resolve_access and listings key off the record), whereas the
        // inverse would silently drop access. A losing concurrent delete sees
        // NotFound here and returns None.
        match self.filesystem.delete(&scope, &path).await {
            Ok(()) => {}
            Err(FilesystemError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(fs_backend(error)),
        }
        // Fail loud: a member delete that errors (anything but NotFound) leaves an
        // orphaned grant under /members/...; if the project id is ever reused, a
        // stale active grant could reappear as real access. Propagate instead of
        // warn-and-continue (see .claude/rules/error-handling.md).
        for member_path in self
            .child_record_paths(&scope, &members_dir(tenant_id, project_id)?)
            .await?
        {
            match self.filesystem.delete(&scope, &member_path).await {
                Ok(()) | Err(FilesystemError::NotFound { .. }) => {}
                Err(error) => return Err(fs_backend(error)),
            }
        }
        Ok(Some(record))
    }

    async fn list_projects_for_user(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        limit: usize,
    ) -> Result<Vec<ProjectRecord>, ProjectError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let scope = self.scope_for(tenant_id);
        let mut projects = Vec::new();
        for path in self
            .child_record_paths(&scope, &records_dir(tenant_id)?)
            .await?
        {
            let Some(project) = self.read_record::<ProjectRecord>(&scope, &path).await? else {
                continue;
            };
            let accessible = &project.owner_user_id == user_id
                || matches!(
                    self.read_record::<ProjectMemberRecord>(
                        &scope,
                        &member_path(tenant_id, &project.project_id, user_id)?,
                    )
                    .await?,
                    Some(member) if member.status == ProjectMemberStatus::Active
                );
            if accessible {
                projects.push(project);
            }
        }
        projects.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.project_id.as_str().cmp(a.project_id.as_str()))
        });
        projects.truncate(limit);
        Ok(projects)
    }

    async fn list_members(
        &self,
        tenant_id: &TenantId,
        project_id: &ProjectId,
    ) -> Result<Vec<ProjectMemberRecord>, ProjectError> {
        let scope = self.scope_for(tenant_id);
        let mut members = Vec::new();
        for path in self
            .child_record_paths(&scope, &members_dir(tenant_id, project_id)?)
            .await?
        {
            if let Some(member) = self
                .read_record::<ProjectMemberRecord>(&scope, &path)
                .await?
            {
                members.push(member);
            }
        }
        members.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.user_id.as_str().cmp(b.user_id.as_str()))
        });
        Ok(members)
    }

    async fn upsert_member(&self, mut record: ProjectMemberRecord) -> Result<(), ProjectError> {
        record.validate()?;
        let scope = self.scope_for(&record.tenant_id);
        let path = member_path(&record.tenant_id, &record.project_id, &record.user_id)?;
        // `created_at` is immutable: a role/status update must not rewrite the
        // grant's original creation time (preserves audit ordering).
        if let Some(existing) = self
            .read_record::<ProjectMemberRecord>(&scope, &path)
            .await?
        {
            record.created_at = existing.created_at;
        }
        self.write_record(&scope, &path, &record, CasExpectation::Any)
            .await
            .map_err(fs_backend)
    }

    async fn remove_member(
        &self,
        tenant_id: &TenantId,
        project_id: &ProjectId,
        user_id: &UserId,
    ) -> Result<Option<ProjectMemberRecord>, ProjectError> {
        let scope = self.scope_for(tenant_id);
        let path = member_path(tenant_id, project_id, user_id)?;
        let Some(member) = self
            .read_record::<ProjectMemberRecord>(&scope, &path)
            .await?
        else {
            return Ok(None);
        };
        match self.filesystem.delete(&scope, &path).await {
            Ok(()) => Ok(Some(member)),
            Err(FilesystemError::NotFound { .. }) => Ok(None),
            Err(error) => Err(fs_backend(error)),
        }
    }

    async fn resolve_access(
        &self,
        tenant_id: &TenantId,
        project_id: &ProjectId,
        user_id: &UserId,
    ) -> Result<Option<ProjectRole>, ProjectError> {
        let scope = self.scope_for(tenant_id);
        let Some(project) = self
            .read_record::<ProjectRecord>(&scope, &record_path(tenant_id, project_id)?)
            .await?
        else {
            return Ok(None);
        };
        if &project.owner_user_id == user_id {
            return Ok(Some(ProjectRole::Owner));
        }
        match self
            .read_record::<ProjectMemberRecord>(
                &scope,
                &member_path(tenant_id, project_id, user_id)?,
            )
            .await?
        {
            Some(member) if member.status == ProjectMemberStatus::Active => Ok(Some(member.role)),
            _ => Ok(None),
        }
    }
}

/// URL-safe path segment for an opaque key part. Empty maps to `_` (a value no
/// base64 encoding produces) so an absent part never collapses to an empty
/// segment.
fn segment(value: &str) -> String {
    if value.is_empty() {
        "_".to_string()
    } else {
        URL_SAFE_NO_PAD.encode(value.as_bytes())
    }
}

fn scoped(raw: &str) -> Result<ScopedPath, ProjectError> {
    ScopedPath::new(raw).map_err(|error| ProjectError::backend("invalid project path", error))
}

fn record_path(tenant: &TenantId, project: &ProjectId) -> Result<ScopedPath, ProjectError> {
    scoped(&format!(
        "{PROJECTS_ROOT}/{}/records/{}.json",
        segment(tenant.as_str()),
        segment(project.as_str()),
    ))
}

fn records_dir(tenant: &TenantId) -> Result<ScopedPath, ProjectError> {
    scoped(&format!(
        "{PROJECTS_ROOT}/{}/records",
        segment(tenant.as_str()),
    ))
}

fn members_dir(tenant: &TenantId, project: &ProjectId) -> Result<ScopedPath, ProjectError> {
    scoped(&format!(
        "{PROJECTS_ROOT}/{}/members/{}",
        segment(tenant.as_str()),
        segment(project.as_str()),
    ))
}

fn member_path(
    tenant: &TenantId,
    project: &ProjectId,
    user: &UserId,
) -> Result<ScopedPath, ProjectError> {
    scoped(&format!(
        "{PROJECTS_ROOT}/{}/members/{}/{}.json",
        segment(tenant.as_str()),
        segment(project.as_str()),
        segment(user.as_str()),
    ))
}

fn fs_backend(error: FilesystemError) -> ProjectError {
    ProjectError::backend("project filesystem", error)
}
