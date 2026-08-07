use std::collections::BTreeSet;

use ironclaw_host_api::ids::UserId;
use serde::{Deserialize, Serialize};

use crate::installations::ExtensionManifestRecord;
use crate::{ManagedUserMembership, ManagedUserMembershipError, UserMembership};

/// Whether a package definition follows its final installation into removal.
/// Existing definitions preserve the historical remove-with-last-install
/// behavior; tenant-registered catalog definitions opt into retention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageDefinitionRetention {
    #[default]
    RemoveWithLastInstallation,
    RetainInCatalog,
}

/// Result of the single-row immutable package-definition admission CAS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageDefinitionAdmissionOutcome {
    Created,
    ExactExisting,
}

/// Durable visibility authority for a user-registered package definition.
///
/// This is deliberately separate from installation ownership. Definition
/// membership controls catalog discovery before installation; an explicit
/// install action creates installation membership later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageDefinitionAudience {
    Managed(ManagedUserMembership),
    /// Compatibility shape for definition rows written before registration
    /// persisted caller identity. The host assigns visibility for this shape
    /// to its configured tenant operator; the registry does not guess policy.
    LegacyOwnerless,
}

impl PackageDefinitionAudience {
    pub fn managed_by(manager: UserId) -> Self {
        Self::Managed(ManagedUserMembership::managed_by(manager))
    }

    pub fn managed_members(
        manager_user_ids: BTreeSet<UserId>,
        member_user_ids: BTreeSet<UserId>,
    ) -> Result<Self, ManagedUserMembershipError> {
        ManagedUserMembership::with_managers_and_members(manager_user_ids, member_user_ids)
            .map(Self::Managed)
    }

    pub fn member_user_ids(&self) -> Option<&BTreeSet<UserId>> {
        match self {
            Self::Managed(managed) => Some(managed.membership().user_ids()),
            Self::LegacyOwnerless => None,
        }
    }

    pub fn membership(&self) -> Option<&UserMembership> {
        match self {
            Self::Managed(managed) => Some(managed.membership()),
            Self::LegacyOwnerless => None,
        }
    }

    pub fn manager_user_ids(&self) -> Option<&BTreeSet<UserId>> {
        match self {
            Self::Managed(managed) => Some(managed.managers().user_ids()),
            Self::LegacyOwnerless => None,
        }
    }

    pub fn visible_to(&self, caller: &UserId) -> bool {
        self.member_user_ids()
            .is_some_and(|user_ids| user_ids.contains(caller))
    }
}

/// One immutable catalog-admission row: the validated package definition and
/// the caller authority that may discover it before installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredPackageDefinition {
    definition: ExtensionManifestRecord,
    audience: PackageDefinitionAudience,
}

impl RegisteredPackageDefinition {
    pub fn managed_by(definition: ExtensionManifestRecord, manager: UserId) -> Self {
        Self {
            definition,
            audience: PackageDefinitionAudience::managed_by(manager),
        }
    }

    pub fn managed_members(
        definition: ExtensionManifestRecord,
        manager_user_ids: BTreeSet<UserId>,
        member_user_ids: BTreeSet<UserId>,
    ) -> Result<Self, ManagedUserMembershipError> {
        Ok(Self {
            definition,
            audience: PackageDefinitionAudience::managed_members(
                manager_user_ids,
                member_user_ids,
            )?,
        })
    }

    pub(crate) fn legacy_ownerless(definition: ExtensionManifestRecord) -> Self {
        Self {
            definition,
            audience: PackageDefinitionAudience::LegacyOwnerless,
        }
    }

    pub fn definition(&self) -> &ExtensionManifestRecord {
        &self.definition
    }

    pub fn audience(&self) -> &PackageDefinitionAudience {
        &self.audience
    }
}
