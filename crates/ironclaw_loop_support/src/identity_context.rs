use async_trait::async_trait;
use ironclaw_memory::DEFAULT_PROMPT_PROTECTED_PATHS;
use ironclaw_turns::{
    LoopMessageRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, LoopContextMessage, LoopRunContext,
        PersonalContextPolicy, PromptMode,
    },
};
use thiserror::Error;

const DEFAULT_IDENTITY_TOKEN_CEILING: u32 = 8_000;
const IDENTITY_REF_PREFIX: &str = "msg:identity.";
const LOOP_SYSTEM_ROLE: &str = "system";

#[async_trait]
pub trait HostIdentityContextSource: Send + Sync {
    async fn load_identity_candidates(
        &self,
        run_context: &LoopRunContext,
        mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError>;

    async fn resolve_identity_message_content(
        &self,
        _run_context: &LoopRunContext,
        _message_ref: &LoopMessageRef,
    ) -> Result<Option<HostIdentityMessageContent>, HostIdentityContextBuildError> {
        Ok(None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostIdentityContextCandidate {
    pub name: IdentityFileName,
    pub message_ref: Option<LoopMessageRef>,
    pub safe_summary: String,
    pub trust_level: IdentityTrustLevel,
    pub applies_when: IdentityApplicability,
    model_visible_bytes: usize,
}

impl HostIdentityContextCandidate {
    pub fn new_trusted(
        name: IdentityFileName,
        message_ref: LoopMessageRef,
        safe_summary: String,
        applies_when: IdentityApplicability,
        model_visible_bytes: usize,
    ) -> Self {
        Self {
            name,
            message_ref: Some(message_ref),
            safe_summary,
            trust_level: IdentityTrustLevel::Trusted,
            applies_when,
            model_visible_bytes,
        }
    }

    pub fn new_installed_summary_only(
        name: IdentityFileName,
        safe_summary: String,
        applies_when: IdentityApplicability,
    ) -> Self {
        Self {
            name,
            message_ref: None,
            safe_summary,
            trust_level: IdentityTrustLevel::Installed,
            applies_when,
            model_visible_bytes: 0,
        }
    }

    fn estimated_model_visible_bytes(&self) -> usize {
        match self.trust_level {
            IdentityTrustLevel::Trusted => self.model_visible_bytes,
            IdentityTrustLevel::Installed => self.safe_summary.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityTrustLevel {
    Installed,
    Trusted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityApplicability {
    Always,
    OnCodeAct,
    OnPersonalContextAllowed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostIdentityMessageContent {
    pub name: IdentityFileName,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityBudget {
    pub token_ceiling: u32,
}

impl IdentityBudget {
    pub fn new(token_ceiling: u32) -> Result<Self, HostIdentityContextBuildError> {
        if token_ceiling == 0 {
            return Err(HostIdentityContextBuildError::BudgetMisconfigured);
        }
        Ok(Self { token_ceiling })
    }
}

impl Default for IdentityBudget {
    fn default() -> Self {
        Self {
            token_ceiling: DEFAULT_IDENTITY_TOKEN_CEILING,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct IdentityFileName(String);

impl IdentityFileName {
    pub fn new(name: impl AsRef<str>) -> Result<Self, HostIdentityContextBuildError> {
        let requested = canonicalize_path_for_match(name.as_ref())?;
        let Some(canonical) = DEFAULT_PROMPT_PROTECTED_PATHS
            .iter()
            .copied()
            .find(|path| canonicalize_path_for_match(path).is_ok_and(|path| path == requested))
        else {
            return Err(HostIdentityContextBuildError::UnknownIdentityFile);
        };
        Ok(Self(canonical.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for IdentityFileName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HostIdentityContextBuildError {
    #[error("identity context source unavailable")]
    SourceUnavailable,
    #[error("identity context file is not prompt-protected")]
    UnknownIdentityFile,
    #[error("identity context path is invalid")]
    InvalidIdentityFile,
    /// Reserved for a future hard-limit mode. Currently, `build_identity_messages_for_run`
    /// truncates silently on budget overflow rather than returning this error.
    #[error("identity context budget exceeded")]
    ContextBudgetExceeded,
    #[error("identity context budget misconfigured")]
    BudgetMisconfigured,
    #[error("identity context internal error")]
    Internal,
}

impl HostIdentityContextBuildError {
    pub fn into_host_error(self) -> AgentLoopHostError {
        let kind = match self {
            Self::SourceUnavailable => AgentLoopHostErrorKind::Unavailable,
            Self::UnknownIdentityFile | Self::InvalidIdentityFile => {
                AgentLoopHostErrorKind::PolicyDenied
            }
            Self::ContextBudgetExceeded => AgentLoopHostErrorKind::BudgetExceeded,
            Self::BudgetMisconfigured | Self::Internal => AgentLoopHostErrorKind::Internal,
        };
        AgentLoopHostError::new(kind, self.to_string())
    }
}

pub async fn build_identity_messages(
    source: &(dyn HostIdentityContextSource + Send + Sync),
    run_context: &LoopRunContext,
    mode: PromptMode,
    budget: IdentityBudget,
) -> Result<Vec<LoopContextMessage>, AgentLoopHostError> {
    let candidates = source
        .load_identity_candidates(run_context, mode)
        .await
        .map_err(HostIdentityContextBuildError::into_host_error)?;
    build_identity_messages_for_run(&candidates, run_context, mode, budget)
}

fn filter_identity_candidates_for_run(
    candidates: &[HostIdentityContextCandidate],
    run_context: &LoopRunContext,
) -> Vec<HostIdentityContextCandidate> {
    candidates
        .iter()
        .filter(|candidate| identity_candidate_allowed_for_run(candidate, run_context))
        .cloned()
        .collect()
}

fn identity_candidate_allowed_for_run(
    candidate: &HostIdentityContextCandidate,
    run_context: &LoopRunContext,
) -> bool {
    match candidate.applies_when {
        IdentityApplicability::Always | IdentityApplicability::OnCodeAct => true,
        IdentityApplicability::OnPersonalContextAllowed => {
            match run_context.resolved_run_profile.personal_context_policy {
                PersonalContextPolicy::Excluded => false,
                PersonalContextPolicy::Allowed => true,
            }
        }
    }
}

pub fn build_identity_messages_for_run(
    candidates: &[HostIdentityContextCandidate],
    run_context: &LoopRunContext,
    mode: PromptMode,
    budget: IdentityBudget,
) -> Result<Vec<LoopContextMessage>, AgentLoopHostError> {
    build_identity_messages_from_candidates(
        &filter_identity_candidates_for_run(candidates, run_context),
        mode,
        budget,
    )
}

fn build_identity_messages_from_candidates(
    candidates: &[HostIdentityContextCandidate],
    mode: PromptMode,
    budget: IdentityBudget,
) -> Result<Vec<LoopContextMessage>, AgentLoopHostError> {
    let mut out = Vec::with_capacity(candidates.len());
    let mut used = 0u32;
    for candidate in candidates {
        if !applies(candidate.applies_when, mode) {
            continue;
        }
        let cost = estimate_cost(candidate);
        if used.saturating_add(cost) > budget.token_ceiling {
            break;
        }
        used = used.saturating_add(cost);
        out.push(LoopContextMessage {
            message_ref: candidate.message_ref.clone(),
            role: LOOP_SYSTEM_ROLE.to_string(),
            safe_summary: candidate.safe_summary.clone(),
        });
    }
    Ok(out)
}

pub fn identity_message_ref(
    name: &IdentityFileName,
    content: &str,
) -> Result<LoopMessageRef, AgentLoopHostError> {
    let hash = stable_identity_hash(name.as_str(), content);
    let slug = identity_ref_slug(name.as_str());
    LoopMessageRef::new(format!("{IDENTITY_REF_PREFIX}{slug}.{hash:016x}")).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "identity message ref could not be represented",
        )
    })
}

pub fn is_identity_model_message_ref(message_ref: &LoopMessageRef) -> bool {
    message_ref.as_str().starts_with(IDENTITY_REF_PREFIX)
}

fn applies(applicability: IdentityApplicability, mode: PromptMode) -> bool {
    match applicability {
        IdentityApplicability::Always => true,
        IdentityApplicability::OnCodeAct => mode == PromptMode::CodeAct,
        IdentityApplicability::OnPersonalContextAllowed => true,
    }
}

fn estimate_cost(candidate: &HostIdentityContextCandidate) -> u32 {
    let bytes = candidate
        .estimated_model_visible_bytes()
        .saturating_add(candidate.name.as_str().len());
    ((bytes as u32).saturating_add(3) / 4).max(1)
}

fn canonicalize_path_for_match(path: &str) -> Result<String, HostIdentityContextBuildError> {
    let trimmed = path.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('/')
        || trimmed.contains('\\')
        || trimmed
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(HostIdentityContextBuildError::InvalidIdentityFile);
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn identity_ref_slug(path: &str) -> String {
    path.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '.'
            }
        })
        .collect::<String>()
        .trim_matches('.')
        .to_string()
}

const FNV_IDENTITY_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_IDENTITY_PRIME: u64 = 0x00000100000001B3;

fn stable_identity_hash(name: &str, content: &str) -> u64 {
    // FNV-1a: non-cryptographic, collision-safe for content addressing within a run.
    // Not suitable for security purposes — used only to detect identity file drift
    // between candidate load and model resolution within the same process.
    let mut hash = FNV_IDENTITY_OFFSET;
    for &byte in name.as_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_IDENTITY_PRIME);
    }
    hash ^= 0xFF;
    hash = hash.wrapping_mul(FNV_IDENTITY_PRIME);
    for &byte in content.as_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_IDENTITY_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use ironclaw_host_api::{TenantId, ThreadId};
    use ironclaw_turns::{
        RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId, TurnScope,
        run_profile::{InMemoryRunProfileResolver, LoopRunContext},
    };

    use super::*;

    #[tokio::test]
    async fn filters_by_applies_when() {
        let context = run_context().await;
        let source = StaticIdentitySource::new(vec![
            trusted("AGENTS.md", "agent", IdentityApplicability::Always),
            trusted("TOOLS.md", "tools", IdentityApplicability::OnCodeAct),
        ]);
        let messages = build_identity_messages(
            &source,
            &context,
            PromptMode::TextOnly,
            IdentityBudget::default(),
        )
        .await
        .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].safe_summary,
            "identity file AGENTS.md available"
        );
    }

    #[tokio::test]
    async fn respects_budget() {
        let context = run_context().await;
        let source = StaticIdentitySource::new(vec![
            installed("AGENTS.md", "a".repeat(100), IdentityApplicability::Always),
            installed("SOUL.md", "b".repeat(100), IdentityApplicability::Always),
            installed("USER.md", "c".repeat(100), IdentityApplicability::Always),
        ]);
        let messages = build_identity_messages(
            &source,
            &context,
            PromptMode::TextOnly,
            IdentityBudget::new(60).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(messages.len(), 2);
    }

    #[tokio::test]
    async fn trusted_budget_uses_model_visible_content_estimate() {
        let context = run_context().await;
        let source = StaticIdentitySource::new(vec![trusted(
            "AGENTS.md",
            "trusted content ".repeat(100),
            IdentityApplicability::Always,
        )]);
        let messages = build_identity_messages(
            &source,
            &context,
            PromptMode::TextOnly,
            IdentityBudget::new(60).unwrap(),
        )
        .await
        .unwrap();
        assert!(messages.is_empty());
    }

    #[tokio::test]
    async fn installed_trust_summary_only() {
        let context = run_context().await;
        let source = StaticIdentitySource::new(vec![
            HostIdentityContextCandidate::new_installed_summary_only(
                IdentityFileName::new("AGENTS.md").unwrap(),
                "installed identity summary".to_string(),
                IdentityApplicability::Always,
            ),
        ]);
        let messages = build_identity_messages(
            &source,
            &context,
            PromptMode::TextOnly,
            IdentityBudget::default(),
        )
        .await
        .unwrap();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].message_ref.is_none());
        assert_eq!(messages[0].safe_summary, "installed identity summary");
    }

    #[tokio::test]
    async fn ordering_is_deterministic() {
        let context = run_context().await;
        let source = StaticIdentitySource::new(vec![
            trusted("SOUL.md", "soul", IdentityApplicability::Always),
            trusted("AGENTS.md", "agent", IdentityApplicability::Always),
        ]);
        let first = build_identity_messages(
            &source,
            &context,
            PromptMode::TextOnly,
            IdentityBudget::default(),
        )
        .await
        .unwrap();
        let second = build_identity_messages(
            &source,
            &context,
            PromptMode::TextOnly,
            IdentityBudget::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap()
        );
        assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    }

    fn trusted(
        name: &str,
        summary_seed: impl Into<String>,
        applies_when: IdentityApplicability,
    ) -> HostIdentityContextCandidate {
        let name = IdentityFileName::new(name).unwrap();
        let summary_seed = summary_seed.into();
        let content_ref = identity_message_ref(&name, &summary_seed).unwrap();
        HostIdentityContextCandidate::new_trusted(
            name.clone(),
            content_ref,
            format!("identity file {} available", name.as_str()),
            applies_when,
            summary_seed.len(),
        )
    }

    fn installed(
        name: &str,
        safe_summary: impl Into<String>,
        applies_when: IdentityApplicability,
    ) -> HostIdentityContextCandidate {
        HostIdentityContextCandidate::new_installed_summary_only(
            IdentityFileName::new(name).unwrap(),
            safe_summary.into(),
            applies_when,
        )
    }

    struct StaticIdentitySource {
        candidates: Vec<HostIdentityContextCandidate>,
        calls: Arc<AtomicUsize>,
    }

    impl StaticIdentitySource {
        fn new(candidates: Vec<HostIdentityContextCandidate>) -> Self {
            Self {
                candidates,
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait]
    impl HostIdentityContextSource for StaticIdentitySource {
        async fn load_identity_candidates(
            &self,
            _run_context: &LoopRunContext,
            _mode: PromptMode,
        ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.candidates.clone())
        }
    }

    async fn run_context() -> LoopRunContext {
        let resolved_run_profile = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        let scope = TurnScope::new(
            TenantId::new("tenant-identity-context").unwrap(),
            None,
            None,
            ThreadId::new("thread-identity-context").unwrap(),
        );
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }
}
