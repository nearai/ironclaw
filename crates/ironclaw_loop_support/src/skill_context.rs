use async_trait::async_trait;
use ironclaw_skills::{ParsedSkill, SkillTrust, parse_skill_md};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, InstalledSkillSnapshot, LoopContextSnippet,
    LoopRunContext, SkillActivationState, SkillContextError, SkillContextService,
    SkillContextSource, SkillRunSnapshot, SkillTrustLevel, SkillVisibility,
};
pub(crate) use ironclaw_turns::run_profile::{
    is_skill_snippet_model_message_ref as is_snippet_model_message_ref,
    skill_snippet_model_message_ref as snippet_model_message_ref,
};
use thiserror::Error;

use crate::SkillSourceKind;

/// Host-owned source for production skill context candidates.
///
/// Implementations own storage/policy lookups. This trait intentionally returns
/// host-approved trust/visibility decisions plus either safe discovery metadata
/// or raw SKILL.md content for loaded candidates so `ironclaw_turns` remains a
/// snapshot-only loop boundary.
#[async_trait]
pub trait HostSkillContextSource: Send + Sync {
    async fn load_skill_context_candidates(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Vec<HostSkillContextCandidate>, HostSkillContextBuildError>;
}

/// Model-visible payload for one host-approved skill candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostSkillContextCandidatePayload {
    /// Raw SKILL.md content for a skill that was selected and loaded.
    LoadedSkillMd(String),
    /// Safe discovery metadata for a skill that has not been loaded.
    DiscoverableMetadata {
        name: String,
        safe_description: String,
    },
    /// Policy metadata for a skill that is not model-visible.
    ///
    /// Host sources should only emit this with non-visible visibility states.
    /// A visible unavailable candidate violates the host-source contract and
    /// fails closed during snapshot construction.
    Unavailable,
}

/// One host-approved skill candidate before parsing and snapshot conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSkillContextCandidate {
    /// Candidate payload. Its shape determines whether the skill is loaded or
    /// only discoverable; invalid cross-field combinations are not representable.
    pub payload: HostSkillContextCandidatePayload,
    /// Host-approved trust state. `None` fails the build closed.
    pub trust: Option<SkillTrust>,
    /// Host-approved model visibility. `None` fails the build closed.
    pub visibility: Option<SkillVisibility>,
    /// Optional deterministic ordering key. Defaults to parsed skill name.
    pub ordering_key: Option<String>,
}

impl HostSkillContextCandidate {
    pub fn loaded(
        skill_md: impl Into<String>,
        trust: Option<SkillTrust>,
        visibility: Option<SkillVisibility>,
    ) -> Self {
        Self {
            payload: HostSkillContextCandidatePayload::LoadedSkillMd(skill_md.into()),
            trust,
            visibility,
            ordering_key: None,
        }
    }

    pub fn unavailable(trust: Option<SkillTrust>, visibility: Option<SkillVisibility>) -> Self {
        Self {
            payload: HostSkillContextCandidatePayload::Unavailable,
            trust,
            visibility,
            ordering_key: None,
        }
    }

    pub fn discoverable(
        name: impl Into<String>,
        safe_description: impl Into<String>,
        trust: Option<SkillTrust>,
        visibility: Option<SkillVisibility>,
    ) -> Self {
        Self {
            payload: HostSkillContextCandidatePayload::DiscoverableMetadata {
                name: name.into(),
                safe_description: safe_description.into(),
            },
            trust,
            visibility,
            ordering_key: None,
        }
    }

    pub fn with_ordering_key(mut self, ordering_key: impl Into<String>) -> Self {
        self.ordering_key = Some(ordering_key.into());
        self
    }

    pub fn loaded_skill_md(&self) -> Option<&str> {
        match &self.payload {
            HostSkillContextCandidatePayload::LoadedSkillMd(skill_md) => Some(skill_md),
            HostSkillContextCandidatePayload::DiscoverableMetadata { .. }
            | HostSkillContextCandidatePayload::Unavailable => None,
        }
    }

    pub fn discoverable_metadata(&self) -> Option<(&str, &str)> {
        match &self.payload {
            HostSkillContextCandidatePayload::DiscoverableMetadata {
                name,
                safe_description,
            } => Some((name, safe_description)),
            HostSkillContextCandidatePayload::LoadedSkillMd(_)
            | HostSkillContextCandidatePayload::Unavailable => None,
        }
    }

    pub fn is_unavailable(&self) -> bool {
        matches!(self.payload, HostSkillContextCandidatePayload::Unavailable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HostSkillContextBuildError {
    #[error("ambiguous skill context activation for '{name}': {sources:?}")]
    AmbiguousSkill {
        name: String,
        sources: Vec<SkillSourceKind>,
    },
    #[error("skill context source unavailable")]
    SourceUnavailable,
    #[error("skill context parse failed")]
    ParseFailed,
    #[error("skill context trust data missing")]
    TrustDataMissing,
    #[error("skill context visibility data missing")]
    VisibilityDataMissing,
    #[error("skill context budget exceeded")]
    ContextBudgetExceeded,
    #[error("skill context unsafe model-visible content")]
    UnsafeModelVisibleContent,
    #[error("skill context budget misconfigured")]
    BudgetMisconfigured,
    #[error("skill context internal error")]
    Internal,
}

impl HostSkillContextBuildError {
    pub fn into_host_error(self) -> AgentLoopHostError {
        let kind = match &self {
            Self::AmbiguousSkill { .. } => AgentLoopHostErrorKind::PolicyDenied,
            Self::SourceUnavailable => AgentLoopHostErrorKind::Unavailable,
            Self::ParseFailed => AgentLoopHostErrorKind::InvalidInvocation,
            Self::TrustDataMissing
            | Self::VisibilityDataMissing
            | Self::UnsafeModelVisibleContent => AgentLoopHostErrorKind::PolicyDenied,
            Self::ContextBudgetExceeded => AgentLoopHostErrorKind::BudgetExceeded,
            Self::BudgetMisconfigured | Self::Internal => AgentLoopHostErrorKind::Internal,
        };
        AgentLoopHostError::new(kind, self.to_string())
    }
}

pub(crate) async fn build_skill_instruction_snippets(
    source: &(dyn HostSkillContextSource + Send + Sync),
    run_context: &LoopRunContext,
) -> Result<Vec<LoopContextSnippet>, AgentLoopHostError> {
    let candidates = source
        .load_skill_context_candidates(run_context)
        .await
        .map_err(HostSkillContextBuildError::into_host_error)?;
    let snapshot = build_skill_run_snapshot(candidates)
        .map_err(HostSkillContextBuildError::into_host_error)?;
    let service = SkillContextService::new(snapshot.clone());
    let snippets = service
        .skill_snippets(&snapshot)
        .await
        .map_err(skill_context_error_to_host_error)?;
    Ok(snippets
        .into_iter()
        .map(|snippet| snippet.into_loop_snippet())
        .collect())
}

pub fn build_skill_run_snapshot(
    candidates: Vec<HostSkillContextCandidate>,
) -> Result<SkillRunSnapshot, HostSkillContextBuildError> {
    if candidates.is_empty() {
        return Ok(SkillRunSnapshot::empty());
    }

    let mut entries = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let trust = candidate
            .trust
            .ok_or(HostSkillContextBuildError::TrustDataMissing)?;
        let visibility = candidate
            .visibility
            .ok_or(HostSkillContextBuildError::VisibilityDataMissing)?;
        if visibility != SkillVisibility::Visible {
            continue;
        }
        match candidate.payload {
            HostSkillContextCandidatePayload::LoadedSkillMd(skill_md) => {
                let parsed = parse_skill_md(&skill_md)
                    .map_err(|_| HostSkillContextBuildError::ParseFailed)?;
                entries.push(parsed_skill_to_snapshot_entry(
                    parsed,
                    trust,
                    visibility,
                    candidate.ordering_key,
                ));
            }
            HostSkillContextCandidatePayload::DiscoverableMetadata {
                name,
                safe_description,
            } => {
                entries.push(discoverable_skill_to_snapshot_entry(
                    name,
                    safe_description,
                    trust,
                    visibility,
                    candidate.ordering_key,
                ));
            }
            HostSkillContextCandidatePayload::Unavailable => {
                return Err(HostSkillContextBuildError::SourceUnavailable);
            }
        }
    }

    Ok(SkillRunSnapshot::from_entries(entries))
}

fn parsed_skill_to_snapshot_entry(
    parsed: ParsedSkill,
    trust: SkillTrust,
    visibility: SkillVisibility,
    ordering_key: Option<String>,
) -> InstalledSkillSnapshot {
    let name = parsed.manifest.name;
    let trust = skill_trust_level(trust);
    let prompt_content = match trust {
        SkillTrustLevel::Installed => None,
        SkillTrustLevel::Trusted => Some(parsed.prompt_content),
    };
    InstalledSkillSnapshot {
        ordering_key: ordering_key.unwrap_or_else(|| name.clone()),
        name,
        trust,
        visibility,
        activation_state: SkillActivationState::Loaded,
        prompt_content,
        safe_description: parsed.manifest.description,
    }
}

fn discoverable_skill_to_snapshot_entry(
    name: String,
    safe_description: String,
    trust: SkillTrust,
    visibility: SkillVisibility,
    ordering_key: Option<String>,
) -> InstalledSkillSnapshot {
    InstalledSkillSnapshot {
        ordering_key: ordering_key.unwrap_or_else(|| name.clone()),
        name,
        trust: skill_trust_level(trust),
        visibility,
        activation_state: SkillActivationState::Discoverable,
        prompt_content: None,
        safe_description,
    }
}

fn skill_trust_level(trust: SkillTrust) -> SkillTrustLevel {
    match trust {
        SkillTrust::Installed => SkillTrustLevel::Installed,
        SkillTrust::Trusted => SkillTrustLevel::Trusted,
    }
}

fn skill_context_error_to_host_error(error: SkillContextError) -> AgentLoopHostError {
    tracing::warn!(
        component = "skill_context",
        operation = "map_context_error",
        error = %error,
        error_debug = ?error,
        "skill context error mapped to safe host error"
    );
    let build_error = match error {
        SkillContextError::TrustDataMissing => HostSkillContextBuildError::TrustDataMissing,
        SkillContextError::VisibilityDataMissing => {
            HostSkillContextBuildError::VisibilityDataMissing
        }
        SkillContextError::ContextBudgetExceeded => {
            HostSkillContextBuildError::ContextBudgetExceeded
        }
        SkillContextError::UnsafeModelVisibleContent => {
            HostSkillContextBuildError::UnsafeModelVisibleContent
        }
        SkillContextError::BudgetMisconfigured => HostSkillContextBuildError::BudgetMisconfigured,
        SkillContextError::InvalidSnapshotVersion | SkillContextError::Internal => {
            HostSkillContextBuildError::Internal
        }
    };
    build_error.into_host_error()
}
