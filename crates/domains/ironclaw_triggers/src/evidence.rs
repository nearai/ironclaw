use async_trait::async_trait;
use ironclaw_host_api::{
    ids::{AgentId, CapabilityId, ProjectId, TenantId, UserId},
    resource::ResourceScope,
    turn::TurnRunId,
};
use serde::{Deserialize, Serialize};

use crate::TriggerRunHistoryStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerCapabilityExecutionStatus {
    Succeeded,
    Failed,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerCapabilityExecutionEvidence {
    pub run_id: TurnRunId,
    pub capability_id: CapabilityId,
    pub status: TriggerCapabilityExecutionStatus,
    pub error_kind: Option<String>,
}

/// Owner scope for reading trigger-run evidence.
///
/// Trigger runs may execute in threads other than the conversation that asks
/// for their status. Keeping thread, mission, and invocation identity out of
/// this type prevents a caller conversation from accidentally filtering away
/// the scheduled run's evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerRunEvidenceScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
}

impl TriggerRunEvidenceScope {
    pub fn from_resource_scope(scope: &ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerRunAssessmentStatus {
    AppearsSuccessful,
    NeedsAttention,
    Unverified,
    RunFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerCapabilityRequirementStatus {
    Succeeded,
    Failed,
    Missing,
    Incomplete,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerCapabilityRequirementAssessment {
    pub capability_id: CapabilityId,
    pub status: TriggerCapabilityRequirementStatus,
    pub error_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerRunAssessment {
    pub status: TriggerRunAssessmentStatus,
    pub capabilities: Vec<TriggerCapabilityRequirementAssessment>,
}

#[derive(Debug, thiserror::Error)]
pub enum TriggerRunEvidenceError {
    #[error("trigger run evidence is unavailable")]
    Unavailable,
}

#[async_trait]
pub trait TriggerRunEvidenceSource: Send + Sync {
    async fn list_capability_evidence(
        &self,
        scope: &TriggerRunEvidenceScope,
        run_ids: &[TurnRunId],
    ) -> Result<Vec<TriggerCapabilityExecutionEvidence>, TriggerRunEvidenceError>;
}

#[derive(Debug, Default)]
pub struct MissingTriggerRunEvidenceSource;

#[async_trait]
impl TriggerRunEvidenceSource for MissingTriggerRunEvidenceSource {
    async fn list_capability_evidence(
        &self,
        _scope: &TriggerRunEvidenceScope,
        _run_ids: &[TurnRunId],
    ) -> Result<Vec<TriggerCapabilityExecutionEvidence>, TriggerRunEvidenceError> {
        Err(TriggerRunEvidenceError::Unavailable)
    }
}

pub fn assess_trigger_run(
    run_status: TriggerRunHistoryStatus,
    run_id: Option<TurnRunId>,
    required_capabilities: &[CapabilityId],
    evidence: Option<&[TriggerCapabilityExecutionEvidence]>,
) -> TriggerRunAssessment {
    let capabilities = if required_capabilities.is_empty() {
        observed_capabilities(run_id, evidence)
    } else {
        required_capabilities
            .iter()
            .cloned()
            .map(|capability_id| assess_capability(run_id, capability_id, evidence))
            .collect::<Vec<_>>()
    };
    if run_status == TriggerRunHistoryStatus::Error {
        return TriggerRunAssessment {
            status: TriggerRunAssessmentStatus::RunFailed,
            capabilities,
        };
    }
    if required_capabilities.is_empty() {
        let status = if capabilities
            .iter()
            .any(|item| item.status == TriggerCapabilityRequirementStatus::Failed)
        {
            TriggerRunAssessmentStatus::NeedsAttention
        } else {
            TriggerRunAssessmentStatus::Unverified
        };
        return TriggerRunAssessment {
            status,
            capabilities,
        };
    }

    let status = if capabilities
        .iter()
        .any(|item| item.status == TriggerCapabilityRequirementStatus::Failed)
    {
        TriggerRunAssessmentStatus::NeedsAttention
    } else if capabilities
        .iter()
        .any(|item| item.status != TriggerCapabilityRequirementStatus::Succeeded)
    {
        TriggerRunAssessmentStatus::Unverified
    } else {
        TriggerRunAssessmentStatus::AppearsSuccessful
    };
    TriggerRunAssessment {
        status,
        capabilities,
    }
}

fn observed_capabilities(
    run_id: Option<TurnRunId>,
    evidence: Option<&[TriggerCapabilityExecutionEvidence]>,
) -> Vec<TriggerCapabilityRequirementAssessment> {
    let Some(run_id) = run_id else {
        return Vec::new();
    };
    let Some(evidence) = evidence else {
        return Vec::new();
    };
    let mut capability_ids = Vec::new();
    for item in evidence.iter().filter(|item| item.run_id == run_id) {
        if !capability_ids.contains(&item.capability_id) {
            capability_ids.push(item.capability_id.clone());
        }
    }
    capability_ids
        .into_iter()
        .map(|capability_id| assess_capability(Some(run_id), capability_id, Some(evidence)))
        .collect()
}

fn assess_capability(
    run_id: Option<TurnRunId>,
    capability_id: CapabilityId,
    evidence: Option<&[TriggerCapabilityExecutionEvidence]>,
) -> TriggerCapabilityRequirementAssessment {
    let Some(evidence) = evidence else {
        return requirement(
            capability_id,
            TriggerCapabilityRequirementStatus::Unavailable,
            None,
        );
    };
    let matching = evidence
        .iter()
        .filter(|item| {
            run_id.as_ref().is_some_and(|run_id| item.run_id == *run_id)
                && item.capability_id == capability_id
        })
        .collect::<Vec<_>>();
    if let Some(failed) = matching
        .iter()
        .rev()
        .find(|item| item.status == TriggerCapabilityExecutionStatus::Failed)
    {
        return requirement(
            capability_id,
            TriggerCapabilityRequirementStatus::Failed,
            failed.error_kind.clone(),
        );
    }
    if matching
        .iter()
        .any(|item| item.status == TriggerCapabilityExecutionStatus::Succeeded)
    {
        return requirement(
            capability_id,
            TriggerCapabilityRequirementStatus::Succeeded,
            None,
        );
    }
    if matching.is_empty() {
        requirement(
            capability_id,
            TriggerCapabilityRequirementStatus::Missing,
            None,
        )
    } else {
        requirement(
            capability_id,
            TriggerCapabilityRequirementStatus::Incomplete,
            None,
        )
    }
}

fn requirement(
    capability_id: CapabilityId,
    status: TriggerCapabilityRequirementStatus,
    error_kind: Option<String>,
) -> TriggerCapabilityRequirementAssessment {
    TriggerCapabilityRequirementAssessment {
        capability_id,
        status,
        error_kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability() -> CapabilityId {
        CapabilityId::new("builtin.outbound_deliver").expect("valid capability id")
    }

    #[test]
    fn required_capability_must_succeed_in_the_same_run() {
        let run_id = TurnRunId::new();
        let evidence = [TriggerCapabilityExecutionEvidence {
            run_id: TurnRunId::new(),
            capability_id: capability(),
            status: TriggerCapabilityExecutionStatus::Succeeded,
            error_kind: None,
        }];

        let assessment = assess_trigger_run(
            TriggerRunHistoryStatus::Ok,
            Some(run_id),
            &[capability()],
            Some(&evidence),
        );

        assert_eq!(assessment.status, TriggerRunAssessmentStatus::Unverified);
        assert_eq!(
            assessment.capabilities[0].status,
            TriggerCapabilityRequirementStatus::Missing
        );
    }

    #[test]
    fn failed_required_capability_needs_attention_and_preserves_error_kind() {
        let run_id = TurnRunId::new();
        let evidence = [TriggerCapabilityExecutionEvidence {
            run_id,
            capability_id: capability(),
            status: TriggerCapabilityExecutionStatus::Failed,
            error_kind: Some("provider_rejected".to_owned()),
        }];

        let assessment = assess_trigger_run(
            TriggerRunHistoryStatus::Ok,
            Some(run_id),
            &[capability()],
            Some(&evidence),
        );

        assert_eq!(
            assessment.status,
            TriggerRunAssessmentStatus::NeedsAttention
        );
        assert_eq!(
            assessment.capabilities[0].error_kind.as_deref(),
            Some("provider_rejected")
        );
    }

    #[test]
    fn unconstrained_run_reports_observed_success_without_claiming_verification() {
        let run_id = TurnRunId::new();
        let evidence = [TriggerCapabilityExecutionEvidence {
            run_id,
            capability_id: capability(),
            status: TriggerCapabilityExecutionStatus::Succeeded,
            error_kind: None,
        }];

        let assessment = assess_trigger_run(
            TriggerRunHistoryStatus::Ok,
            Some(run_id),
            &[],
            Some(&evidence),
        );

        assert_eq!(assessment.status, TriggerRunAssessmentStatus::Unverified);
        assert_eq!(assessment.capabilities.len(), 1);
        assert_eq!(
            assessment.capabilities[0].status,
            TriggerCapabilityRequirementStatus::Succeeded
        );
    }

    #[test]
    fn unconstrained_run_surfaces_observed_failure() {
        let run_id = TurnRunId::new();
        let evidence = [TriggerCapabilityExecutionEvidence {
            run_id,
            capability_id: capability(),
            status: TriggerCapabilityExecutionStatus::Failed,
            error_kind: Some("provider_rejected".to_owned()),
        }];

        let assessment = assess_trigger_run(
            TriggerRunHistoryStatus::Ok,
            Some(run_id),
            &[],
            Some(&evidence),
        );

        assert_eq!(
            assessment.status,
            TriggerRunAssessmentStatus::NeedsAttention
        );
        assert_eq!(assessment.capabilities.len(), 1);
        assert_eq!(
            assessment.capabilities[0].status,
            TriggerCapabilityRequirementStatus::Failed
        );
    }

    #[test]
    fn unavailable_evidence_can_never_appear_successful() {
        let assessment = assess_trigger_run(
            TriggerRunHistoryStatus::Ok,
            Some(TurnRunId::new()),
            &[capability()],
            None,
        );

        assert_eq!(assessment.status, TriggerRunAssessmentStatus::Unverified);
        assert_eq!(
            assessment.capabilities[0].status,
            TriggerCapabilityRequirementStatus::Unavailable
        );
    }

    #[test]
    fn failed_run_keeps_exact_run_capability_evidence() {
        let run_id = TurnRunId::new();
        let evidence = [TriggerCapabilityExecutionEvidence {
            run_id,
            capability_id: capability(),
            status: TriggerCapabilityExecutionStatus::Failed,
            error_kind: Some("provider_rejected".to_owned()),
        }];

        let assessment = assess_trigger_run(
            TriggerRunHistoryStatus::Error,
            Some(run_id),
            &[capability()],
            Some(&evidence),
        );

        assert_eq!(assessment.status, TriggerRunAssessmentStatus::RunFailed);
        assert_eq!(assessment.capabilities.len(), 1);
        assert_eq!(
            assessment.capabilities[0].status,
            TriggerCapabilityRequirementStatus::Failed
        );
        assert_eq!(
            assessment.capabilities[0].error_kind.as_deref(),
            Some("provider_rejected")
        );
    }

    #[test]
    fn any_failed_required_invocation_prevents_a_successful_assessment() {
        let run_id = TurnRunId::new();
        let evidence = [
            TriggerCapabilityExecutionEvidence {
                run_id,
                capability_id: capability(),
                status: TriggerCapabilityExecutionStatus::Succeeded,
                error_kind: None,
            },
            TriggerCapabilityExecutionEvidence {
                run_id,
                capability_id: capability(),
                status: TriggerCapabilityExecutionStatus::Failed,
                error_kind: Some("second_destination_failed".to_owned()),
            },
        ];

        let assessment = assess_trigger_run(
            TriggerRunHistoryStatus::Ok,
            Some(run_id),
            &[capability()],
            Some(&evidence),
        );

        assert_eq!(
            assessment.status,
            TriggerRunAssessmentStatus::NeedsAttention
        );
    }
}
