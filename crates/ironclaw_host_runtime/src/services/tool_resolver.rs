//! Registry-backed [`ToolResolver`]: prebound lane bindings per registry
//! generation.
//!
//! The dispatcher no longer selects a package or runtime kind per invocation
//! (TOOL-1); this resolver constructs one prebound binding per capability
//! whenever the shared registry's version changes, and resolution is a map
//! lookup. Selection failures that used to be minted inside the dispatcher —
//! unknown provider, descriptor/package runtime mismatch, unconfigured
//! runtime backend — are preserved as error bindings so the error surface and
//! the prepared-reservation release semantics are unchanged (TOOL-3).

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use async_trait::async_trait;
use ironclaw_dispatcher::{
    BoundCapabilityAdapter, CapabilityDispatchRequest, ResolvedCapability, RuntimeAdapterResult,
    ToolResolver,
};
use ironclaw_extensions::{ExtensionPackage, ExtensionRegistry, SharedExtensionRegistry};
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, DispatchError, ExtensionId, RuntimeKind, RuntimeLane,
    runtime_policy::EffectiveRuntimePolicy,
};
use ironclaw_resources::ResourceGovernor;

use super::RootFilesystem;
use super::runtime_adapters::{RuntimeLaneExecutor, RuntimeLaneRequest};

/// Prebinds every registry capability to its runtime lane, rebuilt only when
/// the shared registry publishes a new version.
pub(crate) struct RegistryLaneToolResolver<F, G>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
{
    registry: Arc<SharedExtensionRegistry>,
    executor: Arc<RuntimeLaneExecutor<F, G>>,
    filesystem: Arc<F>,
    governor: Arc<G>,
    runtime_policy: EffectiveRuntimePolicy,
    /// When set, only these providers' capabilities resolve here (the
    /// built-in restriction once extension dispatch comes from the active
    /// snapshot). `None` serves the whole registry — compositions without an
    /// extension host.
    provider_allowlist: Option<BTreeSet<ExtensionId>>,
    cache: RwLock<CachedBindings>,
}

struct CachedBindings {
    version: Option<u64>,
    bindings: Arc<HashMap<CapabilityId, ResolvedCapability>>,
}

impl<F, G> RegistryLaneToolResolver<F, G>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
{
    pub(crate) fn new(
        registry: Arc<SharedExtensionRegistry>,
        executor: Arc<RuntimeLaneExecutor<F, G>>,
        filesystem: Arc<F>,
        governor: Arc<G>,
        runtime_policy: EffectiveRuntimePolicy,
        provider_allowlist: Option<BTreeSet<ExtensionId>>,
    ) -> Self {
        Self {
            registry,
            executor,
            filesystem,
            governor,
            runtime_policy,
            provider_allowlist,
            cache: RwLock::new(CachedBindings {
                version: None,
                bindings: Arc::new(HashMap::new()),
            }),
        }
    }

    fn read_cache(&self) -> RwLockReadGuard<'_, CachedBindings> {
        // A poisoned cache holds no invariants beyond "rebuilt on version
        // mismatch"; recover the guard instead of failing dispatch.
        match self.cache.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn write_cache(&self) -> RwLockWriteGuard<'_, CachedBindings> {
        match self.cache.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn build_bindings(
        &self,
        snapshot: &ExtensionRegistry,
    ) -> HashMap<CapabilityId, ResolvedCapability> {
        let mut packages: HashMap<ExtensionId, Arc<ExtensionPackage>> = HashMap::new();
        let mut bindings = HashMap::new();
        for descriptor in snapshot.capabilities() {
            if let Some(allowlist) = &self.provider_allowlist
                && !allowlist.contains(&descriptor.provider)
            {
                continue;
            }
            let adapter = self.bind_capability(snapshot, descriptor, &mut packages);
            bindings.insert(
                descriptor.id.clone(),
                ResolvedCapability {
                    provider: descriptor.provider.clone(),
                    runtime: descriptor.runtime,
                    adapter,
                },
            );
        }
        bindings
    }

    fn bind_capability(
        &self,
        snapshot: &ExtensionRegistry,
        descriptor: &CapabilityDescriptor,
        packages: &mut HashMap<ExtensionId, Arc<ExtensionPackage>>,
    ) -> Arc<dyn BoundCapabilityAdapter> {
        let Some(package) = snapshot.get_extension(&descriptor.provider) else {
            return Arc::new(UnresolvableBoundCapability {
                governor: Arc::clone(&self.governor),
                failure: BindingFailure::UnknownProvider {
                    capability: descriptor.id.clone(),
                    provider: descriptor.provider.clone(),
                },
            });
        };
        let package_runtime = package.manifest.runtime_kind();
        if descriptor.runtime != package_runtime {
            return Arc::new(UnresolvableBoundCapability {
                governor: Arc::clone(&self.governor),
                failure: BindingFailure::RuntimeMismatch {
                    capability: descriptor.id.clone(),
                    descriptor_runtime: descriptor.runtime,
                    package_runtime,
                },
            });
        }
        let Some(lane) = RuntimeLane::from_runtime_kind(descriptor.runtime)
            .filter(|lane| self.executor.supports_lane(*lane))
        else {
            return Arc::new(UnresolvableBoundCapability {
                governor: Arc::clone(&self.governor),
                failure: BindingFailure::MissingRuntimeBackend {
                    runtime: descriptor.runtime,
                },
            });
        };
        let package = packages
            .entry(descriptor.provider.clone())
            .or_insert_with(|| Arc::new(package.clone()));
        Arc::new(LaneBoundCapability {
            package: Arc::clone(package),
            descriptor: Arc::new(descriptor.clone()),
            lane,
            executor: Arc::clone(&self.executor),
            filesystem: Arc::clone(&self.filesystem),
            governor: Arc::clone(&self.governor),
            runtime_policy: self.runtime_policy.clone(),
        })
    }
}

impl<F, G> ToolResolver for RegistryLaneToolResolver<F, G>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
{
    fn resolve(&self, capability_id: &CapabilityId) -> Option<ResolvedCapability> {
        let current = self.registry.version();
        {
            let cache = self.read_cache();
            if cache.version == Some(current) {
                return cache.bindings.get(capability_id).cloned();
            }
        }
        let mut cache = self.write_cache();
        let current = self.registry.version();
        if cache.version != Some(current) {
            // Version is read before the snapshot: a mutation landing between
            // the two reads makes the snapshot newer than the recorded
            // version, which only forces one extra rebuild on the next
            // resolve — never a stale binding.
            let snapshot = self.registry.snapshot();
            cache.bindings = Arc::new(self.build_bindings(&snapshot));
            cache.version = Some(current);
        }
        cache.bindings.get(capability_id).cloned()
    }
}

/// A capability prebound to its runtime lane: the package, descriptor,
/// execution policy, filesystem, and governor are captured once per registry
/// generation; only per-invocation inputs travel in the request.
struct LaneBoundCapability<F, G>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
{
    package: Arc<ExtensionPackage>,
    descriptor: Arc<CapabilityDescriptor>,
    lane: RuntimeLane,
    executor: Arc<RuntimeLaneExecutor<F, G>>,
    filesystem: Arc<F>,
    governor: Arc<G>,
    runtime_policy: EffectiveRuntimePolicy,
}

#[async_trait]
impl<F, G> BoundCapabilityAdapter for LaneBoundCapability<F, G>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
{
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        self.executor
            .dispatch_json(
                self.lane,
                RuntimeLaneRequest {
                    package: &self.package,
                    descriptor: &self.descriptor,
                    filesystem: self.filesystem.as_ref(),
                    governor: self.governor.as_ref(),
                    runtime_policy: &self.runtime_policy,
                    capability_id: &request.capability_id,
                    scope: request.scope,
                    authenticated_actor_user_id: request.authenticated_actor_user_id,
                    run_id: request.run_id,
                    origin: Some(request.origin),
                    estimate: request.estimate,
                    mounts: request.mounts,
                    resource_reservation: request.resource_reservation,
                    input: request.input,
                },
            )
            .await
    }
}

enum BindingFailure {
    UnknownProvider {
        capability: CapabilityId,
        provider: ExtensionId,
    },
    RuntimeMismatch {
        capability: CapabilityId,
        descriptor_runtime: RuntimeKind,
        package_runtime: RuntimeKind,
    },
    MissingRuntimeBackend {
        runtime: RuntimeKind,
    },
}

impl BindingFailure {
    fn to_dispatch_error(&self) -> DispatchError {
        match self {
            Self::UnknownProvider {
                capability,
                provider,
            } => DispatchError::UnknownProvider {
                capability: capability.clone(),
                provider: provider.clone(),
            },
            Self::RuntimeMismatch {
                capability,
                descriptor_runtime,
                package_runtime,
            } => DispatchError::RuntimeMismatch {
                capability: capability.clone(),
                descriptor_runtime: *descriptor_runtime,
                package_runtime: *package_runtime,
            },
            Self::MissingRuntimeBackend { runtime } => {
                DispatchError::MissingRuntimeBackend { runtime: *runtime }
            }
        }
    }
}

/// A binding for a capability whose selection failed (unknown provider,
/// runtime mismatch, unconfigured backend). Invoking it releases any prepared
/// reservation — the leg the dispatcher's validation guard used to own — and
/// fails with the preserved selection error before any lane work.
struct UnresolvableBoundCapability<G>
where
    G: ResourceGovernor + 'static,
{
    governor: Arc<G>,
    failure: BindingFailure,
}

#[async_trait]
impl<G> BoundCapabilityAdapter for UnresolvableBoundCapability<G>
where
    G: ResourceGovernor + 'static,
{
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        if let Some(reservation) = &request.resource_reservation
            && let Err(error) = self.governor.release(reservation.id)
        {
            tracing::warn!(
                reservation_id = %reservation.id,
                error = %error,
                "failed to release prepared resource reservation for unresolvable capability binding"
            );
        }
        Err(self.failure.to_dispatch_error())
    }
}
