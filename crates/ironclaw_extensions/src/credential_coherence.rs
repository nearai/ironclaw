use std::collections::BTreeSet;

use ironclaw_host_api::CredentialHandle;

use crate::v2::{HostApiId, ManifestSectionPath, ManifestV2Error};

/// Credential handle reference reported by one host-api manifest contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencedCredential {
    pub handle: CredentialHandle,
    pub host_api: HostApiId,
    pub section: ManifestSectionPath,
}

impl ReferencedCredential {
    pub fn new(
        handle: CredentialHandle,
        host_api: HostApiId,
        section: ManifestSectionPath,
    ) -> Self {
        Self {
            handle,
            host_api,
            section,
        }
    }
}

pub(crate) fn reject_dangling_credentials(
    declared_credentials: &[CredentialHandle],
    referenced_credentials: &[ReferencedCredential],
) -> Result<(), ManifestV2Error> {
    if declared_credentials.is_empty() {
        return Ok(());
    }

    let declared: BTreeSet<_> = declared_credentials.iter().collect();
    for reference in referenced_credentials {
        if !declared.contains(&reference.handle) {
            return Err(ManifestV2Error::DanglingCredentialHandle {
                handle: reference.handle.clone(),
                host_api: reference.host_api.clone(),
                section: reference.section.clone(),
            });
        }
    }
    Ok(())
}
