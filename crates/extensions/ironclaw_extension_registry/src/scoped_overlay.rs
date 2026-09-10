//! Per-user discovered-package overlay over the global extension registry.
//!
//! Hosted-MCP providers can serve a **per-principal** `tools/list` (e.g. the
//! agent marketplace returns concierge tools for a concierge bearer and the
//! hirer-granted connector tools for a worker-agent bearer). The global
//! [`crate::SharedExtensionRegistry`] holds exactly one surface per extension
//! id, so per-principal discovered surfaces live here instead: an in-memory,
//! TTL-bounded cache keyed by ([`OverlayScope`] = tenant + user + thread,
//! `ExtensionId`).
//!
//! This is a **derived cache**, not lifecycle state: nothing here is
//! persisted, installation records are never touched, and a restart simply
//! re-discovers lazily. Consumers read through [`OverlaidRegistryView`], which
//! prefers the caller's overlay entries and falls back to the global
//! snapshot — the same view feeds the model surface, authorization, dispatch
//! and egress planning so no parallel resolution pipeline exists.
//!
//! Security invariant: entries are keyed by the full tenant + user + thread
//! scope whose credential produced the discovery result — never by a bare
//! `UserId` (unique only within a tenant), and never by owner alone (a managed
//! agent serves many hires under one identity, distinguished only by thread) —
//! and a view only ever merges entries for the single scope it was built for.
//! Cross-user, cross-tenant AND cross-thread leakage are regression-tested
//! failure modes.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use ironclaw_host_api::{
    capability::CapabilityDescriptor,
    ids::{CapabilityId, ExtensionId, TenantId, ThreadId, UserId},
};
use parking_lot::RwLock;

use crate::{CapabilityVisibility, ExtensionPackage, ExtensionRegistry};

/// The scope axes a discovered surface is keyed by: `tenant_id`, `user_id`,
/// **and** `thread_id`.
///
/// The tenant axis is non-negotiable — a bare `UserId` is unique only within a
/// tenant, so keying by user alone lets one tenant's discovered tool surface
/// (and the refresher's negative-cache / single-flight verdicts) bleed into
/// another tenant that reuses the same user id.
///
/// The thread axis is load-bearing for **isolation**, not merely cache
/// efficiency. A managed worker agent serves several hires under ONE IronClaw
/// identity (its own tenant + user); the only thing distinguishing hire A's
/// discovered surface (e.g. a buyer's `timeless__*` connector) from hire B's
/// (`firefly__*`) is the thread each job runs on. Keyed by owner alone, hire
/// A's cached tools would serve hire B — a cross-buyer personal-data leak.
/// `thread_id` is `None` only for non-threaded runtimes (one execution context,
/// so the owner axis is sufficient); consumers treat `None` as its own bucket,
/// never a wildcard. Every overlay and refresher operation takes this scope so
/// no axis can be dropped at a call site.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OverlayScope {
    tenant_id: TenantId,
    user_id: UserId,
    thread_id: Option<ThreadId>,
}

impl OverlayScope {
    pub fn new(tenant_id: TenantId, user_id: UserId, thread_id: Option<ThreadId>) -> Self {
        Self {
            tenant_id,
            user_id,
            thread_id,
        }
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn user_id(&self) -> &UserId {
        &self.user_id
    }

    pub fn thread_id(&self) -> Option<&ThreadId> {
        self.thread_id.as_ref()
    }
}

/// Default lifetime of a discovered per-user surface. It MUST comfortably
/// exceed the longest single run: discovery runs at turn start and the
/// discovered tools are then read live on every capability dispatch, so a TTL
/// shorter than the run makes late tool calls (e.g. a worker agent's final
/// `marketplace__submit_deliverable` after minutes of work — worse under an
/// LLM-provider 5xx storm that stretches the run) vanish from the surface
/// mid-run with `unknown_capability`. 30 min covers any realistic bounded run;
/// a stable per-agent tool set (all of the agent's active connector grants)
/// means the long window does not stale a worker's surface across dispatches,
/// and the turn-start refresher still re-discovers once an idle entry expires.
pub const DEFAULT_SCOPED_OVERLAY_TTL: Duration = Duration::from_secs(1800);

/// How long past its TTL a discovered entry is retained (still served as
/// last-good) before eviction.
///
/// Bounds the growth the thread cache axis would otherwise cause: a one-off
/// job's thread never recurs, so freshness (which only gates *re-discovery* of
/// the same scope) never expires it and it would be served-stale forever.
/// Serving stays available within `[expiry, expiry + retention]` so a
/// concurrent turn that skips the in-flight refresh keeps the surface, but a
/// finished thread's entry is swept once it has been stale this long.
/// Comfortably exceeds any bounded run plus the refresh gap.
pub const OVERLAY_STALE_RETENTION: Duration = Duration::from_secs(1800);

/// Hard cap on total live entries — a backstop against pathological growth
/// (e.g. a burst of concurrent one-off jobs faster than the retention sweep).
/// When exceeded, the entries closest to (or furthest past) expiry are evicted
/// first.
const MAX_OVERLAY_ENTRIES: usize = 4096;

#[derive(Debug, Clone)]
struct OverlayEntry {
    package: Arc<ExtensionPackage>,
    expires_at: Instant,
}

/// Freshness of a cached discovered surface. Freshness gates whether the
/// turn-start refresher **re-discovers**; it does NOT gate whether readers
/// **serve** the entry. Readers always serve the last-good package (fresh or
/// stale) so a concurrent turn that skips the in-flight refresh — or any turn
/// hitting an entry right at TTL expiry — keeps the discovered surface instead
/// of flapping back to the static manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayFreshness {
    /// Within TTL — no re-discovery needed.
    Fresh,
    /// Past TTL — the turn-start refresher should re-discover. The entry stays
    /// served as last-good until discovery replaces it or an auth failure
    /// removes it.
    Stale,
}

/// In-memory discovered-package cache keyed by scope ([`OverlayScope`]:
/// tenant + user + thread) and extension. Composition owns one instance and shares it
/// (via `Arc`) with every registry consumer that resolves capabilities for a
/// scoped request.
#[derive(Debug)]
pub struct ScopedPackageOverlay {
    entries: RwLock<HashMap<(OverlayScope, ExtensionId), OverlayEntry>>,
    /// How long past its TTL an entry is kept (still served last-good) before
    /// eviction. A composition knob: production uses [`OVERLAY_STALE_RETENTION`];
    /// a shorter value tightens the bound where discovered surfaces churn fast.
    stale_retention: Duration,
}

impl Default for ScopedPackageOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl ScopedPackageOverlay {
    pub fn new() -> Self {
        Self::with_stale_retention(OVERLAY_STALE_RETENTION)
    }

    /// Construct with an explicit stale-retention window (see the field).
    pub fn with_stale_retention(stale_retention: Duration) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            stale_retention,
        }
    }

    /// Store (or refresh) `owner`'s discovered surface for `package.id`.
    ///
    /// Opportunistically evicts entries stale beyond the retention window (and
    /// enforces [`MAX_OVERLAY_ENTRIES`]) on the same lock, so the per-thread key
    /// axis cannot grow the cache without bound — inserts run at every turn-start
    /// discovery, so the sweep keeps pace with new threads.
    pub fn insert(&self, owner: OverlayScope, package: ExtensionPackage, ttl: Duration) {
        let key = (owner, package.id.clone());
        let entry = OverlayEntry {
            package: Arc::new(package),
            expires_at: Instant::now() + ttl,
        };
        let mut entries = self.entries.write();
        entries.insert(key, entry);
        self.evict_locked(&mut entries, Instant::now());
    }

    /// Evict entries stale beyond the retention window, then enforce the hard
    /// cap (most-expired first). Called under the write lock from `insert`.
    fn evict_locked(
        &self,
        entries: &mut HashMap<(OverlayScope, ExtensionId), OverlayEntry>,
        now: Instant,
    ) {
        entries.retain(|_, entry| entry.expires_at + self.stale_retention > now);
        if entries.len() > MAX_OVERLAY_ENTRIES {
            let mut by_expiry: Vec<((OverlayScope, ExtensionId), Instant)> = entries
                .iter()
                .map(|(key, entry)| (key.clone(), entry.expires_at))
                .collect();
            by_expiry.sort_by_key(|(_, expires_at)| *expires_at);
            let excess = entries.len() - MAX_OVERLAY_ENTRIES;
            for (key, _) in by_expiry.into_iter().take(excess) {
                entries.remove(&key);
            }
        }
    }

    /// Drop `owner`'s entry for `extension_id` (e.g. missing credential).
    pub fn remove(&self, owner: &OverlayScope, extension_id: &ExtensionId) {
        self.entries
            .write()
            .remove(&(owner.clone(), extension_id.clone()));
    }

    /// Drop every owner's entry for `extension_id` (extension deactivated or
    /// replaced tenant-wide).
    pub fn remove_extension(&self, extension_id: &ExtensionId) {
        self.entries
            .write()
            .retain(|(_, entry_extension), _| entry_extension != extension_id);
    }

    /// The cached package for `(owner, extension_id)` with its freshness, if
    /// any. The turn-start refresher uses the freshness to decide whether to
    /// re-discover; the stale entry is still served as last-good by the reader
    /// paths below.
    pub fn get(
        &self,
        owner: &OverlayScope,
        extension_id: &ExtensionId,
    ) -> Option<(Arc<ExtensionPackage>, OverlayFreshness)> {
        let entries = self.entries.read();
        let entry = entries.get(&(owner.clone(), extension_id.clone()))?;
        let freshness = if entry.expires_at > Instant::now() {
            OverlayFreshness::Fresh
        } else {
            OverlayFreshness::Stale
        };
        Some((Arc::clone(&entry.package), freshness))
    }

    /// Re-arm the TTL on an existing entry (a transient discovery failure kept
    /// the last-good surface; avoid re-running discovery every turn while the
    /// provider is down).
    pub fn touch(&self, owner: &OverlayScope, extension_id: &ExtensionId, ttl: Duration) {
        if let Some(entry) = self
            .entries
            .write()
            .get_mut(&(owner.clone(), extension_id.clone()))
        {
            entry.expires_at = Instant::now() + ttl;
        }
    }

    /// The owner's last-good overlay packages (for grant/provider-trust minting
    /// at surface-request time). Serves stale entries too — see
    /// [`OverlayFreshness`]: freshness gates re-discovery, not serving.
    pub fn packages_for(&self, owner: &OverlayScope) -> Vec<Arc<ExtensionPackage>> {
        self.entries
            .read()
            .iter()
            .filter(|((entry_owner, _), _)| entry_owner == owner)
            .map(|(_, entry)| Arc::clone(&entry.package))
            .collect()
    }

    /// A concrete `ExtensionRegistry` snapshot with the owner's last-good
    /// discovered packages merged in (each overlaid extension's static package
    /// replaced by its discovered one). Returns the `global` Arc unchanged when
    /// the owner has no overlay entries — zero cost for the common case, so
    /// every existing `self.registry.snapshot()` consumer can resolve an
    /// owner's discovered capabilities by reading this instead, with no change
    /// to its `&ExtensionRegistry` API.
    pub fn merged_snapshot(
        &self,
        owner: &OverlayScope,
        global: Arc<ExtensionRegistry>,
    ) -> Arc<ExtensionRegistry> {
        let packages = self.packages_for(owner);
        if packages.is_empty() {
            return global;
        }
        let mut merged = global.as_ref().clone();
        for package in packages {
            // Replace the static package with the discovered one. The
            // discovered package was already validated at discovery time, so use
            // the trusted (non-revalidating) insert to keep this off the
            // per-invocation hot path; the preceding `remove` clears the static
            // capability ids so there is no id collision to check.
            merged.remove(&package.id);
            merged.insert_validated(package.as_ref().clone());
        }
        Arc::new(merged)
    }

    /// Build the owner's merged view over `global` from the last-good
    /// discovered packages (fresh or stale — serving is not freshness-gated).
    pub fn view_for(
        &self,
        owner: &OverlayScope,
        global: Arc<ExtensionRegistry>,
    ) -> OverlaidRegistryView {
        let overlays = self.packages_for(owner);
        OverlaidRegistryView { global, overlays }
    }
}

/// A single user's capability-resolution view: the global registry snapshot
/// with that user's discovered packages layered on top. For an overlaid
/// extension the overlay package **replaces** the global one — its discovered
/// capability set is the extension's surface for this user.
#[derive(Debug, Clone)]
pub struct OverlaidRegistryView {
    global: Arc<ExtensionRegistry>,
    overlays: Vec<Arc<ExtensionPackage>>,
}

impl OverlaidRegistryView {
    /// A view with no overlay entries (scope-less callers).
    pub fn global_only(global: Arc<ExtensionRegistry>) -> Self {
        Self {
            global,
            overlays: Vec::new(),
        }
    }

    pub fn has_overlays(&self) -> bool {
        !self.overlays.is_empty()
    }

    fn overlaid_extension(&self, id: &ExtensionId) -> Option<&ExtensionPackage> {
        self.overlays
            .iter()
            .find(|package| &package.id == id)
            .map(Arc::as_ref)
    }

    pub fn get_extension(&self, id: &ExtensionId) -> Option<&ExtensionPackage> {
        self.overlaid_extension(id)
            .or_else(|| self.global.get_extension(id))
    }

    pub fn get_capability(&self, id: &CapabilityId) -> Option<&CapabilityDescriptor> {
        for package in &self.overlays {
            if let Some(descriptor) = package
                .capabilities
                .iter()
                .find(|descriptor| &descriptor.id == id)
            {
                return Some(descriptor);
            }
            // The overlay replaces this extension's whole surface: a global
            // capability belonging to an overlaid extension is masked even
            // when the discovered set no longer contains it.
            if self
                .global
                .get_capability(id)
                .is_some_and(|descriptor| descriptor.provider == package.id)
            {
                return None;
            }
        }
        self.global.get_capability(id)
    }

    pub fn capability_visibility(&self, id: &CapabilityId) -> Option<CapabilityVisibility> {
        for package in &self.overlays {
            if let Some(capability) = package
                .manifest
                .capabilities
                .iter()
                .find(|capability| &capability.id == id)
            {
                return Some(capability.visibility);
            }
            if self
                .global
                .get_capability(id)
                .is_some_and(|descriptor| descriptor.provider == package.id)
            {
                return None;
            }
        }
        self.global.capability_visibility(id)
    }

    /// All capabilities visible in this view: global capabilities of
    /// non-overlaid extensions plus every overlay capability.
    pub fn capabilities(&self) -> impl Iterator<Item = &CapabilityDescriptor> {
        let overlaid_ids: Vec<&ExtensionId> =
            self.overlays.iter().map(|package| &package.id).collect();
        self.global
            .capabilities()
            .filter(move |descriptor| !overlaid_ids.contains(&&descriptor.provider))
            .chain(
                self.overlays
                    .iter()
                    .flat_map(|package| package.capabilities.iter()),
            )
    }

    /// The overlay packages themselves (for grant/provider-trust minting).
    pub fn overlay_packages(&self) -> impl Iterator<Item = &ExtensionPackage> {
        self.overlays.iter().map(Arc::as_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExtensionManifest, HostPortCatalog, ManifestSource};
    use ironclaw_host_api::path::VirtualPath;

    const STATIC_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "agent-market"
name = "Agent Market"
version = "0.1.0"
description = "Marketplace tools"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "https://market.example.com/mcp"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "agent-market.search_agents"
description = "Search the marketplace"
effects = ["dispatch_capability", "network", "use_secret"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"
runtime_credentials = [
  { handle = "agent-market-token", audience = { scheme = "https", host_pattern = "market.example.com" }, target = { type = "header", name = "authorization", prefix = "Bearer " }, required = true }
]
"#;

    fn capability_provider_contracts() -> crate::HostApiContractRegistry {
        let mut contracts = crate::HostApiContractRegistry::new();
        contracts
            .register(std::sync::Arc::new(
                crate::host_api::capability_provider::CapabilityProviderHostApiContract::new()
                    .expect("capability provider contract"),
            ))
            .expect("register capability provider contract");
        contracts
    }

    fn static_package() -> ExtensionPackage {
        let manifest = ExtensionManifest::parse(
            STATIC_MANIFEST,
            ManifestSource::HostBundled,
            &HostPortCatalog::default(),
            &capability_provider_contracts(),
        )
        .expect("valid manifest");
        ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new("/system/extensions/agent-market").expect("root"),
        )
        .expect("valid package")
    }

    fn discovered_package(tool: &str) -> ExtensionPackage {
        let tools = vec![ironclaw_extension_contracts::hosted_mcp::HostedMcpDiscoveredTool {
            name: tool.to_string(),
            description: format!("discovered {tool}"),
            input_schema: serde_json::json!({"type": "object"}),
            annotations: Default::default(),
        }];
        crate::package_with_discovered_hosted_mcp_tools(&static_package(), &tools)
            .expect("discoverable package")
    }

    fn global_registry() -> Arc<ExtensionRegistry> {
        let mut registry = ExtensionRegistry::new();
        registry.insert(static_package()).expect("insert static");
        Arc::new(registry)
    }

    fn user(id: &str) -> UserId {
        UserId::new(id).expect("user id")
    }

    fn owner(id: &str) -> OverlayScope {
        OverlayScope::new(TenantId::new("tenant-a").expect("tenant"), user(id), None)
    }

    fn owner_in(tenant: &str, id: &str) -> OverlayScope {
        OverlayScope::new(TenantId::new(tenant).expect("tenant"), user(id), None)
    }

    fn owner_thread(id: &str, thread: &str) -> OverlayScope {
        OverlayScope::new(
            TenantId::new("tenant-a").expect("tenant"),
            user(id),
            Some(ThreadId::new(thread).expect("thread id")),
        )
    }

    fn capability(id: &str) -> CapabilityId {
        CapabilityId::new(id).expect("capability id")
    }

    #[test]
    fn stale_entries_beyond_retention_are_evicted_so_one_off_threads_dont_leak() {
        // The thread cache axis means a one-off job's thread never recurs, so
        // freshness alone never expires its entry — retention-based eviction
        // bounds the growth. With a zero retention window, a stale (TTL-0) entry
        // is swept by the next insert's opportunistic eviction.
        let overlay = ScopedPackageOverlay::with_stale_retention(Duration::ZERO);
        let one_off = owner_thread("worker-agent", "thread-a");
        overlay.insert(
            one_off.clone(),
            discovered_package("timeless__list_meetings"),
            Duration::ZERO,
        );
        // A later job's insert triggers the sweep.
        let live = owner_thread("worker-agent", "thread-b");
        overlay.insert(
            live.clone(),
            discovered_package("timeless__list_meetings"),
            Duration::from_secs(60),
        );

        assert!(
            overlay.packages_for(&one_off).is_empty(),
            "one-off thread's stale-beyond-retention entry must be evicted"
        );
        assert_eq!(
            overlay.packages_for(&live).len(),
            1,
            "a fresh entry must survive the sweep"
        );
    }

    #[test]
    fn stale_within_retention_is_still_served_last_good() {
        // Default (non-zero) retention keeps a just-expired entry served, so a
        // concurrent turn that skips the in-flight refresh keeps the surface.
        let overlay = ScopedPackageOverlay::new();
        let job = owner_thread("worker-agent", "thread-a");
        overlay.insert(
            job.clone(),
            discovered_package("timeless__list_meetings"),
            Duration::ZERO,
        );
        // Trigger a sweep with another insert; the just-expired entry is within
        // the retention window, so it is retained.
        overlay.insert(
            owner_thread("worker-agent", "thread-b"),
            discovered_package("timeless__list_meetings"),
            Duration::from_secs(60),
        );
        assert_eq!(
            overlay.packages_for(&job).len(),
            1,
            "stale-but-within-retention entry must still be served last-good"
        );
    }

    #[test]
    fn view_prefers_overlay_and_masks_replaced_static_capabilities() {
        let overlay = ScopedPackageOverlay::new();
        let worker = owner("worker-user");
        overlay.insert(
            worker.clone(),
            discovered_package("timeless__list_meetings"),
            Duration::from_secs(60),
        );

        let view = overlay.view_for(&worker, global_registry());
        assert!(view.has_overlays());
        assert!(
            view.get_capability(&capability("agent-market.timeless__list_meetings"))
                .is_some(),
            "discovered capability must resolve"
        );
        assert!(
            view.get_capability(&capability("agent-market.search_agents"))
                .is_none(),
            "static capability of an overlaid extension must be masked"
        );
        let ids: Vec<&str> = view
            .capabilities()
            .map(|descriptor| descriptor.id.as_str())
            .collect();
        assert_eq!(ids, vec!["agent-market.timeless__list_meetings"]);
    }

    #[test]
    fn merged_snapshot_replaces_static_package_for_owner_and_is_untouched_for_others() {
        let overlay = ScopedPackageOverlay::new();
        let worker = owner("worker-user");
        overlay.insert(
            worker.clone(),
            discovered_package("timeless__list_meetings"),
            Duration::from_secs(60),
        );

        let worker_reg = overlay.merged_snapshot(&worker, global_registry());
        assert!(
            worker_reg
                .get_capability(&capability("agent-market.timeless__list_meetings"))
                .is_some(),
            "merged snapshot resolves the discovered capability by plain get_capability"
        );
        assert!(
            worker_reg
                .get_capability(&capability("agent-market.search_agents"))
                .is_none(),
            "the static capability is replaced in the owner's merged snapshot"
        );

        let other_reg = overlay.merged_snapshot(&owner("other"), global_registry());
        assert!(
            other_reg
                .get_capability(&capability("agent-market.search_agents"))
                .is_some(),
            "another user's merged snapshot keeps the static surface"
        );
        assert!(
            other_reg
                .get_capability(&capability("agent-market.timeless__list_meetings"))
                .is_none()
        );
    }

    #[test]
    fn merged_snapshot_returns_global_arc_unchanged_without_overlay() {
        let overlay = ScopedPackageOverlay::new();
        let global = global_registry();
        let merged = overlay.merged_snapshot(&owner("nobody"), Arc::clone(&global));
        assert!(Arc::ptr_eq(&global, &merged), "zero-cost passthrough");
    }

    #[test]
    fn overlay_entries_never_leak_across_users() {
        let overlay = ScopedPackageOverlay::new();
        let worker = owner("worker-user");
        let other = owner("concierge-user");
        overlay.insert(
            worker.clone(),
            discovered_package("timeless__list_meetings"),
            Duration::from_secs(60),
        );

        let other_view = overlay.view_for(&other, global_registry());
        assert!(!other_view.has_overlays());
        assert!(
            other_view
                .get_capability(&capability("agent-market.timeless__list_meetings"))
                .is_none(),
            "another user's discovered capability must not resolve"
        );
        assert!(
            other_view
                .get_capability(&capability("agent-market.search_agents"))
                .is_some(),
            "the static surface must remain intact for other users"
        );
    }

    #[test]
    fn overlay_entries_never_leak_across_tenants_for_the_same_user_id() {
        // The exact isolation Ilya flagged: same UserId, different tenant must
        // not share a discovered surface or negative-cache verdict.
        let overlay = ScopedPackageOverlay::new();
        let tenant_a = owner_in("tenant-a", "shared-user");
        let tenant_b = owner_in("tenant-b", "shared-user");
        overlay.insert(
            tenant_a.clone(),
            discovered_package("timeless__list_meetings"),
            Duration::from_secs(60),
        );

        let a_view = overlay.view_for(&tenant_a, global_registry());
        assert!(a_view.has_overlays(), "owning tenant sees its surface");

        let b_view = overlay.view_for(&tenant_b, global_registry());
        assert!(
            !b_view.has_overlays(),
            "a different tenant with the same user id must not see it"
        );
        assert!(
            b_view
                .get_capability(&capability("agent-market.timeless__list_meetings"))
                .is_none()
        );
        assert!(overlay.get(&tenant_b, &static_package().id).is_none());
    }

    #[test]
    fn overlay_entries_never_leak_across_threads_for_the_same_owner() {
        // The managed-agent isolation case: buyer A hires the agent and grants
        // `timeless`, buyer B hires the SAME agent and grants `firefly`. Both
        // jobs run under the agent's single IronClaw identity (same tenant +
        // user), distinguished only by the thread each dispatch runs on. Keyed
        // by owner alone, job B would see job A's `timeless` — a cross-buyer
        // personal-data leak. Keyed by thread, each job sees only its own.
        let overlay = ScopedPackageOverlay::new();
        let job_a = owner_thread("worker-agent", "thread-a");
        let job_b = owner_thread("worker-agent", "thread-b");
        overlay.insert(
            job_a.clone(),
            discovered_package("timeless__list_meetings"),
            Duration::from_secs(60),
        );
        overlay.insert(
            job_b.clone(),
            discovered_package("firefly__list_documents"),
            Duration::from_secs(60),
        );

        let a_view = overlay.view_for(&job_a, global_registry());
        assert!(
            a_view
                .get_capability(&capability("agent-market.timeless__list_meetings"))
                .is_some(),
            "job A sees its own connector"
        );
        assert!(
            a_view
                .get_capability(&capability("agent-market.firefly__list_documents"))
                .is_none(),
            "job A must NOT see buyer B's connector"
        );

        let b_view = overlay.view_for(&job_b, global_registry());
        assert!(
            b_view
                .get_capability(&capability("agent-market.firefly__list_documents"))
                .is_some(),
            "job B sees its own connector"
        );
        assert!(
            b_view
                .get_capability(&capability("agent-market.timeless__list_meetings"))
                .is_none(),
            "job B must NOT see buyer A's connector"
        );

        // And a thread-less scope for the same owner is its own bucket — it sees
        // neither job's surface.
        let threadless = owner("worker-agent");
        assert!(!overlay.view_for(&threadless, global_registry()).has_overlays());
    }

    #[test]
    fn readers_serve_stale_last_good_so_concurrent_skipped_turns_keep_the_surface() {
        // Point 2: an entry past TTL is still served (last-good). A concurrent
        // turn that skips the in-flight refresh must not flap to the static
        // manifest.
        let overlay = ScopedPackageOverlay::new();
        let worker = owner("worker-user");
        overlay.insert(
            worker.clone(),
            discovered_package("timeless__list_meetings"),
            Duration::from_secs(0), // already expired
        );

        assert_eq!(
            overlay.get(&worker, &static_package().id).map(|(_, f)| f),
            Some(OverlayFreshness::Stale),
            "the entry is stale (refresher would re-discover)"
        );
        let view = overlay.view_for(&worker, global_registry());
        assert!(
            view.get_capability(&capability("agent-market.timeless__list_meetings"))
                .is_some(),
            "but readers still serve the stale last-good surface"
        );
        assert!(
            overlay
                .merged_snapshot(&worker, global_registry())
                .get_capability(&capability("agent-market.timeless__list_meetings"))
                .is_some(),
            "merged_snapshot serves last-good too"
        );
    }

    #[test]
    fn get_reports_stale_after_ttl_and_touch_re_arms_to_fresh() {
        let overlay = ScopedPackageOverlay::new();
        let worker = owner("worker-user");
        overlay.insert(
            worker.clone(),
            discovered_package("timeless__list_meetings"),
            Duration::from_secs(0),
        );

        let (package, freshness) = overlay
            .get(&worker, &static_package().id)
            .expect("stale entry retained");
        assert_eq!(
            freshness,
            OverlayFreshness::Stale,
            "get() reports Stale so the refresher re-discovers"
        );
        assert_eq!(package.id.as_str(), "agent-market");

        overlay.touch(&worker, &static_package().id, Duration::from_secs(60));
        let (_, freshness) = overlay
            .get(&worker, &static_package().id)
            .expect("touched entry");
        assert_eq!(freshness, OverlayFreshness::Fresh);
    }

    #[test]
    fn remove_extension_clears_every_user() {
        let overlay = ScopedPackageOverlay::new();
        overlay.insert(
            owner("a"),
            discovered_package("t1"),
            Duration::from_secs(60),
        );
        overlay.insert(
            owner("b"),
            discovered_package("t2"),
            Duration::from_secs(60),
        );
        overlay.remove_extension(&static_package().id);
        assert!(overlay.get(&owner("a"), &static_package().id).is_none());
        assert!(overlay.get(&owner("b"), &static_package().id).is_none());
    }
}
