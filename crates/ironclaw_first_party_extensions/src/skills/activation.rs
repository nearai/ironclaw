use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use futures::future::try_join_all;
use ironclaw_loop_support::{
    HostSkillContextBuildError, HostSkillContextCandidate, HostSkillContextSource,
    SkillBundleDescriptor, SkillBundleId, SkillBundleSource, SkillBundleSourceError,
    SkillSourceKind, sort_skill_bundle_descriptors,
};
use ironclaw_skills::{
    LoadedSkill, SkillSource, extract_skill_mentions, parse_skill_md, prefilter_skills,
};
use ironclaw_turns::run_profile::{LoopRunContext, SkillVisibility};
use ironclaw_turns::{TurnRunId, TurnScope};
use thiserror::Error;

/// Maximum number of first-party skills selected for one turn by default.
pub const DEFAULT_MAX_ACTIVE_SKILLS: usize = 4;

/// Maximum estimated skill prompt tokens selected for one turn by default.
pub const DEFAULT_MAX_SKILL_CONTEXT_TOKENS: usize = 4000;

/// Typed request produced by first-party skill activation selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillActivationRequest {
    pub name: String,
    pub source: Option<SkillSourceKind>,
    pub bundle_id: Option<SkillBundleId>,
    pub mode: SkillActivationMode,
}

impl SkillActivationRequest {
    fn resolved(
        name: impl Into<String>,
        bundle_id: SkillBundleId,
        mode: SkillActivationMode,
    ) -> Self {
        Self {
            name: name.into(),
            source: Some(bundle_id.source_kind()),
            bundle_id: Some(bundle_id),
            mode,
        }
    }
}

/// Why a skill activation request was selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillActivationMode {
    ExplicitMention,
    ActivationCriteria,
}

/// Selector limits for conversation-driven first-party skill activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillActivationSelectorConfig {
    pub max_active_skills: usize,
    pub max_context_tokens: usize,
}

impl Default for SkillActivationSelectorConfig {
    fn default() -> Self {
        Self {
            max_active_skills: DEFAULT_MAX_ACTIVE_SKILLS,
            max_context_tokens: DEFAULT_MAX_SKILL_CONTEXT_TOKENS,
        }
    }
}

/// Result of selecting skill activations from one user message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillActivationSelection {
    pub activations: Vec<SkillActivationRequest>,
    pub rewritten_message: String,
    pub feedback: Vec<String>,
}

/// Fully resolved activation output for one user message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillActivationPlan {
    pub selection: SkillActivationSelection,
    activated_bundles: Vec<SkillBundleId>,
}

impl SkillActivationPlan {
    pub fn new(selection: SkillActivationSelection, activated_bundles: Vec<SkillBundleId>) -> Self {
        Self {
            selection,
            activated_bundles,
        }
    }

    pub fn activated_bundles(&self) -> &[SkillBundleId] {
        &self.activated_bundles
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SkillActivationSelectionError {
    #[error("ambiguous skill activation for '{name}': {sources:?}")]
    AmbiguousSkill {
        name: String,
        sources: Vec<SkillSourceKind>,
    },
    #[error("skill activation source unavailable")]
    SourceUnavailable,
    #[error("skill activation parse failed")]
    ParseFailed,
    #[error("skill activation trust data missing")]
    TrustDataMissing,
    #[error("skill activation visibility data missing")]
    VisibilityDataMissing,
    #[error("skill activation context budget exceeded")]
    ContextBudgetExceeded,
    #[error("skill activation internal error")]
    Internal,
}

impl SkillActivationSelectionError {
    fn into_context_error(self) -> HostSkillContextBuildError {
        match self {
            Self::SourceUnavailable => HostSkillContextBuildError::SourceUnavailable,
            Self::ParseFailed | Self::AmbiguousSkill { .. } => {
                HostSkillContextBuildError::ParseFailed
            }
            Self::TrustDataMissing => HostSkillContextBuildError::TrustDataMissing,
            Self::VisibilityDataMissing => HostSkillContextBuildError::VisibilityDataMissing,
            Self::ContextBudgetExceeded => HostSkillContextBuildError::ContextBudgetExceeded,
            Self::Internal => HostSkillContextBuildError::Internal,
        }
    }
}

/// Host skill context source that activates only conversation-selected skills.
///
/// Reborn composition records the current user message for a turn scope before
/// submitting the turn. When the loop builds model context, this source lists
/// visible bundles for the real run context, applies v1-style deterministic
/// activation, and returns candidates only for selected skills.
#[derive(Debug)]
pub struct SelectableSkillContextSource<S>
where
    S: SkillBundleSource + ?Sized,
{
    bundle_source: Arc<S>,
    config: SkillActivationSelectorConfig,
    messages_by_scope: Mutex<HashMap<TurnScope, String>>,
    plan_capture_counts_by_scope: Mutex<HashMap<TurnScope, usize>>,
    plans_by_run: Mutex<HashMap<(TurnScope, TurnRunId), SkillActivationPlan>>,
}

impl<S> SelectableSkillContextSource<S>
where
    S: SkillBundleSource + ?Sized,
{
    pub fn new(bundle_source: Arc<S>, config: SkillActivationSelectorConfig) -> Self {
        Self {
            bundle_source,
            config,
            messages_by_scope: Mutex::new(HashMap::new()),
            plan_capture_counts_by_scope: Mutex::new(HashMap::new()),
            plans_by_run: Mutex::new(HashMap::new()),
        }
    }

    pub fn record_user_message(&self, scope: TurnScope, message: impl Into<String>) {
        lock_or_recover(&self.messages_by_scope, "skill activation message")
            .insert(scope, message.into());
    }

    pub fn clear_scope(&self, scope: &TurnScope) {
        lock_or_recover(&self.messages_by_scope, "skill activation message").remove(scope);
        lock_or_recover(
            &self.plan_capture_counts_by_scope,
            "skill activation plan capture",
        )
        .remove(scope);
        lock_or_recover(&self.plans_by_run, "skill activation plans")
            .retain(|(plan_scope, _), _| plan_scope != scope);
    }

    pub fn bundle_source(&self) -> Arc<S> {
        Arc::clone(&self.bundle_source)
    }

    pub fn capture_next_activation_plan(&self, scope: TurnScope) {
        let mut counts = self.lock_capture_counts();
        *counts.entry(scope).or_default() += 1;
    }

    pub fn cancel_next_activation_plan_capture(&self, scope: &TurnScope) {
        decrement_capture_count(&mut self.lock_capture_counts(), scope);
    }

    pub fn take_activation_plan_for_run(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Option<SkillActivationPlan> {
        lock_or_recover(&self.plans_by_run, "skill activation plans")
            .remove(&(scope.clone(), run_id))
    }

    pub async fn select_activation_plan(
        &self,
        run_context: &LoopRunContext,
        message: &str,
    ) -> Result<SkillActivationPlan, SkillActivationSelectionError> {
        self.resolve_activation_plan(run_context, message).await
    }

    fn message_for_scope(&self, scope: &TurnScope) -> Option<String> {
        lock_or_recover(&self.messages_by_scope, "skill activation message")
            .get(scope)
            .cloned()
    }

    async fn selected_candidates(
        &self,
        run_context: &LoopRunContext,
        message: &str,
    ) -> Result<Vec<HostSkillContextCandidate>, SkillActivationSelectionError> {
        let (plan, candidates) = self
            .resolve_activation_plan_with_candidates(run_context, message)
            .await?;
        if self.should_capture_plan(&run_context.scope) {
            lock_or_recover(&self.plans_by_run, "skill activation plans").insert(
                (run_context.scope.clone(), run_context.run_id),
                plan.clone(),
            );
        }
        if plan.selection.activations.is_empty() {
            return Ok(Vec::new());
        }

        let mut selected = Vec::new();
        for candidate in candidates {
            if plan.activated_bundles.contains(candidate.descriptor.id()) {
                selected.push(candidate.into_context_candidate());
            }
        }
        Ok(selected)
    }

    async fn resolve_activation_plan(
        &self,
        run_context: &LoopRunContext,
        message: &str,
    ) -> Result<SkillActivationPlan, SkillActivationSelectionError> {
        self.resolve_activation_plan_with_candidates(run_context, message)
            .await
            .map(|(plan, _)| plan)
    }

    async fn resolve_activation_plan_with_candidates(
        &self,
        run_context: &LoopRunContext,
        message: &str,
    ) -> Result<(SkillActivationPlan, Vec<ActivationCandidate>), SkillActivationSelectionError>
    {
        if message.trim().is_empty() {
            return Ok((
                SkillActivationPlan::new(
                    SkillActivationSelection {
                        activations: Vec::new(),
                        rewritten_message: message.to_string(),
                        feedback: Vec::new(),
                    },
                    Vec::new(),
                ),
                Vec::new(),
            ));
        }

        let mut descriptors = self
            .bundle_source
            .list_skill_bundles(run_context)
            .await
            .map_err(skill_bundle_source_error_to_selection_error)?;
        sort_skill_bundle_descriptors(&mut descriptors);
        validate_descriptor_policy_metadata(&descriptors)?;

        let candidates =
            load_activation_candidates(self.bundle_source.as_ref(), run_context, &descriptors)
                .await?;
        let selection = select_skill_activations(message, &candidates, &self.config)?;
        let plan = activation_plan_for_candidates(selection, &candidates);
        Ok((plan, candidates))
    }

    fn should_capture_plan(&self, scope: &TurnScope) -> bool {
        let mut counts = self.lock_capture_counts();
        decrement_capture_count(&mut counts, scope)
    }

    fn lock_capture_counts(&self) -> MutexGuard<'_, HashMap<TurnScope, usize>> {
        lock_or_recover(
            &self.plan_capture_counts_by_scope,
            "skill activation plan capture",
        )
    }
}

fn decrement_capture_count(counts: &mut HashMap<TurnScope, usize>, scope: &TurnScope) -> bool {
    let Some(count) = counts.get_mut(scope) else {
        return false;
    };
    *count -= 1;
    if *count == 0 {
        counts.remove(scope);
    }
    true
}

fn lock_or_recover<'a, T>(mutex: &'a Mutex<T>, _label: &str) -> MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[async_trait]
impl<S> HostSkillContextSource for SelectableSkillContextSource<S>
where
    S: SkillBundleSource + ?Sized,
{
    async fn load_skill_context_candidates(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Vec<HostSkillContextCandidate>, HostSkillContextBuildError> {
        let Some(message) = self.message_for_scope(&run_context.scope) else {
            return Ok(Vec::new());
        };
        self.selected_candidates(run_context, &message)
            .await
            .map_err(SkillActivationSelectionError::into_context_error)
    }
}

struct ActivationCandidate {
    descriptor: SkillBundleDescriptor,
    loaded: LoadedSkill,
    skill_md: String,
}

impl ActivationCandidate {
    fn into_context_candidate(self) -> HostSkillContextCandidate {
        HostSkillContextCandidate::new(
            self.skill_md,
            self.descriptor.trust().cloned(),
            self.descriptor.visibility().copied(),
        )
        .with_ordering_key(descriptor_context_ordering_key(&self.descriptor))
    }
}

fn activation_plan_for_candidates(
    selection: SkillActivationSelection,
    _candidates: &[ActivationCandidate],
) -> SkillActivationPlan {
    let activated_bundles = selection
        .activations
        .iter()
        .filter_map(|activation| activation.bundle_id.clone())
        .collect();

    SkillActivationPlan::new(selection, activated_bundles)
}

async fn load_activation_candidates<S>(
    source: &S,
    run_context: &LoopRunContext,
    descriptors: &[SkillBundleDescriptor],
) -> Result<Vec<ActivationCandidate>, SkillActivationSelectionError>
where
    S: SkillBundleSource + ?Sized,
{
    try_join_all(
        descriptors
            .iter()
            .filter(|descriptor| descriptor.visibility() == Some(&SkillVisibility::Visible))
            .map(|descriptor| async move {
                let skill_md = source
                    .read_skill_bundle_file(
                        run_context,
                        descriptor.id(),
                        descriptor.skill_md_path(),
                    )
                    .await
                    .map_err(skill_bundle_source_error_to_selection_error)?;
                let skill_md = String::from_utf8(skill_md)
                    .map_err(|_| SkillActivationSelectionError::ParseFailed)?;
                let loaded = loaded_skill_from_candidate(descriptor, &skill_md)?;
                Ok::<_, SkillActivationSelectionError>(ActivationCandidate {
                    descriptor: descriptor.clone(),
                    loaded,
                    skill_md,
                })
            }),
    )
    .await
}

fn loaded_skill_from_candidate(
    descriptor: &SkillBundleDescriptor,
    skill_md: &str,
) -> Result<LoadedSkill, SkillActivationSelectionError> {
    let parsed =
        parse_skill_md(skill_md).map_err(|_| SkillActivationSelectionError::ParseFailed)?;
    let compiled_patterns = LoadedSkill::compile_patterns(&parsed.manifest.activation.patterns);
    let lowercased_keywords = lowercased(&parsed.manifest.activation.keywords);
    let lowercased_exclude_keywords = lowercased(&parsed.manifest.activation.exclude_keywords);
    let lowercased_tags = lowercased(&parsed.manifest.activation.tags);
    let source = match descriptor.id().source_kind() {
        SkillSourceKind::System => SkillSource::Bundled(PathBuf::new()),
        SkillSourceKind::TenantShared => SkillSource::Workspace(PathBuf::new()),
        SkillSourceKind::User => SkillSource::User(PathBuf::new()),
    };
    Ok(LoadedSkill {
        manifest: parsed.manifest,
        prompt_content: parsed.prompt_content,
        trust: descriptor
            .trust()
            .cloned()
            .ok_or(SkillActivationSelectionError::TrustDataMissing)?,
        source,
        content_hash: descriptor_context_ordering_key(descriptor),
        compiled_patterns,
        lowercased_keywords,
        lowercased_exclude_keywords,
        lowercased_tags,
    })
}

fn select_skill_activations(
    message: &str,
    candidates: &[ActivationCandidate],
    config: &SkillActivationSelectorConfig,
) -> Result<SkillActivationSelection, SkillActivationSelectionError> {
    let loaded_skills: Vec<LoadedSkill> = candidates.iter().map(|c| c.loaded.clone()).collect();
    let mention_normalized_message = normalize_dollar_skill_mentions(message);
    let (explicit, rewritten_message) =
        extract_skill_mentions(&mention_normalized_message, &loaded_skills);
    let explicit_names = extract_explicit_skill_names(message);
    validate_explicit_mentions_are_unambiguous(&explicit_names, candidates)?;

    let mut activations = Vec::new();
    let mut selected_keys = HashSet::new();
    let mut feedback = Vec::new();

    for skill in explicit {
        let candidate = candidate_for_loaded_skill(skill, candidates)?;
        let key = (
            candidate.descriptor.id().source_kind(),
            candidate.loaded.manifest.name.clone(),
        );
        if selected_keys.insert(key) {
            activations.push(SkillActivationRequest::resolved(
                candidate.loaded.manifest.name.clone(),
                candidate.descriptor.id().clone(),
                SkillActivationMode::ExplicitMention,
            ));
            feedback.push(format!(
                "{}: force-activated via explicit mention",
                candidate.loaded.manifest.name
            ));
        }
    }

    let outcome = prefilter_skills(
        &rewritten_message,
        &loaded_skills,
        config.max_active_skills,
        config.max_context_tokens,
        &HashSet::new(),
    );
    feedback.extend(outcome.notes);

    for skill in outcome.selected {
        let candidate = candidate_for_loaded_skill(skill, candidates)?;
        let key = (
            candidate.descriptor.id().source_kind(),
            candidate.loaded.manifest.name.clone(),
        );
        if selected_keys.insert(key) {
            activations.push(SkillActivationRequest::resolved(
                candidate.loaded.manifest.name.clone(),
                candidate.descriptor.id().clone(),
                SkillActivationMode::ActivationCriteria,
            ));
        }
    }

    validate_selected_names_are_unambiguous(&activations, candidates)?;

    Ok(SkillActivationSelection {
        activations,
        rewritten_message,
        feedback,
    })
}

fn candidate_for_loaded_skill<'a>(
    skill: &LoadedSkill,
    candidates: &'a [ActivationCandidate],
) -> Result<&'a ActivationCandidate, SkillActivationSelectionError> {
    candidates
        .iter()
        .find(|candidate| {
            candidate.loaded.manifest.name == skill.manifest.name
                && candidate.loaded.source == skill.source
        })
        .ok_or(SkillActivationSelectionError::Internal)
}

fn validate_explicit_mentions_are_unambiguous(
    explicit_names: &[String],
    candidates: &[ActivationCandidate],
) -> Result<(), SkillActivationSelectionError> {
    for name in explicit_names {
        let sources: Vec<SkillSourceKind> = candidates
            .iter()
            .filter(|candidate| candidate.loaded.manifest.name.eq_ignore_ascii_case(name))
            .map(|candidate| candidate.descriptor.id().source_kind())
            .collect();
        let unique_sources: HashSet<SkillSourceKind> = sources.iter().copied().collect();
        if unique_sources.len() > 1 {
            return Err(SkillActivationSelectionError::AmbiguousSkill {
                name: name.clone(),
                sources,
            });
        }
    }
    Ok(())
}

fn validate_selected_names_are_unambiguous(
    activations: &[SkillActivationRequest],
    _candidates: &[ActivationCandidate],
) -> Result<(), SkillActivationSelectionError> {
    let mut sources_by_name: HashMap<&str, HashSet<SkillSourceKind>> = HashMap::new();
    for activation in activations {
        if let Some(source) = activation.source {
            sources_by_name
                .entry(activation.name.as_str())
                .or_default()
                .insert(source);
        }
    }
    for (name, sources) in sources_by_name {
        if sources.len() > 1 {
            return Err(SkillActivationSelectionError::AmbiguousSkill {
                name: name.to_string(),
                sources: sources.into_iter().collect(),
            });
        }
    }
    Ok(())
}

fn extract_explicit_skill_names(message: &str) -> Vec<String> {
    let mut names = Vec::new();
    let bytes = message.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'/' || bytes[index] == b'$' {
            let is_boundary =
                index == 0 || matches!(bytes[index - 1], b' ' | b'\n' | b'\t' | b'"' | b'(' | b'[');
            if is_boundary {
                let start = index + 1;
                let mut end = start;
                while end < bytes.len()
                    && (bytes[end].is_ascii_lowercase()
                        || bytes[end].is_ascii_uppercase()
                        || bytes[end].is_ascii_digit()
                        || bytes[end] == b'-'
                        || bytes[end] == b'_'
                        || bytes[end] == b'.')
                {
                    end += 1;
                }
                if end > start {
                    names.push(message[start..end].to_string());
                    index = end;
                    continue;
                }
            }
        }
        index += 1;
    }
    names
}

fn normalize_dollar_skill_mentions(message: &str) -> String {
    let mut normalized = message.to_string();
    let bytes = message.as_bytes();
    let mut replacements = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'$' {
            let is_boundary =
                index == 0 || matches!(bytes[index - 1], b' ' | b'\n' | b'\t' | b'"' | b'(' | b'[');
            if is_boundary {
                let start = index + 1;
                let mut end = start;
                while end < bytes.len()
                    && (bytes[end].is_ascii_lowercase()
                        || bytes[end].is_ascii_uppercase()
                        || bytes[end].is_ascii_digit()
                        || bytes[end] == b'-'
                        || bytes[end] == b'_'
                        || bytes[end] == b'.')
                {
                    end += 1;
                }
                if end > start {
                    replacements.push(index);
                    index = end;
                    continue;
                }
            }
        }
        index += 1;
    }

    for index in replacements.into_iter().rev() {
        normalized.replace_range(index..index + 1, "/");
    }
    normalized
}

fn validate_descriptor_policy_metadata(
    descriptors: &[SkillBundleDescriptor],
) -> Result<(), SkillActivationSelectionError> {
    for descriptor in descriptors {
        if descriptor.trust().is_none() {
            return Err(SkillActivationSelectionError::TrustDataMissing);
        }
        if descriptor.visibility().is_none() {
            return Err(SkillActivationSelectionError::VisibilityDataMissing);
        }
    }
    Ok(())
}

fn skill_bundle_source_error_to_selection_error(
    error: SkillBundleSourceError,
) -> SkillActivationSelectionError {
    match error {
        SkillBundleSourceError::SourceUnavailable
        | SkillBundleSourceError::BundleNotFound
        | SkillBundleSourceError::FileNotFound
        | SkillBundleSourceError::PermissionDenied => {
            SkillActivationSelectionError::SourceUnavailable
        }
        SkillBundleSourceError::InvalidBundleId
        | SkillBundleSourceError::InvalidFilePath
        | SkillBundleSourceError::InvalidSkillBundle => SkillActivationSelectionError::ParseFailed,
        SkillBundleSourceError::ContentTooLarge => {
            SkillActivationSelectionError::ContextBudgetExceeded
        }
        SkillBundleSourceError::Internal => SkillActivationSelectionError::Internal,
    }
}

fn lowercased(values: &[String]) -> Vec<String> {
    values.iter().map(|value| value.to_lowercase()).collect()
}

fn descriptor_context_ordering_key(descriptor: &SkillBundleDescriptor) -> String {
    let (source_kind, name, path) = descriptor.ordering_key();
    format!("{}:{}:{}", source_kind.as_str(), name, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId};
    use ironclaw_loop_support::{SkillBundleId, SkillFilePath};
    use ironclaw_skills::SkillTrust;
    use ironclaw_turns::{
        TurnActor, TurnId, TurnRunId,
        run_profile::{
            InMemoryRunProfileResolver, RunProfileResolutionRequest, RunProfileResolver,
        },
    };

    struct StaticSkillBundleSource {
        descriptors: Vec<SkillBundleDescriptor>,
        files: HashMap<(SkillSourceKind, String), Vec<u8>>,
    }

    impl StaticSkillBundleSource {
        fn new(skills: Vec<(SkillSourceKind, &str, &str)>) -> Self {
            let mut descriptors = Vec::new();
            let mut files = HashMap::new();
            for (source, name, skill_md) in skills {
                let id = SkillBundleId::new(source, name).unwrap();
                descriptors.push(SkillBundleDescriptor::new(
                    id.clone(),
                    Some(SkillTrust::Trusted),
                    Some(SkillVisibility::Visible),
                ));
                files.insert((source, name.to_string()), skill_md.as_bytes().to_vec());
            }
            Self { descriptors, files }
        }
    }

    #[async_trait]
    impl SkillBundleSource for StaticSkillBundleSource {
        async fn list_skill_bundles(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Vec<SkillBundleDescriptor>, SkillBundleSourceError> {
            Ok(self.descriptors.clone())
        }

        async fn read_skill_bundle_file(
            &self,
            _run_context: &LoopRunContext,
            bundle_id: &SkillBundleId,
            _path: &SkillFilePath,
        ) -> Result<Vec<u8>, SkillBundleSourceError> {
            self.files
                .get(&(bundle_id.source_kind(), bundle_id.name().to_string()))
                .cloned()
                .ok_or(SkillBundleSourceError::FileNotFound)
        }
    }

    fn skill_md(name: &str, description: &str, keywords: &[&str], prompt: &str) -> String {
        let keyword_list = keywords
            .iter()
            .map(|keyword| format!("\"{}\"", keyword))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "---\nname: {name}\ndescription: {description}\nactivation:\n  keywords: [{keyword_list}]\n---\n\n{prompt}"
        )
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
        .with_actor(TurnActor::new(
            ironclaw_host_api::UserId::new("user-a").unwrap(),
        ))
    }

    #[tokio::test]
    async fn selector_returns_no_context_without_matching_activation() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![(
            SkillSourceKind::User,
            "code-review",
            &skill_md(
                "code-review",
                "Review code",
                &["review"],
                "CODE_REVIEW_SENTINEL",
            ),
        )]));
        let selectable =
            SelectableSkillContextSource::new(source, SkillActivationSelectorConfig::default());
        let context = run_context().await;
        selectable.record_user_message(context.scope.clone(), "hello there");

        let selected = selectable
            .load_skill_context_candidates(&context)
            .await
            .expect("selection succeeds");

        assert!(selected.is_empty());
    }

    #[tokio::test]
    async fn selector_activates_only_keyword_matching_skill() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![
            (
                SkillSourceKind::System,
                "code-review",
                &skill_md(
                    "code-review",
                    "Review code",
                    &["review"],
                    "CODE_REVIEW_SENTINEL",
                ),
            ),
            (
                SkillSourceKind::User,
                "spreadsheet",
                &skill_md(
                    "spreadsheet",
                    "Spreadsheet work",
                    &["sheet"],
                    "SHEET_SENTINEL",
                ),
            ),
        ]));
        let selectable =
            SelectableSkillContextSource::new(source, SkillActivationSelectorConfig::default());
        let context = run_context().await;
        selectable.record_user_message(context.scope.clone(), "please review this PR");

        let selected = selectable
            .load_skill_context_candidates(&context)
            .await
            .expect("selection succeeds");

        assert_eq!(selected.len(), 1);
        assert!(
            selected[0]
                .skill_md
                .as_ref()
                .expect("skill context")
                .contains("CODE_REVIEW_SENTINEL")
        );
    }

    #[tokio::test]
    async fn selector_force_activates_dollar_skill_mention() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![(
            SkillSourceKind::User,
            "code-review",
            &skill_md("code-review", "Review code", &[], "CODE_REVIEW_SENTINEL"),
        )]));
        let selectable =
            SelectableSkillContextSource::new(source, SkillActivationSelectorConfig::default());
        let context = run_context().await;
        selectable.record_user_message(context.scope.clone(), "$code-review this PR");

        let selected = selectable
            .load_skill_context_candidates(&context)
            .await
            .expect("selection succeeds");

        assert_eq!(selected.len(), 1);
    }

    #[tokio::test]
    async fn selector_rejects_ambiguous_explicit_mentions() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![
            (
                SkillSourceKind::System,
                "code-review",
                &skill_md(
                    "code-review",
                    "System review",
                    &[],
                    "SYSTEM_REVIEW_SENTINEL",
                ),
            ),
            (
                SkillSourceKind::User,
                "code-review",
                &skill_md("code-review", "User review", &[], "USER_REVIEW_SENTINEL"),
            ),
        ]));
        let selectable =
            SelectableSkillContextSource::new(source, SkillActivationSelectorConfig::default());
        let context = run_context().await;
        selectable.record_user_message(context.scope.clone(), "/code-review this PR");

        let error = selectable
            .selected_candidates(&context, "/code-review this PR")
            .await
            .expect_err("ambiguous activation should fail");

        assert!(matches!(
            error,
            SkillActivationSelectionError::AmbiguousSkill { .. }
        ));
    }
}
