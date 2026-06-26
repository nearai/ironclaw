//! Per-`(tenant, user)` capability **availability** resolution sourced from the
//! #4544 scoped-lifecycle installation store.
//!
//! The agent loop consults a [`CapabilitySurfaceProfileResolver`] once per turn
//! at host construction (`crates/ironclaw_reborn/src/loop_driver_host.rs`) to
//! decide which capabilities the model may see. The shipped defaults don't read
//! identity: local-dev uses `AllowAll` (everything), production uses `Empty`
//! (nothing). This resolver instead derives the allow-set from what an admin
//! actually granted — it builds a [`ScopedLifecycleSubject`] from the turn's
//! [`LoopRunContext`], asks the [`ScopedLifecycleInstallationStore`] for the
//! effective installations (admin-shared → every user in the tenant;
//! user-private → only its owner; disabled excluded), and maps each installed
//! package to the capability ids it exposes.
//!
//! Availability here is the **installation** signal (issue #5267). Layering the
//! configuration / identity / approval dimensions on top
//! (`resolve_effective_policy`) is the follow-on #5273.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, TenantId, UserId, VirtualPath};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
};
use ironclaw_product_workflow::{
    LifecyclePackageRef, ScopedLifecycleInstallationStore, ScopedLifecycleSubject,
    lifecycle_package_kind_label,
};
use ironclaw_product_workflow_storage::FilesystemScopedLifecycleInstallationStore;
use ironclaw_turns::run_profile::LoopRunContext;
use ironclaw_turns::scope::{TurnActor, TurnScope};

#[cfg(feature = "capability-policy")]
use ironclaw_capability_policy::{
    CapabilityPolicyDeltaStore, PolicyResolver, PolicySubject, StaticCapabilityDefaultPolicySource,
    StoreBackedPolicyResolver,
};
#[cfg(feature = "capability-policy")]
use ironclaw_product_workflow_storage::FilesystemCapabilityPolicyDeltaStore;

use crate::available_extensions::{AvailableExtensionCatalog, visible_capability_ids};

/// Durable virtual root for the local-dev scoped-lifecycle installation store.
///
/// The store's own default root is `/engine/...`, which has **no mounted
/// backend** in the local-dev composite filesystem (it mounts `/tenants`,
/// `/memory`, `/events`, `/projects`, `/system/extensions`). Rooting under the
/// `/tenants` libSQL-backed durable mount makes installs survive restart. The
/// raw composite is used (not a per-user `ScopedFilesystem`), so an admin's
/// `AdminShared` install is tenant-shared — visible to every user's resolver —
/// while the store's own path scheme keeps per-tenant isolation.
///
/// **Both** the availability resolver (#5267, read) and the admin write surface
/// (#5268) must construct the store via [`local_dev_scoped_lifecycle_store`]
/// with this root so writes are visible to reads.
pub(crate) const LOCAL_DEV_SCOPED_LIFECYCLE_ROOT: &str = "/tenants/capability_policy";

/// Construct the local-dev scoped-lifecycle installation store over the durable
/// `/tenants` mount (see [`LOCAL_DEV_SCOPED_LIFECYCLE_ROOT`]).
pub(crate) fn local_dev_scoped_lifecycle_store(
    filesystem: Arc<dyn ironclaw_filesystem::RootFilesystem>,
) -> Arc<dyn ScopedLifecycleInstallationStore> {
    let root = VirtualPath::new(LOCAL_DEV_SCOPED_LIFECYCLE_ROOT)
        .expect("LOCAL_DEV_SCOPED_LIFECYCLE_ROOT is a valid virtual path");
    Arc::new(FilesystemScopedLifecycleInstallationStore::with_root(
        filesystem, root,
    ))
}

/// Durable virtual root for the local-dev capability-policy **delta** store
/// (#5273). Sits under the SAME mounted prefix as the installation store
/// (`/tenants/capability_policy`, the durable libSQL mount) but in a sibling
/// subtree (`/policy_deltas`) so delta leaves never collide with the lifecycle
/// store's `/installations` / `/installation_ids` leaves.
#[cfg(feature = "capability-policy")]
pub(crate) const LOCAL_DEV_CAPABILITY_POLICY_DELTA_ROOT: &str =
    "/tenants/capability_policy/policy_deltas";

/// Construct the local-dev capability-policy delta store over the durable
/// `/tenants` mount. This is the SINGLE delta-store the runtime builds — the
/// dispatch `PolicyResolver` reads it and the admin REST write surface (#5268 /
/// #5273) writes it. Both share this backing so writes are visible to reads.
#[cfg(feature = "capability-policy")]
pub(crate) fn local_dev_capability_policy_delta_store(
    filesystem: Arc<dyn ironclaw_filesystem::RootFilesystem>,
) -> Arc<dyn CapabilityPolicyDeltaStore> {
    let root = VirtualPath::new(LOCAL_DEV_CAPABILITY_POLICY_DELTA_ROOT)
        .expect("LOCAL_DEV_CAPABILITY_POLICY_DELTA_ROOT is a valid virtual path");
    Arc::new(FilesystemCapabilityPolicyDeltaStore::with_root(
        filesystem, root,
    ))
}

/// Build the milestone `PolicyResolver` over the shared delta store (#5261 D3).
///
/// The default source uses the milestone default
/// ([`CapabilityDefaultPolicy::available_default`](ironclaw_capability_policy::CapabilityDefaultPolicy::available_default)):
/// installed == available unless an admin delta hides it, with no admin opinion
/// on identity/approval/config. The resolver is the read path for every
/// dimension; it shares the SAME delta-store `Arc` the admin write surface
/// holds — never construct a second.
#[cfg(feature = "capability-policy")]
pub(crate) fn build_capability_policy_resolver(
    delta_store: Arc<dyn CapabilityPolicyDeltaStore>,
) -> Arc<dyn PolicyResolver> {
    let defaults = StaticCapabilityDefaultPolicySource::new(
        ironclaw_capability_policy::CapabilityDefaultPolicy::available_default(),
    );
    Arc::new(StoreBackedPolicyResolver::new(defaults, delta_store))
}

/// Adapts the shared [`PolicyResolver`] into the loop's host-owned
/// [`LoopCapabilityConfigSource`] (#5261 configuration dimension). Supplies the
/// admin policy config (`EffectivePolicy.config`) to deep-merge into a
/// capability's resolved input before dispatch.
///
/// Holds the SAME resolver `Arc` the availability seam reads, so config and
/// availability stay consistent for a turn.
#[cfg(feature = "capability-policy")]
pub(crate) struct PolicyResolverConfigSource {
    policy: Arc<dyn PolicyResolver>,
}

#[cfg(feature = "capability-policy")]
impl PolicyResolverConfigSource {
    pub(crate) fn new(policy: Arc<dyn PolicyResolver>) -> Self {
        Self { policy }
    }
}

#[cfg(feature = "capability-policy")]
#[async_trait]
impl ironclaw_loop_support::LoopCapabilityConfigSource for PolicyResolverConfigSource {
    async fn config_for(
        &self,
        run_context: &LoopRunContext,
        capability_id: &CapabilityId,
    ) -> Result<Option<serde_json::Value>, ironclaw_turns::run_profile::AgentLoopHostError> {
        // Use the SAME acting-principal derivation the availability seam uses.
        let Some(user_id) = principal_user_id(&run_context.scope, run_context.actor.as_ref())
        else {
            // Ownerless / actor-fallback turn: no subject, so no admin config to
            // overlay. Fail-OPEN — the model input dispatches un-merged.
            return Ok(None);
        };
        let subject = PolicySubject {
            tenant_id: run_context.scope.tenant_id.clone(),
            user_id: user_id.clone(),
        };
        match self.policy.resolve(&subject, capability_id).await {
            Ok(effective) => {
                // `available_default()` config is `Null`; a delta with no
                // `config_patch` leaves it `Null`. Treat `Null` as "no admin
                // opinion" so the merge is skipped entirely.
                if effective.config.is_null() {
                    Ok(None)
                } else {
                    Ok(Some(effective.config))
                }
            }
            Err(error) => {
                // Fail-OPEN (#5261 D5 configuration): a resolver fault or an
                // unavailable backend must NOT end the turn. Drop the admin
                // config and dispatch the un-merged model input. Logged at debug
                // so it never corrupts the REPL/TUI surface (see CLAUDE.md).
                tracing::debug!(
                    %error,
                    capability = %capability_id.as_str(),
                    "capability policy config resolution failed; dispatching un-merged input"
                );
                Ok(None)
            }
        }
    }
}

/// Adapts the shared [`PolicyResolver`] into the dep-light approval module's
/// host-owned [`AdminApprovalSource`] (#5261 D6 approval dimension). Supplies
/// the admin (org-wide) approval opinion (`EffectivePolicy.approval`) so the
/// dispatch approval chain can apply admin Deny/Allow precedence.
///
/// Holds the SAME resolver `Arc` the availability and config seams read, so all
/// three dimensions stay consistent for a turn.
#[cfg(feature = "capability-policy")]
pub(crate) struct PolicyResolverAdminApprovalSource {
    policy: Arc<dyn PolicyResolver>,
}

#[cfg(feature = "capability-policy")]
impl PolicyResolverAdminApprovalSource {
    pub(crate) fn new(policy: Arc<dyn PolicyResolver>) -> Self {
        Self { policy }
    }
}

#[cfg(feature = "capability-policy")]
#[async_trait]
impl crate::profile_approval_authorization::AdminApprovalSource
    for PolicyResolverAdminApprovalSource
{
    async fn admin_approval(
        &self,
        scope: &ironclaw_host_api::ResourceScope,
        capability_id: &CapabilityId,
    ) -> Option<ironclaw_host_api::PermissionMode> {
        let subject = PolicySubject {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
        };
        match self.policy.resolve(&subject, capability_id).await {
            // Only a definite admin opinion (Allow/Deny) is surfaced; `Ask`
            // carries no org-wide decision, so it maps to `None` ("no admin
            // opinion") exactly like a missing row or a fault. This makes the
            // trait doc literally true and lets the dispatch chain fall through
            // to the existing user/profile steps (require_approval_for_profile_policy
            // matches only `Some(Allow)` / `Some(Deny)`, so `None` for `Ask`
            // preserves the fall-through).
            Ok(effective) => match effective.approval {
                ironclaw_host_api::PermissionMode::Allow
                | ironclaw_host_api::PermissionMode::Deny => Some(effective.approval),
                ironclaw_host_api::PermissionMode::Ask => None,
            },
            Err(error) => {
                // Fail-SAFE (#5261 D5 approval): a resolver fault must NOT
                // auto-approve (privilege escalation). Returning `None` makes
                // the dispatch chain treat this as "no admin opinion" and fall
                // through to the existing user/profile steps. Logged at debug so
                // it never corrupts the REPL/TUI surface (see CLAUDE.md).
                tracing::debug!(
                    %error,
                    capability = %capability_id.as_str(),
                    "capability policy approval resolution failed; deferring to user/profile chain"
                );
                None
            }
        }
    }
}

/// Whether the per-`(tenant, user)` capability policy resolver (#5267 / #5261)
/// is active for this runtime. Compiled in by the `capability-policy` feature,
/// but OFF unless `IRONCLAW_REBORN_CAPABILITY_POLICY` is set truthy
/// (`1` / `true` / `yes` / `on`). Enabling the feature alone therefore never
/// changes local-dev behaviour — the operator opts in. Mirrors the
/// `HooksActivationConfig` master-flag-default-off shape with an env toggle.
///
/// Lives here (shared `pub(crate)`) so both `runtime.rs` (availability seam
/// construction) and `factory.rs` (shared delta-store / resolver handle
/// construction) gate on the same switch.
#[cfg(feature = "capability-policy")]
pub(crate) fn capability_policy_activated() -> bool {
    std::env::var("IRONCLAW_REBORN_CAPABILITY_POLICY")
        .map(|value| matches!(value.trim(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

/// Maps an installed package to the capability ids it makes visible to the
/// model. The bare `ScopedLifecycleInstallation` carries only a
/// [`LifecyclePackageRef`]; the capability ids live on the extension manifest
/// (`[capabilities]`, `Visibility::Model`), so resolution needs this lookup.
pub(crate) trait PackageCapabilitySource: Send + Sync {
    fn capabilities_for(&self, package_ref: &LifecyclePackageRef) -> Vec<CapabilityId>;
}

/// A precomputed package → capability-ids map.
///
/// Built once at composition from the [`AvailableExtensionCatalog`]: no
/// cross-boundary facade exists for a live `(tenant, package_ref)` → cap-ids
/// lookup, and `product_workflow` cannot depend on the composition-scoped
/// lifecycle, so the mapping is materialized up front from manifest data.
///
/// Keyed by a stable `kind:id` string because [`LifecyclePackageRef`] is
/// neither `Hash` nor `Ord` (matches the store's own `package_key`).
pub(crate) struct StaticPackageCapabilitySource {
    by_package: HashMap<String, Vec<CapabilityId>>,
}

/// Stable map key for a package ref: `"<kind>:<id>"` (e.g. `extension:web-access`).
fn package_key(package_ref: &LifecyclePackageRef) -> String {
    format!(
        "{}:{}",
        lifecycle_package_kind_label(package_ref.kind),
        package_ref.id.as_str()
    )
}

impl StaticPackageCapabilitySource {
    pub(crate) fn new(
        entries: impl IntoIterator<Item = (LifecyclePackageRef, Vec<CapabilityId>)>,
    ) -> Self {
        let by_package = entries
            .into_iter()
            .map(|(package_ref, capabilities)| (package_key(&package_ref), capabilities))
            .collect();
        Self { by_package }
    }

    /// Seed from an extension catalog: each available extension's `package_ref`
    /// maps to its model-visible capability ids. Covers WASM extensions (the
    /// Acme example's `slack` / `gmail` / `web-access`); skills / MCP /
    /// WASM-kind packages don't expose cap-ids this way yet, so they simply map
    /// to no capabilities and contribute nothing to the allow-set.
    pub(crate) fn from_catalog(catalog: &AvailableExtensionCatalog) -> Self {
        Self::new(catalog.search("").map(|package| {
            (
                package.package_ref.clone(),
                visible_capability_ids(package).cloned().collect::<Vec<_>>(),
            )
        }))
    }
}

impl PackageCapabilitySource for StaticPackageCapabilitySource {
    fn capabilities_for(&self, package_ref: &LifecyclePackageRef) -> Vec<CapabilityId> {
        self.by_package
            .get(&package_key(package_ref))
            .cloned()
            .unwrap_or_default()
    }
}

/// Resolves the per-turn capability allow-set from the scoped-lifecycle
/// installation store. See the module docs.
pub(crate) struct ScopedLifecyclePolicyCapabilitySurfaceResolver {
    installations: Arc<dyn ScopedLifecycleInstallationStore>,
    packages: Arc<dyn PackageCapabilitySource>,
    /// Policy view (#5273): the installed allow-set is intersected with the
    /// capabilities whose [`EffectivePolicy.available`] is `true`. Required (not
    /// `Option`) under the feature — the only construction site always supplies
    /// the shared resolver — so there is no production-always-Some optional Arc.
    #[cfg(feature = "capability-policy")]
    policy: Arc<dyn PolicyResolver>,
}

impl ScopedLifecyclePolicyCapabilitySurfaceResolver {
    pub(crate) fn new(
        installations: Arc<dyn ScopedLifecycleInstallationStore>,
        packages: Arc<dyn PackageCapabilitySource>,
        #[cfg(feature = "capability-policy")] policy: Arc<dyn PolicyResolver>,
    ) -> Self {
        Self {
            installations,
            packages,
            #[cfg(feature = "capability-policy")]
            policy,
        }
    }

    /// Testable core: resolve the allow-set for an explicit principal.
    ///
    /// Fail-closed but *graceful*: this resolver never returns `Err`.
    /// `resolve()` runs at host construction and an `Err` would abort the whole
    /// turn (`HostFactoryError`), so every adverse case denies all capabilities
    /// (empty allowlist) instead — the turn still runs and the user can chat,
    /// while no ungranted capability is ever exposed. The adverse cases:
    ///
    /// - no resolvable user (ownerless / actor-fallback turn);
    /// - a user with no grants;
    /// - a store read failure, *including* a not-yet-created installation set
    ///   (a fresh tenant) — logged, then denied.
    async fn resolve_allow_set(
        &self,
        tenant_id: &TenantId,
        user_id: Option<&UserId>,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        let Some(user_id) = user_id else {
            return Ok(CapabilityAllowSet::Allowlist(BTreeSet::new()));
        };
        let subject = ScopedLifecycleSubject::new(tenant_id.clone(), user_id.clone());
        let effective = match self
            .installations
            .list_effective_installations(subject)
            .await
        {
            Ok(effective) => effective,
            Err(error) => {
                // Denying all (rather than `Err`) keeps a transient store fault
                // — or a tenant that simply has no installations yet — from
                // killing the turn. Logged at debug so it never corrupts the
                // REPL/TUI surface (see CLAUDE.md logging guidance).
                tracing::debug!(
                    %error,
                    "scoped-lifecycle installation lookup failed; denying all capabilities for this turn"
                );
                return Ok(CapabilityAllowSet::Allowlist(BTreeSet::new()));
            }
        };
        let mut allowed = BTreeSet::new();
        for installation in effective.installations {
            allowed.extend(self.packages.capabilities_for(&installation.package_ref));
        }
        // Availability = installed AND not hidden by policy. Off-feature this is
        // the installed-only set (unchanged). On-feature it is the intersection
        // of the installed set with `{capability : EffectivePolicy.available}`.
        #[cfg(feature = "capability-policy")]
        let allowed = {
            let subject = PolicySubject {
                tenant_id: tenant_id.clone(),
                user_id: user_id.clone(),
            };
            let mut available = BTreeSet::new();
            for capability in allowed {
                match self.policy.resolve(&subject, &capability).await {
                    Ok(effective) if effective.available => {
                        available.insert(capability);
                    }
                    // Policy hid this capability → drop it (intersection excludes).
                    Ok(_) => {}
                    Err(error) => {
                        // Fail-closed: a policy fault denies THIS capability for
                        // the turn. Never propagate `Err` — `resolve()` runs at
                        // host construction and an `Err` would abort the whole
                        // turn. Logged at debug so it never corrupts the
                        // REPL/TUI surface (see CLAUDE.md logging guidance).
                        tracing::debug!(
                            %error,
                            capability = %capability.as_str(),
                            "capability policy resolution failed; denying this capability for the turn"
                        );
                    }
                }
            }
            available
        };
        Ok(CapabilityAllowSet::Allowlist(allowed))
    }
}

/// The acting principal for capability availability: the turn's actor (the user
/// driving it) first, then the explicit thread owner. A shared (room-agent)
/// account resolves to its own `UserId` here when a turn is driven as it.
/// Returns `None` for an ownerless / actor-fallback turn → the resolver fails
/// closed to an empty allow-set.
fn principal_user_id<'a>(scope: &'a TurnScope, actor: Option<&'a TurnActor>) -> Option<&'a UserId> {
    actor
        .map(|actor| &actor.user_id)
        .or_else(|| scope.explicit_owner_user_id())
}

#[async_trait]
impl CapabilitySurfaceProfileResolver for ScopedLifecyclePolicyCapabilitySurfaceResolver {
    async fn resolve(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        let user_id = principal_user_id(&run_context.scope, run_context.actor.as_ref());
        self.resolve_allow_set(&run_context.scope.tenant_id, user_id)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;
    use ironclaw_host_api::ThreadId;
    use ironclaw_product_workflow::{
        DeleteScopedLifecycleInstallationRequest, LifecyclePackageKind, ProductWorkflowError,
        ScopedLifecycleActor, ScopedLifecycleInstallation, ScopedLifecycleInstallationId,
        UpsertScopedLifecycleInstallationRequest,
    };

    const TENANT: &str = "tenant:acme";

    fn tenant() -> TenantId {
        TenantId::from_trusted(TENANT.to_string())
    }

    fn user(id: &str) -> UserId {
        UserId::from_trusted(id.to_string())
    }

    fn cap(id: &str) -> CapabilityId {
        CapabilityId::new(id).expect("valid capability id")
    }

    fn pkg(id: &str) -> LifecyclePackageRef {
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, id).expect("valid package ref")
    }

    fn install_id(id: &str) -> ScopedLifecycleInstallationId {
        ScopedLifecycleInstallationId::new(id).expect("valid installation id")
    }

    /// `StaticPackageCapabilitySource` over a fixed map — `web-access` exposes
    /// `nearai.web_search`, `gmail` exposes `gmail.send_message`.
    fn static_source() -> Arc<dyn PackageCapabilitySource> {
        Arc::new(StaticPackageCapabilitySource::new([
            (pkg("web-access"), vec![cap("nearai.web_search")]),
            (pkg("gmail"), vec![cap("gmail.send_message")]),
        ]))
    }

    /// In-memory fake exercising the real default `list_effective_installations`
    /// (which filters `enabled` + ownership-visibility via the production
    /// `resolve_effective_scoped_lifecycle_installations`).
    struct FakeStore {
        installations: Vec<ScopedLifecycleInstallation>,
    }

    #[async_trait]
    impl ScopedLifecycleInstallationStore for FakeStore {
        async fn upsert_installation(
            &self,
            _request: UpsertScopedLifecycleInstallationRequest,
        ) -> Result<(), ProductWorkflowError> {
            Ok(())
        }

        async fn get_installation(
            &self,
            _tenant_id: &TenantId,
            _installation_id: &ScopedLifecycleInstallationId,
        ) -> Result<Option<ScopedLifecycleInstallation>, ProductWorkflowError> {
            Ok(None)
        }

        async fn delete_installation(
            &self,
            _request: DeleteScopedLifecycleInstallationRequest,
        ) -> Result<(), ProductWorkflowError> {
            Ok(())
        }

        async fn list_installations(
            &self,
            _tenant_id: &TenantId,
        ) -> Result<Vec<ScopedLifecycleInstallation>, ProductWorkflowError> {
            Ok(self.installations.clone())
        }
    }

    /// A store whose list always fails — models a backend fault (or a
    /// not-yet-created installation set that the filesystem surfaces as an
    /// error).
    struct FailingStore;

    #[async_trait]
    impl ScopedLifecycleInstallationStore for FailingStore {
        async fn upsert_installation(
            &self,
            _request: UpsertScopedLifecycleInstallationRequest,
        ) -> Result<(), ProductWorkflowError> {
            Ok(())
        }

        async fn get_installation(
            &self,
            _tenant_id: &TenantId,
            _installation_id: &ScopedLifecycleInstallationId,
        ) -> Result<Option<ScopedLifecycleInstallation>, ProductWorkflowError> {
            Ok(None)
        }

        async fn delete_installation(
            &self,
            _request: DeleteScopedLifecycleInstallationRequest,
        ) -> Result<(), ProductWorkflowError> {
            Ok(())
        }

        async fn list_installations(
            &self,
            _tenant_id: &TenantId,
        ) -> Result<Vec<ScopedLifecycleInstallation>, ProductWorkflowError> {
            Err(ProductWorkflowError::InvalidBindingRequest {
                reason: "store unavailable".to_string(),
            })
        }
    }

    fn admin_shared(installation: &str, package: &str) -> ScopedLifecycleInstallation {
        let actor = ScopedLifecycleActor::admin(tenant(), user("user:director"));
        ScopedLifecycleInstallation::admin_shared(
            install_id(installation),
            pkg(package),
            actor,
            Utc::now(),
        )
        .expect("admin actor may create an admin-shared installation")
    }

    fn user_private(installation: &str, package: &str, owner: &str) -> ScopedLifecycleInstallation {
        let actor = ScopedLifecycleActor::user(tenant(), user(owner));
        ScopedLifecycleInstallation::user_private(
            install_id(installation),
            pkg(package),
            actor,
            Utc::now(),
        )
    }

    fn resolver(
        installations: Vec<ScopedLifecycleInstallation>,
    ) -> ScopedLifecyclePolicyCapabilitySurfaceResolver {
        ScopedLifecyclePolicyCapabilitySurfaceResolver::new(
            Arc::new(FakeStore { installations }),
            static_source(),
            // The default-source default is `available_default()` (Available),
            // with no deltas, so the policy intersection is a no-op and the
            // existing installed-only assertions still hold.
            #[cfg(feature = "capability-policy")]
            available_default_policy(),
        )
    }

    /// A policy resolver whose default is `available_default()` (Available) and
    /// that holds the given deltas. With no deltas the intersection is a no-op.
    #[cfg(feature = "capability-policy")]
    fn policy_with_deltas(
        deltas: Vec<ironclaw_capability_policy::CapabilityPolicyDelta>,
    ) -> Arc<dyn PolicyResolver> {
        let store = ironclaw_capability_policy::InMemoryCapabilityPolicyDeltaStore::new();
        for delta in deltas {
            // The InMemory store's upsert is synchronous enough to drive on a
            // local runtime; block on it for the test fixture.
            futures::executor::block_on(store.upsert_delta(&tenant(), delta))
                .expect("seed policy delta");
        }
        build_capability_policy_resolver(Arc::new(store))
    }

    #[cfg(feature = "capability-policy")]
    fn available_default_policy() -> Arc<dyn PolicyResolver> {
        policy_with_deltas(Vec::new())
    }

    #[cfg(feature = "capability-policy")]
    fn hide_user_delta(
        cap_id: &str,
        user_id: &str,
    ) -> ironclaw_capability_policy::CapabilityPolicyDelta {
        ironclaw_capability_policy::CapabilityPolicyDelta {
            scope: ironclaw_capability_policy::PolicyScope::User {
                user_id: user(user_id),
            },
            capability: cap(cap_id),
            availability: Some(ironclaw_capability_policy::Availability::Hidden),
            identity: None,
            approval: None,
            config_patch: None,
        }
    }

    fn allow_ids(set: &CapabilityAllowSet) -> BTreeSet<CapabilityId> {
        match set {
            CapabilityAllowSet::Allowlist(ids) => ids.clone(),
            other => panic!("resolver must return an allowlist, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn admin_shared_install_is_visible_to_any_user_in_tenant() {
        let resolver = resolver(vec![admin_shared("install-web", "web-access")]);

        for who in ["user:bob", "user:carol", "user:director"] {
            let set = resolver
                .resolve_allow_set(&tenant(), Some(&user(who)))
                .await
                .expect("resolve");
            assert!(
                allow_ids(&set).contains(&cap("nearai.web_search")),
                "admin-shared web-access must be visible to {who}"
            );
        }
    }

    #[tokio::test]
    async fn user_private_install_is_visible_only_to_its_owner() {
        let resolver = resolver(vec![user_private("install-gmail", "gmail", "user:bob")]);

        let bob = resolver
            .resolve_allow_set(&tenant(), Some(&user("user:bob")))
            .await
            .expect("resolve");
        assert!(
            allow_ids(&bob).contains(&cap("gmail.send_message")),
            "owner sees their own private install"
        );

        let carol = resolver
            .resolve_allow_set(&tenant(), Some(&user("user:carol")))
            .await
            .expect("resolve");
        assert!(
            !allow_ids(&carol).contains(&cap("gmail.send_message")),
            "a non-owner must not see another user's private install"
        );
    }

    #[tokio::test]
    async fn disabled_install_is_excluded() {
        let mut disabled = admin_shared("install-web", "web-access");
        disabled.enabled = false;
        let resolver = resolver(vec![disabled]);

        let set = resolver
            .resolve_allow_set(&tenant(), Some(&user("user:bob")))
            .await
            .expect("resolve");
        assert!(
            allow_ids(&set).is_empty(),
            "a disabled installation contributes no capabilities"
        );
    }

    #[tokio::test]
    async fn store_failure_denies_all_without_erroring() {
        let resolver = ScopedLifecyclePolicyCapabilitySurfaceResolver::new(
            Arc::new(FailingStore),
            static_source(),
            #[cfg(feature = "capability-policy")]
            available_default_policy(),
        );

        let set = resolver
            .resolve_allow_set(&tenant(), Some(&user("user:bob")))
            .await
            .expect("a store failure must not surface as Err — that would kill the turn");
        assert!(
            allow_ids(&set).is_empty(),
            "store failure → deny all (empty allowlist), graceful fail-closed"
        );
    }

    #[tokio::test]
    async fn no_resolvable_user_fails_closed_to_empty_allowlist() {
        let resolver = resolver(vec![admin_shared("install-web", "web-access")]);

        let set = resolver
            .resolve_allow_set(&tenant(), None)
            .await
            .expect("resolve");
        assert!(
            allow_ids(&set).is_empty(),
            "no principal → deny all (empty allowlist), never All and never Err"
        );
    }

    #[tokio::test]
    async fn package_without_capability_mapping_contributes_nothing() {
        // `shell` is not in the static source → no capabilities, but the
        // mapped `web-access` install still surfaces its capability.
        let resolver = resolver(vec![
            admin_shared("install-shell", "shell"),
            admin_shared("install-web", "web-access"),
        ]);

        let set = resolver
            .resolve_allow_set(&tenant(), Some(&user("user:bob")))
            .await
            .expect("resolve");
        let ids = allow_ids(&set);
        assert_eq!(ids.len(), 1, "only the mapped package contributes");
        assert!(ids.contains(&cap("nearai.web_search")));
    }

    /// A policy resolver that always errors — models a delta-store fault. The
    /// availability seam must deny the capability (drop it), never propagate.
    #[cfg(feature = "capability-policy")]
    struct FailingPolicyResolver;

    #[cfg(feature = "capability-policy")]
    #[async_trait]
    impl PolicyResolver for FailingPolicyResolver {
        async fn resolve(
            &self,
            _subject: &PolicySubject,
            _capability: &CapabilityId,
        ) -> Result<
            ironclaw_capability_policy::EffectivePolicy,
            ironclaw_capability_policy::PolicyError,
        > {
            Err(ironclaw_capability_policy::PolicyError::Unavailable {
                reason: "policy store down".to_string(),
            })
        }
    }

    /// Installed but policy-Hidden (a User-scope `Hidden` delta) → dropped from
    /// the allow-set even though the package is installed.
    #[cfg(feature = "capability-policy")]
    #[tokio::test]
    async fn installed_but_policy_hidden_is_dropped() {
        let resolver = ScopedLifecyclePolicyCapabilitySurfaceResolver::new(
            Arc::new(FakeStore {
                installations: vec![admin_shared("install-web", "web-access")],
            }),
            static_source(),
            policy_with_deltas(vec![hide_user_delta("nearai.web_search", "user:bob")]),
        );

        let bob = resolver
            .resolve_allow_set(&tenant(), Some(&user("user:bob")))
            .await
            .expect("resolve");
        assert!(
            allow_ids(&bob).is_empty(),
            "a policy-Hidden capability is removed even though installed"
        );

        // Carol has no hide delta → the same install stays available to her.
        let carol = resolver
            .resolve_allow_set(&tenant(), Some(&user("user:carol")))
            .await
            .expect("resolve");
        assert!(
            allow_ids(&carol).contains(&cap("nearai.web_search")),
            "the hide delta is per-user; Carol still sees the install"
        );
    }

    /// A policy resolver `Err` denies that capability for the turn (fail-closed)
    /// and never surfaces as `Err` (which would kill the turn).
    #[cfg(feature = "capability-policy")]
    #[tokio::test]
    async fn policy_error_deny_closes_without_erroring() {
        let resolver = ScopedLifecyclePolicyCapabilitySurfaceResolver::new(
            Arc::new(FakeStore {
                installations: vec![admin_shared("install-web", "web-access")],
            }),
            static_source(),
            Arc::new(FailingPolicyResolver),
        );

        let set = resolver
            .resolve_allow_set(&tenant(), Some(&user("user:bob")))
            .await
            .expect("a policy fault must not surface as Err — that would kill the turn");
        assert!(
            allow_ids(&set).is_empty(),
            "policy resolver Err → deny that capability (empty allowlist), graceful fail-closed"
        );
    }

    /// A capability that policy would make available but that is NOT installed
    /// never appears — the intersection only narrows the installed set.
    #[cfg(feature = "capability-policy")]
    #[tokio::test]
    async fn policy_available_but_not_installed_never_appears() {
        // No installations at all; the default policy is Available for every
        // capability, but with nothing installed the allow-set is empty.
        let resolver = ScopedLifecyclePolicyCapabilitySurfaceResolver::new(
            Arc::new(FakeStore {
                installations: Vec::new(),
            }),
            static_source(),
            available_default_policy(),
        );

        let set = resolver
            .resolve_allow_set(&tenant(), Some(&user("user:bob")))
            .await
            .expect("resolve");
        assert!(
            allow_ids(&set).is_empty(),
            "policy-available but not installed → never in the allow-set"
        );
    }

    #[test]
    fn principal_prefers_actor_then_explicit_owner_then_none() {
        let thread = ThreadId::from_trusted("thread:acme".to_string());

        // actor present → actor wins over the explicit owner.
        let scope =
            TurnScope::new_with_owner(tenant(), None, None, thread.clone(), Some(user("user:bob")));
        let actor = TurnActor::new(user("user:carol"));
        assert_eq!(
            principal_user_id(&scope, Some(&actor)),
            Some(&user("user:carol")),
        );

        // no actor → fall back to the explicit thread owner.
        assert_eq!(principal_user_id(&scope, None), Some(&user("user:bob")),);

        // ownerless + no actor → None (resolver then fails closed).
        let ownerless = TurnScope::new_with_owner(tenant(), None, None, thread, None);
        assert_eq!(principal_user_id(&ownerless, None), None);
    }

    #[test]
    fn static_source_from_first_party_catalog_is_populated() {
        let catalog =
            AvailableExtensionCatalog::from_first_party_assets().expect("first-party catalog");
        let source = StaticPackageCapabilitySource::from_catalog(&catalog);

        assert!(
            !source.by_package.is_empty(),
            "catalog seeding must yield package entries"
        );
        let total_caps: usize = source.by_package.values().map(Vec::len).sum();
        assert!(
            total_caps > 0,
            "at least one first-party extension must expose a model-visible capability"
        );
    }
}
