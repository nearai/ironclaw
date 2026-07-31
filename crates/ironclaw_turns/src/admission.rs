use std::collections::HashMap;

use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct TurnAdmissionClass(String);

impl<'de> Deserialize<'de> for TurnAdmissionClass {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl TurnAdmissionClass {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_admission_class(&value)?;
        Ok(Self(value))
    }

    pub fn interactive() -> Self {
        Self("interactive".to_string())
    }

    pub fn mission() -> Self {
        Self("mission".to_string())
    }

    pub fn admin_system() -> Self {
        Self("admin_system".to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_admission_class(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("turn admission class must not be empty".to_string());
    }
    if value.len() > 128 {
        return Err("turn admission class must be at most 128 bytes".to_string());
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err("turn admission class must not contain control characters".to_string());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(
            "turn admission class must contain only lowercase ASCII letters, digits, or _"
                .to_string(),
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnAdmissionAxisKind {
    Tenant,
    ActorUser,
    Project,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnAdmissionBucketKind {
    Total,
    Class,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TurnAdmissionBucketScope {
    Tenant {
        tenant_id: TenantId,
    },
    ActorUser {
        tenant_id: TenantId,
        user_id: UserId,
    },
    Project {
        tenant_id: TenantId,
        project_id: Option<ProjectId>,
    },
    Agent {
        tenant_id: TenantId,
        agent_id: Option<AgentId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnAdmissionBucket {
    pub axis_kind: TurnAdmissionAxisKind,
    pub bucket_kind: TurnAdmissionBucketKind,
    pub admission_class: Option<TurnAdmissionClass>,
    pub scope: TurnAdmissionBucketScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TurnAdmissionLimitSelector {
    axis_kind: TurnAdmissionAxisKind,
    bucket_kind: TurnAdmissionBucketKind,
    admission_class: Option<TurnAdmissionClass>,
}

impl TurnAdmissionLimitSelector {
    fn from_bucket(bucket: &TurnAdmissionBucket) -> Self {
        Self {
            axis_kind: bucket.axis_kind,
            bucket_kind: bucket.bucket_kind,
            admission_class: bucket.admission_class.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnAdmissionLimit {
    pub max_active: Option<u64>,
    pub retry_after_ms: Option<u64>,
}

impl TurnAdmissionLimit {
    pub fn unlimited() -> Self {
        Self {
            max_active: None,
            retry_after_ms: None,
        }
    }

    pub fn max_active(max_active: u64) -> Self {
        Self {
            max_active: Some(max_active),
            retry_after_ms: None,
        }
    }

    pub fn with_retry_after_ms(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnAdmissionLimitUnavailable;

pub trait TurnAdmissionLimitProvider: Send + Sync {
    fn limit_for(
        &self,
        bucket: &TurnAdmissionBucket,
    ) -> Result<TurnAdmissionLimit, TurnAdmissionLimitUnavailable>;
}

#[derive(Debug, Clone, Default)]
pub struct AllowAllTurnAdmissionLimitProvider;

impl TurnAdmissionLimitProvider for AllowAllTurnAdmissionLimitProvider {
    fn limit_for(
        &self,
        _bucket: &TurnAdmissionBucket,
    ) -> Result<TurnAdmissionLimit, TurnAdmissionLimitUnavailable> {
        Ok(TurnAdmissionLimit::unlimited())
    }
}

#[derive(Debug, Clone, Default)]
pub struct StaticTurnAdmissionLimitProvider {
    limits: HashMap<TurnAdmissionLimitSelector, TurnAdmissionLimit>,
    unavailable: bool,
}

impl StaticTurnAdmissionLimitProvider {
    pub fn with_total_limit(mut self, axis_kind: TurnAdmissionAxisKind, max_active: u64) -> Self {
        self.limits.insert(
            TurnAdmissionLimitSelector {
                axis_kind,
                bucket_kind: TurnAdmissionBucketKind::Total,
                admission_class: None,
            },
            TurnAdmissionLimit::max_active(max_active),
        );
        self
    }

    pub fn with_class_limit(
        mut self,
        axis_kind: TurnAdmissionAxisKind,
        admission_class: TurnAdmissionClass,
        max_active: u64,
    ) -> Self {
        self.limits.insert(
            TurnAdmissionLimitSelector {
                axis_kind,
                bucket_kind: TurnAdmissionBucketKind::Class,
                admission_class: Some(admission_class),
            },
            TurnAdmissionLimit::max_active(max_active),
        );
        self
    }

    pub fn unavailable(mut self) -> Self {
        self.unavailable = true;
        self
    }
}

impl TurnAdmissionLimitProvider for StaticTurnAdmissionLimitProvider {
    fn limit_for(
        &self,
        bucket: &TurnAdmissionBucket,
    ) -> Result<TurnAdmissionLimit, TurnAdmissionLimitUnavailable> {
        if self.unavailable {
            return Err(TurnAdmissionLimitUnavailable);
        }
        Ok(self
            .limits
            .get(&TurnAdmissionLimitSelector::from_bucket(bucket))
            .copied()
            .unwrap_or_else(TurnAdmissionLimit::unlimited))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnAdmissionCapacityDenial {
    pub axis_kind: TurnAdmissionAxisKind,
    pub bucket_kind: TurnAdmissionBucketKind,
    pub admission_class: Option<TurnAdmissionClass>,
    pub limit: u64,
    pub active_count: u64,
    pub retry_after_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant_id() -> TenantId {
        TenantId::new("tenant-admission-test").expect("tenant")
    }

    fn user_id() -> UserId {
        UserId::new("user-admission-test").expect("user")
    }

    fn bucket(
        axis_kind: TurnAdmissionAxisKind,
        bucket_kind: TurnAdmissionBucketKind,
        admission_class: Option<TurnAdmissionClass>,
    ) -> TurnAdmissionBucket {
        TurnAdmissionBucket {
            axis_kind,
            bucket_kind,
            admission_class,
            scope: TurnAdmissionBucketScope::ActorUser {
                tenant_id: tenant_id(),
                user_id: user_id(),
            },
        }
    }

    #[test]
    fn admission_class_validation_and_serde_are_fail_closed() {
        for invalid in ["", "Interactive", "has-dash", "has space", "line\nbreak"] {
            assert!(
                TurnAdmissionClass::new(invalid).is_err(),
                "{invalid:?} must be rejected"
            );
        }
        assert!(TurnAdmissionClass::new("a".repeat(129)).is_err());

        let class = TurnAdmissionClass::new("mission_42").expect("valid class");
        assert_eq!(class.as_str(), "mission_42");
        let encoded = serde_json::to_string(&class).expect("serialize class");
        assert_eq!(
            serde_json::from_str::<TurnAdmissionClass>(&encoded).expect("deserialize class"),
            class
        );
        assert!(serde_json::from_str::<TurnAdmissionClass>("\"INVALID\"").is_err());
    }

    #[test]
    fn builtin_admission_classes_are_stable() {
        assert_eq!(TurnAdmissionClass::interactive().as_str(), "interactive");
        assert_eq!(TurnAdmissionClass::mission().as_str(), "mission");
        assert_eq!(TurnAdmissionClass::admin_system().as_str(), "admin_system");
    }

    #[test]
    fn static_limits_select_total_and_class_buckets_independently() {
        let mission = TurnAdmissionClass::mission();
        let provider = StaticTurnAdmissionLimitProvider::default()
            .with_total_limit(TurnAdmissionAxisKind::ActorUser, 5)
            .with_class_limit(TurnAdmissionAxisKind::ActorUser, mission.clone(), 2);

        assert_eq!(
            provider
                .limit_for(&bucket(
                    TurnAdmissionAxisKind::ActorUser,
                    TurnAdmissionBucketKind::Total,
                    None,
                ))
                .expect("total limit"),
            TurnAdmissionLimit::max_active(5)
        );
        assert_eq!(
            provider
                .limit_for(&bucket(
                    TurnAdmissionAxisKind::ActorUser,
                    TurnAdmissionBucketKind::Class,
                    Some(mission),
                ))
                .expect("class limit"),
            TurnAdmissionLimit::max_active(2)
        );
        assert_eq!(
            provider
                .limit_for(&bucket(
                    TurnAdmissionAxisKind::Tenant,
                    TurnAdmissionBucketKind::Total,
                    None,
                ))
                .expect("unspecified limit"),
            TurnAdmissionLimit::unlimited()
        );
    }

    #[test]
    fn limit_provider_unavailability_is_not_treated_as_unlimited() {
        let unavailable = StaticTurnAdmissionLimitProvider::default().unavailable();
        assert_eq!(
            unavailable.limit_for(&bucket(
                TurnAdmissionAxisKind::ActorUser,
                TurnAdmissionBucketKind::Total,
                None,
            )),
            Err(TurnAdmissionLimitUnavailable)
        );
        assert_eq!(
            AllowAllTurnAdmissionLimitProvider
                .limit_for(&bucket(
                    TurnAdmissionAxisKind::ActorUser,
                    TurnAdmissionBucketKind::Total,
                    None,
                ))
                .expect("allow-all limit"),
            TurnAdmissionLimit::unlimited()
        );
    }

    #[test]
    fn admission_limit_retry_hint_and_capacity_denial_round_trip() {
        let limit = TurnAdmissionLimit::max_active(3).with_retry_after_ms(250);
        assert_eq!(limit.max_active, Some(3));
        assert_eq!(limit.retry_after_ms, Some(250));

        let denial = TurnAdmissionCapacityDenial {
            axis_kind: TurnAdmissionAxisKind::Project,
            bucket_kind: TurnAdmissionBucketKind::Class,
            admission_class: Some(TurnAdmissionClass::interactive()),
            limit: 3,
            active_count: 4,
            retry_after_ms: Some(250),
        };
        let encoded = serde_json::to_vec(&denial).expect("serialize denial");
        assert_eq!(
            serde_json::from_slice::<TurnAdmissionCapacityDenial>(&encoded)
                .expect("deserialize denial"),
            denial
        );
    }

    #[test]
    fn admission_bucket_scopes_preserve_authority_dimensions() {
        let tenant = tenant_id();
        let user = user_id();
        let project = ProjectId::new("project-admission-test").expect("project");
        let agent = AgentId::new("agent-admission-test").expect("agent");
        let scopes = [
            TurnAdmissionBucketScope::Tenant {
                tenant_id: tenant.clone(),
            },
            TurnAdmissionBucketScope::ActorUser {
                tenant_id: tenant.clone(),
                user_id: user,
            },
            TurnAdmissionBucketScope::Project {
                tenant_id: tenant.clone(),
                project_id: Some(project),
            },
            TurnAdmissionBucketScope::Agent {
                tenant_id: tenant,
                agent_id: Some(agent),
            },
        ];

        for scope in scopes {
            let encoded = serde_json::to_vec(&scope).expect("serialize scope");
            assert_eq!(
                serde_json::from_slice::<TurnAdmissionBucketScope>(&encoded)
                    .expect("deserialize scope"),
                scope
            );
        }
    }
}
