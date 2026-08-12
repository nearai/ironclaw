use async_trait::async_trait;
use ironclaw_host_api::turn::ReplyTargetBindingRef;
use ironclaw_host_api::{
    Timestamp,
    ids::{AgentId, ProjectId, TenantId, UserId},
};
use serde::{Deserialize, Serialize};

use crate::{CommunicationModality, OutboundDeliveryTargetId, OutboundError};

/// Owner scope for default outbound delivery preferences.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
        Self::personal(tenant_id, user_id)
    }

    pub fn personal(tenant_id: TenantId, user_id: UserId) -> Self {
        Self {
            scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        }
    }

    pub fn shared_agent(
        tenant_id: TenantId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            scope: DeliveryDefaultScope::shared_agent(tenant_id, agent_id, project_id),
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

/// Ceiling on stored notification targets per preference record (spec §7).
/// Enforced at the repository write seam, not by `Deserialize` — a legacy or
/// already-oversized row must still load so it can be corrected. The product
/// writer also rejects oversized requests before registry/backend I/O.
pub const NOTIFICATION_TARGETS_CAP: usize = 8;

/// Durable scoped communication defaults owned by outbound policy.
///
/// Stored reply targets are candidates only. Callers must revalidate every
/// target through the outbound validation path before sending externally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommunicationPreferenceRecord {
    pub scope: DeliveryDefaultScope,
    /// Read-migration input only: the single delivery target rows written
    /// before notification channels existed carried under `final_reply_target`.
    ///
    /// The four per-purpose preference slots (final reply / progress / approval
    /// prompt / auth prompt) are retired — nothing writes them, and delivery is
    /// a model-called tool, never a stored route. This one value survives so
    /// [`CommunicationPreferenceRecord::effective_notification_target_ids`] can
    /// fold an unmigrated row's target into the notification set; it keeps its
    /// historical wire name so such a row survives a rewrite.
    #[serde(rename = "final_reply_target")]
    pub legacy_notification_target: Option<ReplyTargetBindingRef>,
    pub default_modality: Option<CommunicationModality>,
    /// Explicit notification-channel targets. Empty means "fall back to the
    /// legacy single slot" — see
    /// [`CommunicationPreferenceRecord::effective_notification_target_ids`].
    /// Bounded by [`NOTIFICATION_TARGETS_CAP`] at the write seam, not here.
    pub notification_targets: Vec<OutboundDeliveryTargetId>,
    pub updated_at: Timestamp,
    pub updated_by: UserId,
}

impl CommunicationPreferenceRecord {
    pub fn key(&self) -> CommunicationPreferenceKey {
        CommunicationPreferenceKey {
            scope: self.scope.clone(),
        }
    }

    /// Effective notification targets: the stored set, or the legacy
    /// single-slot default migrated read-side (spec §7). Ref→id migration
    /// needs the registry, so it takes the resolved entries.
    pub fn effective_notification_target_ids(
        &self,
        resolve_legacy: impl FnOnce(&ReplyTargetBindingRef) -> Option<OutboundDeliveryTargetId>,
    ) -> Vec<OutboundDeliveryTargetId> {
        if !self.notification_targets.is_empty() {
            return self.notification_targets.clone();
        }
        self.legacy_notification_target
            .as_ref()
            .and_then(resolve_legacy)
            .into_iter()
            .collect()
    }
}

impl<'de> Deserialize<'de> for CommunicationPreferenceRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        /// Accepts every field any historical writer emitted. The three
        /// retired per-purpose slots are read and discarded so a pre-retirement
        /// row still deserializes; `final_reply_target` is the one legacy value
        /// the read-migration still needs, and is carried onto the record.
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default)]
            scope: Option<DeliveryDefaultScope>,
            #[serde(default)]
            tenant_id: Option<TenantId>,
            #[serde(default)]
            user_id: Option<UserId>,
            final_reply_target: Option<ReplyTargetBindingRef>,
            #[allow(
                dead_code,
                reason = "accepted from legacy rows, deliberately discarded"
            )]
            progress_target: Option<ReplyTargetBindingRef>,
            #[allow(
                dead_code,
                reason = "accepted from legacy rows, deliberately discarded"
            )]
            approval_prompt_target: Option<ReplyTargetBindingRef>,
            #[allow(
                dead_code,
                reason = "accepted from legacy rows, deliberately discarded"
            )]
            auth_prompt_target: Option<ReplyTargetBindingRef>,
            default_modality: Option<CommunicationModality>,
            #[serde(default)]
            notification_targets: Vec<OutboundDeliveryTargetId>,
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
            legacy_notification_target: wire.final_reply_target,
            default_modality: wire.default_modality,
            notification_targets: wire.notification_targets,
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
    use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};

    use super::*;

    fn reply_ref(value: &str) -> ReplyTargetBindingRef {
        ReplyTargetBindingRef::new(value).unwrap()
    }

    fn preference_record_with_legacy_target(
        legacy_notification_target: Option<ReplyTargetBindingRef>,
    ) -> CommunicationPreferenceRecord {
        CommunicationPreferenceRecord {
            scope: DeliveryDefaultScope::personal(
                TenantId::new("tenant-notification-targets").unwrap(),
                UserId::new("user-notification-targets").unwrap(),
            ),
            legacy_notification_target,
            default_modality: None,
            notification_targets: Vec::new(),
            updated_at: Utc::now(),
            updated_by: UserId::new("user-notification-targets-updater").unwrap(),
        }
    }

    #[test]
    fn legacy_single_slot_migrates_via_resolver() {
        // Empty stored set: the resolver runs against the legacy single slot
        // and its resolved id becomes the sole effective target.
        let legacy_ref = reply_ref("reply-legacy-slot");
        let resolved = OutboundDeliveryTargetId::new("target:legacy-resolved").unwrap();
        let with_legacy_slot = preference_record_with_legacy_target(Some(legacy_ref.clone()));
        let expected = resolved.clone();
        assert_eq!(
            with_legacy_slot.effective_notification_target_ids(move |target_ref| {
                assert_eq!(target_ref, &legacy_ref);
                Some(expected.clone())
            }),
            vec![resolved]
        );

        // Non-empty stored set: the stored set wins outright and the
        // resolver must never run.
        let mut with_stored_set =
            preference_record_with_legacy_target(Some(reply_ref("reply-legacy-slot-ignored")));
        let stored = vec![OutboundDeliveryTargetId::new("target:stored").unwrap()];
        with_stored_set.notification_targets = stored.clone();
        assert_eq!(
            with_stored_set.effective_notification_target_ids(|_| panic!(
                "resolver must not run when a notification target set is already stored"
            )),
            stored
        );

        // No legacy slot and no stored set: the effective set is empty.
        let without_legacy_slot = preference_record_with_legacy_target(None);
        assert!(
            without_legacy_slot
                .effective_notification_target_ids(|_| panic!(
                    "resolver must not run when there is no legacy notification target"
                ))
                .is_empty()
        );
    }

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
            scope: DeliveryDefaultScope::shared_agent(
                TenantId::new("tenant-pref-json").unwrap(),
                AgentId::new("agent-pref-json").unwrap(),
                Some(ProjectId::new("project-pref-json").unwrap()),
            ),
            legacy_notification_target: None,
            default_modality: Some(CommunicationModality::Text),
            notification_targets: Vec::new(),
            updated_at,
            updated_by: UserId::new("user-pref-json-updater").unwrap(),
        };
        let serialized = serde_json::to_string(&scoped).expect("serialize scoped preference");
        let decoded: CommunicationPreferenceRecord =
            serde_json::from_str(&serialized).expect("deserialize scoped preference");
        assert_eq!(decoded, scoped);

        // A row written before notification channels existed: the three
        // retired per-purpose slots must still deserialize (accepted and
        // discarded), and `final_reply_target` must survive as the
        // read-migration input.
        let legacy = serde_json::json!({
            "tenant_id": "tenant-pref-legacy",
            "user_id": "user-pref-legacy",
            "final_reply_target": "reply:legacy-final",
            "progress_target": "reply:legacy-progress",
            "approval_prompt_target": "reply:legacy-approval",
            "auth_prompt_target": "reply:legacy-auth",
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
        assert_eq!(
            decoded
                .legacy_notification_target
                .as_ref()
                .map(|target| target.as_str()),
            Some("reply:legacy-final"),
            "the legacy single slot must survive as the read-migration input"
        );
        assert!(
            decoded.notification_targets.is_empty(),
            "JSON predating notification_targets must default to an empty vec"
        );
        // ...and it round-trips under its historical wire name, so rewriting
        // an unmigrated row does not strip it.
        let reserialized =
            serde_json::to_value(&decoded).expect("serialize migrated legacy preference");
        assert_eq!(reserialized["final_reply_target"], "reply:legacy-final");

        let missing_scope = serde_json::json!({
            "final_reply_target": null,
            "default_modality": null,
            "updated_at": updated_at,
            "updated_by": "user-pref-missing-updater"
        });
        assert!(serde_json::from_value::<CommunicationPreferenceRecord>(missing_scope).is_err());

        // The cap is enforced by the product-side writer, never by
        // `Deserialize` (see `NOTIFICATION_TARGETS_CAP`): an already-oversized
        // row must load in full so it can be corrected.
        let oversized_targets: Vec<String> = (0..=NOTIFICATION_TARGETS_CAP)
            .map(|index| format!("slack:oversized-{index}"))
            .collect();
        let oversized = serde_json::json!({
            "scope": {"kind": "personal", "tenant_id": "tenant-pref-legacy", "user_id": "user-pref-legacy"},
            "final_reply_target": null,
            "notification_targets": oversized_targets,
            "default_modality": "text",
            "updated_at": updated_at,
            "updated_by": "user-pref-oversized-updater"
        });
        let decoded: CommunicationPreferenceRecord =
            serde_json::from_value(oversized).expect("deserialize oversized preference row");
        assert_eq!(
            decoded.notification_targets.len(),
            NOTIFICATION_TARGETS_CAP + 1,
            "Deserialize must tolerate an oversized stored set in full"
        );
    }
}
