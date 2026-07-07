//! Private-install ownership and slot policy (#5459 P1, #5525 review).
//!
//! Pure decisions only — every store read/write, package registration, and
//! publish/unpublish stays in the parent module. Extracted so the ownership
//! rules (who owns a new install, who may operate an existing one, who may
//! take an occupied slot, and what an eviction must capture for restore) are
//! reviewable and testable in one place instead of interleaved with the
//! lifecycle I/O.

use ironclaw_extensions::{
    ExtensionInstallation, ExtensionManifestRecord, ExtensionPackage, InstallationOwner,
};
use ironclaw_host_api::{ExtensionId, UserId};
use ironclaw_product_workflow::{LifecycleInstallScope, ProductWorkflowError};

/// Derive who a NEW install belongs to (#5459 P1): the tenant operator
/// installs for the whole tenant; anyone else installs privately.
pub(super) fn derive_owner(caller: &UserId, tenant_operator: &UserId) -> InstallationOwner {
    if caller == tenant_operator {
        InstallationOwner::Tenant
    } else {
        InstallationOwner::user(caller.clone())
    }
}

/// Fail-closed visibility check for lifecycle mutations on an existing
/// installation: a user-private install is operable ONLY by its owner —
/// every non-owner, the tenant operator included, gets the masked denial.
/// The admin's sole power over a foreign private slot is the explicit
/// shared-install eviction granted by [`decide_occupied_slot`], never direct
/// activate/remove by id. The error is deliberately the same "is not
/// installed" shape a missing installation produces, so a foreign caller
/// cannot distinguish (or enumerate) other users' private installs.
pub(super) fn ensure_caller_may_operate(
    installation: &ExtensionInstallation,
    caller: &UserId,
) -> Result<(), ProductWorkflowError> {
    match installation.owner() {
        InstallationOwner::Tenant => Ok(()),
        InstallationOwner::User { user_id } if user_id == caller => Ok(()),
        InstallationOwner::User { .. } => Err(ProductWorkflowError::InvalidBindingRequest {
            reason: format!(
                "extension {} is not installed",
                installation.extension_id().as_str()
            ),
        }),
    }
}

/// The one takeover an occupied slot permits.
pub(super) enum OccupiedSlotDecision {
    /// Admin-wins (#5459 P1): a tenant claim seizes the slot by evicting the
    /// existing user-private install.
    EvictPrivateInstall,
}

/// Slot rules for an extension id whose installation row already exists.
///
/// Every rejection wording is part of the masking contract: a foreign member
/// probing another user's private slot learns only that the id is
/// "unavailable" — never that a private install exists or whose it is.
pub(super) fn decide_occupied_slot(
    extension_id: &ExtensionId,
    existing_owner: &InstallationOwner,
    claimant: &InstallationOwner,
) -> Result<OccupiedSlotDecision, ProductWorkflowError> {
    match (existing_owner, claimant) {
        (InstallationOwner::Tenant, InstallationOwner::User { .. }) => {
            Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "extension {} is already available as a shared tool",
                    extension_id.as_str()
                ),
            })
        }
        (InstallationOwner::Tenant, InstallationOwner::Tenant) => {
            Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!("extension {} is already installed", extension_id.as_str()),
            })
        }
        (InstallationOwner::User { user_id }, InstallationOwner::User { user_id: caller }) => {
            if user_id == caller {
                Err(ProductWorkflowError::InvalidBindingRequest {
                    reason: format!("extension {} is already installed", extension_id.as_str()),
                })
            } else {
                // Generic wording: a foreign caller must not learn that a
                // private install exists, let alone whose it is.
                Err(ProductWorkflowError::InvalidBindingRequest {
                    reason: format!("extension id {} is unavailable", extension_id.as_str()),
                })
            }
        }
        (InstallationOwner::User { .. }, InstallationOwner::Tenant) => {
            Ok(OccupiedSlotDecision::EvictPrivateInstall)
        }
    }
}

/// Pre-eviction snapshot of a user-private install, captured by
/// `evict_private_installation` so a tenant install that fails after eviction
/// can restore the victim untouched (#5525 review: non-interference on failed
/// shared installs).
pub(super) struct EvictedPrivateInstall {
    /// Manifest row as of eviction; `None` when a prior partial attempt
    /// already dropped it.
    pub(super) manifest: Option<ExtensionManifestRecord>,
    /// The victim's installation row (owner + activation state).
    pub(super) installation: ExtensionInstallation,
    /// The victim's registered lifecycle package; `None` on the retry path
    /// where a prior attempt already deregistered it.
    pub(super) lifecycle_package: Option<ExtensionPackage>,
}

/// Settings/list projection of an installation owner (#5459 P1).
pub(super) fn install_scope_for_owner(owner: &InstallationOwner) -> Option<LifecycleInstallScope> {
    Some(match owner {
        InstallationOwner::Tenant => LifecycleInstallScope::Shared,
        InstallationOwner::User { .. } => LifecycleInstallScope::Private,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ironclaw_extensions::{
        ExtensionActivationState, ExtensionInstallationId, ExtensionManifestRef,
    };

    fn user(id: &str) -> UserId {
        UserId::new(id).expect("valid user")
    }

    fn installation(owner: InstallationOwner) -> ExtensionInstallation {
        let ext_id = ExtensionId::new("fixture").expect("extension id");
        ExtensionInstallation::new(
            ExtensionInstallationId::new("fixture").expect("installation id"),
            ext_id.clone(),
            ExtensionActivationState::Enabled,
            ExtensionManifestRef::new(ext_id, None),
            Vec::new(),
            Utc::now(),
            owner,
        )
        .expect("installation")
    }

    #[test]
    fn derive_owner_maps_operator_to_tenant_and_members_to_private() {
        let operator = user("operator");
        assert!(derive_owner(&operator, &operator).is_tenant());
        assert_eq!(
            derive_owner(&user("alice"), &operator)
                .as_user()
                .map(UserId::as_str),
            Some("alice")
        );
    }

    #[test]
    fn ensure_caller_may_operate_masks_every_non_owner_including_operator() {
        let tenant_owned = installation(InstallationOwner::Tenant);
        let private = installation(InstallationOwner::user(user("alice")));

        ensure_caller_may_operate(&tenant_owned, &user("bob")).expect("tenant tools are shared");
        ensure_caller_may_operate(&private, &user("alice")).expect("the owner operates her tool");
        for non_owner in ["bob", "operator"] {
            let error = ensure_caller_may_operate(&private, &user(non_owner))
                .expect_err("non-owners must be denied");
            let rendered = error.to_string();
            assert!(
                rendered.contains("is not installed") && !rendered.contains("alice"),
                "denial must mask the private install: {rendered}"
            );
        }
    }

    #[test]
    fn decide_occupied_slot_permits_only_tenant_over_private() {
        let extension_id = ExtensionId::new("fixture").expect("extension id");
        let tenant = InstallationOwner::Tenant;
        let alice = InstallationOwner::user(user("alice"));
        let bob = InstallationOwner::user(user("bob"));

        assert!(matches!(
            decide_occupied_slot(&extension_id, &alice, &tenant),
            Ok(OccupiedSlotDecision::EvictPrivateInstall)
        ));
        for (existing, claimant, expected) in [
            (&tenant, &alice, "already available as a shared tool"),
            (&tenant, &tenant, "already installed"),
            (&alice, &alice, "already installed"),
            (&alice, &bob, "is unavailable"),
        ] {
            let error = decide_occupied_slot(&extension_id, existing, claimant)
                .err()
                .expect("occupied slot rejects this claim");
            let rendered = error.to_string();
            assert!(rendered.contains(expected), "unexpected: {rendered}");
            assert!(
                !rendered.contains("alice"),
                "slot error must not leak the private owner: {rendered}"
            );
        }
    }

    #[test]
    fn install_scope_projects_owner_kind() {
        assert_eq!(
            install_scope_for_owner(&InstallationOwner::Tenant),
            Some(LifecycleInstallScope::Shared)
        );
        assert_eq!(
            install_scope_for_owner(&InstallationOwner::user(user("alice"))),
            Some(LifecycleInstallScope::Private)
        );
    }
}
