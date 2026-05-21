use std::sync::Arc;

use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::ScopedPath;
use ironclaw_loop_support::{
    FilesystemSkillBundleRoot, FilesystemSkillBundleSource, HostSkillContextSource,
    SkillBundleContextSource,
};
use thiserror::Error;

use crate::{SelectableSkillContextSource, SkillActivationSelectorConfig};

const SYSTEM_SKILLS_ROOT: &str = "/system/skills";
const USER_SKILLS_ROOT: &str = "/skills";
const TENANT_SHARED_SKILLS_ROOT: &str = "/tenant-shared/skills";

/// Explicit scoped read handles granted to the first-party skills extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstPartySkillsExtensionHandles {
    system_skills: Option<ScopedPath>,
    user_skills: Option<ScopedPath>,
    tenant_shared_skills: Option<ScopedPath>,
}

impl FirstPartySkillsExtensionHandles {
    /// Handles for the standard Reborn skill roots.
    pub fn reborn_default() -> Result<Self, FirstPartySkillsExtensionError> {
        Ok(Self {
            system_skills: Some(scoped_root(SYSTEM_SKILLS_ROOT)?),
            user_skills: Some(scoped_root(USER_SKILLS_ROOT)?),
            tenant_shared_skills: Some(scoped_root(TENANT_SHARED_SKILLS_ROOT)?),
        })
    }

    /// Handles for deployments that do not expose tenant-shared skills.
    pub fn without_tenant_shared() -> Result<Self, FirstPartySkillsExtensionError> {
        Ok(Self {
            system_skills: Some(scoped_root(SYSTEM_SKILLS_ROOT)?),
            user_skills: Some(scoped_root(USER_SKILLS_ROOT)?),
            tenant_shared_skills: None,
        })
    }

    /// Builds handles from explicit roots and validates that each handle points
    /// at its Reborn-owned skill namespace.
    pub fn new(
        system_skills: Option<ScopedPath>,
        user_skills: Option<ScopedPath>,
        tenant_shared_skills: Option<ScopedPath>,
    ) -> Result<Self, FirstPartySkillsExtensionError> {
        if let Some(root) = system_skills.as_ref() {
            validate_handle_root("read_system_skills", root, SYSTEM_SKILLS_ROOT)?;
        }
        if let Some(root) = user_skills.as_ref() {
            validate_handle_root("read_user_skills", root, USER_SKILLS_ROOT)?;
        }
        if let Some(root) = tenant_shared_skills.as_ref() {
            validate_handle_root("read_tenant_shared_skills", root, TENANT_SHARED_SKILLS_ROOT)?;
        }
        Ok(Self {
            system_skills,
            user_skills,
            tenant_shared_skills,
        })
    }

    pub fn system_skills(&self) -> Option<&ScopedPath> {
        self.system_skills.as_ref()
    }

    pub fn user_skills(&self) -> Option<&ScopedPath> {
        self.user_skills.as_ref()
    }

    pub fn tenant_shared_skills(&self) -> Option<&ScopedPath> {
        self.tenant_shared_skills.as_ref()
    }

    fn bundle_roots(&self) -> Vec<FilesystemSkillBundleRoot> {
        let mut roots = Vec::new();
        if let Some(root) = &self.system_skills {
            roots.push(FilesystemSkillBundleRoot::system(root.clone()));
        }
        if let Some(root) = &self.tenant_shared_skills {
            roots.push(FilesystemSkillBundleRoot::tenant_shared(root.clone()));
        }
        if let Some(root) = &self.user_skills {
            roots.push(FilesystemSkillBundleRoot::user(root.clone()));
        }
        roots
    }
}

/// First-party in-process skills extension.
///
/// It is userland composition: it receives explicit scoped skill read handles
/// and exports loop-facing skill context sources. It does not expose raw
/// filesystem, database, secrets, network, dispatcher, or tool authority.
#[derive(Clone)]
pub struct FirstPartySkillsExtension<F>
where
    F: RootFilesystem + 'static,
{
    bundle_source: Arc<FilesystemSkillBundleSource<F>>,
    context_source: Arc<SkillBundleContextSource<FilesystemSkillBundleSource<F>>>,
}

impl<F> std::fmt::Debug for FirstPartySkillsExtension<F>
where
    F: RootFilesystem + 'static,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FirstPartySkillsExtension")
            .field("bundle_source", &self.bundle_source)
            .finish_non_exhaustive()
    }
}

impl<F> FirstPartySkillsExtension<F>
where
    F: RootFilesystem + 'static,
{
    pub fn new(
        filesystem: Arc<ScopedFilesystem<F>>,
        handles: FirstPartySkillsExtensionHandles,
    ) -> Self {
        let bundle_source = Arc::new(FilesystemSkillBundleSource::new(
            filesystem,
            handles.bundle_roots(),
        ));
        let context_source = Arc::new(SkillBundleContextSource::new(Arc::clone(&bundle_source)));
        Self {
            bundle_source,
            context_source,
        }
    }

    pub fn bundle_source(&self) -> Arc<FilesystemSkillBundleSource<F>> {
        Arc::clone(&self.bundle_source)
    }

    pub fn context_source(&self) -> Arc<SkillBundleContextSource<FilesystemSkillBundleSource<F>>> {
        Arc::clone(&self.context_source)
    }

    pub fn host_skill_context_source(&self) -> Arc<dyn HostSkillContextSource> {
        self.context_source.clone()
    }

    pub fn selectable_skill_context_source(
        &self,
        config: SkillActivationSelectorConfig,
    ) -> Arc<SelectableSkillContextSource<FilesystemSkillBundleSource<F>>> {
        Arc::new(SelectableSkillContextSource::new(
            Arc::clone(&self.bundle_source),
            config,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FirstPartySkillsExtensionError {
    #[error(
        "invalid first-party skills extension handle {handle}: expected {expected}, got {actual}"
    )]
    InvalidHandle {
        handle: &'static str,
        expected: &'static str,
        actual: String,
    },
    #[error("invalid first-party skills extension root path: {0}")]
    InvalidRootPath(String),
}

fn scoped_root(path: &'static str) -> Result<ScopedPath, FirstPartySkillsExtensionError> {
    ScopedPath::new(path)
        .map_err(|reason| FirstPartySkillsExtensionError::InvalidRootPath(reason.to_string()))
}

fn validate_handle_root(
    handle: &'static str,
    root: &ScopedPath,
    expected: &'static str,
) -> Result<(), FirstPartySkillsExtensionError> {
    if root.as_str() == expected {
        return Ok(());
    }
    Err(FirstPartySkillsExtensionError::InvalidHandle {
        handle,
        expected,
        actual: root.as_str().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::{CasExpectation, Entry, InMemoryBackend, RootFilesystem};
    use ironclaw_host_api::{
        AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId, UserId,
        VirtualPath,
    };
    use ironclaw_loop_support::{SkillBundleSource, build_skill_run_snapshot};
    use ironclaw_skills::SkillTrust;
    use ironclaw_turns::{
        TurnActor, TurnId, TurnRunId, TurnScope,
        run_profile::{
            InMemoryRunProfileResolver, LoopRunContext, RunProfileResolutionRequest,
            RunProfileResolver, SkillTrustLevel, SkillVisibility,
        },
    };

    fn skill_md(name: &str, description: &str, prompt: &str) -> Vec<u8> {
        format!("---\nname: {name}\ndescription: {description}\n---\n{prompt}\n").into_bytes()
    }

    async fn run_context() -> LoopRunContext {
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        LoopRunContext::new(
            TurnScope::new(
                TenantId::new("tenant-a").unwrap(),
                Some(AgentId::new("agent-a").unwrap()),
                Some(ProjectId::new("project-a").unwrap()),
                ironclaw_host_api::ThreadId::new("thread-a").unwrap(),
            ),
            TurnId::new(),
            TurnRunId::new(),
            resolved,
        )
        .with_actor(TurnActor::new(UserId::new("user-a").unwrap()))
    }

    fn scoped_filesystem(root: Arc<InMemoryBackend>) -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let view = MountView::new(vec![
            MountGrant::new(
                MountAlias::new("/system/skills").unwrap(),
                VirtualPath::new("/system/skills").unwrap(),
                MountPermissions::read_only(),
            ),
            MountGrant::new(
                MountAlias::new("/tenant-shared").unwrap(),
                VirtualPath::new("/tenants/tenant-a/shared").unwrap(),
                MountPermissions::read_only(),
            ),
            MountGrant::new(
                MountAlias::new("/skills").unwrap(),
                VirtualPath::new("/tenants/tenant-a/users/user-a/skills").unwrap(),
                MountPermissions::read_only(),
            ),
            MountGrant::new(
                MountAlias::new("/workspace").unwrap(),
                VirtualPath::new("/projects/workspace").unwrap(),
                MountPermissions::read_only(),
            ),
        ])
        .unwrap();
        Arc::new(ScopedFilesystem::with_fixed_view(root, view))
    }

    async fn write_file(root: &InMemoryBackend, path: &str, body: Vec<u8>) {
        let path = VirtualPath::new(path).unwrap();
        root.put(&path, Entry::bytes(body), CasExpectation::Any)
            .await
            .unwrap();
    }

    #[test]
    fn default_handles_are_exact_reborn_skill_roots() {
        let handles = FirstPartySkillsExtensionHandles::reborn_default().unwrap();

        assert_eq!(
            handles.system_skills().map(ScopedPath::as_str),
            Some("/system/skills")
        );
        assert_eq!(
            handles.user_skills().map(ScopedPath::as_str),
            Some("/skills")
        );
        assert_eq!(
            handles.tenant_shared_skills().map(ScopedPath::as_str),
            Some("/tenant-shared/skills")
        );
    }

    #[test]
    fn handles_reject_non_skill_roots() {
        let error = FirstPartySkillsExtensionHandles::new(
            None,
            Some(ScopedPath::new("/workspace").unwrap()),
            None,
        )
        .unwrap_err();

        assert_eq!(
            error,
            FirstPartySkillsExtensionError::InvalidHandle {
                handle: "read_user_skills",
                expected: "/skills",
                actual: "/workspace".to_string()
            }
        );
    }

    #[tokio::test]
    async fn extension_exposes_skill_context_from_only_configured_skill_roots() {
        let root = Arc::new(InMemoryBackend::default());
        write_file(
            &root,
            "/system/skills/system-helper/SKILL.md",
            skill_md(
                "system-helper",
                "system helper description",
                "SYSTEM_HELPER_PROMPT_SENTINEL",
            ),
        )
        .await;
        write_file(
            &root,
            "/tenants/tenant-a/users/user-a/skills/user-helper/SKILL.md",
            skill_md(
                "user-helper",
                "user helper description",
                "USER_HELPER_PROMPT_SENTINEL",
            ),
        )
        .await;
        write_file(
            &root,
            "/projects/workspace/workspace-helper/SKILL.md",
            skill_md(
                "workspace-helper",
                "workspace helper description",
                "WORKSPACE_HELPER_PROMPT_SENTINEL",
            ),
        )
        .await;
        let extension = FirstPartySkillsExtension::new(
            scoped_filesystem(root),
            FirstPartySkillsExtensionHandles::without_tenant_shared().unwrap(),
        );

        let candidates = extension
            .host_skill_context_source()
            .load_skill_context_candidates(&run_context().await)
            .await
            .unwrap();
        let snapshot = build_skill_run_snapshot(candidates).unwrap();
        let entries = &snapshot.entries;

        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| {
            entry.name == "system-helper"
                && entry.trust == SkillTrustLevel::Trusted
                && entry.visibility == SkillVisibility::Visible
                && entry
                    .prompt_content
                    .as_deref()
                    .is_some_and(|content| content.contains("SYSTEM_HELPER_PROMPT_SENTINEL"))
        }));
        assert!(entries.iter().any(|entry| {
            entry.name == "user-helper"
                && entry.trust == SkillTrustLevel::Installed
                && entry.visibility == SkillVisibility::Visible
                && entry.prompt_content.is_none()
        }));
        assert!(!entries.iter().any(|entry| entry.name == "workspace-helper"));
    }

    #[tokio::test]
    async fn extension_bundle_source_reads_only_skill_handles() {
        let root = Arc::new(InMemoryBackend::default());
        write_file(
            &root,
            "/tenants/tenant-a/users/user-a/skills/user-helper/SKILL.md",
            skill_md("user-helper", "user helper description", "prompt"),
        )
        .await;
        let extension = FirstPartySkillsExtension::new(
            scoped_filesystem(root),
            FirstPartySkillsExtensionHandles::without_tenant_shared().unwrap(),
        );

        let descriptors = extension
            .bundle_source()
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap();

        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].id().name(), "user-helper");
        assert_eq!(descriptors[0].trust(), Some(&SkillTrust::Installed));
        assert_eq!(descriptors[0].visibility(), Some(&SkillVisibility::Visible));
    }
}
