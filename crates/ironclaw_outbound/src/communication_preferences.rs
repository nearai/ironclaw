use async_trait::async_trait;
use ironclaw_host_api::{ProjectId, TenantId, Timestamp, UserId};
use ironclaw_turns::ReplyTargetBindingRef;
use serde::{Deserialize, Serialize};

use crate::{CommunicationModality, OutboundError};

/// Owner scope for default outbound delivery preferences.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeliveryDefaultScope {
    Personal {
        tenant_id: TenantId,
        user_id: UserId,
    },
    Project {
        tenant_id: TenantId,
        project_id: ProjectId,
    },
}

impl DeliveryDefaultScope {
    pub fn personal(tenant_id: TenantId, user_id: UserId) -> Self {
        Self::Personal { tenant_id, user_id }
    }

    pub fn project(tenant_id: TenantId, project_id: ProjectId) -> Self {
        Self::Project {
            tenant_id,
            project_id,
        }
    }

    pub fn tenant_id(&self) -> &TenantId {
        match self {
            Self::Personal { tenant_id, .. } | Self::Project { tenant_id, .. } => tenant_id,
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
        Self::personal(tenant_id, user_id)
    }

    pub fn personal(tenant_id: TenantId, user_id: UserId) -> Self {
        Self {
            scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        }
    }

    pub fn project(tenant_id: TenantId, project_id: ProjectId) -> Self {
        Self {
            scope: DeliveryDefaultScope::project(tenant_id, project_id),
        }
    }
}

/// Opaque compare token returned with a preference read and required on writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommunicationPreferenceVersion(u64);

impl CommunicationPreferenceVersion {
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub fn raw(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// Durable scoped communication defaults owned by outbound policy.
///
/// Stored reply targets are candidates only. Callers must revalidate every
/// target through the outbound validation path before sending externally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommunicationPreferenceRecord {
    pub scope: DeliveryDefaultScope,
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
        CommunicationPreferenceKey {
            scope: self.scope.clone(),
        }
    }
}

impl<'de> Deserialize<'de> for CommunicationPreferenceRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default)]
            scope: Option<DeliveryDefaultScope>,
            #[serde(default)]
            tenant_id: Option<TenantId>,
            #[serde(default)]
            user_id: Option<UserId>,
            final_reply_target: Option<ReplyTargetBindingRef>,
            progress_target: Option<ReplyTargetBindingRef>,
            approval_prompt_target: Option<ReplyTargetBindingRef>,
            auth_prompt_target: Option<ReplyTargetBindingRef>,
            default_modality: Option<CommunicationModality>,
            updated_at: Timestamp,
            updated_by: UserId,
        }

        let wire = Wire::deserialize(deserializer)?;
        let scope = match (wire.scope, wire.tenant_id, wire.user_id) {
            (Some(scope), _, _) => scope,
            (None, Some(tenant_id), Some(user_id)) => {
                DeliveryDefaultScope::personal(tenant_id, user_id)
            }
            _ => {
                return Err(serde::de::Error::custom(
                    "communication preference scope is required",
                ));
            }
        };
        Ok(Self {
            scope,
            final_reply_target: wire.final_reply_target,
            progress_target: wire.progress_target,
            approval_prompt_target: wire.approval_prompt_target,
            auth_prompt_target: wire.auth_prompt_target,
            default_modality: wire.default_modality,
            updated_at: wire.updated_at,
            updated_by: wire.updated_by,
        })
    }
}

/// Communication preference row plus the optimistic-lock token returned by a read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionedCommunicationPreferenceRecord {
    pub record: CommunicationPreferenceRecord,
    pub version: CommunicationPreferenceVersion,
}

/// Versioned preference write request.
///
/// `expected_version: None` creates a row only when no scoped preference
/// exists. Updates must pass the version returned by
/// [`CommunicationPreferenceRepository::load_communication_preference`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteCommunicationPreferenceRequest {
    pub record: CommunicationPreferenceRecord,
    pub expected_version: Option<CommunicationPreferenceVersion>,
}

/// Store for durable scoped communication delivery preferences.
#[async_trait]
pub trait CommunicationPreferenceRepository: Send + Sync {
    /// Create a scoped preference row when it does not already exist.
    ///
    /// This convenience path is intentionally insert-only. Callers updating an
    /// existing preference must read the current version and use
    /// [`Self::write_communication_preference`] with `expected_version`.
    ///
    /// # Errors
    ///
    /// Returns [`OutboundError::CasConflict`] when the scoped preference row
    /// already exists. Callers that need upsert or update behavior should use
    /// [`Self::write_communication_preference`] with an observed version.
    async fn put_communication_preference(
        &self,
        record: CommunicationPreferenceRecord,
    ) -> Result<(), OutboundError> {
        self.write_communication_preference(WriteCommunicationPreferenceRequest {
            record,
            expected_version: None,
        })
        .await
        .map(|_| ())
    }

    async fn load_communication_preference(
        &self,
        key: CommunicationPreferenceKey,
    ) -> Result<Option<VersionedCommunicationPreferenceRecord>, OutboundError>;

    async fn write_communication_preference(
        &self,
        request: WriteCommunicationPreferenceRequest,
    ) -> Result<VersionedCommunicationPreferenceRecord, OutboundError>;
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ironclaw_host_api::{ProjectId, TenantId, UserId};

    use super::*;

    #[test]
    fn version_next_saturates_at_u64_max() {
        assert_eq!(
            CommunicationPreferenceVersion::from_raw(u64::MAX).next(),
            CommunicationPreferenceVersion::from_raw(u64::MAX)
        );
    }

    #[test]
    fn communication_preference_record_deserializes_scoped_and_legacy_payloads() {
        let updated_at = Utc::now();
        let scoped = CommunicationPreferenceRecord {
            scope: DeliveryDefaultScope::project(
                TenantId::new("tenant-pref-json").unwrap(),
                ProjectId::new("project-pref-json").unwrap(),
            ),
            final_reply_target: None,
            progress_target: None,
            approval_prompt_target: None,
            auth_prompt_target: None,
            default_modality: Some(CommunicationModality::Text),
            updated_at,
            updated_by: UserId::new("user-pref-json-updater").unwrap(),
        };
        let serialized = serde_json::to_string(&scoped).expect("serialize scoped preference");
        let decoded: CommunicationPreferenceRecord =
            serde_json::from_str(&serialized).expect("deserialize scoped preference");
        assert_eq!(decoded, scoped);

        let legacy = serde_json::json!({
            "tenant_id": "tenant-pref-legacy",
            "user_id": "user-pref-legacy",
            "final_reply_target": null,
            "progress_target": null,
            "approval_prompt_target": null,
            "auth_prompt_target": null,
            "default_modality": "text",
            "updated_at": updated_at,
            "updated_by": "user-pref-legacy-updater"
        });
        let decoded: CommunicationPreferenceRecord =
            serde_json::from_value(legacy).expect("deserialize legacy preference");
        assert_eq!(
            decoded.scope,
            DeliveryDefaultScope::personal(
                TenantId::new("tenant-pref-legacy").unwrap(),
                UserId::new("user-pref-legacy").unwrap()
            )
        );

        let missing_scope = serde_json::json!({
            "final_reply_target": null,
            "progress_target": null,
            "approval_prompt_target": null,
            "auth_prompt_target": null,
            "default_modality": null,
            "updated_at": updated_at,
            "updated_by": "user-pref-missing-updater"
        });
        assert!(serde_json::from_value::<CommunicationPreferenceRecord>(missing_scope).is_err());
    }
}
