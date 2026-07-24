//! API-visible first-party mutations for skill activation settings.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionPackage,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, EffectKind, HostApiError, OriginGateMatrix,
    PermissionMode, ProductSurfaceCaller, ResourceEstimate, ResourceProfile, ResourceUsage,
    RuntimeDispatchErrorKind,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use ironclaw_product::{RebornSkillActionResponse, SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID};

pub(crate) fn extend_builtin_first_party_package(
    mut package: ExtensionPackage,
) -> Result<ExtensionPackage, ExtensionError> {
    package.manifest.capabilities.push(manifest()?);
    ExtensionPackage::from_manifest(package.manifest, package.root)
}

pub(crate) fn insert_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    auto_activate_learned: Arc<AtomicBool>,
) -> Result<(), HostApiError> {
    registry.insert_handler(
        CapabilityId::new(SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID)?,
        Arc::new(SetSkillAutoActivateLearnedHandler {
            auto_activate_learned,
        }),
    );
    Ok(())
}

fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    Ok(CapabilityManifest {
        id: CapabilityId::new(SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID)?,
        description: "Set the authenticated user's learned-skill auto-activation default."
            .to_string(),
        effects: vec![EffectKind::WriteFilesystem],
        default_permission: PermissionMode::Allow,
        visibility: CapabilityVisibility::Api,
        input_schema_ref: CapabilityProfileSchemaRef::new(
            "schemas/builtin/skill_auto_activate_learned_set.input.v1.json",
        )?,
        output_schema_ref: Some(CapabilityProfileSchemaRef::new(
            "schemas/builtin/skill_auto_activate_learned_set.output.v1.json",
        )?),
        prompt_doc_ref: None,
        required_host_ports: Vec::new(),
        runtime_credentials: Vec::new(),
        network_targets: Vec::new(),
        max_egress_bytes: None,
        resource_profile: Some(ResourceProfile {
            default_estimate: ResourceEstimate::default()
                .set_wall_clock_ms(500)
                .set_output_bytes(1024),
            hard_ceiling: None,
        }),
        origin_gate_matrix: Some(OriginGateMatrix::product_consent_only()),
    })
}

struct SetSkillAutoActivateLearnedHandler {
    auto_activate_learned: Arc<AtomicBool>,
}

#[async_trait]
impl FirstPartyCapabilityHandler for SetSkillAutoActivateLearnedHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        ensure_declared(&request, started)?;
        authenticated_caller(&request, started)?;
        let enabled = parse_enabled(request.input, started)?;
        self.auto_activate_learned.store(enabled, Ordering::Relaxed);
        let response = RebornSkillActionResponse {
            success: true,
            message: format!(
                "Default skill auto-activation {}",
                if enabled { "enabled" } else { "disabled" }
            ),
        };
        let output = serde_json::to_value(response)
            .map_err(|_| dispatch_error(RuntimeDispatchErrorKind::InvalidResult, started))?;
        Ok(FirstPartyCapabilityResult::new(
            output,
            resource_usage(started),
        ))
    }
}

fn authenticated_caller(
    request: &FirstPartyCapabilityRequest,
    started: Instant,
) -> Result<ProductSurfaceCaller, FirstPartyCapabilityError> {
    if request.authenticated_actor_user_id.as_ref() != Some(&request.scope.user_id) {
        return Err(dispatch_error(
            RuntimeDispatchErrorKind::PolicyDenied,
            started,
        ));
    }
    Ok(ProductSurfaceCaller::new(
        request.scope.tenant_id.clone(),
        request.scope.user_id.clone(),
        request.scope.agent_id.clone(),
        request.scope.project_id.clone(),
    ))
}

fn ensure_declared(
    request: &FirstPartyCapabilityRequest,
    started: Instant,
) -> Result<(), FirstPartyCapabilityError> {
    if request.capability_id.as_str() == SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY_ID {
        Ok(())
    } else {
        Err(dispatch_error(
            RuntimeDispatchErrorKind::UndeclaredCapability,
            started,
        ))
    }
}

fn parse_enabled(
    input: serde_json::Value,
    started: Instant,
) -> Result<bool, FirstPartyCapabilityError> {
    let object = input
        .as_object()
        .ok_or_else(|| dispatch_error(RuntimeDispatchErrorKind::InputEncode, started))?;
    let enabled = object
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| dispatch_error(RuntimeDispatchErrorKind::InputEncode, started))?;
    if object.len() == 1 {
        Ok(enabled)
    } else {
        Err(dispatch_error(
            RuntimeDispatchErrorKind::InputEncode,
            started,
        ))
    }
}

fn dispatch_error(kind: RuntimeDispatchErrorKind, started: Instant) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::new(kind).with_usage(resource_usage(started))
}

fn resource_usage(started: Instant) -> ResourceUsage {
    ResourceUsage::default()
        .set_wall_clock_ms(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_is_api_only_filesystem_write() {
        let manifest = manifest().expect("manifest");
        assert_eq!(manifest.visibility, CapabilityVisibility::Api);
        assert_eq!(manifest.default_permission, PermissionMode::Allow);
        assert_eq!(manifest.effects, vec![EffectKind::WriteFilesystem]);
    }
}
