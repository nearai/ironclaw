use std::{path::PathBuf, sync::Arc};

use ironclaw_filesystem::{DiskFilesystem, FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    HostApiError, HostPath, InvocationId, MountAlias, MountGrant, MountPermissions, MountView,
    ResourceScope, UserId, VirtualPath,
};

use crate::{
    SkillContentRequest, SkillContentResult, SkillInstallRequest, SkillInstallResult,
    SkillInstallSource, SkillManagementContext, SkillManagementError, SkillRemoveRequest,
    SkillRemoveResult, SkillSearchRequest, SkillSearchResult, SkillSummary, SkillUpdateRequest,
    SkillUpdateResult, install_skill, list_skills, read_skill_content, remove_skill, search_skills,
    update_skill,
};

pub type ScopedSkillManagementMountResolver =
    dyn Fn(&ResourceScope) -> Result<MountView, HostApiError> + Send + Sync;

fn scoped_skill_management_mount_view(scope: &ResourceScope) -> Result<MountView, HostApiError> {
    MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/skills")?,
            VirtualPath::new(format!(
                "/projects/tenants/{}/users/{}/skills",
                scope.tenant_id.as_str(),
                scope.user_id.as_str()
            ))?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/system/skills")?,
            VirtualPath::new("/projects/system/skills")?,
            MountPermissions::read_only(),
        ),
    ])
}

#[derive(Clone)]
pub struct ScopedSkillManagementPort {
    owner_user_id: UserId,
    filesystem: Arc<dyn RootFilesystem>,
    mount_resolver: Arc<ScopedSkillManagementMountResolver>,
}

impl ScopedSkillManagementPort {
    pub fn new(
        owner_user_id: UserId,
        filesystem: Arc<dyn RootFilesystem>,
        mounts: MountView,
    ) -> Self {
        let resolver = Arc::new(move |_scope: &ResourceScope| Ok(mounts.clone()));
        Self::new_with_mount_resolver(owner_user_id, filesystem, resolver)
    }

    pub fn new_with_mount_resolver(
        owner_user_id: UserId,
        filesystem: Arc<dyn RootFilesystem>,
        mount_resolver: Arc<ScopedSkillManagementMountResolver>,
    ) -> Self {
        Self {
            owner_user_id,
            filesystem,
            mount_resolver,
        }
    }

    /// The scope->mount-view resolver this port was composed with. Product
    /// capability invokers reuse it so skill-management gestures dispatched
    /// through the product surface resolve the same mounts the agent-loop skill
    /// tools do.
    pub fn mount_resolver(&self) -> Arc<ScopedSkillManagementMountResolver> {
        Arc::clone(&self.mount_resolver)
    }

    pub fn owner_scope(&self) -> Result<ResourceScope, ScopedSkillManagementError> {
        ResourceScope::local_default(self.owner_user_id.clone(), InvocationId::new())
            .map_err(invalid_skill_context)
    }

    fn context_for_scope(
        &self,
        scope: ResourceScope,
    ) -> Result<SkillManagementContext, ScopedSkillManagementError> {
        let mounts = (self.mount_resolver)(&scope).map_err(invalid_skill_context)?;
        Ok(SkillManagementContext::new(
            self.filesystem.clone(),
            mounts,
            scope,
        ))
    }

    pub async fn list_for_scope(
        &self,
        scope: ResourceScope,
    ) -> Result<Vec<SkillSummary>, ScopedSkillManagementError> {
        let context = self.context_for_scope(scope)?;
        Ok(list_skills(&context).await?)
    }

    pub async fn search_for_scope(
        &self,
        scope: ResourceScope,
        query: &str,
        limit: usize,
    ) -> Result<SkillSearchResult, ScopedSkillManagementError> {
        let context = self.context_for_scope(scope)?;
        Ok(search_skills(&context, SkillSearchRequest { query, limit }).await?)
    }

    pub async fn read_content_for_scope(
        &self,
        scope: ResourceScope,
        name: &str,
    ) -> Result<SkillContentResult, ScopedSkillManagementError> {
        let context = self.context_for_scope(scope)?;
        Ok(read_skill_content(&context, SkillContentRequest { name }).await?)
    }

    pub async fn update_for_scope(
        &self,
        scope: ResourceScope,
        name: &str,
        content: &str,
    ) -> Result<SkillUpdateResult, ScopedSkillManagementError> {
        let context = self.context_for_scope(scope)?;
        Ok(update_skill(&context, SkillUpdateRequest { name, content }).await?)
    }

    pub async fn install_for_scope(
        &self,
        scope: ResourceScope,
        name: Option<&str>,
        content: &str,
    ) -> Result<SkillInstallResult, ScopedSkillManagementError> {
        let context = self.context_for_scope(scope)?;
        Ok(install_skill(
            &context,
            SkillInstallRequest {
                name,
                content,
                files: &[],
                source: SkillInstallSource::User,
                source_url: None,
            },
        )
        .await?)
    }

    pub async fn remove_for_scope(
        &self,
        scope: ResourceScope,
        name: &str,
    ) -> Result<SkillRemoveResult, ScopedSkillManagementError> {
        let context = self.context_for_scope(scope)?;
        Ok(remove_skill(&context, SkillRemoveRequest { name }).await?)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScopedSkillManagementError {
    #[error("invalid skill management context: {reason}")]
    InvalidContext { reason: String },
    #[error("skill management failed: {0:?}")]
    Skill(SkillManagementError),
}

impl From<SkillManagementError> for ScopedSkillManagementError {
    fn from(error: SkillManagementError) -> Self {
        Self::Skill(error)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScopedSkillManagementBuildError {
    #[error("invalid skill management configuration: {reason}")]
    InvalidConfig { reason: String },
    #[error("skill management filesystem build failed")]
    Filesystem(#[from] FilesystemError),
    #[error("skill management mount view construction failed")]
    Mount(#[from] HostApiError),
}

pub fn build_scoped_skill_management_port<F>(
    owner_user_id: UserId,
    filesystem: Arc<F>,
) -> Arc<ScopedSkillManagementPort>
where
    F: RootFilesystem + 'static,
{
    let mount_resolver: Arc<ScopedSkillManagementMountResolver> =
        Arc::new(scoped_skill_management_mount_view);
    let filesystem: Arc<dyn RootFilesystem> = filesystem;
    Arc::new(ScopedSkillManagementPort::new_with_mount_resolver(
        owner_user_id,
        filesystem,
        mount_resolver,
    ))
}

pub fn build_existing_standalone_skill_management_port(
    owner_id: impl Into<String>,
    standalone_storage_root: impl Into<PathBuf>,
) -> Result<Option<Arc<ScopedSkillManagementPort>>, ScopedSkillManagementBuildError> {
    let owner_id = owner_id.into();
    let standalone_storage_root = standalone_storage_root.into();
    if !standalone_storage_root.try_exists().map_err(|error| {
        ScopedSkillManagementBuildError::InvalidConfig {
            reason: format!("standalone skill storage root could not be inspected: {error}"),
        }
    })? {
        return Ok(None);
    }
    if !standalone_storage_root.is_dir() {
        return Err(ScopedSkillManagementBuildError::InvalidConfig {
            reason: "standalone skill storage root is not a directory".to_string(),
        });
    }

    let mut filesystem = DiskFilesystem::new();
    filesystem.mount_local(
        VirtualPath::new("/projects")?,
        HostPath::from_path_buf(standalone_storage_root),
    )?;
    let owner_user_id =
        UserId::new(owner_id).map_err(|error| ScopedSkillManagementBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    Ok(Some(build_scoped_skill_management_port(
        owner_user_id,
        Arc::new(filesystem),
    )))
}

fn invalid_skill_context(error: impl std::fmt::Display) -> ScopedSkillManagementError {
    ScopedSkillManagementError::InvalidContext {
        reason: error.to_string(),
    }
}
