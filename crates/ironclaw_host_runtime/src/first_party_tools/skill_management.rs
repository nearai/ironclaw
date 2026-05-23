use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    CapabilityId, EffectKind, HostApiError, PermissionMode, ResourceUsage, RuntimeDispatchErrorKind,
};
use ironclaw_skills::{
    SkillInstallRequest, SkillManagementContext, SkillManagementError, SkillManagementErrorKind,
    SkillRemoveRequest, SkillSummary, install_skill, list_skills, remove_skill,
};
use serde_json::{Value, json};

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
        let output = match request.capability_id.as_str() {
            SKILL_LIST_CAPABILITY_ID => dispatch_list(&request).await?,
            SKILL_INSTALL_CAPABILITY_ID => dispatch_install(&request).await?,
            SKILL_REMOVE_CAPABILITY_ID => dispatch_remove(&request).await?,
            _ => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::UndeclaredCapability,
                ));
            }
        };
        Ok(FirstPartyCapabilityResult::new(
            output,
            ResourceUsage::default(),
        ))
    }
}

async fn dispatch_list(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    let context = management_context(request)?;
    let skills = list_skills(&context).await.map_err(capability_error)?;
    Ok(json!({
        "skills": skills.iter().map(skill_summary_json).collect::<Vec<_>>(),
        "count": skills.len(),
    }))
}

async fn dispatch_install(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    let content = request
        .input
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(input_error)?;
    let name = request.input.get("name").and_then(Value::as_str);
    let context = management_context(request)?;
    let installed = install_skill(&context, SkillInstallRequest { name, content })
        .await
        .map_err(capability_error)?;

    Ok(json!({
        "installed": true,
        "name": installed.name,
        "path": installed.scoped_path,
    }))
}

async fn dispatch_remove(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    let name = request
        .input
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(input_error)?;
    let context = management_context(request)?;
    let removed = remove_skill(&context, SkillRemoveRequest { name })
        .await
        .map_err(capability_error)?;

    Ok(json!({
        "removed": true,
        "name": removed.name,
    }))
}

fn management_context(
    request: &FirstPartyCapabilityRequest,
) -> Result<SkillManagementContext, FirstPartyCapabilityError> {
    let Some(mounts) = request.mounts.as_ref() else {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    };
    Ok(SkillManagementContext::new(
        Arc::clone(&request.services.filesystem),
        mounts.clone(),
        request.scope.clone(),
    ))
}

fn skill_summary_json(skill: &SkillSummary) -> Value {
    json!({
        "name": skill.name,
        "version": skill.version,
        "description": skill.description,
        "source": skill.source.as_str(),
        "keywords": skill.keywords,
        "tags": skill.tags,
        "requires_skills": skill.requires_skills,
    })
}

fn input_error() -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
}

fn capability_error(error: SkillManagementError) -> FirstPartyCapabilityError {
    let kind = match error.kind() {
        SkillManagementErrorKind::InvalidInput => RuntimeDispatchErrorKind::InputEncode,
        SkillManagementErrorKind::FilesystemDenied => RuntimeDispatchErrorKind::FilesystemDenied,
        SkillManagementErrorKind::NotFound
        | SkillManagementErrorKind::Conflict
        | SkillManagementErrorKind::InvalidSkill => RuntimeDispatchErrorKind::Guest,
        SkillManagementErrorKind::Resource => RuntimeDispatchErrorKind::Resource,
    };
    FirstPartyCapabilityError::new(kind)
}
