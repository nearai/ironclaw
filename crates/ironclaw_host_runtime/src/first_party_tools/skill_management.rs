use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_first_party_extensions::skills::{
    SkillManagementCapabilityError, SkillManagementCapabilityKind,
    SkillManagementCapabilityRequest, dispatch,
};
use ironclaw_host_api::{
    CapabilityId, EffectKind, HostApiError, PermissionMode, ResourceUsage, RuntimeDispatchErrorKind,
};

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

use super::{first_party_capability_manifest, resource_profile};

pub const SKILL_LIST_CAPABILITY_ID: &str = "builtin.skill_list";
pub const SKILL_INSTALL_CAPABILITY_ID: &str = "builtin.skill_install";
pub const SKILL_REMOVE_CAPABILITY_ID: &str = "builtin.skill_remove";

pub(super) fn manifests() -> Result<Vec<CapabilityManifest>, ExtensionError> {
    Ok(vec![
        first_party_capability_manifest(
            SKILL_LIST_CAPABILITY_ID,
            "List Reborn filesystem skills visible to the current local-dev agent",
            vec![EffectKind::ReadFilesystem],
            PermissionMode::Allow,
            resource_profile(),
        )?,
        first_party_capability_manifest(
            SKILL_INSTALL_CAPABILITY_ID,
            "Install a SKILL.md document into the current user's Reborn skill root",
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            PermissionMode::Ask,
            resource_profile(),
        )?,
        first_party_capability_manifest(
            SKILL_REMOVE_CAPABILITY_ID,
            "Remove a user-installed Reborn filesystem skill",
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            PermissionMode::Ask,
            resource_profile(),
        )?,
    ])
}

pub(super) fn insert_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
) -> Result<(), HostApiError> {
    let handler = Arc::new(SkillManagementToolHandler);
    registry.insert_handler(
        CapabilityId::new(SKILL_LIST_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(
        CapabilityId::new(SKILL_INSTALL_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(CapabilityId::new(SKILL_REMOVE_CAPABILITY_ID)?, handler);
    Ok(())
}

struct SkillManagementToolHandler;

#[async_trait]
impl FirstPartyCapabilityHandler for SkillManagementToolHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let kind = match request.capability_id.as_str() {
            SKILL_LIST_CAPABILITY_ID => SkillManagementCapabilityKind::List,
            SKILL_INSTALL_CAPABILITY_ID => SkillManagementCapabilityKind::Install,
            SKILL_REMOVE_CAPABILITY_ID => SkillManagementCapabilityKind::Remove,
            _ => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::UndeclaredCapability,
                ));
            }
        };
        let skill_request = SkillManagementCapabilityRequest::new(
            kind,
            &request.scope,
            request.mounts.as_ref(),
            Arc::clone(&request.services.filesystem),
            &request.input,
        );
        let output = dispatch(&skill_request)
            .await
            .map_err(skill_management_error)?;
        Ok(FirstPartyCapabilityResult::new(
            output,
            ResourceUsage::default(),
        ))
    }
}

fn skill_management_error(error: SkillManagementCapabilityError) -> FirstPartyCapabilityError {
    tracing::debug!(
        runtime_dispatch_error_kind = %error.kind(),
        "skill management error mapped to first-party capability error"
    );
    FirstPartyCapabilityError::new(error.kind())
}
