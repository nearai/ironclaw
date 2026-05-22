use std::sync::Arc;

use ironclaw_filesystem::RootFilesystem;
use ironclaw_loop_support::{FilesystemSkillBundleSource, HostSkillContextSource};

use crate::{
    SelectableSkillContextSource, SkillActivationSelectorConfig, SkillExecutionAdapter,
    skills::FirstPartySkillsExtension,
};

/// Loaded first-party extension ports exposed to Reborn composition.
///
/// This is not an extension registry. Extension identity, manifests,
/// installation state, and activation lifecycle belong to `ironclaw_extensions`.
/// This type is only the already-composed in-process port set that Reborn
/// runtime assembly is allowed to consume.
#[derive(Debug, Clone)]
pub struct LoadedFirstPartyExtensions<F>
where
    F: RootFilesystem + 'static,
{
    skills: Option<FirstPartySkillsExtension<F>>,
}

impl<F> Default for LoadedFirstPartyExtensions<F>
where
    F: RootFilesystem + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<F> LoadedFirstPartyExtensions<F>
where
    F: RootFilesystem + 'static,
{
    pub fn new() -> Self {
        Self { skills: None }
    }

    pub fn with_skills(mut self, extension: FirstPartySkillsExtension<F>) -> Self {
        self.skills = Some(extension);
        self
    }

    pub fn skills(&self) -> Option<&FirstPartySkillsExtension<F>> {
        self.skills.as_ref()
    }

    pub fn skill_context_source(&self) -> Option<Arc<dyn HostSkillContextSource>> {
        self.skills
            .as_ref()
            .map(FirstPartySkillsExtension::host_skill_context_source)
    }

    pub fn selectable_skill_context_source(
        &self,
        config: SkillActivationSelectorConfig,
    ) -> Option<Arc<SelectableSkillContextSource<FilesystemSkillBundleSource<F>>>> {
        self.skills
            .as_ref()
            .map(|skills| skills.selectable_skill_context_source(config))
    }

    pub fn skill_execution_adapter(
        &self,
    ) -> Option<Arc<SkillExecutionAdapter<FilesystemSkillBundleSource<F>>>> {
        self.skills
            .as_ref()
            .map(FirstPartySkillsExtension::skill_execution_adapter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FirstPartySkillsExtensionHandles, skills::FirstPartySkillsExtension};
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        MountAlias, MountGrant, MountPermissions, MountView, ScopedPath, TenantId, VirtualPath,
    };

    fn filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new("/skills").unwrap(),
            VirtualPath::new("/tenants/tenant-a/users/user-a/skills").unwrap(),
            MountPermissions::read_only(),
        )])
        .unwrap();
        Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::default()),
            view,
        ))
    }

    fn skills_extension() -> FirstPartySkillsExtension<InMemoryBackend> {
        FirstPartySkillsExtension::new(
            filesystem(),
            FirstPartySkillsExtensionHandles::new(
                None,
                Some(ScopedPath::new("/skills").unwrap()),
                None,
            )
            .unwrap(),
            TenantId::new("tenant-a").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn loaded_ports_start_without_extensions() {
        let loaded = LoadedFirstPartyExtensions::<InMemoryBackend>::new();

        assert!(loaded.skills().is_none());
        assert!(loaded.skill_context_source().is_none());
        assert!(loaded.skill_execution_adapter().is_none());
    }

    #[test]
    fn loaded_ports_expose_skills_ports() {
        let loaded = LoadedFirstPartyExtensions::new().with_skills(skills_extension());

        assert!(loaded.skills().is_some());
        assert!(loaded.skill_context_source().is_some());
        assert!(loaded.skill_execution_adapter().is_some());
    }
}
