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

#[cfg(feature = "capability-policy")]
use ironclaw_capability_policy::{PolicyResolver, PolicySubject};
#[cfg(feature = "capability-policy")]
use ironclaw_host_api::UserRole;

use crate::available_extensions::{AvailableExtensionCatalog, visible_capability_ids};
use crate::capability_policy_engine::principal_user_id;

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
    /// Built-in capability governance (#5261 D4). Built-in capabilities
    /// (`builtin.shell`, `builtin.web_fetch`, …) are NOT contributed by any
    /// installable extension package — they ship with the host — so seeding the
    /// allow-set only from `effective.installations` would drop them and the
    /// on-policy `apply_capability_filter` would deny `builtin.shell` for every
    /// user. These ids are folded into the base `allowed` set unconditionally
    /// (treated as always-installed), then the per-member policy intersection
    /// can still hide them via a User-scope `Hidden` delta. Populated at
    /// construction from the registry snapshot minus the installable-package
    /// caps (see the construction site in `runtime.rs`). Feature-gated so the
    /// feature-off build is byte-unchanged.
    #[cfg(feature = "capability-policy")]
    builtins: Vec<CapabilityId>,
}

impl ScopedLifecyclePolicyCapabilitySurfaceResolver {
    pub(crate) fn new(
        installations: Arc<dyn ScopedLifecycleInstallationStore>,
        packages: Arc<dyn PackageCapabilitySource>,
        #[cfg(feature = "capability-policy")] policy: Arc<dyn PolicyResolver>,
        #[cfg(feature = "capability-policy")] builtins: Vec<CapabilityId>,
    ) -> Self {
        Self {
            installations,
            packages,
            #[cfg(feature = "capability-policy")]
            policy,
            #[cfg(feature = "capability-policy")]
            builtins,
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
        // The acting user's role (#5261 D3). Feature-gated to keep the
        // feature-off build byte-unchanged: it is only consulted by the
        // on-feature admin bypass + per-member policy intersection below.
        #[cfg(feature = "capability-policy")] role: UserRole,
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
        // Built-in governance (#5261 D4): builtins ship with the host and are
        // never contributed by an installable extension package, so they would
        // otherwise be absent from `allowed` and dropped by the on-policy
        // filter. Seed them unconditionally (always-installed); the per-member
        // policy intersection below can still hide an individual builtin via a
        // User-scope `Hidden` delta, and admins bypass that intersection. Empty
        // when the registry exposed no builtins (or feature-off, where this
        // block is compiled out and the set stays installed-only as before).
        #[cfg(feature = "capability-policy")]
        allowed.extend(self.builtins.iter().cloned());
        // Availability = installed AND not hidden by policy. Off-feature this is
        // the installed-only set (unchanged). On-feature it is the intersection
        // of the installed set with `{capability : EffectivePolicy.available}`,
        // EXCEPT for admins/owner who bypass the intersection entirely.
        #[cfg(feature = "capability-policy")]
        let allowed = {
            // Role-aware availability (#5261 D3). Owner/Admin intentionally see
            // the FULL installed+builtin surface: the per-capability policy
            // intersection is skipped BEFORE any `self.policy.resolve(...)` call,
            // so neither a per-user User-scope `Hidden` delta NOR a tenant-scope
            // `Hidden` delta (which `scope_applies_to_subject` would apply to
            // everyone) can cap them. This is a deliberate read-time bypass — not
            // a clear-on-promote — so demoting an admin back to Member instantly
            // re-applies their still-stored hide deltas on the next turn.
            if role.is_admin() {
                return Ok(CapabilityAllowSet::Allowlist(allowed));
            }
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

#[cfg(test)]
impl ScopedLifecyclePolicyCapabilitySurfaceResolver {
    /// Test-only shim that drives [`resolve_allow_set`] with an explicit role.
    ///
    /// `resolve_allow_set`'s `role` parameter only exists under the
    /// `capability-policy` feature (so the feature-off build is byte-unchanged),
    /// which would force every test call site to feature-gate the argument. This
    /// shim takes `role` unconditionally (`UserRole` is always in scope) and
    /// forwards it only on-feature, keeping the test call sites uniform.
    async fn resolve_allow_set_as(
        &self,
        tenant_id: &TenantId,
        user_id: Option<&UserId>,
        #[cfg_attr(not(feature = "capability-policy"), allow(unused_variables))]
        role: ironclaw_host_api::UserRole,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        self.resolve_allow_set(
            tenant_id,
            user_id,
            #[cfg(feature = "capability-policy")]
            role,
        )
        .await
    }
}

#[async_trait]
impl CapabilitySurfaceProfileResolver for ScopedLifecyclePolicyCapabilitySurfaceResolver {
    async fn resolve(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        let user_id = principal_user_id(&run_context.scope, run_context.actor.as_ref());
        // Role-aware availability (#5261 D3): the acting user's role rides on the
        // `TurnActor` (carried from `WebUiAuthenticatedCaller::actor()`). Absent
        // an actor (ownerless / channel-bound turns) fall back to the
        // least-privilege default so a non-WebUI caller is never treated as an
        // admin. Only consulted on-feature.
        #[cfg(feature = "capability-policy")]
        let role = run_context
            .actor
            .as_ref()
            .map(|actor| actor.role)
            .unwrap_or(UserRole::Member);
        self.resolve_allow_set(
            &run_context.scope.tenant_id,
            user_id,
            #[cfg(feature = "capability-policy")]
            role,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;
    // `UserRole` is imported explicitly (not via `super::*`, where it is
    // feature-gated) so the `resolve_allow_set_as` role argument compiles in
    // both feature configurations.
    use ironclaw_host_api::{ThreadId, UserRole};
    // `principal_user_id` moved to `capability_policy_engine`; the test below
    // still exercises it here (alongside the resolver test helpers).
    use ironclaw_product_workflow::{
        DeleteScopedLifecycleInstallationRequest, LifecyclePackageKind, ProductWorkflowError,
        ScopedLifecycleActor, ScopedLifecycleInstallation, ScopedLifecycleInstallationId,
        UpsertScopedLifecycleInstallationRequest,
    };
    use ironclaw_turns::scope::{TurnActor, TurnScope};

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
            // No builtins by default — keeps the existing installed-only
            // assertions exact (the D4 builtin seeding is exercised by the
            // dedicated builtin tests below via `resolver_with_builtins`).
            #[cfg(feature = "capability-policy")]
            Vec::new(),
        )
    }

    /// Like [`resolver`] but seeds the always-available builtin capability ids
    /// (#5261 D4). `builtin.shell` is the canonical builtin in these tests.
    #[cfg(feature = "capability-policy")]
    fn resolver_with_builtins(
        installations: Vec<ScopedLifecycleInstallation>,
        policy: Arc<dyn PolicyResolver>,
        builtins: Vec<CapabilityId>,
    ) -> ScopedLifecyclePolicyCapabilitySurfaceResolver {
        ScopedLifecyclePolicyCapabilitySurfaceResolver::new(
            Arc::new(FakeStore { installations }),
            static_source(),
            policy,
            builtins,
        )
    }

    /// A policy resolver whose default is `available_default()` (Available) and
    /// that holds the given deltas. With no deltas the intersection is a no-op.
    #[cfg(feature = "capability-policy")]
    fn policy_with_deltas(
        deltas: Vec<ironclaw_capability_policy::CapabilityPolicyDelta>,
    ) -> Arc<dyn PolicyResolver> {
        use ironclaw_capability_policy::CapabilityPolicyDeltaStore;
        let store = ironclaw_capability_policy::InMemoryCapabilityPolicyDeltaStore::new();
        for delta in deltas {
            // The InMemory store's upsert is synchronous enough to drive on a
            // local runtime; block on it for the test fixture.
            futures::executor::block_on(store.upsert_delta(&tenant(), delta))
                .expect("seed policy delta");
        }
        crate::capability_policy_engine::build_capability_policy_resolver(Arc::new(store))
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

    /// A tenant-scope `Hidden` delta — `scope_applies_to_subject` applies it to
    /// every user, so it would cap a member; an admin/owner must bypass it.
    #[cfg(feature = "capability-policy")]
    fn hide_tenant_delta(cap_id: &str) -> ironclaw_capability_policy::CapabilityPolicyDelta {
        ironclaw_capability_policy::CapabilityPolicyDelta {
            scope: ironclaw_capability_policy::PolicyScope::Tenant,
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
                .resolve_allow_set_as(&tenant(), Some(&user(who)), UserRole::Member)
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
            .resolve_allow_set_as(&tenant(), Some(&user("user:bob")), UserRole::Member)
            .await
            .expect("resolve");
        assert!(
            allow_ids(&bob).contains(&cap("gmail.send_message")),
            "owner sees their own private install"
        );

        let carol = resolver
            .resolve_allow_set_as(&tenant(), Some(&user("user:carol")), UserRole::Member)
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
            .resolve_allow_set_as(&tenant(), Some(&user("user:bob")), UserRole::Member)
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
            #[cfg(feature = "capability-policy")]
            Vec::new(),
        );

        let set = resolver
            .resolve_allow_set_as(&tenant(), Some(&user("user:bob")), UserRole::Member)
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
            .resolve_allow_set_as(&tenant(), None, UserRole::Member)
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
            .resolve_allow_set_as(&tenant(), Some(&user("user:bob")), UserRole::Member)
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
            Vec::new(),
        );

        let bob = resolver
            .resolve_allow_set_as(&tenant(), Some(&user("user:bob")), UserRole::Member)
            .await
            .expect("resolve");
        assert!(
            allow_ids(&bob).is_empty(),
            "a policy-Hidden capability is removed even though installed"
        );

        // Carol has no hide delta → the same install stays available to her.
        let carol = resolver
            .resolve_allow_set_as(&tenant(), Some(&user("user:carol")), UserRole::Member)
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
            Vec::new(),
        );

        let set = resolver
            .resolve_allow_set_as(&tenant(), Some(&user("user:bob")), UserRole::Member)
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
            Vec::new(),
        );

        let set = resolver
            .resolve_allow_set_as(&tenant(), Some(&user("user:bob")), UserRole::Member)
            .await
            .expect("resolve");
        assert!(
            allow_ids(&set).is_empty(),
            "policy-available but not installed → never in the allow-set"
        );
    }

    // --- #5261 D3 role-aware availability + D4 builtin governance ---

    /// (a) An Admin (and an Owner) get the FULL installed+builtin surface even
    /// when a User-scope `Hidden` delta is present for them — the bypass skips
    /// the per-capability policy intersection entirely.
    #[cfg(feature = "capability-policy")]
    #[tokio::test]
    async fn admin_bypasses_user_scope_hide_and_sees_full_surface() {
        let shell = cap("builtin.shell");
        let resolver = resolver_with_builtins(
            vec![admin_shared("install-web", "web-access")],
            // Hide BOTH the installed cap and the builtin for this user.
            policy_with_deltas(vec![
                hide_user_delta("nearai.web_search", "user:officer"),
                hide_user_delta("builtin.shell", "user:officer"),
            ]),
            vec![shell.clone()],
        );

        for role in [UserRole::Admin, UserRole::Owner] {
            let set = resolver
                .resolve_allow_set_as(&tenant(), Some(&user("user:officer")), role)
                .await
                .expect("resolve");
            let ids = allow_ids(&set);
            assert!(
                ids.contains(&cap("nearai.web_search")),
                "{role:?} must see the installed cap despite a User-scope hide (bypass)"
            );
            assert!(
                ids.contains(&shell),
                "{role:?} must see the builtin despite a User-scope hide (bypass)"
            );
        }

        // Same deltas DO cap a member — proves the bypass is role-gated.
        let member = resolver
            .resolve_allow_set_as(&tenant(), Some(&user("user:officer")), UserRole::Member)
            .await
            .expect("resolve");
        assert!(
            allow_ids(&member).is_empty(),
            "a member with both caps hidden sees nothing — the bypass is admin-only"
        );
    }

    /// (b) A tenant-scope `Hidden` delta (which `scope_applies_to_subject`
    /// applies to everyone) also does NOT cap an admin/owner.
    #[cfg(feature = "capability-policy")]
    #[tokio::test]
    async fn admin_bypasses_tenant_scope_hide() {
        let resolver = resolver_with_builtins(
            vec![admin_shared("install-web", "web-access")],
            policy_with_deltas(vec![hide_tenant_delta("nearai.web_search")]),
            Vec::new(),
        );

        let officer = resolver
            .resolve_allow_set_as(&tenant(), Some(&user("user:officer")), UserRole::Admin)
            .await
            .expect("resolve");
        assert!(
            allow_ids(&officer).contains(&cap("nearai.web_search")),
            "an admin must not be capped by a tenant-scope hide"
        );

        // The same tenant-scope hide DOES cap a member.
        let member = resolver
            .resolve_allow_set_as(&tenant(), Some(&user("user:alice")), UserRole::Member)
            .await
            .expect("resolve");
        assert!(
            !allow_ids(&member).contains(&cap("nearai.web_search")),
            "a member IS capped by the tenant-scope hide"
        );
    }

    /// (c) A builtin (`builtin.shell`) is available-by-default to a member even
    /// though no installable package contributes it.
    #[cfg(feature = "capability-policy")]
    #[tokio::test]
    async fn builtin_is_available_by_default_to_member() {
        let shell = cap("builtin.shell");
        let resolver = resolver_with_builtins(
            // No installations at all — the builtin must still surface.
            Vec::new(),
            available_default_policy(),
            vec![shell.clone()],
        );

        let set = resolver
            .resolve_allow_set_as(&tenant(), Some(&user("user:alice")), UserRole::Member)
            .await
            .expect("resolve");
        assert!(
            allow_ids(&set).contains(&shell),
            "builtin.shell is always-available (seeded) to a member by default"
        );
    }

    /// (d) A member's User-scope `Hidden` delta on `builtin.shell` drops it —
    /// builtins are governable per-user just like installed caps.
    #[cfg(feature = "capability-policy")]
    #[tokio::test]
    async fn member_user_scope_hide_drops_builtin() {
        let shell = cap("builtin.shell");
        let resolver = resolver_with_builtins(
            Vec::new(),
            policy_with_deltas(vec![hide_user_delta("builtin.shell", "user:alice")]),
            vec![shell.clone()],
        );

        let alice = resolver
            .resolve_allow_set_as(&tenant(), Some(&user("user:alice")), UserRole::Member)
            .await
            .expect("resolve");
        assert!(
            !allow_ids(&alice).contains(&shell),
            "a member's User-scope hide on builtin.shell drops the builtin"
        );

        // A different member with no hide still sees the builtin (per-user).
        let bob = resolver
            .resolve_allow_set_as(&tenant(), Some(&user("user:bob")), UserRole::Member)
            .await
            .expect("resolve");
        assert!(
            allow_ids(&bob).contains(&shell),
            "the hide is per-user; an unhidden member still sees builtin.shell"
        );
    }

    /// (e) Existing member `installed ∩ policy` behavior still holds with the
    /// builtin seeding present: a member sees the installed cap AND the builtin,
    /// but a User-scope hide on the installed cap drops only that one.
    #[cfg(feature = "capability-policy")]
    #[tokio::test]
    async fn member_installed_intersect_policy_holds_with_builtins() {
        let shell = cap("builtin.shell");
        let resolver = resolver_with_builtins(
            vec![admin_shared("install-web", "web-access")],
            policy_with_deltas(vec![hide_user_delta("nearai.web_search", "user:alice")]),
            vec![shell.clone()],
        );

        let alice = resolver
            .resolve_allow_set_as(&tenant(), Some(&user("user:alice")), UserRole::Member)
            .await
            .expect("resolve");
        let ids = allow_ids(&alice);
        assert!(
            !ids.contains(&cap("nearai.web_search")),
            "the installed cap is hidden for alice (intersection still applies to members)"
        );
        assert!(
            ids.contains(&shell),
            "the builtin (not hidden) remains available to the member"
        );

        // bob has no hide → sees both the installed cap and the builtin.
        let bob = resolver
            .resolve_allow_set_as(&tenant(), Some(&user("user:bob")), UserRole::Member)
            .await
            .expect("resolve");
        let bob_ids = allow_ids(&bob);
        assert!(bob_ids.contains(&cap("nearai.web_search")));
        assert!(bob_ids.contains(&shell));
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
