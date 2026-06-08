use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, Timestamp, UserId};
use ironclaw_turns::ReplyTargetBindingRef;
use serde::{Deserialize, Deserializer, Serialize};

use crate::{CommunicationModality, OutboundError};

/// Owner scope for default outbound delivery preferences.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum DeliveryDefaultScope {
    Personal {
        tenant_id: TenantId,
        user_id: UserId,
    },
    SharedAgent {
        tenant_id: TenantId,
        agent_id: AgentId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project_id: Option<ProjectId>,
    },
}

impl DeliveryDefaultScope {
    pub fn personal(tenant_id: TenantId, user_id: UserId) -> Self {
        Self::Personal { tenant_id, user_id }
    }

    pub fn shared_agent(
        tenant_id: TenantId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self::SharedAgent {
            tenant_id,
            agent_id,
            project_id,
        }
    }

    pub fn tenant_id(&self) -> &TenantId {
        match self {
            Self::Personal { tenant_id, .. } | Self::SharedAgent { tenant_id, .. } => tenant_id,
        }
    }
}

/// Scoped lookup key for outbound-owned communication preferences.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommunicationPreferenceKey {
    pub scope: DeliveryDefaultScope,
}

impl CommunicationPreferenceKey {
    pub fn new(tenant_id: TenantId, user_id: UserId) -> Self {
        Self {
            scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        }
    }

    pub fn for_scope(scope: DeliveryDefaultScope) -> Self {
        Self { scope }
    }
}

/// Opaque version for compare-and-swap writes to communication preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommunicationPreferenceVersion(pub(crate) u64);

impl CommunicationPreferenceVersion {
    pub fn from_backend(raw: u64) -> Self {
        Self(raw)
    }

    pub fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// Compare-and-swap expectation for scoped communication preference writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "version")]
pub enum CommunicationPreferenceWriteExpectation {
    Any,
    Absent,
    Version(CommunicationPreferenceVersion),
}

/// Versioned communication preference returned by repositories that support
/// caller-visible stale-write detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionedCommunicationPreferenceRecord {
    pub record: CommunicationPreferenceRecord,
    pub version: CommunicationPreferenceVersion,
}

/// Named preference target slot updated by product configuration callers.
///
/// Each variant maps to one optional target in
/// [`CommunicationPreferenceTargets`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunicationPreferenceSlot {
    FinalReply,
    Progress,
    ApprovalPrompt,
    AuthPrompt,
}

/// Candidate reply targets for each outbound communication purpose.
///
/// Targets are durable defaults only. Callers must revalidate the selected
/// target through outbound policy before sending externally.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunicationPreferenceTargets {
    pub final_reply: Option<ReplyTargetBindingRef>,
    pub progress: Option<ReplyTargetBindingRef>,
    pub approval_prompt: Option<ReplyTargetBindingRef>,
    pub auth_prompt: Option<ReplyTargetBindingRef>,
}

impl CommunicationPreferenceTargets {
    pub fn target(&self, slot: CommunicationPreferenceSlot) -> Option<&ReplyTargetBindingRef> {
        match slot {
            CommunicationPreferenceSlot::FinalReply => self.final_reply.as_ref(),
            CommunicationPreferenceSlot::Progress => self.progress.as_ref(),
            CommunicationPreferenceSlot::ApprovalPrompt => self.approval_prompt.as_ref(),
            CommunicationPreferenceSlot::AuthPrompt => self.auth_prompt.as_ref(),
        }
    }

    pub fn target_mut(
        &mut self,
        slot: CommunicationPreferenceSlot,
    ) -> &mut Option<ReplyTargetBindingRef> {
        match slot {
            CommunicationPreferenceSlot::FinalReply => &mut self.final_reply,
            CommunicationPreferenceSlot::Progress => &mut self.progress,
            CommunicationPreferenceSlot::ApprovalPrompt => &mut self.approval_prompt,
            CommunicationPreferenceSlot::AuthPrompt => &mut self.auth_prompt,
        }
    }
}

/// Compare-and-swap request for updating one communication preference slot.
///
/// `expected_version` is the version observed by the caller. If the record has
/// advanced but the selected slot still equals `expected_current_target`, the
/// repository may merge the disjoint slot update and preserve the newer values
/// in other slots. If the selected slot changed, the write fails with
/// [`OutboundError::CasConflict`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCommunicationPreferenceSlotRequest {
    pub key: CommunicationPreferenceKey,
    pub expected_version: CommunicationPreferenceVersion,
    pub slot: CommunicationPreferenceSlot,
    pub expected_current_target: Option<ReplyTargetBindingRef>,
    pub target: Option<ReplyTargetBindingRef>,
    pub updated_at: Timestamp,
    pub updated_by: UserId,
}

/// Durable scoped communication defaults owned by outbound policy.
///
/// Stored reply targets are candidates only. Callers must revalidate every
/// target through the outbound validation path before sending externally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommunicationPreferenceRecord {
    pub scope: DeliveryDefaultScope,
    pub targets: CommunicationPreferenceTargets,
    pub default_modality: Option<CommunicationModality>,
    pub updated_at: Timestamp,
    pub updated_by: UserId,
}

impl<'de> Deserialize<'de> for CommunicationPreferenceRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireRecord {
            scope: Option<DeliveryDefaultScope>,
            tenant_id: Option<TenantId>,
            user_id: Option<UserId>,
            targets: Option<CommunicationPreferenceTargets>,
            final_reply_target: Option<ReplyTargetBindingRef>,
            progress_target: Option<ReplyTargetBindingRef>,
            approval_prompt_target: Option<ReplyTargetBindingRef>,
            auth_prompt_target: Option<ReplyTargetBindingRef>,
            default_modality: Option<CommunicationModality>,
            updated_at: Timestamp,
            updated_by: UserId,
        }

        let wire = WireRecord::deserialize(deserializer)?;
        let scope = match wire.scope {
            Some(scope) => scope,
            None => DeliveryDefaultScope::personal(
                wire.tenant_id
                    .ok_or_else(|| serde::de::Error::missing_field("scope or legacy tenant_id"))?,
                wire.user_id
                    .ok_or_else(|| serde::de::Error::missing_field("scope or legacy user_id"))?,
            ),
        };
        let targets = wire.targets.unwrap_or(CommunicationPreferenceTargets {
            final_reply: wire.final_reply_target,
            progress: wire.progress_target,
            approval_prompt: wire.approval_prompt_target,
            auth_prompt: wire.auth_prompt_target,
        });
        Ok(Self {
            scope,
            targets,
            default_modality: wire.default_modality,
            updated_at: wire.updated_at,
            updated_by: wire.updated_by,
        })
    }
}

impl CommunicationPreferenceRecord {
    pub fn key(&self) -> CommunicationPreferenceKey {
        CommunicationPreferenceKey::for_scope(self.scope.clone())
    }
}

/// Store for durable scoped communication delivery preferences.
#[async_trait]
pub trait CommunicationPreferenceRepository: Send + Sync {
    async fn put_communication_preference(
        &self,
        record: CommunicationPreferenceRecord,
    ) -> Result<(), OutboundError> {
        self.write_communication_preference(record, CommunicationPreferenceWriteExpectation::Any)
            .await
            .map(|_| ())
    }

    async fn load_communication_preference(
        &self,
        key: CommunicationPreferenceKey,
    ) -> Result<Option<CommunicationPreferenceRecord>, OutboundError> {
        Ok(self
            .load_versioned_communication_preference(key)
            .await?
            .map(|versioned| versioned.record))
    }

    async fn write_communication_preference(
        &self,
        record: CommunicationPreferenceRecord,
        expectation: CommunicationPreferenceWriteExpectation,
    ) -> Result<CommunicationPreferenceVersion, OutboundError>;

    async fn load_versioned_communication_preference(
        &self,
        key: CommunicationPreferenceKey,
    ) -> Result<Option<VersionedCommunicationPreferenceRecord>, OutboundError>;

    async fn update_communication_preference_slot(
        &self,
        request: UpdateCommunicationPreferenceSlotRequest,
    ) -> Result<VersionedCommunicationPreferenceRecord, OutboundError> {
        if request.updated_by.as_str().is_empty() {
            return Err(OutboundError::InvalidRequest {
                reason: "communication preference updater is required",
            });
        }
        for _ in 0..5 {
            let Some(mut versioned) = self
                .load_versioned_communication_preference(request.key.clone())
                .await?
            else {
                return Err(OutboundError::CasConflict);
            };
            if versioned.record.key() != request.key {
                return Err(OutboundError::Backend);
            }
            if versioned.version != request.expected_version
                && versioned.record.targets.target(request.slot)
                    != request.expected_current_target.as_ref()
            {
                return Err(OutboundError::CasConflict);
            }
            *versioned.record.targets.target_mut(request.slot) = request.target.clone();
            versioned.record.updated_at = request.updated_at;
            versioned.record.updated_by = request.updated_by.clone();
            match self
                .write_communication_preference(
                    versioned.record.clone(),
                    CommunicationPreferenceWriteExpectation::Version(versioned.version),
                )
                .await
            {
                Ok(version) => {
                    return Ok(VersionedCommunicationPreferenceRecord {
                        record: versioned.record,
                        version,
                    });
                }
                Err(OutboundError::CasConflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(OutboundError::Backend)
    }
}
