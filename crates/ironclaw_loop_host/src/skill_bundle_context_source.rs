use std::sync::Arc;

use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt, stream};

use crate::{
    HostSkillContextBuildError, HostSkillContextCandidate, HostSkillContextSource,
    SkillBundleDescriptor, SkillBundleSource, SkillBundleSourceError,
    sort_skill_bundle_descriptors,
};
use ironclaw_turns::run_profile::{LoopRunContext, SkillVisibility};

const MAX_CONCURRENT_SKILL_BUNDLE_CONTEXT_READS: usize = 8;
const MAX_VISIBLE_SKILL_BUNDLE_CONTEXT_CANDIDATES: usize = 100;

/// Adapts portable skill bundles into model-context candidates.
///
/// This adapter is intentionally policy-thin: it requires host-supplied trust
/// and visibility metadata from [`crate::SkillBundleDescriptor`], prefers safe
/// descriptor discovery metadata for visible bundles, and leaves final snapshot
/// trust/visibility enforcement to [`crate::build_skill_run_snapshot`].
pub struct SkillBundleContextSource<S>
where
    S: SkillBundleSource + ?Sized,
{
    bundle_source: Arc<S>,
}

impl<S> SkillBundleContextSource<S>
where
    S: SkillBundleSource + ?Sized,
{
    /// Creates an adapter over a host-approved skill bundle source.
    pub fn new(bundle_source: Arc<S>) -> Self {
        Self { bundle_source }
    }

    /// Returns the wrapped bundle source.
    pub fn bundle_source(&self) -> &S {
        self.bundle_source.as_ref()
    }
}

impl<S> std::fmt::Debug for SkillBundleContextSource<S>
where
    S: SkillBundleSource + ?Sized,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SkillBundleContextSource")
            .field("bundle_source", &"<SkillBundleSource>")
            .finish()
    }
}

#[async_trait]
impl<S> HostSkillContextSource for SkillBundleContextSource<S>
where
    S: SkillBundleSource + ?Sized,
{
    async fn load_skill_context_candidates(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Vec<HostSkillContextCandidate>, HostSkillContextBuildError> {
        let mut descriptors = self
            .bundle_source
            .list_skill_bundles(run_context)
            .await
            .map_err(skill_bundle_source_error_to_context_error)?;
        sort_skill_bundle_descriptors(&mut descriptors);

        validate_descriptor_policy_metadata(&descriptors)?;
        validate_visible_descriptor_count(&descriptors)?;

        stream::iter(descriptors.into_iter().enumerate())
            .map(|(index, descriptor)| async move {
                self.load_descriptor_context_candidate(index, descriptor)
            })
            .buffered(MAX_CONCURRENT_SKILL_BUNDLE_CONTEXT_READS)
            .try_collect()
            .await
    }
}

impl<S> SkillBundleContextSource<S>
where
    S: SkillBundleSource + ?Sized,
{
    fn load_descriptor_context_candidate(
        &self,
        index: usize,
        descriptor: SkillBundleDescriptor,
    ) -> Result<HostSkillContextCandidate, HostSkillContextBuildError> {
        let trust = descriptor.trust().cloned();
        let visibility = descriptor.visibility().copied();
        let ordering_key = descriptor_context_ordering_key(index);

        if visibility != Some(SkillVisibility::Visible) {
            // Preserve host policy metadata on unavailable candidates so downstream
            // snapshot construction keeps one fail-closed validation path.
            return Ok(HostSkillContextCandidate::unavailable(trust, visibility)
                .with_ordering_key(ordering_key));
        }

        Ok(HostSkillContextCandidate::discoverable(
            descriptor.id().name(),
            descriptor.description(),
            trust,
            visibility,
        )
        .with_ordering_key(ordering_key))
    }
}

fn skill_bundle_source_error_to_context_error(
    error: SkillBundleSourceError,
) -> HostSkillContextBuildError {
    tracing::warn!(
        component = "skill_bundle_context_source",
        operation = "map_source_error",
        error = %error,
        error_debug = ?error,
        "skill bundle source error mapped to safe skill context error"
    );
    // Collapse bundle-source internals into the public-safe context error taxonomy:
    // unavailable, parse/policy failure, budget exhaustion, or internal bug.
    match error {
        SkillBundleSourceError::SourceUnavailable
        | SkillBundleSourceError::BundleNotFound
        | SkillBundleSourceError::FileNotFound
        | SkillBundleSourceError::PermissionDenied => HostSkillContextBuildError::SourceUnavailable,
        SkillBundleSourceError::InvalidBundleId
        | SkillBundleSourceError::InvalidFilePath
        | SkillBundleSourceError::InvalidSkillBundle
        | SkillBundleSourceError::BundleUtf8DecodeFailed
        | SkillBundleSourceError::ManifestParseFailed => HostSkillContextBuildError::ParseFailed,
        SkillBundleSourceError::ContentTooLarge
        | SkillBundleSourceError::BundleScanLimitExceeded => {
            HostSkillContextBuildError::ContextBudgetExceeded
        }
        SkillBundleSourceError::DuplicateSourceKind | SkillBundleSourceError::Internal => {
            HostSkillContextBuildError::Internal
        }
    }
}

fn validate_descriptor_policy_metadata(
    descriptors: &[SkillBundleDescriptor],
) -> Result<(), HostSkillContextBuildError> {
    for descriptor in descriptors {
        if descriptor.trust().is_none() {
            return Err(HostSkillContextBuildError::TrustDataMissing);
        }
        if descriptor.visibility().is_none() {
            return Err(HostSkillContextBuildError::VisibilityDataMissing);
        }
    }
    Ok(())
}

fn validate_visible_descriptor_count(
    descriptors: &[SkillBundleDescriptor],
) -> Result<(), HostSkillContextBuildError> {
    let visible_count = descriptors
        .iter()
        .filter(|descriptor| descriptor.visibility().copied() == Some(SkillVisibility::Visible))
        .count();

    if visible_count > MAX_VISIBLE_SKILL_BUNDLE_CONTEXT_CANDIDATES {
        return Err(HostSkillContextBuildError::ContextBudgetExceeded);
    }

    Ok(())
}

fn descriptor_context_ordering_key(index: usize) -> String {
    format!("{index:016}")
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use ironclaw_skills::SkillTrust;
    use ironclaw_turns::{
        RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId, TurnScope,
        run_profile::InMemoryRunProfileResolver,
    };

    use super::*;
    use crate::{
        SkillBundleDescriptor, SkillFilePath, skill_context::build_skill_instruction_snippets,
    };
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};

    async fn run_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-a").unwrap(),
            Some(AgentId::new("agent-a").unwrap()),
            Some(ProjectId::new("project-a").unwrap()),
            ThreadId::new("thread-a").unwrap(),
        );
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved)
    }

    fn descriptor(
        source_kind: crate::SkillSourceKind,
        name: &str,
        trust: Option<SkillTrust>,
        visibility: Option<SkillVisibility>,
    ) -> SkillBundleDescriptor {
        SkillBundleDescriptor::new(
            crate::SkillBundleId::new(source_kind, name).unwrap(),
            trust,
            visibility,
            format!("{name} description"),
        )
    }

    #[derive(Default)]
    struct StaticSkillBundleSource {
        descriptors: Vec<SkillBundleDescriptor>,
        files: Mutex<HashMap<String, Vec<u8>>>,
        list_error: Option<SkillBundleSourceError>,
        read_errors: Mutex<HashMap<String, SkillBundleSourceError>>,
        reads: Mutex<Vec<String>>,
    }

    impl StaticSkillBundleSource {
        fn new(descriptors: Vec<SkillBundleDescriptor>) -> Self {
            Self {
                descriptors,
                files: Mutex::new(HashMap::new()),
                list_error: None,
                read_errors: Mutex::new(HashMap::new()),
                reads: Mutex::new(Vec::new()),
            }
        }

        fn with_list_error(mut self, error: SkillBundleSourceError) -> Self {
            self.list_error = Some(error);
            self
        }

        fn reads(&self) -> Vec<String> {
            self.reads.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl SkillBundleSource for StaticSkillBundleSource {
        async fn list_skill_bundles(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Vec<SkillBundleDescriptor>, SkillBundleSourceError> {
            if let Some(error) = &self.list_error {
                return Err(error.clone());
            }
            Ok(self.descriptors.clone())
        }

        async fn read_skill_bundle_file(
            &self,
            _run_context: &LoopRunContext,
            bundle_id: &crate::SkillBundleId,
            path: &crate::SkillFilePath,
        ) -> Result<Vec<u8>, SkillBundleSourceError> {
            let key = format!("{bundle_id}:{path}");
            self.reads.lock().unwrap().push(key.clone());
            if let Some(error) = self.read_errors.lock().unwrap().get(&key) {
                return Err(error.clone());
            }
            self.files
                .lock()
                .unwrap()
                .get(&key)
                .cloned()
                .ok_or(SkillBundleSourceError::FileNotFound)
        }
    }

    #[tokio::test]
    async fn adapter_handles_empty_bundle_list() {
        let source = Arc::new(StaticSkillBundleSource::new(Vec::new()));
        let adapter = SkillBundleContextSource::new(source);

        let candidates = adapter
            .load_skill_context_candidates(&run_context().await)
            .await
            .unwrap();

        assert!(candidates.is_empty());
    }

    #[tokio::test]
    async fn adapter_rejects_too_many_visible_bundles_without_reads() {
        let descriptors = (0..=MAX_VISIBLE_SKILL_BUNDLE_CONTEXT_CANDIDATES)
            .map(|index| {
                descriptor(
                    crate::SkillSourceKind::User,
                    &format!("skill-{index:03}"),
                    Some(SkillTrust::Trusted),
                    Some(SkillVisibility::Visible),
                )
            })
            .collect::<Vec<_>>();
        let source = Arc::new(StaticSkillBundleSource::new(descriptors));
        let adapter = SkillBundleContextSource::new(Arc::clone(&source));

        let error = adapter
            .load_skill_context_candidates(&run_context().await)
            .await
            .unwrap_err();

        assert_eq!(error, HostSkillContextBuildError::ContextBudgetExceeded);
        assert!(source.reads().is_empty());
    }

    #[tokio::test]
    async fn adapter_maps_budget_source_errors_from_bundle_listing() {
        for source_error in [
            SkillBundleSourceError::ContentTooLarge,
            SkillBundleSourceError::BundleScanLimitExceeded,
        ] {
            let source =
                Arc::new(StaticSkillBundleSource::default().with_list_error(source_error.clone()));
            let adapter = SkillBundleContextSource::new(Arc::clone(&source));

            let error = adapter
                .load_skill_context_candidates(&run_context().await)
                .await
                .unwrap_err();

            assert_eq!(
                error,
                HostSkillContextBuildError::ContextBudgetExceeded,
                "{source_error:?} must map to budget exceeded"
            );
            assert!(source.reads().is_empty());
        }
    }

    #[tokio::test]
    async fn adapter_reads_visible_trusted_bundle_as_discoverable_model_snippet() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![
            descriptor(
                crate::SkillSourceKind::System,
                "alpha",
                Some(SkillTrust::Trusted),
                Some(SkillVisibility::Visible),
            )
            .with_description("safe alpha description"),
        ]));
        let adapter = SkillBundleContextSource::new(Arc::clone(&source));

        let snippets = build_skill_instruction_snippets(&adapter, &run_context().await)
            .await
            .unwrap();

        assert!(source.reads().is_empty());
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].snippet_ref, "skill:alpha");
        assert!(snippets[0].safe_summary.contains("safe alpha description"));
        assert!(!snippets[0].safe_summary.contains("trusted alpha prompt"));
        assert!(snippets[0].model_content.contains("safe alpha description"));
        assert!(!snippets[0].model_content.contains("trusted alpha prompt"));
    }

    #[tokio::test]
    async fn adapter_keeps_installed_bundle_prompt_out_of_model_snippet() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![
            descriptor(
                crate::SkillSourceKind::User,
                "alpha",
                Some(SkillTrust::Installed),
                Some(SkillVisibility::Visible),
            )
            .with_description("safe installed description"),
        ]));
        let adapter = SkillBundleContextSource::new(Arc::clone(&source));

        let snippets = build_skill_instruction_snippets(&adapter, &run_context().await)
            .await
            .unwrap();

        assert_eq!(snippets.len(), 1);
        assert!(
            snippets[0]
                .safe_summary
                .contains("safe installed description")
        );
        assert!(
            !snippets[0]
                .model_content
                .contains("RAW_INSTALLED_PROMPT_SENTINEL")
        );
        assert!(source.reads().is_empty());
    }

    #[tokio::test]
    async fn adapter_does_not_read_hidden_or_denied_bundles() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![
            descriptor(
                crate::SkillSourceKind::System,
                "hidden",
                Some(SkillTrust::Trusted),
                Some(SkillVisibility::Hidden),
            ),
            descriptor(
                crate::SkillSourceKind::User,
                "denied",
                Some(SkillTrust::Installed),
                Some(SkillVisibility::Denied),
            ),
        ]));
        let adapter = SkillBundleContextSource::new(Arc::clone(&source));

        let snippets = build_skill_instruction_snippets(&adapter, &run_context().await)
            .await
            .unwrap();

        assert!(snippets.is_empty());
        assert!(source.reads().is_empty());
    }

    #[tokio::test]
    async fn adapter_fails_closed_when_policy_metadata_is_missing_without_reads() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![descriptor(
            crate::SkillSourceKind::User,
            "alpha",
            None,
            Some(SkillVisibility::Visible),
        )]));
        let adapter = SkillBundleContextSource::new(Arc::clone(&source));

        let error = adapter
            .load_skill_context_candidates(&run_context().await)
            .await
            .unwrap_err();

        assert_eq!(error, HostSkillContextBuildError::TrustDataMissing);
        assert!(source.reads().is_empty());
    }

    #[tokio::test]
    async fn adapter_fails_closed_when_visibility_metadata_is_missing_without_reads() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![descriptor(
            crate::SkillSourceKind::User,
            "alpha",
            Some(SkillTrust::Trusted),
            None,
        )]));
        let adapter = SkillBundleContextSource::new(Arc::clone(&source));

        let error = adapter
            .load_skill_context_candidates(&run_context().await)
            .await
            .unwrap_err();

        assert_eq!(error, HostSkillContextBuildError::VisibilityDataMissing);
        assert!(source.reads().is_empty());
    }

    #[tokio::test]
    async fn adapter_validates_all_policy_metadata_before_reading_visible_bundles() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![
            descriptor(
                crate::SkillSourceKind::System,
                "alpha",
                Some(SkillTrust::Trusted),
                Some(SkillVisibility::Visible),
            )
            .with_description("alpha description"),
            descriptor(
                crate::SkillSourceKind::User,
                "bravo",
                Some(SkillTrust::Trusted),
                None,
            ),
        ]));
        let adapter = SkillBundleContextSource::new(Arc::clone(&source));

        let error = adapter
            .load_skill_context_candidates(&run_context().await)
            .await
            .unwrap_err();

        assert_eq!(error, HostSkillContextBuildError::VisibilityDataMissing);
        assert!(source.reads().is_empty());
    }

    #[tokio::test]
    async fn adapter_sorts_candidates_by_bundle_descriptor_ordering_key() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![
            descriptor(
                crate::SkillSourceKind::User,
                "bravo",
                Some(SkillTrust::Trusted),
                Some(SkillVisibility::Visible),
            )
            .with_description("bravo description"),
            descriptor(
                crate::SkillSourceKind::System,
                "alpha",
                Some(SkillTrust::Trusted),
                Some(SkillVisibility::Visible),
            )
            .with_description("alpha description"),
        ]));
        let adapter = SkillBundleContextSource::new(source);

        let candidates = adapter
            .load_skill_context_candidates(&run_context().await)
            .await
            .unwrap();

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.ordering_key.as_deref().unwrap())
                .collect::<Vec<_>>(),
            vec!["0000000000000000", "0000000000000001"]
        );
    }

    #[tokio::test]
    async fn adapter_preserves_descriptor_path_in_candidate_order() {
        let nested_descriptor = descriptor(
            crate::SkillSourceKind::User,
            "alpha",
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        )
        .with_skill_md_path(SkillFilePath::new("nested/SKILL.md").unwrap())
        .with_description("nested description");
        let source = Arc::new(StaticSkillBundleSource::new(vec![
            nested_descriptor,
            descriptor(
                crate::SkillSourceKind::User,
                "alpha",
                Some(SkillTrust::Trusted),
                Some(SkillVisibility::Visible),
            )
            .with_description("root description"),
        ]));
        let adapter = SkillBundleContextSource::new(source);

        let candidates = adapter
            .load_skill_context_candidates(&run_context().await)
            .await
            .unwrap();
        let ordering_keys = candidates
            .iter()
            .map(|candidate| candidate.ordering_key.as_deref().unwrap())
            .collect::<Vec<_>>();
        let descriptions = candidates
            .iter()
            .map(|candidate| {
                let (_, description) = candidate.discoverable_metadata().unwrap();
                description
            })
            .collect::<Vec<_>>();

        assert_eq!(ordering_keys, vec!["0000000000000000", "0000000000000001"]);
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.loaded_skill_md().is_none())
        );
        assert_eq!(descriptions, vec!["root description", "nested description"]);
    }

    #[tokio::test]
    async fn adapter_preserves_hidden_and_denied_candidates_as_unavailable() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![
            descriptor(
                crate::SkillSourceKind::System,
                "hidden",
                Some(SkillTrust::Trusted),
                Some(SkillVisibility::Hidden),
            ),
            descriptor(
                crate::SkillSourceKind::User,
                "denied",
                Some(SkillTrust::Installed),
                Some(SkillVisibility::Denied),
            ),
        ]));
        let adapter = SkillBundleContextSource::new(Arc::clone(&source));

        let candidates = adapter
            .load_skill_context_candidates(&run_context().await)
            .await
            .unwrap();

        assert_eq!(candidates.len(), 2);
        assert!(
            candidates
                .iter()
                .all(HostSkillContextCandidate::is_unavailable)
        );
        assert_eq!(candidates[0].trust, Some(SkillTrust::Trusted));
        assert_eq!(candidates[0].visibility, Some(SkillVisibility::Hidden));
        assert_eq!(
            candidates[0].ordering_key.as_deref(),
            Some("0000000000000000")
        );
        assert_eq!(candidates[1].trust, Some(SkillTrust::Installed));
        assert_eq!(candidates[1].visibility, Some(SkillVisibility::Denied));
        assert_eq!(
            candidates[1].ordering_key.as_deref(),
            Some("0000000000000001")
        );
        assert!(source.reads().is_empty());
    }

    #[tokio::test]
    async fn adapter_maps_list_source_errors() {
        let source = Arc::new(
            StaticSkillBundleSource::new(Vec::new())
                .with_list_error(SkillBundleSourceError::SourceUnavailable),
        );
        let adapter = SkillBundleContextSource::new(source);

        let error = adapter
            .load_skill_context_candidates(&run_context().await)
            .await
            .unwrap_err();

        assert_eq!(error, HostSkillContextBuildError::SourceUnavailable);
    }

    #[tokio::test]
    async fn adapter_maps_bundle_scan_limit_to_budget_error() {
        let source = Arc::new(
            StaticSkillBundleSource::new(Vec::new())
                .with_list_error(SkillBundleSourceError::BundleScanLimitExceeded),
        );
        let adapter = SkillBundleContextSource::new(source);

        let error = adapter
            .load_skill_context_candidates(&run_context().await)
            .await
            .unwrap_err();

        assert_eq!(error, HostSkillContextBuildError::ContextBudgetExceeded);
    }

    #[tokio::test]
    async fn adapter_maps_duplicate_source_kind_to_internal_error() {
        let source = Arc::new(
            StaticSkillBundleSource::new(Vec::new())
                .with_list_error(SkillBundleSourceError::DuplicateSourceKind),
        );
        let adapter = SkillBundleContextSource::new(source);

        let error = adapter
            .load_skill_context_candidates(&run_context().await)
            .await
            .unwrap_err();

        assert_eq!(error, HostSkillContextBuildError::Internal);
    }

    #[tokio::test]
    async fn adapter_maps_parse_source_errors() {
        for source_error in [
            SkillBundleSourceError::InvalidBundleId,
            SkillBundleSourceError::InvalidFilePath,
            SkillBundleSourceError::InvalidSkillBundle,
            SkillBundleSourceError::BundleUtf8DecodeFailed,
            SkillBundleSourceError::ManifestParseFailed,
        ] {
            let source =
                Arc::new(StaticSkillBundleSource::new(Vec::new()).with_list_error(source_error));
            let adapter = SkillBundleContextSource::new(source);

            let error = adapter
                .load_skill_context_candidates(&run_context().await)
                .await
                .unwrap_err();

            assert_eq!(error, HostSkillContextBuildError::ParseFailed);
        }
    }
}
