//! Provider-neutral learning review vocabulary.
//!
//! The host creates these candidate records after a successful run. Memory
//! providers do not receive a write until a later admission phase promotes a
//! candidate.

use async_trait::async_trait;
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, UserId},
    turn::TurnRunId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_LEARNING_MEMORY_PROPOSALS: usize = 4;
pub const MAX_LEARNING_PROPOSAL_BYTES: usize = 512;
pub const MAX_LEARNING_SOURCE_REFERENCES: usize = 16;
pub const MAX_LEARNING_SKILL_REASON_BYTES: usize = 512;
pub const MAX_LEARNING_UNRESOLVED_PROPOSALS: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLearningProposalKind {
    Fact,
    Preference,
    Procedure,
    Episode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningExplicitness {
    Explicit,
    Inferred,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryLearningProposal {
    pub kind: MemoryLearningProposalKind,
    pub content: String,
    pub source_message_indices: Vec<u16>,
    pub confidence_basis_points: u16,
    pub explicitness: LearningExplicitness,
    pub tainted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningAction {
    Skip,
    Distill,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningDecision {
    pub action: LearningAction,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub source_message_indices: Vec<u16>,
    /// Host-derived taint: true when the decision cites tool or subagent data.
    /// Tainted decisions must never be promoted to an installed skill.
    #[serde(default)]
    pub tainted: bool,
}

impl LearningDecision {
    pub fn skip() -> Self {
        Self {
            action: LearningAction::Skip,
            reason: None,
            source_message_indices: Vec::new(),
            tainted: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningReview {
    #[serde(default)]
    pub memory: Vec<MemoryLearningProposal>,
    pub skill: LearningDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LearningReviewValidationError {
    #[error("too many memory proposals")]
    TooManyMemoryProposals,
    #[error("memory proposal is empty")]
    EmptyMemoryProposal,
    #[error("memory proposal is too large")]
    MemoryProposalTooLarge,
    #[error("memory proposal confidence is invalid")]
    InvalidConfidence,
    #[error("learning source reference is missing")]
    MissingSourceReference,
    #[error("too many learning source references")]
    TooManySourceReferences,
    #[error("learning source references are invalid")]
    InvalidSourceReferences,
    #[error("skill learning decision is invalid")]
    InvalidSkillDecision,
    #[error("skill learning reason is too large")]
    SkillReasonTooLarge,
}

impl LearningReview {
    pub fn validate(&self) -> Result<(), LearningReviewValidationError> {
        if self.memory.len() > MAX_LEARNING_MEMORY_PROPOSALS {
            return Err(LearningReviewValidationError::TooManyMemoryProposals);
        }
        for proposal in &self.memory {
            if proposal.content.trim().is_empty() {
                return Err(LearningReviewValidationError::EmptyMemoryProposal);
            }
            if proposal.content.len() > MAX_LEARNING_PROPOSAL_BYTES {
                return Err(LearningReviewValidationError::MemoryProposalTooLarge);
            }
            if proposal.confidence_basis_points > 10_000 {
                return Err(LearningReviewValidationError::InvalidConfidence);
            }
            validate_source_references(&proposal.source_message_indices)?;
        }

        match self.skill.action {
            LearningAction::Skip => {
                if self.skill.reason.is_some() || !self.skill.source_message_indices.is_empty() {
                    return Err(LearningReviewValidationError::InvalidSkillDecision);
                }
            }
            LearningAction::Distill => {
                let Some(reason) = self.skill.reason.as_deref() else {
                    return Err(LearningReviewValidationError::InvalidSkillDecision);
                };
                if reason.trim().is_empty() {
                    return Err(LearningReviewValidationError::InvalidSkillDecision);
                }
                if reason.len() > MAX_LEARNING_SKILL_REASON_BYTES {
                    return Err(LearningReviewValidationError::SkillReasonTooLarge);
                }
                validate_source_references(&self.skill.source_message_indices)?;
            }
        }
        Ok(())
    }
}

fn validate_source_references(indices: &[u16]) -> Result<(), LearningReviewValidationError> {
    if indices.is_empty() {
        return Err(LearningReviewValidationError::MissingSourceReference);
    }
    if indices.len() > MAX_LEARNING_SOURCE_REFERENCES {
        return Err(LearningReviewValidationError::TooManySourceReferences);
    }
    if indices.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(LearningReviewValidationError::InvalidSourceReferences);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningScope {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
}

impl LearningScope {
    pub fn new(
        tenant_id: TenantId,
        user_id: UserId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            tenant_id,
            user_id,
            agent_id,
            project_id,
        }
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn user_id(&self) -> &UserId {
        &self.user_id
    }

    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    pub fn project_id(&self) -> Option<&ProjectId> {
        self.project_id.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningCandidateStatus {
    Candidate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct LearningIdempotencyKey(String);

impl<'de> Deserialize<'de> for LearningIdempotencyKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let Some(run_id) = value.strip_prefix("learning-review:") else {
            return Err(serde::de::Error::custom("invalid learning idempotency key"));
        };
        TurnRunId::parse(run_id)
            .map(|_| Self(value))
            .map_err(|_| serde::de::Error::custom("invalid learning idempotency key"))
    }
}

impl LearningIdempotencyKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningReviewRecord {
    pub run_id: TurnRunId,
    pub idempotency_key: LearningIdempotencyKey,
    pub scope: LearningScope,
    pub status: LearningCandidateStatus,
    pub review: LearningReview,
}

impl LearningReviewRecord {
    pub fn new(
        run_id: TurnRunId,
        scope: LearningScope,
        review: LearningReview,
    ) -> Result<Self, LearningReviewValidationError> {
        review.validate()?;
        Ok(Self {
            run_id,
            idempotency_key: LearningIdempotencyKey(format!("learning-review:{run_id}")),
            scope,
            status: LearningCandidateStatus::Candidate,
            review,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LearningCandidateInsert {
    Created,
    AlreadyExists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LearningCandidateStoreError {
    #[error("learning candidate data is invalid")]
    InvalidData,
    #[error("learning candidate store is unavailable")]
    Unavailable,
}

#[async_trait]
pub trait LearningCandidateStore: Send + Sync {
    async fn insert_if_absent(
        &self,
        record: &LearningReviewRecord,
    ) -> Result<LearningCandidateInsert, LearningCandidateStoreError>;

    async fn get(
        &self,
        scope: &LearningScope,
        run_id: TurnRunId,
    ) -> Result<Option<LearningReviewRecord>, LearningCandidateStoreError>;

    async fn list_unresolved(
        &self,
        scope: &LearningScope,
    ) -> Result<Vec<LearningReviewRecord>, LearningCandidateStoreError>;
}
