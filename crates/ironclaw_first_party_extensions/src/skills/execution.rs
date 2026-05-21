use std::sync::Arc;

use ironclaw_loop_support::{SkillBundleId, SkillBundleSource};
use ironclaw_turns::run_profile::LoopRunContext;
use thiserror::Error;

use super::{
    SelectableSkillContextSource, SkillActivationSelection, SkillActivationSelectionError,
    SkillBundleAssetReadError, SkillBundleAssetReader,
};

/// Prepares an activated skill for a Reborn loop run.
///
/// This adapter performs deterministic activation selection and grants
/// bundle-relative asset reads for selected skills only. It intentionally does
/// not execute scripts; future script support should go through
/// `ironclaw_process`.
#[derive(Debug)]
pub struct SkillExecutionAdapter<S>
where
    S: SkillBundleSource + ?Sized,
{
    selector: Arc<SelectableSkillContextSource<S>>,
}

impl<S> SkillExecutionAdapter<S>
where
    S: SkillBundleSource + ?Sized,
{
    pub fn new(selector: Arc<SelectableSkillContextSource<S>>) -> Self {
        Self { selector }
    }

    pub fn selector(&self) -> &Arc<SelectableSkillContextSource<S>> {
        &self.selector
    }

    pub async fn prepare(
        &self,
        run_context: &LoopRunContext,
        message: &str,
    ) -> Result<SkillExecutionPlan<S>, SkillExecutionAdapterError> {
        let activation_plan = self
            .selector
            .select_activation_plan(run_context, message)
            .await
            .map_err(SkillExecutionAdapterError::Activation)?;
        let asset_reader = SkillBundleAssetReader::new(
            self.selector.bundle_source(),
            activation_plan.activated_bundles().iter().cloned(),
        );
        let activated_bundles = activation_plan.activated_bundles().to_vec();
        Ok(SkillExecutionPlan {
            selection: activation_plan.selection,
            activated_bundles,
            asset_reader,
        })
    }
}

/// Prepared skill execution inputs for a Reborn loop run.
#[derive(Debug, Clone)]
pub struct SkillExecutionPlan<S>
where
    S: SkillBundleSource + ?Sized,
{
    pub selection: SkillActivationSelection,
    activated_bundles: Vec<SkillBundleId>,
    asset_reader: SkillBundleAssetReader<S>,
}

impl<S> SkillExecutionPlan<S>
where
    S: SkillBundleSource + ?Sized,
{
    pub fn activated_bundles(&self) -> &[SkillBundleId] {
        &self.activated_bundles
    }

    pub fn asset_reader(&self) -> &SkillBundleAssetReader<S> {
        &self.asset_reader
    }
}

#[derive(Debug, Error)]
pub enum SkillExecutionAdapterError {
    #[error("skill activation failed: {0}")]
    Activation(#[from] SkillActivationSelectionError),
    #[error("skill bundle asset read failed: {0}")]
    Asset(#[from] SkillBundleAssetReadError),
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use ironclaw_loop_support::{
        SkillBundleDescriptor, SkillBundleId, SkillBundleSource, SkillBundleSourceError,
        SkillFilePath, SkillSourceKind,
    };
    use ironclaw_skills::SkillTrust;
    use ironclaw_turns::{
        RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId, TurnScope,
        run_profile::{InMemoryRunProfileResolver, SkillVisibility},
    };

    use super::super::{
        SkillActivationMode, SkillActivationRequest, SkillActivationSelectorConfig,
    };
    use super::*;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};

    struct StaticSkillBundleSource {
        descriptors: Vec<SkillBundleDescriptor>,
        files: HashMap<(SkillSourceKind, String, String), Vec<u8>>,
    }

    struct StaticSkillSpec<'a> {
        source: SkillSourceKind,
        name: &'a str,
        skill_md: &'a str,
        extra_files: Vec<(&'a str, &'a str)>,
    }

    impl StaticSkillBundleSource {
        fn new(skills: Vec<StaticSkillSpec<'_>>) -> Self {
            let mut descriptors = Vec::new();
            let mut files = HashMap::new();
            for skill in skills {
                let source = skill.source;
                let name = skill.name;
                let id = SkillBundleId::new(source, name).unwrap();
                descriptors.push(SkillBundleDescriptor::new(
                    id,
                    Some(SkillTrust::Trusted),
                    Some(SkillVisibility::Visible),
                ));
                files.insert(
                    (source, name.to_string(), "SKILL.md".to_string()),
                    skill.skill_md.as_bytes().to_vec(),
                );
                for (path, content) in skill.extra_files {
                    files.insert(
                        (source, name.to_string(), path.to_string()),
                        content.as_bytes().to_vec(),
                    );
                }
            }
            Self { descriptors, files }
        }
    }

    #[async_trait]
    impl SkillBundleSource for StaticSkillBundleSource {
        async fn list_skill_bundles(
            &self,
            _run_context: &ironclaw_turns::run_profile::LoopRunContext,
        ) -> Result<Vec<SkillBundleDescriptor>, SkillBundleSourceError> {
            Ok(self.descriptors.clone())
        }

        async fn read_skill_bundle_file(
            &self,
            _run_context: &ironclaw_turns::run_profile::LoopRunContext,
            bundle_id: &SkillBundleId,
            path: &SkillFilePath,
        ) -> Result<Vec<u8>, SkillBundleSourceError> {
            self.files
                .get(&(
                    bundle_id.source_kind(),
                    bundle_id.name().to_string(),
                    path.as_str().to_string(),
                ))
                .cloned()
                .ok_or(SkillBundleSourceError::FileNotFound)
        }
    }

    async fn run_context() -> ironclaw_turns::run_profile::LoopRunContext {
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        ironclaw_turns::run_profile::LoopRunContext::new(
            TurnScope::new(
                TenantId::new("tenant-a").unwrap(),
                Some(AgentId::new("agent-a").unwrap()),
                Some(ProjectId::new("project-a").unwrap()),
                ThreadId::new("thread-a").unwrap(),
            ),
            TurnId::new(),
            TurnRunId::new(),
            resolved,
        )
    }

    fn skill_md(name: &str, keyword: &str, body: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: {name}\nactivation:\n  keywords: [\"{keyword}\"]\n---\n{body}\n"
        )
    }

    #[tokio::test]
    async fn prepare_grants_asset_reads_only_for_activated_bundles() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![
            StaticSkillSpec {
                source: SkillSourceKind::User,
                name: "code-review",
                skill_md: &skill_md("code-review", "review", "Review code."),
                extra_files: vec![("references/policy.md", "review policy")],
            },
            StaticSkillSpec {
                source: SkillSourceKind::User,
                name: "docs",
                skill_md: &skill_md("docs", "docs", "Write docs."),
                extra_files: vec![("references/style.md", "docs style")],
            },
        ]));
        let selector = Arc::new(SelectableSkillContextSource::new(
            source,
            SkillActivationSelectorConfig::default(),
        ));
        let adapter = SkillExecutionAdapter::new(selector);
        let context = run_context().await;

        let plan = adapter
            .prepare(&context, "please review this PR")
            .await
            .unwrap();

        assert_eq!(plan.selection.activations.len(), 1);
        assert_eq!(plan.selection.activations[0].name, "code-review");
        let asset = plan
            .asset_reader()
            .read_file_for_activation(
                &context,
                &plan.selection.activations[0],
                "references/policy.md",
            )
            .await
            .unwrap();
        assert_eq!(asset.into_utf8().unwrap(), "review policy");

        let inactive = SkillActivationRequest {
            name: "docs".to_string(),
            source: Some(SkillSourceKind::User),
            mode: SkillActivationMode::ExplicitMention,
        };
        let error = plan
            .asset_reader()
            .read_file_for_activation(&context, &inactive, "references/style.md")
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            SkillBundleAssetReadError::InactiveSkill { .. }
        ));
    }

    #[tokio::test]
    async fn asset_reader_rejects_invalid_relative_paths() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![StaticSkillSpec {
            source: SkillSourceKind::User,
            name: "code-review",
            skill_md: &skill_md("code-review", "review", "Review code."),
            extra_files: vec![("references/policy.md", "review policy")],
        }]));
        let selector = Arc::new(SelectableSkillContextSource::new(
            source,
            SkillActivationSelectorConfig::default(),
        ));
        let adapter = SkillExecutionAdapter::new(selector);
        let context = run_context().await;
        let plan = adapter.prepare(&context, "$code-review").await.unwrap();

        let error = plan
            .asset_reader()
            .read_file_for_activation(&context, &plan.selection.activations[0], "../secret")
            .await
            .unwrap_err();

        assert_eq!(error, SkillBundleAssetReadError::InvalidPath);
    }
}
