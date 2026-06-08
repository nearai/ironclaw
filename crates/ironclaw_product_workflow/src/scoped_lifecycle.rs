//! Scoped lifecycle ownership model for production Reborn package administration.
//!
//! This module is intentionally package-kind agnostic. Extensions, skills, MCP,
//! and WASM adapters project into these records; admin/user inheritance is
//! resolved here instead of being reimplemented in each product surface.

use std::collections::BTreeMap;
use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{TenantId, UserId};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Value;

use crate::{
    LifecyclePackageRef, ProductWorkflowError,
    lifecycle::{LIFECYCLE_ID_MAX_BYTES, lifecycle_package_kind_label, validate_lifecycle_string},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopedLifecycleInstallationId(String);

impl ScopedLifecycleInstallationId {
    pub fn new(value: impl Into<String>) -> Result<Self, ProductWorkflowError> {
        validate_lifecycle_string(
            value.into(),
            "scoped lifecycle installation id",
            LIFECYCLE_ID_MAX_BYTES,
        )
        .map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for ScopedLifecycleInstallationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ScopedLifecycleInstallationId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ScopedLifecycleInstallationId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopedLifecycleActorRole {
    Admin,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedLifecycleActor {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub role: ScopedLifecycleActorRole,
}

impl ScopedLifecycleActor {
    pub fn admin(tenant_id: TenantId, user_id: UserId) -> Self {
        Self {
            tenant_id,
            user_id,
            role: ScopedLifecycleActorRole::Admin,
        }
    }

    pub fn user(tenant_id: TenantId, user_id: UserId) -> Self {
        Self {
            tenant_id,
            user_id,
            role: ScopedLifecycleActorRole::User,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScopedLifecycleOwnership {
    AdminShared {
        tenant_id: TenantId,
    },
    UserPrivate {
        tenant_id: TenantId,
        user_id: UserId,
    },
}

impl ScopedLifecycleOwnership {
    pub fn tenant_id(&self) -> &TenantId {
        match self {
            Self::AdminShared { tenant_id } | Self::UserPrivate { tenant_id, .. } => tenant_id,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::AdminShared { .. } => "admin_shared",
            Self::UserPrivate { .. } => "user_private",
        }
    }

    pub fn is_visible_to(&self, subject: &ScopedLifecycleSubject) -> bool {
        match self {
            Self::AdminShared { tenant_id } => tenant_id == &subject.tenant_id,
            Self::UserPrivate { tenant_id, user_id } => {
                tenant_id == &subject.tenant_id && user_id == &subject.user_id
            }
        }
    }

    pub fn can_be_mutated_by(&self, actor: &ScopedLifecycleActor) -> bool {
        match self {
            Self::AdminShared { tenant_id } => {
                tenant_id == &actor.tenant_id && actor.role == ScopedLifecycleActorRole::Admin
            }
            Self::UserPrivate { tenant_id, user_id } => {
                tenant_id == &actor.tenant_id && user_id == &actor.user_id
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedLifecycleSubject {
    pub tenant_id: TenantId,
    pub user_id: UserId,
}

impl ScopedLifecycleSubject {
    pub fn new(tenant_id: TenantId, user_id: UserId) -> Self {
        Self { tenant_id, user_id }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScopedLifecycleInstallation {
    pub installation_id: ScopedLifecycleInstallationId,
    pub package_ref: LifecyclePackageRef,
    pub ownership: ScopedLifecycleOwnership,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
    pub created_by: ScopedLifecycleActor,
    pub updated_by: ScopedLifecycleActor,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ScopedLifecycleInstallation {
    pub fn admin_shared(
        installation_id: ScopedLifecycleInstallationId,
        package_ref: LifecyclePackageRef,
        actor: ScopedLifecycleActor,
        now: DateTime<Utc>,
    ) -> Result<Self, ProductWorkflowError> {
        if actor.role != ScopedLifecycleActorRole::Admin {
            return Err(ProductWorkflowError::BindingAccessDenied);
        }
        Ok(Self {
            installation_id,
            package_ref,
            ownership: ScopedLifecycleOwnership::AdminShared {
                tenant_id: actor.tenant_id.clone(),
            },
            enabled: true,
            config: None,
            created_by: actor.clone(),
            updated_by: actor,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn user_private(
        installation_id: ScopedLifecycleInstallationId,
        package_ref: LifecyclePackageRef,
        actor: ScopedLifecycleActor,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            installation_id,
            package_ref,
            ownership: ScopedLifecycleOwnership::UserPrivate {
                tenant_id: actor.tenant_id.clone(),
                user_id: actor.user_id.clone(),
            },
            enabled: true,
            config: None,
            created_by: actor.clone(),
            updated_by: actor,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn tenant_id(&self) -> &TenantId {
        self.ownership.tenant_id()
    }

    pub fn can_be_mutated_by(&self, actor: &ScopedLifecycleActor) -> bool {
        self.ownership.can_be_mutated_by(actor)
    }

    pub fn validate(&self) -> Result<(), ProductWorkflowError> {
        if !self.can_be_mutated_by(&self.created_by) {
            return invalid_installation("created_by cannot create scoped lifecycle installation");
        }
        if !self.can_be_mutated_by(&self.updated_by) {
            return invalid_installation("updated_by cannot mutate scoped lifecycle installation");
        }
        if self.created_at > self.updated_at {
            return invalid_installation("created_at must not be after updated_at");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpsertScopedLifecycleInstallationRequest {
    pub actor: ScopedLifecycleActor,
    pub installation: ScopedLifecycleInstallation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteScopedLifecycleInstallationRequest {
    pub actor: ScopedLifecycleActor,
    pub tenant_id: TenantId,
    pub installation_id: ScopedLifecycleInstallationId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectiveScopedLifecycleInstallations {
    pub subject: ScopedLifecycleSubject,
    pub installations: Vec<ScopedLifecycleInstallation>,
}

pub fn resolve_effective_scoped_lifecycle_installations(
    subject: ScopedLifecycleSubject,
    candidates: impl IntoIterator<Item = ScopedLifecycleInstallation>,
) -> EffectiveScopedLifecycleInstallations {
    let mut effective = BTreeMap::<String, ScopedLifecycleInstallation>::new();

    for installation in candidates {
        if !installation.enabled || !installation.ownership.is_visible_to(&subject) {
            continue;
        }
        let key = package_key(&installation.package_ref);
        match installation.ownership {
            ScopedLifecycleOwnership::AdminShared { .. } => {
                effective.entry(key).or_insert(installation);
            }
            ScopedLifecycleOwnership::UserPrivate { .. } => {
                effective.insert(key, installation);
            }
        }
    }

    EffectiveScopedLifecycleInstallations {
        subject,
        installations: effective.into_values().collect(),
    }
}

#[async_trait]
pub trait ScopedLifecycleInstallationStore: Send + Sync {
    async fn upsert_installation(
        &self,
        request: UpsertScopedLifecycleInstallationRequest,
    ) -> Result<(), ProductWorkflowError>;

    async fn get_installation(
        &self,
        tenant_id: &TenantId,
        installation_id: &ScopedLifecycleInstallationId,
    ) -> Result<Option<ScopedLifecycleInstallation>, ProductWorkflowError>;

    async fn delete_installation(
        &self,
        request: DeleteScopedLifecycleInstallationRequest,
    ) -> Result<(), ProductWorkflowError>;

    async fn list_installations(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<ScopedLifecycleInstallation>, ProductWorkflowError>;

    async fn list_effective_installations(
        &self,
        subject: ScopedLifecycleSubject,
    ) -> Result<EffectiveScopedLifecycleInstallations, ProductWorkflowError> {
        let candidates = self.list_installations(&subject.tenant_id).await?;
        Ok(resolve_effective_scoped_lifecycle_installations(
            subject, candidates,
        ))
    }
}

fn package_key(package_ref: &LifecyclePackageRef) -> String {
    format!(
        "{}:{}",
        lifecycle_package_kind_label(package_ref.kind),
        package_ref.id.as_str()
    )
}

fn invalid_installation<T>(reason: &'static str) -> Result<T, ProductWorkflowError> {
    Err(ProductWorkflowError::InvalidBindingRequest {
        reason: reason.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::{LifecyclePackageKind, LifecyclePackageRef};

    #[test]
    fn effective_resolution_inherits_shared_and_user_private_installations() {
        let tenant = tenant("tenant-alpha");
        let admin = ScopedLifecycleActor::admin(tenant.clone(), user("admin-alpha"));
        let user = ScopedLifecycleActor::user(tenant.clone(), user("user-alpha"));
        let subject = ScopedLifecycleSubject::new(tenant, user.user_id.clone());

        let shared = ScopedLifecycleInstallation::admin_shared(
            install_id("shared-github"),
            package("github"),
            admin,
            Utc::now(),
        )
        .expect("admin shared install");
        let private = ScopedLifecycleInstallation::user_private(
            install_id("private-notion"),
            package("notion"),
            user,
            Utc::now(),
        );

        let effective = resolve_effective_scoped_lifecycle_installations(
            subject,
            [shared.clone(), private.clone()],
        );

        assert_eq!(effective.installations, vec![shared, private]);
    }

    #[test]
    fn user_private_installation_overrides_shared_package_for_same_user() {
        let tenant = tenant("tenant-alpha");
        let admin = ScopedLifecycleActor::admin(tenant.clone(), user("admin-alpha"));
        let user = ScopedLifecycleActor::user(tenant.clone(), user("user-alpha"));
        let subject = ScopedLifecycleSubject::new(tenant, user.user_id.clone());
        let package_ref = package("github");

        let shared = ScopedLifecycleInstallation::admin_shared(
            install_id("shared-github"),
            package_ref.clone(),
            admin,
            Utc::now(),
        )
        .expect("admin shared install");
        let private = ScopedLifecycleInstallation::user_private(
            install_id("private-github"),
            package_ref,
            user,
            Utc::now(),
        );

        let effective =
            resolve_effective_scoped_lifecycle_installations(subject, [shared, private.clone()]);

        assert_eq!(effective.installations, vec![private]);
    }

    #[test]
    fn normal_user_cannot_mutate_admin_shared_installation() {
        let tenant = tenant("tenant-alpha");
        let admin = ScopedLifecycleActor::admin(tenant.clone(), user("admin-alpha"));
        let user = ScopedLifecycleActor::user(tenant, user("user-alpha"));
        let shared = ScopedLifecycleInstallation::admin_shared(
            install_id("shared-github"),
            package("github"),
            admin,
            Utc::now(),
        )
        .expect("admin shared install");

        assert!(!shared.can_be_mutated_by(&user));
    }

    #[test]
    fn validation_rejects_inconsistent_audit_actor() {
        let tenant = tenant("tenant-alpha");
        let owner = ScopedLifecycleActor::user(tenant.clone(), user("user-alpha"));
        let other_user = ScopedLifecycleActor::user(tenant, user("user-beta"));
        let mut installation = ScopedLifecycleInstallation::user_private(
            install_id("private-github"),
            package("github"),
            owner,
            Utc::now(),
        );
        installation.updated_by = other_user;

        assert!(matches!(
            installation.validate(),
            Err(ProductWorkflowError::InvalidBindingRequest { .. })
        ));
    }

    fn tenant(id: &str) -> TenantId {
        TenantId::new(id).expect("valid tenant")
    }

    fn user(id: &str) -> UserId {
        UserId::new(id).expect("valid user")
    }

    fn install_id(id: &str) -> ScopedLifecycleInstallationId {
        ScopedLifecycleInstallationId::new(id).expect("valid installation id")
    }

    fn package(id: &str) -> LifecyclePackageRef {
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, id).expect("valid package")
    }
}
