//! The operator tool-catalog port (PROPOSAL §6.1.3).
//!
//! The operator/settings surface lists the capabilities a caller may see and
//! set per-tool permissions on them. What tools exist is an extension-host
//! question (it owns the active snapshot), so the catalog is a port defined at
//! the product boundary and implemented below it — the same inversion as
//! [`crate::delivery`].
//!
//! Never here: permission policy, override storage, or any catalog
//! implementation.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    capability::{EffectKind, PermissionMode},
    ids::{CapabilityId, ExtensionId, UserId},
};

/// One tool as the operator surface sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornOperatorToolInfo {
    pub capability_id: CapabilityId,
    pub provider: ExtensionId,
    pub description: Arc<str>,
    pub default_permission: PermissionMode,
    pub effects: Arc<[EffectKind]>,
}

#[async_trait]
pub trait RebornOperatorToolCatalog: Send + Sync {
    /// Tools visible to `caller` in the operator/settings surface (#5459 P1).
    ///
    /// The settings/tools routes are authenticated-caller routes (not
    /// operator-gated), so a member reads this catalog. It MUST therefore be
    /// filtered by installation owner exactly like the model capability
    /// surface: tenant-shared tools for everyone, user-private tools only for
    /// their owner. An unfiltered catalog would disclose another user's
    /// private install (its capability id, description, effects) — the leak
    /// this parameter closes.
    async fn list_operator_tools(&self, caller: &UserId) -> Vec<RebornOperatorToolInfo>;
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_host_api::ids::CapabilityId;

    static_assertions::assert_obj_safe!(RebornOperatorToolCatalog);

    /// A catalog that applies the ownership filter the port's doc comment
    /// requires. It is a *double*, so it cannot prove the production catalog
    /// filters — that lives in composition and is tested there. What it does
    /// prove is the property this crate owns: the port hands the
    /// implementation the caller, and its shape admits a per-caller answer.
    /// A port that dropped `caller` (or returned one global list) could not
    /// satisfy this test at all.
    struct OwnershipFilteredCatalog;

    const SHARED: &str = "builtin.http_fetch";
    const ALICE_PRIVATE: &str = "alice.private_tool";
    const BOB_PRIVATE: &str = "bob.private_tool";

    fn tool(capability_id: &str) -> RebornOperatorToolInfo {
        RebornOperatorToolInfo {
            capability_id: CapabilityId::new(capability_id).expect("valid capability id"),
            provider: ExtensionId::new("web_access").expect("valid extension id"),
            description: Arc::from("a tool"),
            default_permission: PermissionMode::Ask,
            effects: Arc::from(Vec::new()),
        }
    }

    #[async_trait]
    impl RebornOperatorToolCatalog for OwnershipFilteredCatalog {
        async fn list_operator_tools(&self, caller: &UserId) -> Vec<RebornOperatorToolInfo> {
            let mut tools = vec![tool(SHARED)];
            match caller.as_str() {
                "alice" => tools.push(tool(ALICE_PRIVATE)),
                "bob" => tools.push(tool(BOB_PRIVATE)),
                _ => {}
            }
            tools
        }
    }

    async fn ids_for(catalog: &dyn RebornOperatorToolCatalog, user: &str) -> Vec<String> {
        catalog
            .list_operator_tools(&UserId::new(user).expect("valid user id"))
            .await
            .into_iter()
            .map(|info| info.capability_id.as_str().to_string())
            .collect()
    }

    #[tokio::test]
    async fn the_catalog_is_caller_scoped_so_a_private_install_cannot_cross_callers() {
        // #5459 P1: the settings/tools routes are authenticated-caller routes,
        // not operator-gated, so a member reads this catalog. The `caller`
        // parameter is the disclosure control; a catalog that ignored it would
        // hand every member every other member's private installs.
        let catalog: Arc<dyn RebornOperatorToolCatalog> = Arc::new(OwnershipFilteredCatalog);

        let alice = ids_for(catalog.as_ref(), "alice").await;
        let bob = ids_for(catalog.as_ref(), "bob").await;

        assert!(alice.contains(&SHARED.to_string()));
        assert!(bob.contains(&SHARED.to_string()));

        assert!(alice.contains(&ALICE_PRIVATE.to_string()));
        assert!(
            !alice.contains(&BOB_PRIVATE.to_string()),
            "alice must not see bob's private install: {alice:?}"
        );
        assert!(bob.contains(&BOB_PRIVATE.to_string()));
        assert!(
            !bob.contains(&ALICE_PRIVATE.to_string()),
            "bob must not see alice's private install: {bob:?}"
        );
        assert_ne!(alice, bob, "the answer must be able to differ by caller");
    }

    #[tokio::test]
    async fn a_caller_with_no_private_installs_still_gets_the_shared_set() {
        // An empty-for-this-caller answer must be reachable and must not be an
        // error, or a fresh tenant cannot render the settings surface at all.
        let catalog: Arc<dyn RebornOperatorToolCatalog> = Arc::new(OwnershipFilteredCatalog);
        assert_eq!(
            ids_for(catalog.as_ref(), "carol").await,
            vec![SHARED.to_string()]
        );
    }
}
