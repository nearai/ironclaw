//! Membership install policy (#5459 P1, 2026-07-08 pivot).
//!
//! Pure decisions only — every store read/write, package registration, and
//! publish/unpublish stays in the parent module. Extracted so the ownership
//! rules (who holds a new install, who may operate an existing one, and what
//! a repeated install of the same id does) are reviewable and testable in
//! one place instead of interleaved with the lifecycle I/O.
//!
//! Contract (`docs/plans/2026-07-01-private-tool-installs.md`): a tenant
//! (operator) install makes a tool available to everyone; a member install
//! makes it available to that member — and any number of members can
//! independently install the same tool by joining the one installation
//! row's member set. An operator install of a member-held id EVICTS every
//! member's private installation by replacing the member set with `Tenant`
//! in a single row write.

use std::collections::BTreeSet;

use ironclaw_extensions::{ExtensionInstallation, InstallationOwner};
use ironclaw_host_api::UserId;
use ironclaw_product_workflow::{LifecycleInstallScope, ProductWorkflowError};

/// Derive who a NEW install belongs to (#5459 P1): the tenant operator
/// installs for the whole tenant; anyone else installs for themselves.
pub(super) fn derive_owner(caller: &UserId, tenant_operator: &UserId) -> InstallationOwner {
    if caller == tenant_operator {
        InstallationOwner::Tenant
    } else {
        InstallationOwner::user(caller.clone())
    }
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
    if installation.owner().visible_to(caller) {
        return Ok(());
    }
    Err(ProductWorkflowError::InvalidBindingRequest {
        reason: format!(
            "extension {} is not installed",
            installation.extension_id().as_str()
        ),
    })
}

/// Membership rules for an extension id whose installation row already
/// exists. `Err` is always "already installed": under membership the
/// outcome of installing never depends on whether OTHER users hold the
/// tool, so there is no ownership state left to mask on this path.
pub(super) fn decide_install_on_existing(
    extension_id: &ironclaw_host_api::ExtensionId,
    existing_owner: &InstallationOwner,
    caller: &UserId,
    tenant_operator: &UserId,
) -> Result<InstallationOwner, ProductWorkflowError> {
    let already_installed = || ProductWorkflowError::InvalidBindingRequest {
        reason: format!("extension {} is already installed", extension_id.as_str()),
    };
    match existing_owner {
        // A tenant-shared tool is already available to every caller.
        InstallationOwner::Tenant => Err(already_installed()),
        InstallationOwner::Users { user_ids } => {
            if caller == tenant_operator {
                // Operator install evicts every member's private
                // installation; the tenant row takes the id and everyone
                // (the evicted members included) reaches the tool through
                // the shared install.
                Ok(InstallationOwner::Tenant)
            } else if user_ids.contains(caller) {
                Err(already_installed())
            } else {
                // JOIN: the caller becomes a member alongside the others.
                let mut user_ids = user_ids.clone();
                user_ids.insert(caller.clone());
                Ok(InstallationOwner::users(user_ids).map_err(|error| {
                    ProductWorkflowError::InvalidBindingRequest {
                        reason: format!("installation owner update failed: {error}"),
                    }
                })?)
            }
        }
    }
}

/// What a member's remove does to the installation row.
pub(super) enum RemoveDecision {
    /// Other members still hold the tool: the caller leaves the member set
    /// in a single row rewrite; no teardown.
    LeaveMembers(InstallationOwner),
    /// The caller is the last holder (sole member, or the operator removing
    /// a tenant-shared tool): full teardown.
    TearDown,
}

/// Membership rules for removing an installation the caller may operate
/// (callers must pass [`ensure_caller_may_operate`] first; tenant rows are
/// additionally operator-only to remove, enforced by the parent module).
pub(super) fn decide_remove(
    existing_owner: &InstallationOwner,
    caller: &UserId,
) -> Result<RemoveDecision, ProductWorkflowError> {
    match existing_owner {
        InstallationOwner::Tenant => Ok(RemoveDecision::TearDown),
        InstallationOwner::Users { user_ids } => {
            let remaining: BTreeSet<UserId> = user_ids
                .iter()
                .filter(|member| *member != caller)
                .cloned()
                .collect();
            if remaining.is_empty() {
                Ok(RemoveDecision::TearDown)
            } else {
                Ok(RemoveDecision::LeaveMembers(
                    InstallationOwner::users(remaining).map_err(|error| {
                        ProductWorkflowError::InvalidBindingRequest {
                            reason: format!("installation owner update failed: {error}"),
                        }
                    })?,
                ))
            }
        }
    }
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
    use ironclaw_extensions::{
        ExtensionActivationState, ExtensionInstallationId, ExtensionManifestRef,
    };
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
            ExtensionActivationState::Enabled,
            ExtensionManifestRef::new(ext_id, None),
            Vec::new(),
            Utc::now(),
            owner,
        )
        .expect("installation")
    }

    #[test]
    fn derive_owner_maps_operator_to_tenant_and_members_to_singleton() {
        let operator = user("operator");
        assert!(derive_owner(&operator, &operator).is_tenant());
        let alice = user("alice");
        assert!(derive_owner(&alice, &operator).visible_to(&alice));
        assert!(!derive_owner(&alice, &operator).is_tenant());
    }

    #[test]
    fn ensure_caller_may_operate_masks_every_non_member_including_operator() {
        let tenant_owned = installation(InstallationOwner::Tenant);
        let held = installation(members(&["alice", "bob"]));

        ensure_caller_may_operate(&tenant_owned, &user("carol")).expect("tenant tools are shared");
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
    fn decide_install_joins_members_and_evicts_to_tenant_for_operator() {
        let extension_id = ExtensionId::new("fixture").expect("extension id");
        let operator = user("operator");

        // Member joins an existing member set.
        let Ok(joined) = decide_install_on_existing(
            &extension_id,
            &members(&["alice"]),
            &user("bob"),
            &operator,
        ) else {
            panic!("a second member must join");
        };
        assert!(joined.visible_to(&user("alice")) && joined.visible_to(&user("bob")));

        // Operator evicts the whole member set to Tenant.
        let Ok(evicted) = decide_install_on_existing(
            &extension_id,
            &members(&["alice", "bob"]),
            &operator,
            &operator,
        ) else {
            panic!("operator install must evict to tenant");
        };
        assert!(evicted.is_tenant());

        // Real duplicates are "already installed" — and never leak members.
        for (existing, caller) in [
            (InstallationOwner::Tenant, user("alice")),
            (InstallationOwner::Tenant, operator.clone()),
            (members(&["alice"]), user("alice")),
        ] {
            let error = decide_install_on_existing(&extension_id, &existing, &caller, &operator)
                .expect_err("duplicate install rejected");
            let rendered = error.to_string();
            assert!(
                rendered.contains("already installed") && !rendered.contains("alice"),
                "unexpected: {rendered}"
            );
        }
    }

    #[test]
    fn decide_remove_leaves_members_until_the_last_holder() {
        let Ok(RemoveDecision::LeaveMembers(remaining)) =
            decide_remove(&members(&["alice", "bob"]), &user("alice"))
        else {
            panic!("a member with co-holders leaves the set");
        };
        assert!(!remaining.visible_to(&user("alice")) && remaining.visible_to(&user("bob")));

        assert!(matches!(
            decide_remove(&members(&["bob"]), &user("bob")),
            Ok(RemoveDecision::TearDown)
        ));
        assert!(matches!(
            decide_remove(&InstallationOwner::Tenant, &user("operator")),
            Ok(RemoveDecision::TearDown)
        ));
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
