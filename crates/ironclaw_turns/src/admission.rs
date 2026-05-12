use std::collections::HashMap;

use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use serde::{Deserialize, Serialize};

use crate::{TurnActor, TurnRunId, TurnScope};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnAdmissionReservationRecord {
    pub run_id: TurnRunId,
    pub admission_class: TurnAdmissionClass,
    pub buckets: Vec<TurnAdmissionBucket>,
    pub released: bool,
}

pub(crate) fn admission_buckets(
    scope: &TurnScope,
    actor: &TurnActor,
    admission_class: &TurnAdmissionClass,
) -> Vec<TurnAdmissionBucket> {
    let tenant_id = scope.tenant_id.clone();
    let total_and_class = |axis_kind: TurnAdmissionAxisKind, scope: TurnAdmissionBucketScope| {
        [
            TurnAdmissionBucket {
                axis_kind,
                bucket_kind: TurnAdmissionBucketKind::Total,
                admission_class: None,
                scope: scope.clone(),
            },
            TurnAdmissionBucket {
                axis_kind,
                bucket_kind: TurnAdmissionBucketKind::Class,
                admission_class: Some(admission_class.clone()),
                scope,
            },
        ]
    };

    let mut buckets = Vec::with_capacity(8);
    buckets.extend(total_and_class(
        TurnAdmissionAxisKind::Tenant,
        TurnAdmissionBucketScope::Tenant {
            tenant_id: tenant_id.clone(),
        },
    ));
    buckets.extend(total_and_class(
        TurnAdmissionAxisKind::ActorUser,
        TurnAdmissionBucketScope::ActorUser {
            tenant_id: tenant_id.clone(),
            user_id: actor.user_id.clone(),
        },
    ));
    buckets.extend(total_and_class(
        TurnAdmissionAxisKind::Project,
        TurnAdmissionBucketScope::Project {
            tenant_id: tenant_id.clone(),
            project_id: scope.project_id.clone(),
        },
    ));
    buckets.extend(total_and_class(
        TurnAdmissionAxisKind::Agent,
        TurnAdmissionBucketScope::Agent {
            tenant_id,
            agent_id: scope.agent_id.clone(),
        },
    ));
    buckets
}
