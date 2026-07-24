//! Product-boundary projection for caller-owned installations.
//!
//! Membership joins/leaves are domain transitions owned by
//! `ironclaw_extensions::InstallationOwner`. This module only maps that
//! domain state onto product-boundary concerns: initial caller ownership,
//! masked authorization errors, and the legacy install-scope wire.
//!
//! Admin configuration is tenant-scoped, but installation/removal and
//! manifest-declared personal setup are
//! always caller-scoped. Multiple callers share the immutable package/runtime
//! publication while their membership and derived readiness remain independent.
//! `InstallationOwner::Tenant` is accepted only as a persisted
//! compatibility shape and is narrowed before normal operation.

use ironclaw_extensions::{ExtensionInstallation, InstallationOwner};
use ironclaw_host_api::UserId;
use ironclaw_product::{LifecycleInstallScope, ProductWorkflowError};

/// Every install is caller-owned. Tenant-scoped extension configuration is a
/// separate manifest-declared deployment concern and never installs a tool on
/// behalf of users.
pub(super) fn derive_owner(caller: &UserId) -> InstallationOwner {
    InstallationOwner::user(caller.clone())
}

/// Fail-closed visibility check for lifecycle mutations on an existing
/// installation: a member-held install is operable ONLY by its members —
/// every non-member, the tenant operator included, gets the masked denial.
/// The error is deliberately the same "is not installed" shape a missing
/// installation produces, so a non-member cannot distinguish (or enumerate)
/// tools other users hold.
pub(super) fn ensure_caller_may_operate(
    installation: &ExtensionInstallation,
    caller: &UserId,
) -> Result<(), ProductWorkflowError> {
    if !installation.owner().is_tenant() && installation.owner().visible_to(caller) {
        return Ok(());
    }
    Err(ProductWorkflowError::InvalidBindingRequest {
        reason: format!(
            "extension {} is not installed",
            installation.extension_id().as_str()
        ),
    })
}

/// Settings/list projection of an installation owner (#5459 P1). Rows are
/// caller-filtered before projection, so a member-held row shown to a
/// viewer is by construction one they hold — "mine".
pub(super) fn install_scope_for_owner(owner: &InstallationOwner) -> LifecycleInstallScope {
    match owner {
        InstallationOwner::Tenant => LifecycleInstallScope::Shared,
        InstallationOwner::Users { .. } => LifecycleInstallScope::Private,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ironclaw_extensions::{ExtensionInstallationId, ExtensionManifestRef};
    use ironclaw_host_api::ExtensionId;

    fn user(id: &str) -> UserId {
        UserId::new(id).expect("valid user")
    }

    fn members(ids: &[&str]) -> InstallationOwner {
        InstallationOwner::users(ids.iter().map(|id| user(id)).collect()).expect("member set")
    }

    fn installation(owner: InstallationOwner) -> ExtensionInstallation {
        let ext_id = ExtensionId::new("fixture").expect("extension id");
        ExtensionInstallation::new(
            ExtensionInstallationId::new("fixture").expect("installation id"),
            ext_id.clone(),
            ExtensionManifestRef::new(ext_id, None),
            Vec::new(),
            Utc::now(),
            owner,
        )
        .expect("installation")
    }

    #[test]
    fn derive_owner_maps_every_caller_to_a_private_installation() {
        let operator = user("operator");
        assert!(derive_owner(&operator).visible_to(&operator));
        assert!(!derive_owner(&operator).is_tenant());
        let alice = user("alice");
        assert!(derive_owner(&alice).visible_to(&alice));
        assert!(!derive_owner(&alice).is_tenant());
    }

    #[test]
    fn ensure_caller_may_operate_masks_every_non_member_including_operator() {
        let tenant_owned = installation(InstallationOwner::Tenant);
        let held = installation(members(&["alice", "bob"]));

        ensure_caller_may_operate(&tenant_owned, &user("carol"))
            .expect_err("legacy tenant rows are not user installations");
        for member in ["alice", "bob"] {
            ensure_caller_may_operate(&held, &user(member)).expect("members operate their tool");
        }
        for non_member in ["carol", "operator"] {
            let error = ensure_caller_may_operate(&held, &user(non_member))
                .expect_err("non-members must be denied");
            let rendered = error.to_string();
            assert!(
                rendered.contains("is not installed")
                    && !rendered.contains("alice")
                    && !rendered.contains("bob"),
                "denial must mask the membership: {rendered}"
            );
        }
    }

    #[test]
    fn install_scope_projects_owner_kind() {
        assert_eq!(
            install_scope_for_owner(&InstallationOwner::Tenant),
            LifecycleInstallScope::Shared
        );
        assert_eq!(
            install_scope_for_owner(&members(&["alice"])),
            LifecycleInstallScope::Private
        );
    }
}
