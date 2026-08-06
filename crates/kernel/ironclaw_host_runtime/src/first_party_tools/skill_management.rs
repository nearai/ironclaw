//! Registrar-side adapter for the first-party skill-management tools.
//!
//! WS3 moved the executable half — URL/GitHub source fetching, bundle
//! extraction, and install-input normalization — into
//! `ironclaw_extension_support::skills`. What stays here is the host's own
//! concern: the capability manifests the builtin package declares, the
//! registry wiring, and the translation between the host's dispatch types and
//! the executor's neutral request/error pair — the same executor/adapter seam
//! every binary-registered first-party tool already uses.

use std::{sync::Arc, time::Instant};

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use async_trait::async_trait;
use ironclaw_extension_registry::{CapabilityManifest, ExtensionError};
use ironclaw_extension_support::skills::{
    SkillManagementCapabilityError, SkillManagementCapabilityKind,
    SkillManagementCapabilityRequest, SkillUrlFetchContext, dispatch, resolve_install_input,
};
use ironclaw_host_api::{
    capability::{EffectKind, PermissionMode},
    dispatch::RuntimeDispatchErrorKind,
    error::HostApiError,
    ids::CapabilityId,
    resource::ResourceUsage,
};

use super::{first_party_capability_manifest, resource_profile};

pub const SKILL_LIST_CAPABILITY_ID: &str = "builtin.skill_list";
pub const SKILL_INSTALL_CAPABILITY_ID: &str = "builtin.skill_install";
pub const SKILL_UPDATE_CAPABILITY_ID: &str = "builtin.skill_update";
pub const SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID: &str = "builtin.skill_auto_activate_set";
pub const SKILL_REMOVE_CAPABILITY_ID: &str = "builtin.skill_remove";

pub(super) fn manifests() -> Result<Vec<CapabilityManifest>, ExtensionError> {
    Ok(vec![
        first_party_capability_manifest(
            SKILL_LIST_CAPABILITY_ID,
            "List Reborn filesystem skills visible to the current standalone agent",
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
            SKILL_UPDATE_CAPABILITY_ID,
            "Update a user-installed Reborn filesystem skill",
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            PermissionMode::Ask,
            resource_profile(),
        )?,
        first_party_capability_manifest(
            SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID,
            "Enable or disable criteria-based activation for a user-installed Reborn filesystem skill",
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
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
    registry.insert_handler(
        CapabilityId::new(SKILL_UPDATE_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(
        CapabilityId::new(SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID)?,
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
            SKILL_UPDATE_CAPABILITY_ID => SkillManagementCapabilityKind::Update,
            SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID => SkillManagementCapabilityKind::SetAutoActivate,
            SKILL_REMOVE_CAPABILITY_ID => SkillManagementCapabilityKind::Remove,
            _ => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::UndeclaredCapability,
                ));
            }
        };
        let mut usage = ResourceUsage::default();
        let input = if kind == SkillManagementCapabilityKind::Install {
            resolve_install_input(
                &request.input,
                &skill_url_fetch_context(&request),
                &mut usage,
            )
            .await
            // Deliberately NOT `skill_management_error`: the install-input path
            // never logged before this executor moved out of the crate, and a
            // move-only change must not add a log line. The `dispatch` arm below
            // keeps the debug record it already had.
            .map_err(|error| {
                FirstPartyCapabilityError::new(error.kind())
                    .with_usage(usage_with_elapsed(&usage, started))
            })?
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

/// The host-runtime-free slice of an already-authorized dispatch input the
/// skill-URL fetch path needs. Everything else the host holds (mounts,
/// filesystem, secret staging, process ports) stays on this side of the seam.
fn skill_url_fetch_context(request: &FirstPartyCapabilityRequest) -> SkillUrlFetchContext {
    SkillUrlFetchContext {
        capability_id: request.capability_id.clone(),
        scope: request.scope.clone(),
        runtime_http_egress: request.services.runtime_http_egress.clone(),
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
