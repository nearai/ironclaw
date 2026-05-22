use std::sync::Arc;

use ironclaw_loop_support::{SkillBundleId, SkillBundleSource};
use ironclaw_turns::run_profile::LoopRunContext;
use ironclaw_turns::{TurnRunId, TurnScope};
use thiserror::Error;

use super::{
    SelectableSkillContextSource, SkillActivationPlan, SkillActivationRequest,
    SkillActivationSelection, SkillActivationSelectionError, SkillBundleAsset,
    SkillBundleAssetReadError, SkillBundleAssetReader,
};

/// Prepares an activated skill for a Reborn loop run.
///
/// This adapter performs deterministic activation selection and grants
/// bundle-relative asset reads for selected skills only. It intentionally does
/// not execute scripts; future script support should go through
/// `ironclaw_processes`.
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

    pub fn capture_next_activation_plan(
        &self,
        scope: TurnScope,
    ) -> Result<(), SkillActivationSelectionError> {
        self.selector.capture_next_activation_plan(scope)
    }

    pub fn cancel_next_activation_plan_capture(
        &self,
        scope: &TurnScope,
    ) -> Result<(), SkillActivationSelectionError> {
        self.selector.cancel_next_activation_plan_capture(scope)
    }

    pub fn clear_scope(&self, scope: &TurnScope) -> Result<(), SkillActivationSelectionError> {
        self.selector.clear_scope(scope)
    }

    pub fn take_activation_plan_for_run(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Option<super::SkillActivationPlan>, SkillActivationSelectionError> {
        self.selector.take_activation_plan_for_run(scope, run_id)
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

    pub async fn read_file_for_activation(
        &self,
        run_context: &LoopRunContext,
        plan: &SkillActivationPlan,
        activation: &SkillActivationRequest,
        path: impl AsRef<str>,
    ) -> Result<SkillBundleAsset, SkillBundleAssetReadError> {
        let asset_reader = SkillBundleAssetReader::new(
            self.selector.bundle_source(),
            plan.activated_bundles().iter().cloned(),
        );
        asset_reader
            .read_file_for_activation(run_context, activation, path)
            .await
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
        errors: HashMap<(SkillSourceKind, String, String), SkillBundleSourceError>,
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
            Self {
                descriptors,
                files,
                errors: HashMap::new(),
            }
        }

        fn with_error(
            mut self,
            source: SkillSourceKind,
            name: &str,
            path: &str,
            error: SkillBundleSourceError,
        ) -> Self {
            self.errors
                .insert((source, name.to_string(), path.to_string()), error);
            self
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
            if let Some(error) = self.errors.get(&(
                bundle_id.source_kind(),
                bundle_id.name().to_string(),
                path.as_str().to_string(),
            )) {
                return Err(error.clone());
            }
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
            bundle_id: Some(SkillBundleId::new(SkillSourceKind::User, "docs").unwrap()),
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

    #[tokio::test]
    async fn asset_reader_rejects_unresolved_activation() {
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
        let unresolved = SkillActivationRequest {
            name: "code-review".to_string(),
            source: None,
            bundle_id: None,
            mode: SkillActivationMode::ExplicitMention,
        };

        let error = plan
            .asset_reader()
            .read_file_for_activation(&context, &unresolved, "references/policy.md")
            .await
            .unwrap_err();

        assert_eq!(
            error,
            SkillBundleAssetReadError::UnresolvedActivation {
                name: "code-review".to_string()
            }
        );
    }

    #[tokio::test]
    async fn asset_reader_maps_active_bundle_source_errors() {
        let source = Arc::new(
            StaticSkillBundleSource::new(vec![StaticSkillSpec {
                source: SkillSourceKind::User,
                name: "code-review",
                skill_md: &skill_md("code-review", "review", "Review code."),
                extra_files: vec![("references/policy.md", "review policy")],
            }])
            .with_error(
                SkillSourceKind::User,
                "code-review",
                "references/denied.md",
                SkillBundleSourceError::PermissionDenied,
            )
            .with_error(
                SkillSourceKind::User,
                "code-review",
                "references/large.md",
                SkillBundleSourceError::ContentTooLarge,
            ),
        );
        let selector = Arc::new(SelectableSkillContextSource::new(
            source,
            SkillActivationSelectorConfig::default(),
        ));
        let adapter = SkillExecutionAdapter::new(selector);
        let context = run_context().await;
        let plan = adapter.prepare(&context, "$code-review").await.unwrap();

        for (path, expected) in [
            ("references/missing.md", SkillBundleAssetReadError::NotFound),
            (
                "references/denied.md",
                SkillBundleAssetReadError::PermissionDenied,
            ),
            (
                "references/large.md",
                SkillBundleAssetReadError::ContentTooLarge,
            ),
        ] {
            let error = plan
                .asset_reader()
                .read_file_for_activation(&context, &plan.selection.activations[0], path)
                .await
                .unwrap_err();

            assert_eq!(error, expected);
        }
    }

    #[tokio::test]
    async fn asset_reader_uses_bundle_id_when_manifest_name_differs() {
        let bundle_id = SkillBundleId::new(SkillSourceKind::User, "bundle-directory").unwrap();
        let source = Arc::new(StaticSkillBundleSource {
            descriptors: vec![SkillBundleDescriptor::new(
                bundle_id.clone(),
                Some(SkillTrust::Trusted),
                Some(SkillVisibility::Visible),
            )],
            files: HashMap::from([
                (
                    (
                        SkillSourceKind::User,
                        "bundle-directory".to_string(),
                        "SKILL.md".to_string(),
                    ),
                    skill_md("manifest-name", "manifest", "Review code.")
                        .as_bytes()
                        .to_vec(),
                ),
                (
                    (
                        SkillSourceKind::User,
                        "bundle-directory".to_string(),
                        "references/policy.md".to_string(),
                    ),
                    b"bundle policy".to_vec(),
                ),
            ]),
            errors: HashMap::new(),
        });
        let selector = Arc::new(SelectableSkillContextSource::new(
            source,
            SkillActivationSelectorConfig::default(),
        ));
        let adapter = SkillExecutionAdapter::new(selector);
        let context = run_context().await;
        let plan = adapter.prepare(&context, "$manifest-name").await.unwrap();

        assert_eq!(plan.selection.activations[0].name, "manifest-name");
        assert_eq!(
            plan.selection.activations[0].bundle_id.as_ref(),
            Some(&bundle_id)
        );
        let asset = plan
            .asset_reader()
            .read_file_for_activation(
                &context,
                &plan.selection.activations[0],
                "references/policy.md",
            )
            .await
            .unwrap();

        assert_eq!(asset.into_utf8().unwrap(), "bundle policy");
    }
}
