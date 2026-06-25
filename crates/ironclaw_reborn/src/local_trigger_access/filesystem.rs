use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::{SecondsFormat, Utc};
use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, Filter, IndexKey, IndexKind, IndexName, IndexSpec,
    IndexValue, Page, RecordKind, RootFilesystem, ScopedFilesystem, VersionedEntry,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, ScopedPath, TenantId, UserId,
};
use serde::{Deserialize, Serialize};

use super::types::{
    LocalTriggerAccessReconciliation, LocalTriggerAccessRole, LocalTriggerAccessSeed,
    LocalTriggerAccessSource, LocalTriggerAccessStatus, LocalTriggerAccessStore,
    RebornLocalTriggerAccessStoreError, backend, optional_scope_key,
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
    role: LocalTriggerAccessRole,
    status: LocalTriggerAccessStatus,
    source: LocalTriggerAccessSource,
    created_at: String,
    updated_at: String,
}

struct FilesystemReconciliationContext<'a> {
    tenant_id: &'a TenantId,
    agent_id: Option<&'a AgentId>,
    project_id: Option<&'a ProjectId>,
    source: LocalTriggerAccessSource,
    allowed: &'a BTreeSet<String>,
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
        let body = serde_json::to_value(record).map_err(backend)?;
        let entry = Entry::record(trigger_access_record_kind()?, &body)
            .map_err(backend)?
            .with_indexed(
                index_key_tenant_id()?,
                IndexValue::Text(record.tenant_id.clone()),
            )
            .with_indexed(
                index_key_user_id()?,
                IndexValue::Text(record.user_id.clone()),
            )
            .with_indexed(
                index_key_agent_id()?,
                IndexValue::Text(optional_scope_key(record.agent_id.as_deref()).to_string()),
            )
            .with_indexed(
                index_key_project_id()?,
                IndexValue::Text(optional_scope_key(record.project_id.as_deref()).to_string()),
            )
            .with_indexed(
                index_key_role()?,
                IndexValue::Text(record.role.as_str().to_string()),
            )
            .with_indexed(
                index_key_status()?,
                IndexValue::Text(record.status.as_str().to_string()),
            )
            .with_indexed(
                index_key_source()?,
                IndexValue::Text(record.source.as_str().to_string()),
            );
        Ok(entry)
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
        context: &FilesystemReconciliationContext<'_>,
        path: &ScopedPath,
        user_id: &UserId,
    ) -> Result<(), RebornLocalTriggerAccessStoreError> {
        let scope = tenant_shared_scope(
            context.tenant_id,
            user_id,
            context.agent_id,
            context.project_id,
        );
        for _ in 0..FILESYSTEM_CAS_RETRIES {
            let Some((mut record, version)) = self.read_record(&scope, path).await? else {
                return Ok(());
            };
            if !record_matches_scope(
                &record,
                context.tenant_id,
                user_id,
                context.agent_id,
                context.project_id,
            ) || record.source != context.source
                || record.status != LocalTriggerAccessStatus::Active
                || context.allowed.contains(record.user_id.as_str())
            {
                return Ok(());
            }
            record.status = LocalTriggerAccessStatus::Inactive;
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

    async fn ensure_reconciliation_indexes(
        &self,
        scope: &ResourceScope,
        users_root: &ScopedPath,
    ) -> Result<(), RebornLocalTriggerAccessStoreError> {
        self.ensure_exact_index(scope, users_root, index_name_source()?, index_key_source()?)
            .await?;
        self.ensure_exact_index(scope, users_root, index_name_status()?, index_key_status()?)
            .await?;
        Ok(())
    }

    async fn ensure_exact_index(
        &self,
        scope: &ResourceScope,
        prefix: &ScopedPath,
        name: IndexName,
        key: IndexKey,
    ) -> Result<(), RebornLocalTriggerAccessStoreError> {
        let spec = IndexSpec::new(name, vec![key], IndexKind::Exact);
        match self.filesystem.ensure_index(scope, prefix, &spec).await {
            Ok(()) => Ok(()),
            Err(FilesystemError::Unsupported { .. }) => Ok(()),
            Err(error) => Err(backend(error)),
        }
    }

    async fn query_active_reconciliation_records(
        &self,
        scope: &ResourceScope,
        users_root: &ScopedPath,
        source: LocalTriggerAccessSource,
    ) -> Result<Vec<FilesystemLocalTriggerAccessRecord>, RebornLocalTriggerAccessStoreError> {
        self.ensure_reconciliation_indexes(scope, users_root)
            .await?;
        let filter = active_reconciliation_filter(source)?;
        let mut records = Vec::new();
        let mut offset = 0;
        loop {
            let page = Page::new(offset, Page::MAX_LIMIT);
            let entries = match self
                .filesystem
                .query(scope, users_root, &filter, page)
                .await
            {
                Ok(entries) => entries,
                Err(error) if is_not_found(&error) => return Ok(records),
                Err(error) => return Err(backend(error)),
            };
            let received = entries.len();
            for entry in entries {
                records.push(deserialize_query_record(entry)?);
            }
            if received < Page::MAX_LIMIT as usize {
                break;
            }
            offset = offset.saturating_add(received as u64);
        }
        Ok(records)
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
            role: seed.role,
            status: LocalTriggerAccessStatus::Active,
            source: seed.source,
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
        let allowed: BTreeSet<String> = reconciliation
            .user_ids
            .iter()
            .map(|user_id| user_id.as_str().to_string())
            .collect();
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
        let context = FilesystemReconciliationContext {
            tenant_id: reconciliation.tenant_id,
            agent_id: reconciliation.agent_id,
            project_id: reconciliation.project_id,
            source: reconciliation.source,
            allowed: &allowed,
        };
        let records = self
            .query_active_reconciliation_records(&scope, &users_root, reconciliation.source)
            .await?;
        for record in records {
            let Ok(user_id) = UserId::new(record.user_id.clone()) else {
                continue;
            };
            let path =
                access_record_path(reconciliation.agent_id, reconciliation.project_id, &user_id)?;
            self.deactivate_stale_record(&context, &path, &user_id)
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
                && record.status == LocalTriggerAccessStatus::Active,
        )
    }
}

#[derive(Debug)]
enum FilesystemAccessPutError {
    VersionMismatch,
    Other(RebornLocalTriggerAccessStoreError),
}

const FILESYSTEM_CAS_RETRIES: usize = 8;
const TRIGGER_ACCESS_ROOT: &str = "/tenant-shared/reborn-trigger-access";
const TRIGGER_ACCESS_RECORD_KIND: &str = "reborn_trigger_access";
const TRIGGER_ACCESS_SOURCE_INDEX_NAME: &str = "reborn_trigger_access_source";
const TRIGGER_ACCESS_STATUS_INDEX_NAME: &str = "reborn_trigger_access_status";
const TENANT_ID_INDEX_KEY: &str = "tenant_id";
const USER_ID_INDEX_KEY: &str = "user_id";
const AGENT_ID_INDEX_KEY: &str = "agent_id";
const PROJECT_ID_INDEX_KEY: &str = "project_id";
const ROLE_INDEX_KEY: &str = "role";
const STATUS_INDEX_KEY: &str = "status";
const SOURCE_INDEX_KEY: &str = "source";

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

fn deserialize_query_record(
    entry: VersionedEntry,
) -> Result<FilesystemLocalTriggerAccessRecord, RebornLocalTriggerAccessStoreError> {
    serde_json::from_slice(&entry.entry.body).map_err(backend)
}

fn active_reconciliation_filter(
    source: LocalTriggerAccessSource,
) -> Result<Filter, RebornLocalTriggerAccessStoreError> {
    Ok(Filter::And(vec![
        Filter::Eq {
            key: index_key_source()?,
            value: IndexValue::Text(source.as_str().to_string()),
        },
        Filter::Eq {
            key: index_key_status()?,
            value: IndexValue::Text(LocalTriggerAccessStatus::Active.as_str().to_string()),
        },
    ]))
}

fn trigger_access_record_kind() -> Result<RecordKind, RebornLocalTriggerAccessStoreError> {
    RecordKind::new(TRIGGER_ACCESS_RECORD_KIND).map_err(backend)
}

fn index_name(value: &'static str) -> Result<IndexName, RebornLocalTriggerAccessStoreError> {
    IndexName::new(value).map_err(backend)
}

fn index_key(value: &'static str) -> Result<IndexKey, RebornLocalTriggerAccessStoreError> {
    IndexKey::new(value).map_err(backend)
}

fn index_name_source() -> Result<IndexName, RebornLocalTriggerAccessStoreError> {
    index_name(TRIGGER_ACCESS_SOURCE_INDEX_NAME)
}

fn index_name_status() -> Result<IndexName, RebornLocalTriggerAccessStoreError> {
    index_name(TRIGGER_ACCESS_STATUS_INDEX_NAME)
}

fn index_key_tenant_id() -> Result<IndexKey, RebornLocalTriggerAccessStoreError> {
    index_key(TENANT_ID_INDEX_KEY)
}

fn index_key_user_id() -> Result<IndexKey, RebornLocalTriggerAccessStoreError> {
    index_key(USER_ID_INDEX_KEY)
}

fn index_key_agent_id() -> Result<IndexKey, RebornLocalTriggerAccessStoreError> {
    index_key(AGENT_ID_INDEX_KEY)
}

fn index_key_project_id() -> Result<IndexKey, RebornLocalTriggerAccessStoreError> {
    index_key(PROJECT_ID_INDEX_KEY)
}

fn index_key_role() -> Result<IndexKey, RebornLocalTriggerAccessStoreError> {
    index_key(ROLE_INDEX_KEY)
}

fn index_key_status() -> Result<IndexKey, RebornLocalTriggerAccessStoreError> {
    index_key(STATUS_INDEX_KEY)
}

fn index_key_source() -> Result<IndexKey, RebornLocalTriggerAccessStoreError> {
    index_key(SOURCE_INDEX_KEY)
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
        let stale_user_id = UserId::new("fs-trigger-stale.json").expect("stale user id");
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

    #[tokio::test]
    async fn filesystem_store_reconcile_skips_invalid_indexed_user_id() {
        let store = store();
        let tenant_id = TenantId::new("fs-trigger-tenant").expect("tenant id");
        let valid_user_id = UserId::new("fs-trigger-user").expect("user id");
        let agent_id = AgentId::new("fs-trigger-agent").expect("agent id");
        let project_id = ProjectId::new("fs-trigger-project").expect("project id");
        let scope = tenant_shared_scope(
            &tenant_id,
            &valid_user_id,
            Some(&agent_id),
            Some(&project_id),
        );
        let users_root =
            access_scope_users_root(Some(&agent_id), Some(&project_id)).expect("access users root");
        let malformed_path = ScopedPath::new(format!("{}/malformed.json", users_root.as_str()))
            .expect("malformed record path");
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let malformed_record = FilesystemLocalTriggerAccessRecord {
            tenant_id: tenant_id.as_str().to_string(),
            user_id: "bad/user".to_string(),
            agent_id: Some(agent_id.as_str().to_string()),
            project_id: Some(project_id.as_str().to_string()),
            role: LocalTriggerAccessRole::Owner,
            status: LocalTriggerAccessStatus::Active,
            source: LocalTriggerAccessSource::LocalDevEnvBootstrap,
            created_at: now.clone(),
            updated_at: now,
        };

        store
            .put_record(
                &scope,
                &malformed_path,
                &malformed_record,
                CasExpectation::Absent,
            )
            .await
            .expect("seed malformed indexed access record");

        store
            .reconcile_local_access(LocalTriggerAccessReconciliation {
                tenant_id: &tenant_id,
                user_ids: std::slice::from_ref(&valid_user_id),
                agent_id: Some(&agent_id),
                project_id: Some(&project_id),
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevEnvBootstrap,
            })
            .await
            .expect("invalid indexed user id should not abort reconciliation");

        assert!(
            store
                .has_active_local_access(
                    &tenant_id,
                    &valid_user_id,
                    Some(&agent_id),
                    Some(&project_id),
                )
                .await
                .expect("check valid local access"),
            "reconciliation still seeds valid users when one indexed record is malformed"
        );
    }
}
