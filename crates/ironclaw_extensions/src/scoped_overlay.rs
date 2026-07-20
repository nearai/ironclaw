//! Per-user discovered-package overlay over the global extension registry.
//!
//! Hosted-MCP providers can serve a **per-principal** `tools/list` (e.g. the
//! agent marketplace returns concierge tools for a concierge bearer and the
//! hirer-granted connector tools for a worker-agent bearer). The global
//! [`crate::SharedExtensionRegistry`] holds exactly one surface per extension
//! id, so per-user discovered surfaces live here instead: an in-memory,
//! TTL-bounded cache keyed by `(UserId, ExtensionId)`.
//!
//! This is a **derived cache**, not lifecycle state: nothing here is
//! persisted, installation records are never touched, and a restart simply
//! re-discovers lazily. Consumers read through [`OverlaidRegistryView`], which
//! prefers the caller's overlay entries and falls back to the global
//! snapshot — the same view feeds the model surface, authorization, dispatch
//! and egress planning so no parallel resolution pipeline exists.
//!
//! Security invariant: entries are keyed by the exact `UserId` whose
//! credential produced the discovery result, and a view only ever merges
//! entries for the single user it was built for. Cross-user leakage is a
//! regression-tested failure mode.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use ironclaw_host_api::{CapabilityDescriptor, CapabilityId, ExtensionId, UserId};
use parking_lot::RwLock;

use crate::{CapabilityVisibility, ExtensionPackage, ExtensionRegistry};

/// Default lifetime of a discovered per-user surface before the next turn
/// re-runs discovery. Short enough that a fresh hire's newly granted tools
/// appear within a couple of minutes; long enough to keep discovery off the
/// per-turn hot path.
pub const DEFAULT_SCOPED_OVERLAY_TTL: Duration = Duration::from_secs(180);

#[derive(Debug, Clone)]
struct OverlayEntry {
    package: Arc<ExtensionPackage>,
    expires_at: Instant,
}

/// Freshness of a cached per-user discovered surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayFreshness {
    /// Within TTL — usable without re-discovery.
    Fresh,
    /// Past TTL — re-discovery should run; the stale entry remains available
    /// as a last-good fallback for transient discovery failures.
    Stale,
}

/// In-memory per-user discovered-package cache. Composition owns one instance
/// and shares it (via `Arc`) with every registry consumer that resolves
/// capabilities for a scoped request.
#[derive(Debug, Default)]
pub struct ScopedPackageOverlay {
    entries: RwLock<HashMap<(UserId, ExtensionId), OverlayEntry>>,
}

impl ScopedPackageOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store (or refresh) `user`'s discovered surface for `package.id`.
    pub fn insert(&self, user: UserId, package: ExtensionPackage, ttl: Duration) {
        let key = (user, package.id.clone());
        let entry = OverlayEntry {
            package: Arc::new(package),
            expires_at: Instant::now() + ttl,
        };
        self.entries.write().insert(key, entry);
    }

    /// Drop `user`'s entry for `extension_id` (e.g. on deactivation).
    pub fn remove(&self, user: &UserId, extension_id: &ExtensionId) {
        self.entries
            .write()
            .remove(&(user.clone(), extension_id.clone()));
    }

    /// Drop every user's entry for `extension_id` (extension deactivated or
    /// replaced tenant-wide).
    pub fn remove_extension(&self, extension_id: &ExtensionId) {
        self.entries
            .write()
            .retain(|(_, entry_extension), _| entry_extension != extension_id);
    }

    /// The cached package for `(user, extension_id)` with its freshness, if
    /// any. Stale entries are returned (marked [`OverlayFreshness::Stale`])
    /// so callers can keep the last-good surface across transient discovery
    /// failures.
    pub fn get(
        &self,
        user: &UserId,
        extension_id: &ExtensionId,
    ) -> Option<(Arc<ExtensionPackage>, OverlayFreshness)> {
        let entries = self.entries.read();
        let entry = entries.get(&(user.clone(), extension_id.clone()))?;
        let freshness = if entry.expires_at > Instant::now() {
            OverlayFreshness::Fresh
        } else {
            OverlayFreshness::Stale
        };
        Some((Arc::clone(&entry.package), freshness))
    }

    /// Re-arm the TTL on an existing entry (transient discovery failure kept
    /// the last-good surface; avoid re-running discovery every turn while the
    /// provider is down).
    pub fn touch(&self, user: &UserId, extension_id: &ExtensionId, ttl: Duration) {
        if let Some(entry) = self
            .entries
            .write()
            .get_mut(&(user.clone(), extension_id.clone()))
        {
            entry.expires_at = Instant::now() + ttl;
        }
    }

    /// The caller's unexpired overlay packages (for grant/provider-trust
    /// minting at surface-request time).
    pub fn fresh_packages_for(&self, user: &UserId) -> Vec<Arc<ExtensionPackage>> {
        let now = Instant::now();
        self.entries
            .read()
            .iter()
            .filter(|((entry_user, _), entry)| entry_user == user && entry.expires_at > now)
            .map(|(_, entry)| Arc::clone(&entry.package))
            .collect()
    }

    /// A concrete `ExtensionRegistry` snapshot with the caller's fresh
    /// discovered packages merged in (each overlaid extension's static package
    /// replaced by its discovered one). Returns the `global` Arc unchanged when
    /// the user has no fresh overlay entries — zero cost for the common case,
    /// so every existing `self.registry.snapshot()` consumer can resolve a
    /// user's discovered capabilities by reading this instead, with no change
    /// to its `&ExtensionRegistry` API.
    pub fn merged_snapshot(
        &self,
        user: &UserId,
        global: Arc<ExtensionRegistry>,
    ) -> Arc<ExtensionRegistry> {
        let packages = self.fresh_packages_for(user);
        if packages.is_empty() {
            return global;
        }
        let mut merged = global.as_ref().clone();
        for package in packages {
            merged.remove(&package.id);
            let id = package.id.clone();
            if merged.insert(package.as_ref().clone()).is_err() {
                // A discovered package is a re-projection of an already-validated
                // manifest, so this branch is not expected; on the theoretical
                // failure restore the global entry rather than drop the extension.
                if let Some(original) = global.get_extension(&id) {
                    let _ = merged.insert(original.clone());
                }
            }
        }
        Arc::new(merged)
    }

    /// Build the single-user merged view over `global`. Only unexpired
    /// entries participate: a stale surface must not silently serve past its
    /// TTL when re-discovery has not confirmed it (callers that want the
    /// last-good-on-transient behavior re-arm via [`Self::touch`]).
    pub fn view_for(&self, user: &UserId, global: Arc<ExtensionRegistry>) -> OverlaidRegistryView {
        let now = Instant::now();
        let overlays: Vec<Arc<ExtensionPackage>> = self
            .entries
            .read()
            .iter()
            .filter(|((entry_user, _), entry)| entry_user == user && entry.expires_at > now)
            .map(|(_, entry)| Arc::clone(&entry.package))
            .collect();
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
    use ironclaw_host_api::VirtualPath;

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

[[capabilities]]
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

    fn static_package() -> ExtensionPackage {
        let manifest = ExtensionManifest::parse(
            STATIC_MANIFEST,
            ManifestSource::HostBundled,
            &HostPortCatalog::default(),
        )
        .expect("valid manifest");
        ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new("/system/extensions/agent-market").expect("root"),
        )
        .expect("valid package")
    }

    fn discovered_package(tool: &str) -> ExtensionPackage {
        let tools = vec![crate::HostedMcpDiscoveredTool {
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

    fn capability(id: &str) -> CapabilityId {
        CapabilityId::new(id).expect("capability id")
    }

    #[test]
    fn view_prefers_overlay_and_masks_replaced_static_capabilities() {
        let overlay = ScopedPackageOverlay::new();
        let worker = user("worker-user");
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
        let worker = user("worker-user");
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

        let other_reg = overlay.merged_snapshot(&user("other"), global_registry());
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
        let merged = overlay.merged_snapshot(&user("nobody"), Arc::clone(&global));
        assert!(Arc::ptr_eq(&global, &merged), "zero-cost passthrough");
    }

    #[test]
    fn overlay_entries_never_leak_across_users() {
        let overlay = ScopedPackageOverlay::new();
        let worker = user("worker-user");
        let other = user("concierge-user");
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
    fn expired_entries_drop_out_of_views_but_remain_as_stale_last_good() {
        let overlay = ScopedPackageOverlay::new();
        let worker = user("worker-user");
        overlay.insert(
            worker.clone(),
            discovered_package("timeless__list_meetings"),
            Duration::from_secs(0),
        );

        let view = overlay.view_for(&worker, global_registry());
        assert!(!view.has_overlays(), "expired entry must not serve");
        let (package, freshness) = overlay
            .get(&worker, &static_package().id)
            .expect("stale entry retained");
        assert_eq!(freshness, OverlayFreshness::Stale);
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
            user("a"),
            discovered_package("t1"),
            Duration::from_secs(60),
        );
        overlay.insert(
            user("b"),
            discovered_package("t2"),
            Duration::from_secs(60),
        );
        overlay.remove_extension(&static_package().id);
        assert!(overlay.get(&user("a"), &static_package().id).is_none());
        assert!(overlay.get(&user("b"), &static_package().id).is_none());
    }
}
