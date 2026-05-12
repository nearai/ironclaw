use async_trait::async_trait;
use ironclaw_skills::{ParsedSkill, SkillTrust, parse_skill_md};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, InstalledSkillSnapshot, LoopContextSnippet,
    LoopRunContext, MAX_UNTRUSTED_CONTEXT_SUMMARY_BYTES, SkillContextError, SkillContextService,
    SkillContextSource, SkillRunSnapshot, SkillTrustLevel, SkillVisibility, UntrustedContextKind,
    untrusted_context_summary,
};
pub(crate) use ironclaw_turns::run_profile::{
    is_skill_snippet_model_message_ref as is_snippet_model_message_ref,
    skill_snippet_model_message_ref as snippet_model_message_ref,
};
use thiserror::Error;

/// Host-owned source for production skill context candidates.
///
/// Implementations own storage/policy lookups. This trait intentionally returns
/// host-approved trust/visibility decisions plus raw SKILL.md content only for
/// visible candidates so `ironclaw_turns` remains a snapshot-only loop boundary.
#[async_trait]
pub trait HostSkillContextSource: Send + Sync {
    async fn load_skill_context_candidates(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Vec<HostSkillContextCandidate>, HostSkillContextBuildError>;
}

/// One host-approved skill candidate before parsing and snapshot conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSkillContextCandidate {
    /// Raw SKILL.md content from the production skill source.
    ///
    /// Hidden/denied candidates may omit raw content; they are policy-filtered
    /// before parsing so invisible skills cannot fail prompt construction via
    /// malformed prompt files.
    pub skill_md: Option<String>,
    /// Host-approved trust state. `None` fails the build closed.
    pub trust: Option<SkillTrust>,
    /// Host-approved model visibility. `None` fails the build closed.
    pub visibility: Option<SkillVisibility>,
    /// Optional deterministic ordering key. Defaults to parsed skill name.
    pub ordering_key: Option<String>,
}

impl HostSkillContextCandidate {
    pub fn new(
        skill_md: impl Into<String>,
        trust: Option<SkillTrust>,
        visibility: Option<SkillVisibility>,
    ) -> Self {
        Self {
            skill_md: Some(skill_md.into()),
            trust,
            visibility,
            ordering_key: None,
        }
    }

    pub fn unavailable(trust: Option<SkillTrust>, visibility: Option<SkillVisibility>) -> Self {
        Self {
            skill_md: None,
            trust,
            visibility,
            ordering_key: None,
        }
    }

    pub fn with_ordering_key(mut self, ordering_key: impl Into<String>) -> Self {
        self.ordering_key = Some(ordering_key.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HostSkillContextBuildError {
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
        let kind = match self {
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

pub async fn build_skill_instruction_snippets(
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
        let skill_md = candidate
            .skill_md
            .ok_or(HostSkillContextBuildError::SourceUnavailable)?;
        let parsed =
            parse_skill_md(&skill_md).map_err(|_| HostSkillContextBuildError::ParseFailed)?;
        entries.push(parsed_skill_to_snapshot_entry(
            parsed,
            trust,
            visibility,
            candidate.ordering_key,
        )?);
    }

    Ok(SkillRunSnapshot::from_entries(entries))
}

fn parsed_skill_to_snapshot_entry(
    parsed: ParsedSkill,
    trust: SkillTrust,
    visibility: SkillVisibility,
    ordering_key: Option<String>,
) -> Result<InstalledSkillSnapshot, HostSkillContextBuildError> {
    let name = parsed.manifest.name;
    let trust = skill_trust_level(trust);
    let safe_description = match trust {
        SkillTrustLevel::Installed => untrusted_context_summary(
            UntrustedContextKind::Skill,
            &parsed.manifest.description,
            MAX_UNTRUSTED_CONTEXT_SUMMARY_BYTES,
        )
        .ok_or(HostSkillContextBuildError::UnsafeModelVisibleContent)?
        .as_str()
        .to_string(),
        SkillTrustLevel::Trusted => parsed.manifest.description,
    };
    let prompt_content = match trust {
        SkillTrustLevel::Installed => {
            let summary = untrusted_context_summary(
                UntrustedContextKind::Skill,
                &parsed.prompt_content,
                MAX_UNTRUSTED_CONTEXT_SUMMARY_BYTES,
            )
            .ok_or(HostSkillContextBuildError::UnsafeModelVisibleContent)?;
            Some(summary.as_str().to_string())
        }
        SkillTrustLevel::Trusted => Some(parsed.prompt_content),
    };
    Ok(InstalledSkillSnapshot {
        ordering_key: ordering_key.unwrap_or_else(|| name.clone()),
        name,
        trust,
        visibility,
        prompt_content,
        safe_description,
    })
}

fn skill_trust_level(trust: SkillTrust) -> SkillTrustLevel {
    match trust {
        SkillTrust::Installed => SkillTrustLevel::Installed,
        SkillTrust::Trusted => SkillTrustLevel::Trusted,
    }
}

fn skill_context_error_to_host_error(error: SkillContextError) -> AgentLoopHostError {
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
