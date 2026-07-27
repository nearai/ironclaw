use std::collections::BTreeMap;

use ironclaw_host_api::{ExtensionId, SecretHandle};

use crate::installations::{
    ExtensionCredentialBinding, ExtensionCredentialHandle, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationPersistedParts,
    InstallationOwner,
};

/// Reduce persisted installation rows to one deterministic row per extension.
///
/// The reducer is shared by persistence and migration adapters so the generic
/// installation policy has one owner: the extension state model. The canonical
/// row uses the extension id as its installation id, gives tenant ownership
/// precedence over member ownership, unions member owners, merges agreeing
/// credential bindings, selects the newest health snapshot (with a typed
/// installation-id tie-break), and preserves the maximum row update timestamp.
pub fn canonicalize_installation_rows(
    installations: Vec<ExtensionInstallation>,
) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
    let mut grouped = BTreeMap::<ExtensionId, Vec<ExtensionInstallation>>::new();
    for installation in installations {
        grouped
            .entry(installation.extension_id().clone())
            .or_default()
            .push(installation);
    }

    grouped
        .into_iter()
        .map(|(extension_id, rows)| {
            let Some(first) = rows.first() else {
                return Err(ExtensionInstallationError::InvalidInstallation {
                    reason: "installation group unexpectedly empty".to_string(),
                });
            };

            let manifest_ref = first.manifest_ref().clone();
            if rows.iter().any(|row| row.manifest_ref() != &manifest_ref) {
                return Err(ExtensionInstallationError::ConflictingManifestReference {
                    extension_id: extension_id.clone(),
                });
            }

            let owner = if rows.iter().any(|row| row.owner().is_tenant()) {
                InstallationOwner::Tenant
            } else {
                let user_ids = rows
                    .iter()
                    .filter_map(|row| row.owner().members())
                    .flat_map(|members| members.iter().cloned())
                    .collect();
                InstallationOwner::users(user_ids)?
            };
            let mut bindings_by_handle = BTreeMap::<ExtensionCredentialHandle, SecretHandle>::new();
            for row in &rows {
                for binding in row.credential_bindings() {
                    let handle = binding.credential_handle().clone();
                    if let Some(existing) = bindings_by_handle.get(&handle) {
                        if existing != binding.secret_handle() {
                            return Err(ExtensionInstallationError::ConflictingCredentialBinding {
                                extension_id: extension_id.clone(),
                                handle,
                            });
                        }
                    } else {
                        bindings_by_handle.insert(handle, binding.secret_handle().clone());
                    }
                }
            }
            let credential_bindings = bindings_by_handle
                .into_iter()
                .map(|(handle, secret_handle)| {
                    ExtensionCredentialBinding::new(handle, secret_handle)
                })
                .collect();

            let health = rows
                .iter()
                .max_by(|left, right| {
                    left.health()
                        .checked_at()
                        .cmp(&right.health().checked_at())
                        .then_with(|| left.installation_id().cmp(right.installation_id()))
                })
                .map(|row| row.health().clone())
                .ok_or_else(|| ExtensionInstallationError::InvalidInstallation {
                    reason: "installation group unexpectedly empty".to_string(),
                })?;
            let updated_at = rows
                .iter()
                .map(ExtensionInstallation::updated_at)
                .max()
                .ok_or_else(|| ExtensionInstallationError::InvalidInstallation {
                    reason: "installation group unexpectedly empty".to_string(),
                })?;
            let installation_id = ExtensionInstallationId::new(extension_id.as_str())?;

            ExtensionInstallation::from_persisted_parts(ExtensionInstallationPersistedParts {
                installation_id,
                extension_id,
                manifest_ref,
                credential_bindings,
                health,
                updated_at,
                owner,
            })
        })
        .collect()
}
