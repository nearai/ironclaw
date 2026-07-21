//! The immutable active snapshot and its resolver views (overview.md §5.1).
//!
//! Activation publishes one immutable `Arc<ActiveSnapshot>`; readers resolve
//! through it, and in-flight work keeps the `Arc` it started with, so an
//! upgrade never tears a running invocation. The snapshot is built once per
//! generation and never mutated.

use std::collections::BTreeMap;
use std::sync::Arc;

use ironclaw_extensions::ResolvedExtensionManifest;
use ironclaw_host_api::{CapabilityId, ToolAdapter};
use ironclaw_product_adapters::ChannelAdapter;

/// One activated extension's bound behavior plus its resolved contract.
pub struct ActiveExtension {
    pub extension_id: String,
    pub installation_id: String,
    pub resolved: Arc<ResolvedExtensionManifest>,
    pub tools: Option<Arc<dyn ToolAdapter>>,
    pub channel: Option<Arc<dyn ChannelAdapter>>,
}

/// A monotonically increasing snapshot generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Generation(pub u64);

/// The immutable active set for one generation.
pub struct ActiveSnapshot {
    generation: Generation,
    /// Extensions keyed by extension id.
    extensions: BTreeMap<String, Arc<ActiveExtension>>,
    /// Capability id → owning extension id (built once; every capability id
    /// is unique across active extensions, enforced at activation).
    capability_owner: BTreeMap<CapabilityId, String>,
    /// Ingress `route_suffix` → owning extension id (unique across active
    /// extensions, enforced at activation).
    route_owner: BTreeMap<String, String>,
}

/// One prebound tool binding a resolver returns.
pub struct ResolvedToolBinding {
    pub adapter: Arc<dyn ToolAdapter>,
    pub declaration: Arc<ResolvedExtensionManifest>,
    pub generation: Generation,
}

impl ActiveSnapshot {
    /// The empty generation-0 snapshot.
    pub fn empty() -> Arc<Self> {
        Arc::new(Self {
            generation: Generation(0),
            extensions: BTreeMap::new(),
            capability_owner: BTreeMap::new(),
            route_owner: BTreeMap::new(),
        })
    }

    /// Build the next snapshot from an extension set, checking global
    /// conflicts (duplicate capability id or ingress route across active
    /// extensions → `SnapshotConflict`).
    pub fn build(
        generation: Generation,
        extensions: Vec<Arc<ActiveExtension>>,
    ) -> Result<Arc<Self>, SnapshotConflict> {
        let mut by_id = BTreeMap::new();
        let mut capability_owner = BTreeMap::new();
        let mut route_owner = BTreeMap::new();

        for extension in extensions {
            for tool in &extension.resolved.tools {
                if let Some(existing) =
                    capability_owner.insert(tool.id.clone(), extension.extension_id.clone())
                {
                    return Err(SnapshotConflict::DuplicateCapability {
                        capability_id: tool.id.as_str().to_string(),
                        first: existing,
                        second: extension.extension_id.clone(),
                    });
                }
            }
            if let Some(channel) = &extension.resolved.channel
                && let Some(ingress) = &channel.ingress
            {
                let suffix = ingress.route_suffix.as_str().to_string();
                if let Some(existing) =
                    route_owner.insert(suffix.clone(), extension.extension_id.clone())
                {
                    return Err(SnapshotConflict::DuplicateRoute {
                        route_suffix: suffix,
                        first: existing,
                        second: extension.extension_id.clone(),
                    });
                }
            }
            by_id.insert(extension.extension_id.clone(), extension);
        }

        Ok(Arc::new(Self {
            generation,
            extensions: by_id,
            capability_owner,
            route_owner,
        }))
    }

    pub fn generation(&self) -> Generation {
        self.generation
    }

    /// Resolve a prebound tool adapter by capability id.
    pub fn resolve_tool(&self, capability_id: &CapabilityId) -> Option<ResolvedToolBinding> {
        let owner = self.capability_owner.get(capability_id)?;
        let extension = self.extensions.get(owner)?;
        let adapter = extension.tools.clone()?;
        Some(ResolvedToolBinding {
            adapter,
            declaration: Arc::clone(&extension.resolved),
            generation: self.generation,
        })
    }

    /// Resolve the active extension serving
    /// `/webhooks/extensions/{extension_id}/{route_suffix}` — the extension
    /// must be active, declare an inbound channel, and declare exactly this
    /// ingress suffix.
    pub fn resolve_channel_ingress(
        &self,
        extension_id: &str,
        route_suffix: &str,
    ) -> Option<Arc<ActiveExtension>> {
        let extension = self.extensions.get(extension_id)?;
        let channel = extension.resolved.channel.as_ref()?;
        let ingress = channel.ingress.as_ref()?;
        if !channel.inbound || ingress.route_suffix.as_str() != route_suffix {
            return None;
        }
        Some(Arc::clone(extension))
    }

    /// Resolve an active extension by id.
    pub fn extension(&self, extension_id: &str) -> Option<Arc<ActiveExtension>> {
        self.extensions.get(extension_id).cloned()
    }

    /// Active extension ids, sorted.
    pub fn extension_ids(&self) -> Vec<String> {
        self.extensions.keys().cloned().collect()
    }

    /// Whether a capability id or ingress route would conflict with the
    /// active set (used to reject a staged next generation before publish).
    pub fn would_conflict(&self, candidate: &ActiveExtension) -> Option<SnapshotConflict> {
        for tool in &candidate.resolved.tools {
            if let Some(existing) = self.capability_owner.get(&tool.id)
                && existing != &candidate.extension_id
            {
                return Some(SnapshotConflict::DuplicateCapability {
                    capability_id: tool.id.as_str().to_string(),
                    first: existing.clone(),
                    second: candidate.extension_id.clone(),
                });
            }
        }
        if let Some(channel) = &candidate.resolved.channel
            && let Some(ingress) = &channel.ingress
        {
            let suffix = ingress.route_suffix.as_str();
            if let Some(existing) = self.route_owner.get(suffix)
                && existing != &candidate.extension_id
            {
                return Some(SnapshotConflict::DuplicateRoute {
                    route_suffix: suffix.to_string(),
                    first: existing.clone(),
                    second: candidate.extension_id.clone(),
                });
            }
        }
        None
    }
}

/// A global activation conflict against the active set.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotConflict {
    #[error("capability id `{capability_id}` is declared by both `{first}` and `{second}`")]
    DuplicateCapability {
        capability_id: String,
        first: String,
        second: String,
    },
    #[error("ingress route `{route_suffix}` is declared by both `{first}` and `{second}`")]
    DuplicateRoute {
        route_suffix: String,
        first: String,
        second: String,
    },
    #[error(
        "capability id `{capability_id}` declared by `{extension_id}` collides with a host built-in"
    )]
    ReservedCapability {
        capability_id: String,
        extension_id: String,
    },
    #[error(
        "ingress route `{route}` declared by `{extension_id}` collides with a fixed host route"
    )]
    ReservedRoute { route: String, extension_id: String },
}
