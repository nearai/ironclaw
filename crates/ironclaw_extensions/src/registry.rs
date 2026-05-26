use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
};

use ironclaw_host_api::{CapabilityDescriptor, CapabilityId, ExtensionId};
use parking_lot::RwLock;

use crate::{CapabilityVisibility, ExtensionError, ExtensionPackage};

/// Registry of validated extension packages and declared capabilities.
#[derive(Debug, Default, Clone)]
pub struct ExtensionRegistry {
    packages: HashMap<ExtensionId, ExtensionPackage>,
    capabilities: HashMap<CapabilityId, CapabilityDescriptor>,
    capability_visibility: HashMap<CapabilityId, CapabilityVisibility>,
    extension_order: Vec<ExtensionId>,
    capability_order: Vec<CapabilityId>,
}

impl ExtensionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, package: ExtensionPackage) -> Result<(), ExtensionError> {
        self.validate_insertable(&package)?;
        self.insert_validated(package);
        Ok(())
    }

    pub(crate) fn validate_insertable(
        &self,
        package: &ExtensionPackage,
    ) -> Result<(), ExtensionError> {
        validate_package_consistency(package)?;

        if self.packages.contains_key(&package.id) {
            return Err(ExtensionError::DuplicateExtension {
                id: package.id.clone(),
            });
        }

        self.validate_capabilities_available(package, None)
    }

    pub(crate) fn validate_replacement(
        &self,
        package: &ExtensionPackage,
    ) -> Result<(), ExtensionError> {
        validate_package_consistency(package)?;
        self.existing_package(&package.id)?;
        self.validate_capabilities_available(package, Some(&package.id))
    }

    fn validate_capabilities_available(
        &self,
        package: &ExtensionPackage,
        replacing: Option<&ExtensionId>,
    ) -> Result<(), ExtensionError> {
        let mut seen_capabilities = HashSet::new();
        for descriptor in &package.capabilities {
            let capability_belongs_to_replaced_package = replacing
                .and_then(|id| self.packages.get(id))
                .map(|current| {
                    current
                        .capabilities
                        .iter()
                        .any(|current| current.id == descriptor.id)
                })
                .unwrap_or(false);
            if !seen_capabilities.insert(descriptor.id.clone())
                || (self.capabilities.contains_key(&descriptor.id)
                    && !capability_belongs_to_replaced_package)
            {
                return Err(ExtensionError::DuplicateCapability {
                    id: descriptor.id.clone(),
                });
            }
            if descriptor.provider != package.id {
                return Err(ExtensionError::InvalidManifest {
                    reason: format!(
                        "descriptor {} provider {} does not match package {}",
                        descriptor.id, descriptor.provider, package.id
                    ),
                });
            }
        }

        Ok(())
    }

    pub(crate) fn existing_package(
        &self,
        id: &ExtensionId,
    ) -> Result<&ExtensionPackage, ExtensionError> {
        self.packages
            .get(id)
            .ok_or_else(|| ExtensionError::ExtensionNotFound { id: id.clone() })
    }

    pub(crate) fn insert_validated(&mut self, package: ExtensionPackage) {
        for descriptor in &package.capabilities {
            self.capability_order.push(descriptor.id.clone());
            self.capabilities
                .insert(descriptor.id.clone(), descriptor.clone());
            if let Some(capability) = package
                .manifest
                .capabilities
                .iter()
                .find(|capability| capability.id == descriptor.id)
            {
                self.capability_visibility
                    .insert(descriptor.id.clone(), capability.visibility);
            }
        }
        self.extension_order.push(package.id.clone());
        self.packages.insert(package.id.clone(), package);
    }

    pub(crate) fn replace_validated(&mut self, package: ExtensionPackage) {
        let id = package.id.clone();
        let extension_index = self
            .extension_order
            .iter()
            .position(|extension_id| extension_id == &id);
        let Some(current) = self.packages.remove(&id) else {
            return;
        };
        let current_capability_ids = current
            .capabilities
            .iter()
            .map(|descriptor| descriptor.id.clone())
            .collect::<HashSet<_>>();
        let capability_insert_index = self
            .capability_order
            .iter()
            .position(|capability_id| current_capability_ids.contains(capability_id))
            .unwrap_or(self.capability_order.len());

        for capability_id in &current_capability_ids {
            self.capabilities.remove(capability_id);
            self.capability_visibility.remove(capability_id);
        }
        self.capability_order
            .retain(|capability_id| !current_capability_ids.contains(capability_id));
        for (offset, descriptor) in package.capabilities.iter().enumerate() {
            self.capability_order
                .insert(capability_insert_index + offset, descriptor.id.clone());
            self.capabilities
                .insert(descriptor.id.clone(), descriptor.clone());
            if let Some(capability) = package
                .manifest
                .capabilities
                .iter()
                .find(|capability| capability.id == descriptor.id)
            {
                self.capability_visibility
                    .insert(descriptor.id.clone(), capability.visibility);
            }
        }
        if let Some(index) = extension_index {
            self.extension_order[index] = id.clone();
        } else {
            self.extension_order.push(id.clone());
        }
        self.packages.insert(id, package);
    }

    pub fn update(&mut self, package: ExtensionPackage) -> Result<(), ExtensionError> {
        self.validate_replacement(&package)?;
        self.replace_validated(package);
        Ok(())
    }

    pub fn remove(&mut self, id: &ExtensionId) -> Option<ExtensionPackage> {
        let package = self.packages.remove(id)?;
        self.extension_order
            .retain(|extension_id| extension_id != id);
        for descriptor in &package.capabilities {
            self.capabilities.remove(&descriptor.id);
            self.capability_visibility.remove(&descriptor.id);
            self.capability_order
                .retain(|capability_id| capability_id != &descriptor.id);
        }
        Some(package)
    }

    pub fn get_extension(&self, id: &ExtensionId) -> Option<&ExtensionPackage> {
        self.packages.get(id)
    }

    pub fn get_capability(&self, id: &CapabilityId) -> Option<&CapabilityDescriptor> {
        self.capabilities.get(id)
    }

    pub fn capability_visibility(&self, id: &CapabilityId) -> Option<CapabilityVisibility> {
        self.capability_visibility.get(id).copied()
    }

    pub fn extensions(&self) -> impl Iterator<Item = &ExtensionPackage> {
        self.extension_order
            .iter()
            .filter_map(|id| self.packages.get(id))
    }

    pub fn capabilities(&self) -> impl Iterator<Item = &CapabilityDescriptor> {
        self.capability_order
            .iter()
            .filter_map(|id| self.capabilities.get(id))
    }
}

#[derive(Debug, Clone)]
pub struct SharedExtensionRegistry {
    inner: Arc<RwLock<Arc<ExtensionRegistry>>>,
    version: Arc<AtomicU64>,
}

impl SharedExtensionRegistry {
    pub fn new(registry: ExtensionRegistry) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Arc::new(registry))),
            version: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn snapshot(&self) -> Arc<ExtensionRegistry> {
        Arc::clone(&self.inner.read())
    }

    pub fn snapshot_owned(&self) -> ExtensionRegistry {
        self.snapshot().as_ref().clone()
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    pub fn insert(&self, package: ExtensionPackage) -> Result<(), ExtensionError> {
        self.with_mut(|registry| registry.insert(package))
    }

    pub fn update(&self, package: ExtensionPackage) -> Result<(), ExtensionError> {
        self.with_mut(|registry| registry.update(package))
    }

    pub fn upsert(&self, package: ExtensionPackage) -> Result<(), ExtensionError> {
        self.with_mut(|registry| {
            if registry.get_extension(&package.id).is_some() {
                registry.update(package)
            } else {
                registry.insert(package)
            }
        })
    }

    pub fn remove(&self, id: &ExtensionId) -> Option<ExtensionPackage> {
        self.with_mut(|registry| registry.remove(id))
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut ExtensionRegistry) -> R) -> R {
        let result = f(Arc::make_mut(&mut self.inner.write()));
        self.version.fetch_add(1, Ordering::AcqRel);
        result
    }

    pub fn replace(&self, registry: ExtensionRegistry) {
        *self.inner.write() = Arc::new(registry);
        self.version.fetch_add(1, Ordering::AcqRel);
    }
}

impl Default for SharedExtensionRegistry {
    fn default() -> Self {
        Self::new(ExtensionRegistry::default())
    }
}

pub(crate) fn validate_package_consistency(
    package: &ExtensionPackage,
) -> Result<(), ExtensionError> {
    crate::ensure_extension_root_matches(&package.manifest.id, &package.root)?;
    if package.id != package.manifest.id {
        return Err(ExtensionError::InvalidManifest {
            reason: format!(
                "package id {} does not match manifest/root id {}",
                package.id, package.manifest.id
            ),
        });
    }
    if package.capabilities.len() != package.manifest.capabilities.len() {
        return Err(ExtensionError::InvalidManifest {
            reason: "package capability descriptors do not match manifest declarations".to_string(),
        });
    }
    let expected_prefix = format!("{}.", package.manifest.id.as_str());
    for (descriptor, manifest_capability) in package
        .capabilities
        .iter()
        .zip(package.manifest.capabilities.iter())
    {
        if !manifest_capability
            .id
            .as_str()
            .starts_with(&expected_prefix)
        {
            return Err(ExtensionError::InvalidManifest {
                reason: format!(
                    "capability id {} must be provider-prefixed with {}",
                    manifest_capability.id.as_str(),
                    expected_prefix
                ),
            });
        }
        let expected = CapabilityDescriptor {
            id: manifest_capability.id.clone(),
            provider: package.manifest.id.clone(),
            runtime: package.manifest.runtime_kind(),
            trust_ceiling: package.manifest.descriptor_trust_default,
            description: manifest_capability.description.clone(),
            parameters_schema: crate::descriptor_schema_ref(manifest_capability),
            effects: manifest_capability.effects.clone(),
            default_permission: manifest_capability.default_permission,
            runtime_credentials: manifest_capability.runtime_credentials.clone(),
            resource_profile: manifest_capability.resource_profile.clone(),
        };
        if descriptor != &expected {
            return Err(ExtensionError::InvalidManifest {
                reason: "package capability descriptors do not match manifest declarations"
                    .to_string(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::thread;

    use ironclaw_host_api::{HostPortCatalog, VirtualPath};

    use super::*;
    use crate::{ExtensionManifest, ManifestSource};

    #[test]
    fn shared_registry_upsert_inserts_new_and_updates_existing() {
        let registry = SharedExtensionRegistry::default();
        let initial = test_package("fixture", &["search"]);
        registry.upsert(initial).expect("insert through upsert");

        let updated = test_package("fixture", &["write"]);
        registry.upsert(updated).expect("update through upsert");
        let snapshot = registry.snapshot();

        assert!(snapshot.get_extension(&extension_id("fixture")).is_some());
        assert!(
            snapshot
                .get_capability(&capability_id("fixture.search"))
                .is_none()
        );
        assert!(
            snapshot
                .get_capability(&capability_id("fixture.write"))
                .is_some()
        );
    }

    #[test]
    fn shared_registry_methods_cover_mutation_and_snapshot_paths() {
        let registry = SharedExtensionRegistry::default();
        registry
            .insert(test_package("alpha", &["read"]))
            .expect("insert package");
        registry
            .update(test_package("alpha", &["write"]))
            .expect("update package");
        assert!(
            registry
                .snapshot()
                .get_capability(&capability_id("alpha.write"))
                .is_some()
        );

        let held_snapshot = registry.snapshot();
        registry
            .upsert(test_package("beta", &["read"]))
            .expect("copy-on-write insert while snapshot is held");
        assert!(held_snapshot.get_extension(&extension_id("beta")).is_none());
        assert!(
            registry
                .snapshot()
                .get_extension(&extension_id("beta"))
                .is_some()
        );

        let owned = registry.snapshot_owned();
        assert!(owned.get_extension(&extension_id("alpha")).is_some());

        let removed = registry
            .remove(&extension_id("alpha"))
            .expect("remove package");
        assert_eq!(removed.id, extension_id("alpha"));
        assert!(
            registry
                .snapshot()
                .get_extension(&extension_id("alpha"))
                .is_none()
        );

        let mut replacement = ExtensionRegistry::new();
        replacement
            .insert(test_package("gamma", &["read"]))
            .expect("replacement insert");
        registry.replace(replacement);
        assert!(
            registry
                .snapshot()
                .get_extension(&extension_id("gamma"))
                .is_some()
        );
        assert!(
            registry
                .snapshot()
                .get_extension(&extension_id("beta"))
                .is_none()
        );
    }

    #[test]
    fn shared_registry_concurrent_insert_and_snapshot() {
        let registry = SharedExtensionRegistry::default();
        let writer_registry = registry.clone();
        let writer = thread::spawn(move || {
            for id in ["alpha", "beta", "gamma"] {
                writer_registry
                    .upsert(test_package(id, &["read"]))
                    .expect("writer upsert");
            }
        });

        let reader_registry = registry.clone();
        let reader = thread::spawn(move || {
            for _ in 0..16 {
                let _ = reader_registry.snapshot();
            }
        });

        writer.join().expect("writer thread");
        reader.join().expect("reader thread");

        let snapshot = registry.snapshot();
        for id in ["alpha", "beta", "gamma"] {
            assert!(snapshot.get_extension(&extension_id(id)).is_some());
        }
    }

    fn test_package(extension_id: &str, capabilities: &[&str]) -> ExtensionPackage {
        let capability_blocks = capabilities
            .iter()
            .map(|name| {
                format!(
                    r#"
[[capabilities]]
id = "{extension_id}.{name}"
description = "{name}"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/{name}.input.json"
output_schema_ref = "schemas/{name}.output.json"
"#
                )
            })
            .collect::<String>();
        let manifest = format!(
            r#"
schema_version = "reborn.extension_manifest.v2"
id = "{extension_id}"
name = "{extension_id}"
version = "0.1.0"
description = "test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{extension_id}.wasm"
{capability_blocks}
"#
        );
        let manifest = ExtensionManifest::parse(
            &manifest,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
        )
        .expect("manifest parses");
        ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new(format!("/system/extensions/{extension_id}")).expect("root"),
        )
        .expect("package builds")
    }

    fn extension_id(value: &str) -> ExtensionId {
        ExtensionId::new(value.to_string()).expect("extension id")
    }

    fn capability_id(value: &str) -> CapabilityId {
        CapabilityId::new(value.to_string()).expect("capability id")
    }
}
