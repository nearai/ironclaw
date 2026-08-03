//! What the operator-configuration capabilities tell a caller when the store
//! underneath them fails.
//!
//! Driven through `operator_config_capability::insert_handler` and the
//! resolved registry handlers — the seam composition wires
//! (`ironclaw_reborn_composition::factory`) — rather than the private helper,
//! so the branch selection (`state` string -> which store call runs) is under
//! test alongside the failure mapping itself.
//!
//! Lives here rather than in the crate's inline `mod tests` on purpose: this
//! is a whole extra fixture stack (three store doubles plus a catalog), and
//! appending it to `src/operator_config_capability.rs` pushed that file below
//! git's rename-similarity threshold against its pre-move home in
//! `ironclaw_extension_host`, which made the changed-code coverage gate treat
//! all 500 of its unchanged lines as newly added.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_approvals::{
    AutoApproveSettingStore, PersistentApprovalAction, PersistentApprovalPolicyError,
    PersistentApprovalPolicyKey, PersistentApprovalPolicyStore, ToolPermissionOverrideStore,
};
use ironclaw_extension_manager::operator_config_capability::insert_handler;
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    capability::{EffectKind, PermissionMode},
    dispatch::RuntimeDispatchErrorKind,
    ids::{CapabilityId, ExtensionId, InvocationId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    resource::ResourceScope,
    scope::Principal,
};
use ironclaw_host_runtime::{FirstPartyCapabilityRegistry, FirstPartyCapabilityRequest};
use ironclaw_product::{
    OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID,
    OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID,
};
use ironclaw_product_contracts::operator_tools::{
    RebornOperatorToolCatalog, RebornOperatorToolInfo,
};

/// Every store this pair of capabilities writes through can fail, and each
/// failure has to reach the caller as `Backend` — not as a success, and not as
/// an input error the WebUI would render as "you typed something wrong". Five
/// distinct writes reach a store, each on its own branch:
///
///   * the auto-approve toggle;
///   * `default`      -> the override *clear*;
///   * `always_allow` -> the persistent-policy *grant*;
///   * `ask_each_time`/`disabled` -> the override *set*;
///   * every non-`always_allow` branch -> the persistent-policy *revoke*,
///     whose `UnknownPolicy` is deliberately swallowed while any other error
///     must not be.
///
/// The last one matters most: `revoke_persistent_policy` treats "there was no
/// policy" as success, so a real backend failure folded into that same arm
/// would report a revoked auto-approval grant that is still live.
///
/// Each double delegates every *other* method to a working store, so the only
/// thing that differs between cases is the one failing call.
#[tokio::test]
async fn every_store_failure_surfaces_as_backend_rather_than_a_silent_success() {
    let user = UserId::new("operator").expect("user id");
    let scope =
        ResourceScope::local_default(user.clone(), InvocationId::new()).expect("resource scope");

    // 1. The auto-approve toggle.
    let stores = approval_stores();
    let mut registry = FirstPartyCapabilityRegistry::new();
    insert_handler(
        &mut registry,
        Arc::new(SetFailsAutoApproveStore),
        Arc::clone(&stores.overrides),
        Arc::clone(&stores.persistent_policies),
        Arc::new(StaticToolCatalog(Vec::new())),
    )
    .expect("insert handlers");
    let auto_approve_handler = registry
        .get(&CapabilityId::new(OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID).expect("id"))
        .expect("auto-approve handler");
    let error = auto_approve_handler
        .dispatch(authenticated_request(
            OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY_ID,
            &scope,
            &user,
            serde_json::json!({ "enabled": true }),
        ))
        .await
        .expect_err("a failed auto-approve write must not report success");
    assert_eq!(
        error.kind(),
        Some(RuntimeDispatchErrorKind::Backend),
        "an auto-approve store failure is a backend failure"
    );

    // 2-5. The four tool-permission writes.
    let capability_id = CapabilityId::new("ext.search").expect("capability id");
    let tool = search_tool(&capability_id);
    // (case, submitted state, the override store fails, the policy revoke fails)
    let cases: Vec<(&str, &str, bool, bool)> = vec![
        (
            "the override clear behind `default`",
            "default",
            true,
            false,
        ),
        (
            "the persistent grant behind `always_allow`",
            "always_allow",
            false,
            false,
        ),
        (
            "the override write behind `disabled`",
            "disabled",
            true,
            false,
        ),
        (
            "the policy revoke behind `ask_each_time`",
            "ask_each_time",
            false,
            true,
        ),
    ];
    for (case, state, overrides_fail, revoke_fails) in cases {
        let stores = approval_stores();
        let overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStorePort> =
            if overrides_fail {
                Arc::new(WriteFailsOverrideStore {
                    inner: Arc::clone(&stores.overrides),
                })
            } else {
                Arc::clone(&stores.overrides)
            };
        // `always_allow` is the one branch that never revokes, so its failure
        // has to come from the grant instead.
        let persistent_policies: Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStorePort> =
            Arc::new(FailingPersistentApprovalPolicyStore {
                inner: Arc::clone(&stores.persistent_policies),
                revoke_fails,
                allow_fails: state == "always_allow",
            });
        let mut registry = FirstPartyCapabilityRegistry::new();
        insert_handler(
            &mut registry,
            Arc::clone(&stores.auto_approve),
            overrides,
            persistent_policies,
            Arc::new(StaticToolCatalog(vec![tool.clone()])),
        )
        .expect("insert handlers");
        let handler = registry
            .get(&CapabilityId::new(OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID).expect("id"))
            .expect("tool permission handler");

        let Err(error) = handler
            .dispatch(authenticated_request(
                OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID,
                &scope,
                &user,
                serde_json::json!({
                    "capability_id": capability_id.as_str(),
                    "state": state,
                }),
            ))
            .await
        else {
            panic!("{case} must not report success");
        };
        assert_eq!(
            error.kind(),
            Some(RuntimeDispatchErrorKind::Backend),
            "{case} must reach the caller as a backend failure"
        );
    }
}

/// The counterpart the case above cannot assert: a `revoke` that answers
/// `UnknownPolicy` — "there was nothing to revoke" — is the normal path for a
/// tool that never had a persistent grant, and must stay a success. Without
/// this, folding every revoke error into the failure arm would still pass.
#[tokio::test]
async fn a_revoke_with_no_policy_to_revoke_is_not_a_failure() {
    let capability_id = CapabilityId::new("ext.search").expect("capability id");
    let tool = search_tool(&capability_id);
    let stores = approval_stores();
    let mut registry = FirstPartyCapabilityRegistry::new();
    insert_handler(
        &mut registry,
        Arc::clone(&stores.auto_approve),
        Arc::clone(&stores.overrides),
        Arc::clone(&stores.persistent_policies),
        Arc::new(StaticToolCatalog(vec![tool])),
    )
    .expect("insert handlers");
    let handler = registry
        .get(&CapabilityId::new(OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID).expect("id"))
        .expect("tool permission handler");
    let user = UserId::new("operator").expect("user id");
    let scope =
        ResourceScope::local_default(user.clone(), InvocationId::new()).expect("resource scope");

    // Nothing was ever granted, so the handler's revoke hits `UnknownPolicy`.
    assert!(matches!(
        stores
            .persistent_policies
            .revoke(&PersistentApprovalPolicyKey::new(
                &scope.tenant_user_settings_scope(),
                PersistentApprovalAction::Dispatch,
                capability_id.clone(),
                Principal::Extension(ExtensionId::new("ext").expect("provider id")),
            ))
            .await,
        Err(PersistentApprovalPolicyError::UnknownPolicy)
    ));

    let result = handler
        .dispatch(authenticated_request(
            OPERATOR_CONFIG_SET_TOOL_PERMISSION_CAPABILITY_ID,
            &scope,
            &user,
            serde_json::json!({
                "capability_id": capability_id.as_str(),
                "state": "ask_each_time",
            }),
        ))
        .await
        .expect("`nothing to revoke` is the ordinary case, not a backend failure");
    assert_eq!(result.output["state"], "ask_each_time");
}

fn search_tool(capability_id: &CapabilityId) -> RebornOperatorToolInfo {
    RebornOperatorToolInfo {
        capability_id: capability_id.clone(),
        provider: ExtensionId::new("ext").expect("provider id"),
        description: Arc::from("Search"),
        default_permission: PermissionMode::Ask,
        effects: Arc::<[EffectKind]>::from(vec![EffectKind::Network]),
    }
}

/// One authenticated operator call, shaped the way the product surface stamps
/// it: the scope's user is the verified actor.
fn authenticated_request(
    capability_id: &str,
    scope: &ResourceScope,
    user: &UserId,
    input: serde_json::Value,
) -> FirstPartyCapabilityRequest {
    let mut request = FirstPartyCapabilityRequest::request_for_test(
        CapabilityId::new(capability_id).expect("capability id"),
        scope.clone(),
        input,
        None,
    );
    request.authenticated_actor_user_id = Some(user.clone());
    request
}

struct ApprovalStores {
    auto_approve: Arc<dyn ironclaw_approvals::AutoApproveSettingStorePort>,
    overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStorePort>,
    persistent_policies: Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStorePort>,
}

/// The three real production approval stores over one in-memory filesystem, so
/// every call a case does not deliberately break behaves as it does in
/// production.
fn approval_stores() -> ApprovalStores {
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/approvals").expect("test approvals mount alias"),
            VirtualPath::new("/projects/approvals").expect("test approvals mount target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("test mount view"),
    ));
    ApprovalStores {
        auto_approve: Arc::new(AutoApproveSettingStore::new(Arc::clone(&scoped))),
        overrides: Arc::new(ToolPermissionOverrideStore::new(Arc::clone(&scoped))),
        persistent_policies: Arc::new(PersistentApprovalPolicyStore::new(scoped)),
    }
}

/// An auto-approve store whose write always fails.
struct SetFailsAutoApproveStore;

#[async_trait]
impl ironclaw_approvals::AutoApproveSettingStorePort for SetFailsAutoApproveStore {
    async fn set(
        &self,
        _input: ironclaw_approvals::AutoApproveSettingInput,
    ) -> Result<
        ironclaw_approvals::AutoApproveSettingRecord,
        ironclaw_approvals::ToolPermissionStoreError,
    > {
        Err(ironclaw_approvals::ToolPermissionStoreError::Filesystem(
            "injected auto-approve write failure".to_string(),
        ))
    }

    async fn get(
        &self,
        _key: &ironclaw_approvals::AutoApproveSettingKey,
    ) -> Result<
        Option<ironclaw_approvals::AutoApproveSettingRecord>,
        ironclaw_approvals::ToolPermissionStoreError,
    > {
        Ok(None)
    }
}

/// An override store whose *mutations* fail while reads keep working, so a case
/// can tell "the write failed" from "the store is gone".
struct WriteFailsOverrideStore {
    inner: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStorePort>,
}

#[async_trait]
impl ironclaw_approvals::CapabilityPermissionOverrideStorePort for WriteFailsOverrideStore {
    async fn set(
        &self,
        _input: ironclaw_approvals::CapabilityPermissionOverrideInput,
    ) -> Result<
        ironclaw_approvals::CapabilityPermissionOverrideRecord,
        ironclaw_approvals::CapabilityPermissionStoreError,
    > {
        Err(
            ironclaw_approvals::CapabilityPermissionStoreError::Filesystem(
                "injected override write failure".to_string(),
            ),
        )
    }

    async fn get(
        &self,
        key: &ironclaw_approvals::CapabilityPermissionOverrideKey,
    ) -> Result<
        Option<ironclaw_approvals::CapabilityPermissionOverrideRecord>,
        ironclaw_approvals::CapabilityPermissionStoreError,
    > {
        self.inner.get(key).await
    }

    async fn clear(
        &self,
        _key: &ironclaw_approvals::CapabilityPermissionOverrideKey,
    ) -> Result<(), ironclaw_approvals::CapabilityPermissionStoreError> {
        Err(
            ironclaw_approvals::CapabilityPermissionStoreError::Filesystem(
                "injected override clear failure".to_string(),
            ),
        )
    }
}

/// A persistent-policy store with each fallible call switched independently, so
/// a case can fail exactly one of `allow`/`revoke` and leave the other on its
/// real behaviour.
struct FailingPersistentApprovalPolicyStore {
    inner: Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStorePort>,
    revoke_fails: bool,
    allow_fails: bool,
}

#[async_trait]
impl ironclaw_approvals::PersistentApprovalPolicyStorePort
    for FailingPersistentApprovalPolicyStore
{
    async fn allow(
        &self,
        input: ironclaw_approvals::PersistentApprovalPolicyInput,
    ) -> Result<ironclaw_approvals::PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        if self.allow_fails {
            return Err(PersistentApprovalPolicyError::Filesystem(
                "injected persistent policy write failure".to_string(),
            ));
        }
        self.inner.allow(input).await
    }

    async fn lookup(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<Option<ironclaw_approvals::PersistentApprovalPolicy>, PersistentApprovalPolicyError>
    {
        self.inner.lookup(key).await
    }

    async fn revoke(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<ironclaw_approvals::PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        if self.revoke_fails {
            return Err(PersistentApprovalPolicyError::Filesystem(
                "injected persistent policy revoke failure".to_string(),
            ));
        }
        self.inner.revoke(key).await
    }

    async fn revoke_if_source_approval_request(
        &self,
        key: &PersistentApprovalPolicyKey,
        source_approval_request_id: ironclaw_host_api::ids::ApprovalRequestId,
    ) -> Result<Option<ironclaw_approvals::PersistentApprovalPolicy>, PersistentApprovalPolicyError>
    {
        self.inner
            .revoke_if_source_approval_request(key, source_approval_request_id)
            .await
    }
}

struct StaticToolCatalog(Vec<RebornOperatorToolInfo>);

#[async_trait]
impl RebornOperatorToolCatalog for StaticToolCatalog {
    async fn list_operator_tools(&self, _caller: &UserId) -> Vec<RebornOperatorToolInfo> {
        self.0.clone()
    }
}
