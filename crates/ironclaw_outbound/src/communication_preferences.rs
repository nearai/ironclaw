use async_trait::async_trait;
use ironclaw_host_api::{TenantId, Timestamp, UserId};
use ironclaw_turns::ReplyTargetBindingRef;
use serde::{Deserialize, Serialize};

use crate::{CommunicationModality, OutboundError};

/// Tenant/user lookup key for outbound-owned communication preferences.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommunicationPreferenceKey {
    pub tenant_id: TenantId,
    pub user_id: UserId,
}

impl CommunicationPreferenceKey {
    pub fn new(tenant_id: TenantId, user_id: UserId) -> Self {
        Self { tenant_id, user_id }
    }
}

/// Durable tenant/user communication defaults owned by outbound policy.
///
/// Stored reply targets are candidates only. Callers must revalidate every
/// target through the outbound validation path before sending externally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunicationPreferenceRecord {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub final_reply_target: Option<ReplyTargetBindingRef>,
    pub progress_target: Option<ReplyTargetBindingRef>,
    pub approval_prompt_target: Option<ReplyTargetBindingRef>,
    pub auth_prompt_target: Option<ReplyTargetBindingRef>,
    pub default_modality: Option<CommunicationModality>,
    pub updated_at: Timestamp,
    pub updated_by: UserId,
}

impl CommunicationPreferenceRecord {
    pub fn key(&self) -> CommunicationPreferenceKey {
        CommunicationPreferenceKey::new(self.tenant_id.clone(), self.user_id.clone())
    }
}

/// Store for durable tenant/user communication delivery preferences.
#[async_trait]
pub trait CommunicationPreferenceRepository: Send + Sync {
    async fn put_communication_preference(
        &self,
        record: CommunicationPreferenceRecord,
    ) -> Result<(), OutboundError>;

    async fn load_communication_preference(
        &self,
        key: CommunicationPreferenceKey,
    ) -> Result<Option<CommunicationPreferenceRecord>, OutboundError>;
}
