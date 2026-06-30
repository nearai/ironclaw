use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_first_party_extensions::skills::{
    SkillManagementCapabilityError, SkillManagementCapabilityKind,
    SkillManagementCapabilityRequest, dispatch,
};
use ironclaw_host_api::{
    CapabilityId, EffectKind, HostApiError, PermissionMode, ResourceUsage, RuntimeDispatchErrorKind,
};
use ironclaw_skills::InstalledSkillMetadataSource;
use serde_json::{Map, Value, json};

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

use super::{
    first_party_capability_manifest, resource_profile, skill_url_install::fetch_skill_url_payload,
};

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
            "Install a SKILL.md document, HTTPS SKILL.md URL, ZIP bundle, or GitHub skill repository/tree into the current user's Reborn skill root",
            vec![
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::DeleteFilesystem,
                EffectKind::Network,
            ],
            PermissionMode::Ask,
            resource_profile(),
        )?,
        first_party_capability_manifest(
            SKILL_REMOVE_CAPABILITY_ID,
            "Remove a user-installed Reborn filesystem skill",
            vec![
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::DeleteFilesystem,
            ],
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
        let started = Instant::now();
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
        let mut usage = ResourceUsage::default();
        let input = if kind == SkillManagementCapabilityKind::Install {
            skill_install_input(&request, &mut usage)
                .await
                .map_err(|error| error.with_usage(usage_with_elapsed(&usage, started)))?
        } else {
            request.input.clone()
        };
        let skill_request = SkillManagementCapabilityRequest::new(
            kind,
            &request.scope,
            request.mounts.as_ref(),
            Arc::clone(&request.services.filesystem),
            &input,
        );
        let output = dispatch(&skill_request).await.map_err(|error| {
            skill_management_error(error).with_usage(usage_with_elapsed(&usage, started))
        })?;
        Ok(FirstPartyCapabilityResult::new(
            output,
            usage_with_elapsed(&usage, started),
        ))
    }
}

async fn skill_install_input(
    request: &FirstPartyCapabilityRequest,
    usage: &mut ResourceUsage,
) -> Result<Value, FirstPartyCapabilityError> {
    let Some(object) = request.input.as_object() else {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    };
    let has_content = object.contains_key("content");
    let url = object
        .get("url")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    match (has_content, url) {
        (true, None)
            if !object.contains_key("files")
                && !object.contains_key("source")
                && !object.contains_key("source_url") =>
        {
            Ok(request.input.clone())
        }
        (false, Some(url)) => {
            let payload = fetch_skill_url_payload(request, url, usage).await?;
            let mut rewritten = Map::new();
            if let Some(name) = object.get("name").cloned() {
                rewritten.insert("name".to_string(), name);
            }
            rewritten.insert("content".to_string(), Value::String(payload.content));
            rewritten.insert(
                "source".to_string(),
                Value::String(
                    InstalledSkillMetadataSource::InstalledUrl
                        .as_str()
                        .to_string(),
                ),
            );
            rewritten.insert("source_url".to_string(), Value::String(url.to_string()));
            if !payload.files.is_empty() {
                rewritten.insert(
                    "files".to_string(),
                    Value::Array(
                        payload
                            .files
                            .into_iter()
                            .map(|file| {
                                json!({
                                    "path": file.path.display().to_string(),
                                    "bytes_base64": BASE64_STANDARD.encode(file.contents),
                                })
                            })
                            .collect(),
                    ),
                );
            }
            Ok(Value::Object(rewritten))
        }
        _ => Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        )),
    }
}

fn usage_with_elapsed(usage: &ResourceUsage, started: Instant) -> ResourceUsage {
    let mut usage = usage.clone();
    usage.wall_clock_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    usage
}

fn skill_management_error(error: SkillManagementCapabilityError) -> FirstPartyCapabilityError {
    tracing::debug!(
        runtime_dispatch_error_kind = %error.kind(),
        "skill management error mapped to first-party capability error"
    );
    FirstPartyCapabilityError::new(error.kind())
}
