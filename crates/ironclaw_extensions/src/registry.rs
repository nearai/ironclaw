use std::collections::{HashMap, HashSet};

use ironclaw_host_api::{CapabilityDescriptor, CapabilityId, ExtensionId};

use crate::{ExtensionError, ExtensionPackage, ManifestSource};

/// Registry of validated extension packages and declared capabilities.
#[derive(Debug, Default)]
pub struct ExtensionRegistry {
    packages: HashMap<ExtensionId, ExtensionPackage>,
    capabilities: HashMap<CapabilityId, CapabilityDescriptor>,
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
        }
        self.capability_order
            .retain(|capability_id| !current_capability_ids.contains(capability_id));
        for (offset, descriptor) in package.capabilities.iter().enumerate() {
            self.capability_order
                .insert(capability_insert_index + offset, descriptor.id.clone());
            self.capabilities
                .insert(descriptor.id.clone(), descriptor.clone());
        }
        if let Some(index) = extension_index {
            self.extension_order[index] = id.clone();
        } else {
            self.extension_order.push(id.clone());
        }
        self.packages.insert(id, package);
    }

    pub(crate) fn remove_validated(&mut self, id: &ExtensionId) -> Option<ExtensionPackage> {
        let package = self.packages.remove(id)?;
        self.extension_order
            .retain(|extension_id| extension_id != id);
        for descriptor in &package.capabilities {
            self.capabilities.remove(&descriptor.id);
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

pub(crate) fn validate_package_consistency(
    package: &ExtensionPackage,
) -> Result<(), ExtensionError> {
    let expected = ExtensionPackage::from_manifest(package.manifest.clone(), package.root.clone())?;
    if package.id != expected.id {
        return Err(ExtensionError::InvalidManifest {
            reason: format!(
                "package id {} does not match manifest/root id {}",
                package.id, expected.id
            ),
        });
    }
    // Host-bundled packages may pre-resolve manifest schema refs into concrete
    // descriptors; all other descriptor fields must still match the manifest.
    if package.capabilities != expected.capabilities
        && !host_bundled_capabilities_match_except_parameters_schema(package, &expected)
    {
        return Err(ExtensionError::InvalidManifest {
            reason: "package capability descriptors do not match manifest declarations".to_string(),
        });
    }
    Ok(())
}

fn host_bundled_capabilities_match_except_parameters_schema(
    package: &ExtensionPackage,
    expected: &ExtensionPackage,
) -> bool {
    if package.manifest.source != ManifestSource::HostBundled
        || package.capabilities.len() != expected.capabilities.len()
    {
        return false;
    }

    let mut normalized = package.capabilities.clone();
    for (actual, expected) in normalized.iter_mut().zip(&expected.capabilities) {
        actual.parameters_schema = expected.parameters_schema.clone();
    }
    normalized == expected.capabilities
}
