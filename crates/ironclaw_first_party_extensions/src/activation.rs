use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt, stream};
use ironclaw_loop_support::{
    HostSkillContextBuildError, HostSkillContextCandidate, HostSkillContextSource,
    SkillBundleDescriptor, SkillBundleId, SkillBundleSource, SkillBundleSourceError,
    SkillSourceKind, sort_skill_bundle_descriptors,
};
use ironclaw_skills::{
    LoadedSkill, SkillSource, extract_skill_mentions, parse_skill_md, prefilter_skills,
    skill_token_cost,
};
use ironclaw_turns::run_profile::{LoopRunContext, SkillVisibility};
use ironclaw_turns::{AcceptedMessageRef, TurnRunId, TurnScope};
use thiserror::Error;

/// Maximum number of first-party skills selected for one turn by default.
pub const DEFAULT_MAX_ACTIVE_SKILLS: usize = 4;

/// Maximum estimated skill prompt tokens selected for one turn by default.
pub const DEFAULT_MAX_SKILL_CONTEXT_TOKENS: usize = 4000;

const MAX_CONCURRENT_SKILL_ACTIVATION_LOADS: usize = 16;
const MAX_ACTIVATION_CACHE_ENTRIES: usize = 1024;

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
    pub fn empty(selection: SkillActivationSelection) -> Self {
        Self {
            selection,
            activated_bundles: Vec::new(),
        }
    }

    pub(crate) fn new(
        selection: SkillActivationSelection,
        activated_bundles: Vec<SkillBundleId>,
    ) -> Self {
        Self {
            selection,
            activated_bundles,
        }
    }

    pub fn activated_bundles(&self) -> &[SkillBundleId] {
        &self.activated_bundles
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedSkillActivationPlan {
    pub plan: SkillActivationPlan,
    pub run_context: LoopRunContext,
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
            Self::AmbiguousSkill { name, sources } => {
                HostSkillContextBuildError::AmbiguousSkill { name, sources }
            }
            Self::ParseFailed => HostSkillContextBuildError::ParseFailed,
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
    setup_marker_source: Option<Arc<dyn SetupMarkerSource>>,
    messages_by_run: Mutex<HashMap<SkillActivationMessageKey, SkillActivationMessage>>,
    activation_cache: Mutex<HashMap<ActivationCandidateCacheKey, CachedActivationCandidate>>,
    plans_by_run: Mutex<HashMap<(TurnScope, TurnRunId), CapturedSkillActivationPlan>>,
}

/// Source of already-satisfied setup markers for one-time setup skills.
#[async_trait]
pub(crate) trait SetupMarkerSource: std::fmt::Debug + Send + Sync {
    async fn satisfied_setup_markers(
        &self,
        run_context: &LoopRunContext,
        markers: &HashSet<String>,
    ) -> Result<HashSet<String>, SkillActivationSelectionError>;
}

impl<S> SelectableSkillContextSource<S>
where
    S: SkillBundleSource + ?Sized,
{
    pub fn new(bundle_source: Arc<S>, config: SkillActivationSelectorConfig) -> Self {
        Self {
            bundle_source,
            config,
            setup_marker_source: None,
            messages_by_run: Mutex::new(HashMap::new()),
            activation_cache: Mutex::new(HashMap::new()),
            plans_by_run: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn with_setup_marker_source<T>(mut self, source: Arc<T>) -> Self
    where
        T: SetupMarkerSource + 'static,
    {
        self.setup_marker_source = Some(source);
        self
    }

    pub fn record_user_message(
        &self,
        scope: TurnScope,
        accepted_message_ref: AcceptedMessageRef,
        message: impl Into<String>,
    ) -> Result<(), SkillActivationSelectionError> {
        self.record_message(scope, accepted_message_ref, message, false)
    }

    pub(crate) fn record_user_message_for_execution(
        &self,
        scope: TurnScope,
        accepted_message_ref: AcceptedMessageRef,
        message: impl Into<String>,
    ) -> Result<(), SkillActivationSelectionError> {
        self.record_message(scope, accepted_message_ref, message, true)
    }

    fn record_message(
        &self,
        scope: TurnScope,
        accepted_message_ref: AcceptedMessageRef,
        message: impl Into<String>,
        capture_plan: bool,
    ) -> Result<(), SkillActivationSelectionError> {
        self.messages_by_run
            .lock()
            .map_err(|_| SkillActivationSelectionError::Internal)?
            .insert(
                SkillActivationMessageKey::new(scope, accepted_message_ref),
                SkillActivationMessage {
                    text: message.into(),
                    capture_plan,
                },
            );
        Ok(())
    }

    pub(crate) fn bundle_source(&self) -> Arc<S> {
        Arc::clone(&self.bundle_source)
    }

    pub(crate) fn take_activation_plan_for_run(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Option<CapturedSkillActivationPlan>, SkillActivationSelectionError> {
        Ok(self
            .plans_by_run
            .lock()
            .map_err(|_| SkillActivationSelectionError::Internal)?
            .remove(&(scope.clone(), run_id)))
    }

    pub async fn select_activation_plan(
        &self,
        run_context: &LoopRunContext,
        message: &str,
    ) -> Result<SkillActivationPlan, SkillActivationSelectionError> {
        self.resolve_activation_plan(run_context, message).await
    }

    pub fn clear_accepted_message(
        &self,
        scope: &TurnScope,
        accepted_message_ref: &AcceptedMessageRef,
    ) -> Result<(), SkillActivationSelectionError> {
        self.messages_by_run
            .lock()
            .map_err(|_| SkillActivationSelectionError::Internal)?
            .remove(&SkillActivationMessageKey::new(
                scope.clone(),
                accepted_message_ref.clone(),
            ));
        Ok(())
    }

    fn take_message_for_run(
        &self,
        scope: &TurnScope,
        accepted_message_ref: &AcceptedMessageRef,
    ) -> Result<Option<SkillActivationMessage>, SkillActivationSelectionError> {
        Ok(self
            .messages_by_run
            .lock()
            .map_err(|_| SkillActivationSelectionError::Internal)?
            .remove(&SkillActivationMessageKey::new(
                scope.clone(),
                accepted_message_ref.clone(),
            )))
    }

    async fn selected_candidates(
        &self,
        run_context: &LoopRunContext,
        message: &str,
        capture_plan: bool,
    ) -> Result<Vec<HostSkillContextCandidate>, SkillActivationSelectionError> {
        let (plan, candidates) = self
            .resolve_activation_plan_with_candidates(run_context, message)
            .await?;
        if capture_plan {
            self.plans_by_run
                .lock()
                .map_err(|_| SkillActivationSelectionError::Internal)?
                .insert(
                    (run_context.scope.clone(), run_context.run_id),
                    CapturedSkillActivationPlan {
                        plan: plan.clone(),
                        run_context: run_context.clone(),
                    },
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
                SkillActivationPlan::empty(SkillActivationSelection {
                    activations: Vec::new(),
                    rewritten_message: message.to_string(),
                    feedback: Vec::new(),
                }),
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

        let candidates = self
            .load_activation_candidates(run_context, &descriptors)
            .await?;
        let satisfied_setup_markers = self
            .satisfied_setup_markers(run_context, &candidates)
            .await?;
        let selection =
            select_skill_activations(message, &candidates, &self.config, &satisfied_setup_markers)?;
        let plan = activation_plan_for_candidates(selection);
        Ok((plan, candidates))
    }

    async fn satisfied_setup_markers(
        &self,
        run_context: &LoopRunContext,
        candidates: &[ActivationCandidate],
    ) -> Result<HashSet<String>, SkillActivationSelectionError> {
        let markers = candidates
            .iter()
            .filter_map(|candidate| {
                candidate
                    .loaded
                    .manifest
                    .activation
                    .setup_marker
                    .as_ref()
                    .cloned()
            })
            .collect::<HashSet<_>>();
        if markers.is_empty() {
            return Ok(HashSet::new());
        }
        let Some(source) = self.setup_marker_source.as_deref() else {
            return Ok(HashSet::new());
        };
        source.satisfied_setup_markers(run_context, &markers).await
    }

    async fn load_activation_candidates(
        &self,
        run_context: &LoopRunContext,
        descriptors: &[SkillBundleDescriptor],
    ) -> Result<Vec<ActivationCandidate>, SkillActivationSelectionError> {
        stream::iter(0..descriptors.len())
            .map(|index| async move {
                let descriptor = &descriptors[index];
                if descriptor.visibility() != Some(&SkillVisibility::Visible) {
                    return Ok(None);
                }
                let descriptor = descriptor.clone();
                let skill_md = self
                    .bundle_source
                    .read_skill_bundle_file(
                        run_context,
                        descriptor.id(),
                        descriptor.skill_md_path(),
                    )
                    .await
                    .map_err(skill_bundle_source_error_to_selection_error)?;
                self.activation_candidate_from_skill_md(&descriptor, skill_md)
                    .map(Some)
            })
            .buffered(MAX_CONCURRENT_SKILL_ACTIVATION_LOADS)
            .try_filter_map(|candidate| async move { Ok(candidate) })
            .try_collect()
            .await
    }

    fn activation_candidate_from_skill_md(
        &self,
        descriptor: &SkillBundleDescriptor,
        skill_md: Vec<u8>,
    ) -> Result<ActivationCandidate, SkillActivationSelectionError> {
        let cache_key = ActivationCandidateCacheKey::new(descriptor, &skill_md);
        let skill_md =
            String::from_utf8(skill_md).map_err(|_| SkillActivationSelectionError::ParseFailed)?;
        if let Some(cached) = self
            .activation_cache
            .lock()
            .map_err(|_| SkillActivationSelectionError::Internal)?
            .get(&cache_key)
            .cloned()
        {
            return Ok(ActivationCandidate {
                descriptor: descriptor.clone(),
                loaded: cached.loaded,
                skill_md,
            });
        }

        let loaded = loaded_skill_from_candidate(descriptor, &skill_md)?;
        let mut cache = self
            .activation_cache
            .lock()
            .map_err(|_| SkillActivationSelectionError::Internal)?;
        if let Some(cached) = cache.get(&cache_key).cloned() {
            return Ok(ActivationCandidate {
                descriptor: descriptor.clone(),
                loaded: cached.loaded,
                skill_md,
            });
        }
        if cache.len() >= MAX_ACTIVATION_CACHE_ENTRIES {
            cache.clear();
        }
        cache.insert(
            cache_key,
            CachedActivationCandidate {
                loaded: loaded.clone(),
            },
        );
        Ok(ActivationCandidate {
            descriptor: descriptor.clone(),
            loaded,
            skill_md,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SkillActivationMessageKey {
    scope: TurnScope,
    accepted_message_ref: AcceptedMessageRef,
}

impl SkillActivationMessageKey {
    fn new(scope: TurnScope, accepted_message_ref: AcceptedMessageRef) -> Self {
        Self {
            scope,
            accepted_message_ref,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillActivationMessage {
    text: String,
    capture_plan: bool,
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
        let Some(accepted_message_ref) = run_context.accepted_message_ref.as_ref() else {
            return Ok(Vec::new());
        };
        let Some(message) = self
            .take_message_for_run(&run_context.scope, accepted_message_ref)
            .map_err(SkillActivationSelectionError::into_context_error)?
        else {
            return Ok(Vec::new());
        };
        self.selected_candidates(run_context, &message.text, message.capture_plan)
            .await
            .map_err(SkillActivationSelectionError::into_context_error)
    }
}

struct ActivationCandidate {
    descriptor: SkillBundleDescriptor,
    loaded: LoadedSkill,
    skill_md: String,
}

#[derive(Debug, Clone)]
struct CachedActivationCandidate {
    loaded: LoadedSkill,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ActivationCandidateCacheKey {
    source_kind: SkillSourceKind,
    name: String,
    skill_md_path: String,
    content_hash: String,
    trust: Option<ironclaw_skills::SkillTrust>,
    visibility: Option<SkillVisibility>,
}

impl ActivationCandidateCacheKey {
    fn new(descriptor: &SkillBundleDescriptor, skill_md: &[u8]) -> Self {
        Self {
            source_kind: descriptor.id().source_kind(),
            name: descriptor.id().name().to_string(),
            skill_md_path: descriptor.skill_md_path().as_str().to_string(),
            content_hash: descriptor
                .provenance()
                .content_hash
                .clone()
                .unwrap_or_else(|| content_hash(skill_md)),
            trust: descriptor.trust().copied(),
            visibility: descriptor.visibility().copied(),
        }
    }
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

fn activation_plan_for_candidates(selection: SkillActivationSelection) -> SkillActivationPlan {
    let activated_bundles = selection
        .activations
        .iter()
        .filter_map(|activation| activation.bundle_id.clone())
        .collect();

    SkillActivationPlan::new(selection, activated_bundles)
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
    satisfied_setup_markers: &HashSet<String>,
) -> Result<SkillActivationSelection, SkillActivationSelectionError> {
    let active_candidates =
        candidates_with_unsatisfied_setup_markers(candidates, satisfied_setup_markers);
    let loaded_skills: Vec<LoadedSkill> =
        active_candidates.iter().map(|c| c.loaded.clone()).collect();
    let mention_normalized_message = normalize_dollar_skill_mentions(message);
    let (explicit, rewritten_message) =
        extract_skill_mentions(&mention_normalized_message, &loaded_skills);
    let explicit_names = extract_explicit_skill_names(message);
    validate_explicit_mentions_are_unambiguous(&explicit_names, &active_candidates)?;

    let mut activations = Vec::new();
    let mut selected_keys = HashSet::new();
    let mut feedback = Vec::new();
    let mut remaining_slots = config.max_active_skills;
    let mut remaining_tokens = config.max_context_tokens;

    for skill in explicit {
        let candidate = candidate_for_loaded_skill(skill, &active_candidates)?;
        let key = (
            candidate.descriptor.id().source_kind(),
            candidate.loaded.manifest.name.clone(),
        );
        if selected_keys.insert(key) {
            reserve_skill_budget(skill, &mut remaining_slots, &mut remaining_tokens)?;
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
        remaining_slots,
        remaining_tokens,
        satisfied_setup_markers,
    );
    feedback.extend(outcome.notes);

    for skill in outcome.selected {
        let candidate = candidate_for_loaded_skill(skill, &active_candidates)?;
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

    validate_selected_names_are_unambiguous(&activations)?;

    Ok(SkillActivationSelection {
        activations,
        rewritten_message,
        feedback,
    })
}

fn candidates_with_unsatisfied_setup_markers<'a>(
    candidates: &'a [ActivationCandidate],
    satisfied_setup_markers: &HashSet<String>,
) -> Vec<&'a ActivationCandidate> {
    candidates
        .iter()
        .filter(|candidate| {
            candidate
                .loaded
                .manifest
                .activation
                .setup_marker
                .as_ref()
                .map_or(true, |marker| !satisfied_setup_markers.contains(marker))
        })
        .collect()
}

fn candidate_for_loaded_skill<'a>(
    skill: &LoadedSkill,
    candidates: &'a [&ActivationCandidate],
) -> Result<&'a ActivationCandidate, SkillActivationSelectionError> {
    candidates
        .iter()
        .find(|candidate| {
            candidate.loaded.manifest.name == skill.manifest.name
                && candidate.loaded.source == skill.source
        })
        .ok_or(SkillActivationSelectionError::Internal)
        .copied()
}

fn validate_explicit_mentions_are_unambiguous(
    explicit_names: &[String],
    candidates: &[&ActivationCandidate],
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
    let chars: Vec<(usize, char)> = message.char_indices().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index].1 == '/' || chars[index].1 == '$' {
            let is_boundary = index == 0 || is_skill_mention_boundary(chars[index - 1].1);
            if is_boundary {
                let start = index + 1;
                let mut end = start;
                while end < chars.len()
                    && (chars[end].1.is_ascii_alphanumeric()
                        || matches!(chars[end].1, '-' | '_' | '.'))
                {
                    end += 1;
                }
                if end > start {
                    let start_byte = chars[start].0;
                    let end_byte = chars
                        .get(end)
                        .map(|(byte_index, _)| *byte_index)
                        .unwrap_or(message.len());
                    names.push(message[start_byte..end_byte].to_string());
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
    let mut replacements = Vec::new();
    let chars: Vec<(usize, char)> = message.char_indices().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index].1 == '$' {
            let is_boundary = index == 0 || is_skill_mention_boundary(chars[index - 1].1);
            if is_boundary {
                let start = index + 1;
                let mut end = start;
                while end < chars.len()
                    && (chars[end].1.is_ascii_alphanumeric()
                        || matches!(chars[end].1, '-' | '_' | '.'))
                {
                    end += 1;
                }
                if end > start {
                    replacements.push(chars[index].0);
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

fn is_skill_mention_boundary(previous: char) -> bool {
    matches!(previous, ' ' | '\n' | '\t' | '"' | '(' | '[') || !previous.is_ascii()
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
        | SkillBundleSourceError::InvalidSkillBundle
        | SkillBundleSourceError::BundleUtf8DecodeFailed
        | SkillBundleSourceError::ManifestParseFailed => SkillActivationSelectionError::ParseFailed,
        SkillBundleSourceError::ContentTooLarge
        | SkillBundleSourceError::BundleScanLimitExceeded => {
            SkillActivationSelectionError::ContextBudgetExceeded
        }
        SkillBundleSourceError::DuplicateSourceKind | SkillBundleSourceError::Internal => {
            SkillActivationSelectionError::Internal
        }
    }
}

fn lowercased(values: &[String]) -> Vec<String> {
    values.iter().map(|value| value.to_lowercase()).collect()
}

fn reserve_skill_budget(
    skill: &LoadedSkill,
    remaining_slots: &mut usize,
    remaining_tokens: &mut usize,
) -> Result<(), SkillActivationSelectionError> {
    if *remaining_slots == 0 {
        return Err(SkillActivationSelectionError::ContextBudgetExceeded);
    }
    let cost = skill_token_cost(skill);
    if cost > *remaining_tokens {
        return Err(SkillActivationSelectionError::ContextBudgetExceeded);
    }
    *remaining_slots -= 1;
    *remaining_tokens -= cost;
    Ok(())
}

fn descriptor_context_ordering_key(descriptor: &SkillBundleDescriptor) -> String {
    let (source_kind, name, path) = descriptor.ordering_key();
    length_prefixed_key_components([source_kind.as_str(), name, path])
}

fn length_prefixed_key_components<const N: usize>(components: [&str; N]) -> String {
    let mut key = String::new();
    for component in components {
        key.push_str(&component.len().to_string());
        key.push(':');
        key.push_str(component);
        key.push('|');
    }
    key
}

fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
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

    struct ErroringListSkillBundleSource {
        error: SkillBundleSourceError,
    }

    struct ChangingSkillBundleSource {
        descriptor: SkillBundleDescriptor,
        first: Vec<u8>,
        second: Vec<u8>,
        reads: std::sync::atomic::AtomicUsize,
    }

    #[derive(Debug)]
    struct StaticSetupMarkerSource {
        satisfied_markers: HashSet<String>,
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

    impl ErroringListSkillBundleSource {
        fn new(error: SkillBundleSourceError) -> Self {
            Self { error }
        }
    }

    impl ChangingSkillBundleSource {
        fn new(name: &str, first: String, second: String) -> Self {
            let id = SkillBundleId::new(SkillSourceKind::User, name).unwrap();
            let descriptor = SkillBundleDescriptor::new(
                id,
                Some(SkillTrust::Trusted),
                Some(SkillVisibility::Visible),
            )
            .with_provenance(
                ironclaw_loop_support::SkillBundleProvenance::new(SkillSourceKind::User)
                    .with_content_hash("stable-test-hash"),
            );
            Self {
                descriptor,
                first: first.into_bytes(),
                second: second.into_bytes(),
                reads: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    impl StaticSetupMarkerSource {
        fn new(satisfied_markers: &[&str]) -> Self {
            Self {
                satisfied_markers: satisfied_markers
                    .iter()
                    .map(|marker| marker.to_string())
                    .collect(),
            }
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

    #[async_trait]
    impl SkillBundleSource for ErroringListSkillBundleSource {
        async fn list_skill_bundles(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Vec<SkillBundleDescriptor>, SkillBundleSourceError> {
            Err(self.error.clone())
        }

        async fn read_skill_bundle_file(
            &self,
            _run_context: &LoopRunContext,
            _bundle_id: &SkillBundleId,
            _path: &SkillFilePath,
        ) -> Result<Vec<u8>, SkillBundleSourceError> {
            Err(SkillBundleSourceError::Internal)
        }
    }

    #[async_trait]
    impl SkillBundleSource for ChangingSkillBundleSource {
        async fn list_skill_bundles(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Vec<SkillBundleDescriptor>, SkillBundleSourceError> {
            Ok(vec![self.descriptor.clone()])
        }

        async fn read_skill_bundle_file(
            &self,
            _run_context: &LoopRunContext,
            _bundle_id: &SkillBundleId,
            _path: &SkillFilePath,
        ) -> Result<Vec<u8>, SkillBundleSourceError> {
            let read = self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if read == 0 {
                Ok(self.first.clone())
            } else {
                Ok(self.second.clone())
            }
        }
    }

    #[async_trait]
    impl SetupMarkerSource for StaticSetupMarkerSource {
        async fn satisfied_setup_markers(
            &self,
            _run_context: &LoopRunContext,
            markers: &HashSet<String>,
        ) -> Result<HashSet<String>, SkillActivationSelectionError> {
            Ok(markers
                .intersection(&self.satisfied_markers)
                .cloned()
                .collect())
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

    fn skill_md_with_activation(name: &str, activation: &str, prompt: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: {name} description\nactivation:\n{activation}\n---\n\n{prompt}"
        )
    }

    async fn run_context() -> LoopRunContext {
        run_context_for("thread-a", "msg:run-a").await
    }

    async fn run_context_for(thread_id: &str, accepted_message: &str) -> LoopRunContext {
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        LoopRunContext::new(
            TurnScope::new(
                TenantId::new("tenant-a").unwrap(),
                Some(AgentId::new("agent-a").unwrap()),
                Some(ProjectId::new("project-a").unwrap()),
                ironclaw_host_api::ThreadId::new(thread_id).unwrap(),
            ),
            TurnId::new(),
            TurnRunId::new(),
            resolved,
        )
        .with_accepted_message_ref(AcceptedMessageRef::new(accepted_message).unwrap())
        .with_actor(TurnActor::new(
            ironclaw_host_api::UserId::new("user-a").unwrap(),
        ))
    }

    fn accepted_message_ref(context: &LoopRunContext) -> AcceptedMessageRef {
        context
            .accepted_message_ref
            .clone()
            .expect("run context accepted message ref")
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
        selectable
            .record_user_message(
                context.scope.clone(),
                accepted_message_ref(&context),
                "hello there",
            )
            .expect("record message");

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
        selectable
            .record_user_message(
                context.scope.clone(),
                accepted_message_ref(&context),
                "please review this PR",
            )
            .expect("record message");

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
    async fn selector_suppresses_explicit_skill_when_setup_marker_is_satisfied() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![(
            SkillSourceKind::User,
            "setup-helper",
            &skill_md_with_activation(
                "setup-helper",
                "  keywords: [\"setup-helper\"]\n  setup_marker: \"markers/setup-helper.done\"",
                "SETUP_HELPER_SENTINEL",
            ),
        )]));
        let setup_markers = Arc::new(StaticSetupMarkerSource::new(&["markers/setup-helper.done"]));
        let selectable =
            SelectableSkillContextSource::new(source, SkillActivationSelectorConfig::default())
                .with_setup_marker_source(setup_markers);
        let context = run_context().await;
        selectable
            .record_user_message(
                context.scope.clone(),
                accepted_message_ref(&context),
                "$setup-helper",
            )
            .expect("record message");

        let selected = selectable
            .load_skill_context_candidates(&context)
            .await
            .expect("selection succeeds");

        assert!(
            selected.is_empty(),
            "setup markers must suppress explicit and natural-language activation"
        );
    }

    #[tokio::test]
    async fn selector_keeps_recorded_messages_isolated_by_accepted_message_ref() {
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
        let first_context = run_context().await;
        let second_context = LoopRunContext::new(
            first_context.scope.clone(),
            first_context.turn_id,
            TurnRunId::new(),
            first_context.resolved_run_profile.clone(),
        )
        .with_accepted_message_ref(AcceptedMessageRef::new("msg:run-b").unwrap())
        .with_actor(first_context.actor().expect("actor").clone());

        selectable
            .record_user_message(
                first_context.scope.clone(),
                accepted_message_ref(&first_context),
                "please review this PR",
            )
            .expect("record first message");
        selectable
            .record_user_message(
                second_context.scope.clone(),
                accepted_message_ref(&second_context),
                "hello there",
            )
            .expect("record second message");

        let first_selected = selectable
            .load_skill_context_candidates(&first_context)
            .await
            .expect("first selection succeeds");
        assert_eq!(first_selected.len(), 1);

        let first_selected_after_clear = selectable
            .load_skill_context_candidates(&first_context)
            .await
            .expect("first selection after clear succeeds");
        assert!(first_selected_after_clear.is_empty());

        let second_selected = selectable
            .load_skill_context_candidates(&second_context)
            .await
            .expect("second selection succeeds");
        assert!(
            second_selected.is_empty(),
            "clearing one run must not remove another run's recorded message"
        );
    }

    #[tokio::test]
    async fn clear_accepted_message_removes_only_requested_message() {
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
        let first_context = run_context().await;
        let second_context = run_context_for("thread-a", "msg:run-b").await;

        selectable
            .record_user_message(
                first_context.scope.clone(),
                accepted_message_ref(&first_context),
                "please review this PR",
            )
            .expect("record first message");
        selectable
            .record_user_message(
                second_context.scope.clone(),
                accepted_message_ref(&second_context),
                "please review this PR",
            )
            .expect("record second message");

        selectable
            .clear_accepted_message(&first_context.scope, &accepted_message_ref(&first_context))
            .expect("clear first message");

        let first_selected = selectable
            .load_skill_context_candidates(&first_context)
            .await
            .expect("first selection succeeds");
        assert!(
            first_selected.is_empty(),
            "cleared message should not activate skills"
        );

        let second_selected = selectable
            .load_skill_context_candidates(&second_context)
            .await
            .expect("second selection succeeds");
        assert_eq!(
            second_selected.len(),
            1,
            "clearing one accepted message must not remove another message"
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
        selectable
            .record_user_message(
                context.scope.clone(),
                accepted_message_ref(&context),
                "$code-review this PR",
            )
            .expect("record message");

        let selected = selectable
            .load_skill_context_candidates(&context)
            .await
            .expect("selection succeeds");

        assert_eq!(selected.len(), 1);
    }

    #[tokio::test]
    async fn selector_force_activates_bracketed_dollar_skill_mention() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![(
            SkillSourceKind::User,
            "code-review",
            &skill_md("code-review", "Review code", &[], "CODE_REVIEW_SENTINEL"),
        )]));
        let selectable =
            SelectableSkillContextSource::new(source, SkillActivationSelectorConfig::default());
        let context = run_context().await;
        selectable
            .record_user_message(
                context.scope.clone(),
                accepted_message_ref(&context),
                "[$code-review](/skills/code-review/SKILL.md) this PR",
            )
            .expect("record message");

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
        selectable
            .record_user_message(
                context.scope.clone(),
                accepted_message_ref(&context),
                "/code-review this PR",
            )
            .expect("record message");

        let error = selectable
            .selected_candidates(&context, "/code-review this PR", false)
            .await
            .expect_err("ambiguous activation should fail");

        assert!(matches!(
            error,
            SkillActivationSelectionError::AmbiguousSkill { .. }
        ));
    }

    #[tokio::test]
    async fn selector_activates_skills_from_tags_and_patterns() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![
            (
                SkillSourceKind::System,
                "tag-helper",
                &skill_md_with_activation(
                    "tag-helper",
                    "  tags: [\"release\"]",
                    "TAG_HELPER_SENTINEL",
                ),
            ),
            (
                SkillSourceKind::User,
                "pattern-helper",
                &skill_md_with_activation(
                    "pattern-helper",
                    "  patterns: [\"deploy\\\\s+plan\"]",
                    "PATTERN_HELPER_SENTINEL",
                ),
            ),
            (
                SkillSourceKind::User,
                "quiet-helper",
                &skill_md("quiet-helper", "Quiet", &["quiet"], "QUIET_HELPER_SENTINEL"),
            ),
        ]));
        let selectable =
            SelectableSkillContextSource::new(source, SkillActivationSelectorConfig::default());
        let context = run_context().await;
        selectable
            .record_user_message(
                context.scope.clone(),
                accepted_message_ref(&context),
                "review release deploy plan",
            )
            .expect("record message");

        let selected = selectable
            .load_skill_context_candidates(&context)
            .await
            .expect("selection succeeds");
        let combined = selected
            .iter()
            .map(|candidate| candidate.skill_md.as_deref().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(selected.len(), 2);
        assert!(combined.contains("TAG_HELPER_SENTINEL"));
        assert!(combined.contains("PATTERN_HELPER_SENTINEL"));
        assert!(!combined.contains("QUIET_HELPER_SENTINEL"));
    }

    #[tokio::test]
    async fn selector_respects_configured_active_skill_and_token_limits() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![
            (
                SkillSourceKind::System,
                "alpha-helper",
                &skill_md_with_activation(
                    "alpha-helper",
                    "  keywords: [\"shared\"]\n  max_context_tokens: 2",
                    "ALPHA_SENTINEL",
                ),
            ),
            (
                SkillSourceKind::User,
                "beta-helper",
                &skill_md_with_activation(
                    "beta-helper",
                    "  keywords: [\"shared\"]\n  max_context_tokens: 2",
                    "BETA_SENTINEL",
                ),
            ),
        ]));
        let selectable = SelectableSkillContextSource::new(
            source,
            SkillActivationSelectorConfig {
                max_active_skills: 1,
                max_context_tokens: 4,
            },
        );
        let context = run_context().await;
        selectable
            .record_user_message(
                context.scope.clone(),
                accepted_message_ref(&context),
                "shared",
            )
            .expect("record message");

        let selected = selectable
            .load_skill_context_candidates(&context)
            .await
            .expect("selection succeeds");

        assert_eq!(selected.len(), 1);

        selectable
            .record_user_message(
                context.scope.clone(),
                accepted_message_ref(&context),
                "/alpha-helper /beta-helper",
            )
            .expect("record message");
        let error = selectable
            .selected_candidates(&context, "/alpha-helper /beta-helper", false)
            .await
            .expect_err("explicit activation should honor active skill limit");
        assert_eq!(error, SkillActivationSelectionError::ContextBudgetExceeded);
    }

    #[tokio::test]
    async fn selector_maps_ambiguous_activation_to_context_error() {
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
        selectable
            .record_user_message(
                context.scope.clone(),
                accepted_message_ref(&context),
                "/code-review this PR",
            )
            .expect("record message");

        let error = selectable
            .load_skill_context_candidates(&context)
            .await
            .expect_err("ambiguous activation should fail");

        assert!(matches!(
            error,
            HostSkillContextBuildError::AmbiguousSkill { .. }
        ));
    }

    #[tokio::test]
    async fn selector_extracts_explicit_mentions_after_multibyte_text() {
        let source = Arc::new(StaticSkillBundleSource::new(vec![(
            SkillSourceKind::User,
            "code-review",
            &skill_md("code-review", "Review code", &[], "CODE_REVIEW_SENTINEL"),
        )]));
        let selectable =
            SelectableSkillContextSource::new(source, SkillActivationSelectorConfig::default());
        let context = run_context().await;
        selectable
            .record_user_message(
                context.scope.clone(),
                accepted_message_ref(&context),
                "café/code-review this PR",
            )
            .expect("record slash message");

        let selected = selectable
            .load_skill_context_candidates(&context)
            .await
            .expect("slash selection succeeds");
        assert_eq!(selected.len(), 1);

        selectable
            .record_user_message(
                context.scope.clone(),
                accepted_message_ref(&context),
                "café$code-review this PR",
            )
            .expect("record dollar message");
        let selected = selectable
            .load_skill_context_candidates(&context)
            .await
            .expect("dollar selection succeeds");
        assert_eq!(selected.len(), 1);
    }

    #[tokio::test]
    async fn selector_reuses_parsed_skill_for_stable_content_hash() {
        let source = Arc::new(ChangingSkillBundleSource::new(
            "code-review",
            skill_md(
                "code-review",
                "Review code",
                &["review"],
                "CODE_REVIEW_SENTINEL",
            ),
            "not valid skill md".to_string(),
        ));
        let selectable = SelectableSkillContextSource::new(
            source.clone(),
            SkillActivationSelectorConfig::default(),
        );
        let context = run_context().await;

        for _ in 0..2 {
            selectable
                .record_user_message(
                    context.scope.clone(),
                    accepted_message_ref(&context),
                    "please review this",
                )
                .expect("record message");
            let selected = selectable
                .load_skill_context_candidates(&context)
                .await
                .expect("cached selection succeeds");
            assert_eq!(selected.len(), 1);
        }

        assert_eq!(
            source.reads.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "cache avoids reparsing but still reads the current bundle content"
        );
    }

    #[test]
    fn activation_cache_is_bounded_under_skill_churn() {
        let source = Arc::new(StaticSkillBundleSource::new(Vec::new()));
        let selectable =
            SelectableSkillContextSource::new(source, SkillActivationSelectorConfig::default());

        for index in 0..=MAX_ACTIVATION_CACHE_ENTRIES {
            let name = format!("skill-{index}");
            let descriptor = SkillBundleDescriptor::new(
                SkillBundleId::new(SkillSourceKind::User, &name).unwrap(),
                Some(SkillTrust::Trusted),
                Some(SkillVisibility::Visible),
            );
            selectable
                .activation_candidate_from_skill_md(
                    &descriptor,
                    skill_md(&name, "Review code", &["review"], "CODE_REVIEW_SENTINEL")
                        .into_bytes(),
                )
                .expect("skill parses");
        }

        let cache_len = selectable.activation_cache.lock().unwrap().len();
        assert!(
            cache_len <= MAX_ACTIVATION_CACHE_ENTRIES,
            "activation cache must stay bounded"
        );
    }

    #[tokio::test]
    async fn selector_reports_source_unavailable_on_bundle_list_error() {
        let source = Arc::new(ErroringListSkillBundleSource::new(
            SkillBundleSourceError::SourceUnavailable,
        ));
        let selectable =
            SelectableSkillContextSource::new(source, SkillActivationSelectorConfig::default());
        let context = run_context().await;

        let error = selectable
            .selected_candidates(&context, "review", false)
            .await
            .expect_err("list error should fail closed");
        assert_eq!(error, SkillActivationSelectionError::SourceUnavailable);
    }

    #[tokio::test]
    async fn selector_reports_internal_on_internal_bundle_list_error() {
        let source = Arc::new(ErroringListSkillBundleSource::new(
            SkillBundleSourceError::Internal,
        ));
        let selectable =
            SelectableSkillContextSource::new(source, SkillActivationSelectorConfig::default());
        let context = run_context().await;

        let error = selectable
            .selected_candidates(&context, "review", false)
            .await
            .expect_err("internal error should fail closed");
        assert_eq!(error, SkillActivationSelectionError::Internal);
    }

    #[tokio::test]
    async fn selector_reports_parse_failed_on_invalid_skill_md() {
        let source = Arc::new(StaticSkillBundleSource {
            descriptors: vec![SkillBundleDescriptor::new(
                SkillBundleId::new(SkillSourceKind::User, "bad-helper").unwrap(),
                Some(SkillTrust::Trusted),
                Some(SkillVisibility::Visible),
            )],
            files: HashMap::from([(
                (SkillSourceKind::User, "bad-helper".to_string()),
                b"not valid skill md".to_vec(),
            )]),
        });
        let selectable =
            SelectableSkillContextSource::new(source, SkillActivationSelectorConfig::default());
        let context = run_context().await;

        let error = selectable
            .selected_candidates(&context, "bad helper", false)
            .await
            .expect_err("invalid skill md should fail closed");
        assert_eq!(error, SkillActivationSelectionError::ParseFailed);
    }

    #[tokio::test]
    async fn selector_reports_trust_missing_on_descriptor_without_trust() {
        let source = Arc::new(StaticSkillBundleSource {
            descriptors: vec![SkillBundleDescriptor::new(
                SkillBundleId::new(SkillSourceKind::User, "code-review").unwrap(),
                None,
                Some(SkillVisibility::Visible),
            )],
            files: HashMap::new(),
        });
        let selectable =
            SelectableSkillContextSource::new(source, SkillActivationSelectorConfig::default());
        let context = run_context().await;

        let error = selectable
            .selected_candidates(&context, "review", false)
            .await
            .expect_err("missing trust should fail closed");
        assert_eq!(error, SkillActivationSelectionError::TrustDataMissing);
    }

    #[tokio::test]
    async fn selector_reports_visibility_missing_on_descriptor_without_visibility() {
        let source = Arc::new(StaticSkillBundleSource {
            descriptors: vec![SkillBundleDescriptor::new(
                SkillBundleId::new(SkillSourceKind::User, "code-review").unwrap(),
                Some(SkillTrust::Trusted),
                None,
            )],
            files: HashMap::new(),
        });
        let selectable =
            SelectableSkillContextSource::new(source, SkillActivationSelectorConfig::default());
        let context = run_context().await;

        let error = selectable
            .selected_candidates(&context, "review", false)
            .await
            .expect_err("missing visibility should fail closed");
        assert_eq!(error, SkillActivationSelectionError::VisibilityDataMissing);
    }

    #[tokio::test]
    async fn execution_message_capture_stores_and_consumes_plan_once() {
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

        selectable
            .record_user_message_for_execution(
                context.scope.clone(),
                accepted_message_ref(&context),
                "please review this",
            )
            .expect("record message");
        let selected = selectable
            .load_skill_context_candidates(&context)
            .await
            .expect("selection succeeds");
        assert_eq!(selected.len(), 1);
        let plan = selectable
            .take_activation_plan_for_run(&context.scope, context.run_id)
            .expect("take captured plan")
            .expect("plan should be captured");
        assert_eq!(plan.plan.selection.activations.len(), 1);
        assert!(
            selectable
                .take_activation_plan_for_run(&context.scope, context.run_id)
                .expect("take is repeatable")
                .is_none(),
            "captured plans are single-consumer"
        );
    }

    #[tokio::test]
    async fn clear_accepted_message_removes_pending_execution_capture() {
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
        let captured_a = run_context_for("thread-a", "msg:a-captured").await;
        let pending_a = run_context_for("thread-a", "msg:a-pending").await;
        let captured_b = run_context_for("thread-b", "msg:b-captured").await;

        selectable
            .record_user_message_for_execution(
                captured_a.scope.clone(),
                accepted_message_ref(&captured_a),
                "please review this",
            )
            .expect("record captured scope a message");
        selectable
            .load_skill_context_candidates(&captured_a)
            .await
            .expect("scope a selection succeeds");

        selectable
            .record_user_message_for_execution(
                pending_a.scope.clone(),
                accepted_message_ref(&pending_a),
                "please review this",
            )
            .expect("record pending scope a message");

        selectable
            .record_user_message_for_execution(
                captured_b.scope.clone(),
                accepted_message_ref(&captured_b),
                "please review this",
            )
            .expect("record captured scope b message");
        selectable
            .load_skill_context_candidates(&captured_b)
            .await
            .expect("scope b selection succeeds");

        selectable
            .clear_accepted_message(&pending_a.scope, &accepted_message_ref(&pending_a))
            .expect("clear pending scope a message");

        assert!(
            selectable
                .take_activation_plan_for_run(&captured_a.scope, captured_a.run_id)
                .expect("take cleared scope a plan")
                .is_some(),
            "clearing a pending message must not remove an already captured plan"
        );
        assert!(
            selectable
                .load_skill_context_candidates(&pending_a)
                .await
                .expect("pending scope a selection after clear succeeds")
                .is_empty(),
            "clearing the accepted message removes its pending execution capture"
        );
        assert!(
            selectable
                .take_activation_plan_for_run(&captured_b.scope, captured_b.run_id)
                .expect("take scope b plan")
                .is_some(),
            "clearing one accepted message must not remove another scope's plan"
        );
    }

    #[test]
    fn explicit_name_extraction_matches_valid_dotted_skill_names() {
        assert_eq!(
            extract_explicit_skill_names("please use /skill.v2"),
            vec!["skill.v2".to_string()]
        );
        assert!(ironclaw_skills::validate_skill_name("skill.v2"));
    }
}
