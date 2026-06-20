use std::collections::BTreeSet;

use ironclaw_host_api::{CredentialHandle, CredentialHandleError};

use crate::v2::{
    HostApiId, HostApiManifestProjection, HostApiRefV2, ManifestSectionPath, ManifestV2Error,
};

/// Credential handle reference reported by one host-api manifest contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencedCredential {
    pub handle: CredentialHandle,
    pub host_api: HostApiId,
    pub section: ManifestSectionPath,
}

impl HostApiManifestProjection {
    pub fn declare_credential_handles<I, S>(
        &mut self,
        handles: I,
    ) -> Result<(), CredentialHandleError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for handle in handles {
            self.declared_credentials
                .push(CredentialHandle::new(handle.as_ref())?);
        }
        Ok(())
    }

    pub fn reference_credential_handles<I, S>(
        &mut self,
        host_api: &HostApiRefV2,
        handles: I,
    ) -> Result<(), CredentialHandleError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for handle in handles {
            self.referenced_credentials.push(ReferencedCredential {
                handle: CredentialHandle::new(handle.as_ref())?,
                host_api: host_api.id.clone(),
                section: host_api.section.clone(),
            });
        }
        Ok(())
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
