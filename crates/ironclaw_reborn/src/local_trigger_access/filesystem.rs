use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::{SecondsFormat, Utc};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FileType, FilesystemError, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, ScopedPath, TenantId, UserId,
};
use serde::{Deserialize, Serialize};

use super::types::{
    LocalTriggerAccessReconciliation, LocalTriggerAccessSeed, LocalTriggerAccessSource,
    LocalTriggerAccessStatus, LocalTriggerAccessStore, RebornLocalTriggerAccessStoreError, backend,
};

/// Filesystem-backed local trigger access repository.
pub struct RebornFilesystemLocalTriggerAccessStore<F>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FilesystemLocalTriggerAccessRecord {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    role: String,
    status: String,
    source: String,
    created_at: String,
    updated_at: String,
}

impl<F> RebornFilesystemLocalTriggerAccessStore<F>
where
    F: RootFilesystem + 'static,
{
    /// Build a store over the host filesystem abstraction. The root
    /// filesystem backend has already run its own migrations at composition
    /// time; this store owns only JSON record shapes and scoped paths.
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    fn record_entry(
        record: &FilesystemLocalTriggerAccessRecord,
    ) -> Result<Entry, RebornLocalTriggerAccessStoreError> {
        let body = serde_json::to_vec_pretty(record).map_err(backend)?;
        Ok(Entry::bytes(body).with_content_type(ContentType::json()))
    }

    async fn read_record(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<
        Option<(
            FilesystemLocalTriggerAccessRecord,
            ironclaw_filesystem::RecordVersion,
        )>,
        RebornLocalTriggerAccessStoreError,
    > {
        let Some(versioned) = self.filesystem.get(scope, path).await.map_err(backend)? else {
            return Ok(None);
        };
        let record = serde_json::from_slice(&versioned.entry.body).map_err(backend)?;
        Ok(Some((record, versioned.version)))
    }

    async fn put_record(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        record: &FilesystemLocalTriggerAccessRecord,
        cas: CasExpectation,
    ) -> Result<(), FilesystemAccessPutError> {
        let entry = Self::record_entry(record).map_err(FilesystemAccessPutError::Other)?;
        match self.filesystem.put(scope, path, entry, cas).await {
            Ok(_) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => {
                Err(FilesystemAccessPutError::VersionMismatch)
            }
            Err(error) => Err(FilesystemAccessPutError::Other(backend(error))),
        }
    }

    async fn deactivate_stale_record(
        &self,
        tenant_id: &TenantId,
        path: &ScopedPath,
        user_id: &UserId,
        agent_id: Option<&AgentId>,
        project_id: Option<&ProjectId>,
        source: LocalTriggerAccessSource,
        allowed: &BTreeSet<&str>,
    ) -> Result<(), RebornLocalTriggerAccessStoreError> {
        let scope = tenant_shared_scope(tenant_id, user_id, agent_id, project_id);
        for _ in 0..FILESYSTEM_CAS_RETRIES {
            let Some((mut record, version)) = self.read_record(&scope, path).await? else {
                return Ok(());
            };
            if !record_matches_scope(&record, tenant_id, user_id, agent_id, project_id)
                || record.source != source.as_str()
                || record.status != LocalTriggerAccessStatus::Active.as_str()
                || allowed.contains(record.user_id.as_str())
            {
                return Ok(());
            }
            record.status = LocalTriggerAccessStatus::Inactive.as_str().to_string();
            record.updated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
            match self
                .put_record(&scope, path, &record, CasExpectation::Version(version))
                .await
            {
                Ok(()) => return Ok(()),
                Err(FilesystemAccessPutError::VersionMismatch) => continue,
                Err(FilesystemAccessPutError::Other(error)) => return Err(error),
            }
        }
        Err(backend(format!(
            "filesystem CAS retries exhausted for path {}",
            path.as_str()
        )))
    }

    /// Seed the local trigger access row used by Reborn-owned fire-time trigger
    /// authorization. Existing rows are left untouched so an operator can
    /// revoke or edit access without the next boot or login silently
    /// re-granting it.
    pub async fn seed_local_access(
        &self,
        seed: LocalTriggerAccessSeed<'_>,
    ) -> Result<(), RebornLocalTriggerAccessStoreError> {
        let scope =
            tenant_shared_scope(seed.tenant_id, seed.user_id, seed.agent_id, seed.project_id);
        let path = access_record_path(seed.agent_id, seed.project_id, seed.user_id)?;
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let record = FilesystemLocalTriggerAccessRecord {
            tenant_id: seed.tenant_id.as_str().to_string(),
            user_id: seed.user_id.as_str().to_string(),
            agent_id: seed.agent_id.map(|agent_id| agent_id.as_str().to_string()),
            project_id: seed
                .project_id
                .map(|project_id| project_id.as_str().to_string()),
            role: seed.role.as_str().to_string(),
            status: LocalTriggerAccessStatus::Active.as_str().to_string(),
            source: seed.source.as_str().to_string(),
            created_at: now.clone(),
            updated_at: now,
        };
        match self
            .put_record(&scope, &path, &record, CasExpectation::Absent)
            .await
        {
            Ok(()) => Ok(()),
            Err(FilesystemAccessPutError::VersionMismatch) => Ok(()),
            Err(FilesystemAccessPutError::Other(error)) => Err(error),
        }
    }

    /// Reconcile bootstrap-owned local trigger access rows for one exact scope.
    pub async fn reconcile_local_access(
        &self,
        reconciliation: LocalTriggerAccessReconciliation<'_>,
    ) -> Result<(), RebornLocalTriggerAccessStoreError> {
        let allowed: BTreeSet<&str> = reconciliation.user_ids.iter().map(UserId::as_str).collect();
        let bootstrap_user = match reconciliation.user_ids.first() {
            Some(user_id) => user_id.clone(),
            None => trigger_access_bootstrap_user_id()?,
        };
        let scope = tenant_shared_scope(
            reconciliation.tenant_id,
            &bootstrap_user,
            reconciliation.agent_id,
            reconciliation.project_id,
        );
        let users_root =
            access_scope_users_root(reconciliation.agent_id, reconciliation.project_id)?;
        let entries = match self.filesystem.list_dir(&scope, &users_root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => Vec::new(),
            Err(error) => return Err(backend(error)),
        };
        for entry in entries {
            if entry.file_type != FileType::File || !entry.name.ends_with(".json") {
                continue;
            }
            let user_key = entry.name.trim_end_matches(".json");
            let user_id = UserId::new(user_key.to_string()).map_err(backend)?;
            let path =
                access_record_path(reconciliation.agent_id, reconciliation.project_id, &user_id)?;
            self.deactivate_stale_record(
                reconciliation.tenant_id,
                &path,
                &user_id,
                reconciliation.agent_id,
                reconciliation.project_id,
                reconciliation.source,
                &allowed,
            )
            .await?;
        }

        for user_id in reconciliation.user_ids {
            self.seed_local_access(LocalTriggerAccessSeed {
                tenant_id: reconciliation.tenant_id,
                user_id,
                agent_id: reconciliation.agent_id,
                project_id: reconciliation.project_id,
                role: reconciliation.role,
                source: reconciliation.source,
            })
            .await?;
        }
        Ok(())
    }

    /// Return whether a local trigger user has active access for the exact
    /// tenant/agent/project tuple on a trigger fire request.
    pub async fn has_active_local_access(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        agent_id: Option<&AgentId>,
        project_id: Option<&ProjectId>,
    ) -> Result<bool, RebornLocalTriggerAccessStoreError> {
        let scope = tenant_shared_scope(tenant_id, user_id, agent_id, project_id);
        let path = access_record_path(agent_id, project_id, user_id)?;
        let Some((record, _version)) = self.read_record(&scope, &path).await? else {
            return Ok(false);
        };
        Ok(
            record_matches_scope(&record, tenant_id, user_id, agent_id, project_id)
                && record.status == LocalTriggerAccessStatus::Active.as_str(),
        )
    }
}

enum FilesystemAccessPutError {
    VersionMismatch,
    Other(RebornLocalTriggerAccessStoreError),
}

const FILESYSTEM_CAS_RETRIES: usize = 8;
const TRIGGER_ACCESS_ROOT: &str = "/tenant-shared/reborn-trigger-access";

fn tenant_shared_scope(
    tenant_id: &TenantId,
    user_id: &UserId,
    agent_id: Option<&AgentId>,
    project_id: Option<&ProjectId>,
) -> ResourceScope {
    ResourceScope {
        tenant_id: tenant_id.clone(),
        user_id: user_id.clone(),
        agent_id: agent_id.cloned(),
        project_id: project_id.cloned(),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn trigger_access_bootstrap_user_id() -> Result<UserId, RebornLocalTriggerAccessStoreError> {
    UserId::new("trigger-access-bootstrap").map_err(backend)
}

fn access_scope_users_root(
    agent_id: Option<&AgentId>,
    project_id: Option<&ProjectId>,
) -> Result<ScopedPath, RebornLocalTriggerAccessStoreError> {
    ScopedPath::new(format!(
        "{}/agents/{}/projects/{}/users",
        TRIGGER_ACCESS_ROOT,
        optional_axis_path(agent_id.map(AgentId::as_str)),
        optional_axis_path(project_id.map(ProjectId::as_str))
    ))
    .map_err(backend)
}

fn access_record_path(
    agent_id: Option<&AgentId>,
    project_id: Option<&ProjectId>,
    user_id: &UserId,
) -> Result<ScopedPath, RebornLocalTriggerAccessStoreError> {
    ScopedPath::new(format!(
        "{}/{}.json",
        access_scope_users_root(agent_id, project_id)?.as_str(),
        user_id.as_str()
    ))
    .map_err(backend)
}

fn optional_axis_path(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("some/{value}"),
        None => "none".to_string(),
    }
}

fn record_matches_scope(
    record: &FilesystemLocalTriggerAccessRecord,
    tenant_id: &TenantId,
    user_id: &UserId,
    agent_id: Option<&AgentId>,
    project_id: Option<&ProjectId>,
) -> bool {
    record.tenant_id == tenant_id.as_str()
        && record.user_id == user_id.as_str()
        && record.agent_id.as_deref() == agent_id.map(AgentId::as_str)
        && record.project_id.as_deref() == project_id.map(ProjectId::as_str)
}

fn is_not_found(error: &FilesystemError) -> bool {
    matches!(error, FilesystemError::NotFound { .. })
}

#[async_trait::async_trait]
impl<F> LocalTriggerAccessStore for RebornFilesystemLocalTriggerAccessStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn seed_local_access(
        &self,
        seed: LocalTriggerAccessSeed<'_>,
    ) -> Result<(), RebornLocalTriggerAccessStoreError> {
        RebornFilesystemLocalTriggerAccessStore::seed_local_access(self, seed).await
    }

    async fn reconcile_local_access(
        &self,
        reconciliation: LocalTriggerAccessReconciliation<'_>,
    ) -> Result<(), RebornLocalTriggerAccessStoreError> {
        RebornFilesystemLocalTriggerAccessStore::reconcile_local_access(self, reconciliation).await
    }

    async fn has_active_local_access(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        agent_id: Option<&AgentId>,
        project_id: Option<&ProjectId>,
    ) -> Result<bool, RebornLocalTriggerAccessStoreError> {
        RebornFilesystemLocalTriggerAccessStore::has_active_local_access(
            self, tenant_id, user_id, agent_id, project_id,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_trigger_access::{LocalTriggerAccessRole, LocalTriggerAccessSource};
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    fn store() -> RebornFilesystemLocalTriggerAccessStore<InMemoryBackend> {
        let root = Arc::new(InMemoryBackend::default());
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new("/tenant-shared").expect("mount alias"),
            VirtualPath::new("/tenants/fs-trigger/shared").expect("virtual path"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(root, view));
        RebornFilesystemLocalTriggerAccessStore::new(filesystem)
    }

    #[tokio::test]
    async fn filesystem_store_reconciles_and_checks_exact_scope() {
        let store = store();
        let tenant_id = TenantId::new("fs-trigger-tenant").expect("tenant id");
        let user_id = UserId::new("fs-trigger-user").expect("user id");
        let stale_user_id = UserId::new("fs-trigger-stale").expect("stale user id");
        let agent_id = AgentId::new("fs-trigger-agent").expect("agent id");
        let project_id = ProjectId::new("fs-trigger-project").expect("project id");
        let other_project_id = ProjectId::new("fs-trigger-other-project").expect("project id");

        store
            .seed_local_access(LocalTriggerAccessSeed {
                tenant_id: &tenant_id,
                user_id: &stale_user_id,
                agent_id: Some(&agent_id),
                project_id: Some(&project_id),
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevEnvBootstrap,
            })
            .await
            .expect("seed stale local access");

        store
            .reconcile_local_access(LocalTriggerAccessReconciliation {
                tenant_id: &tenant_id,
                user_ids: std::slice::from_ref(&user_id),
                agent_id: Some(&agent_id),
                project_id: Some(&project_id),
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevEnvBootstrap,
            })
            .await
            .expect("reconcile local access");

        assert!(
            store
                .has_active_local_access(&tenant_id, &user_id, Some(&agent_id), Some(&project_id))
                .await
                .expect("check active local access"),
            "the reconciled filesystem record allows the exact scope"
        );
        assert!(
            !store
                .has_active_local_access(
                    &tenant_id,
                    &user_id,
                    Some(&agent_id),
                    Some(&other_project_id),
                )
                .await
                .expect("check wrong project access"),
            "filesystem trigger access is exact-project, not a wildcard"
        );
        assert!(
            !store
                .has_active_local_access(
                    &tenant_id,
                    &stale_user_id,
                    Some(&agent_id),
                    Some(&project_id),
                )
                .await
                .expect("check stale local access"),
            "reconciliation deactivates stale filesystem records for the same source"
        );
    }
}
