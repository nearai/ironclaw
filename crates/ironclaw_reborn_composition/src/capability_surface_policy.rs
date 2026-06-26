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
use ironclaw_host_api::{CapabilityId, TenantId, UserId};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
};
use ironclaw_product_workflow::{
    LifecyclePackageRef, ScopedLifecycleInstallationStore, ScopedLifecycleSubject,
    lifecycle_package_kind_label,
};
use ironclaw_turns::run_profile::LoopRunContext;
use ironclaw_turns::scope::{TurnActor, TurnScope};

use crate::available_extensions::{AvailableExtensionCatalog, visible_capability_ids};

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
}

impl ScopedLifecyclePolicyCapabilitySurfaceResolver {
    pub(crate) fn new(
        installations: Arc<dyn ScopedLifecycleInstallationStore>,
        packages: Arc<dyn PackageCapabilitySource>,
    ) -> Self {
        Self {
            installations,
            packages,
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
        )
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
