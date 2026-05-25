use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_first_party_extensions::skills::{
    SkillManagementCapabilityError, SkillManagementCapabilityKind,
    SkillManagementCapabilityRequest, dispatch,
};
use ironclaw_host_api::{
    CapabilityId, EffectKind, HostApiError, NetworkMethod, NetworkPolicy, PermissionMode,
    ResourceUsage, RuntimeDispatchErrorKind, RuntimeHttpEgressError, RuntimeHttpEgressReasonCode,
    RuntimeHttpEgressRequest, RuntimeKind,
};
use serde_json::Value;

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

use super::{first_party_capability_manifest, resource_profile};

pub const SKILL_LIST_CAPABILITY_ID: &str = "builtin.skill_list";
pub const SKILL_INSTALL_CAPABILITY_ID: &str = "builtin.skill_install";
pub const SKILL_REMOVE_CAPABILITY_ID: &str = "builtin.skill_remove";

const SKILL_URL_RESPONSE_BODY_LIMIT_BYTES: u64 = 64 * 1024;
const SKILL_URL_FETCH_TIMEOUT_MS: u32 = 10_000;

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
            "Install a SKILL.md document or HTTPS URL into the current user's Reborn skill root",
            vec![
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
            ],
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
        let mut usage = ResourceUsage::default();
        let input = if kind == SkillManagementCapabilityKind::Install {
            skill_install_input(&request, &mut usage).await?
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
        let output = dispatch(&skill_request)
            .await
            .map_err(|error| skill_management_error(error).with_usage(usage.clone()))?;
        Ok(FirstPartyCapabilityResult::new(output, usage))
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
    let has_content = object.get("content").and_then(Value::as_str).is_some();
    let url = object
        .get("url")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    match (has_content, url) {
        (true, None) => Ok(request.input.clone()),
        (false, Some(url)) => {
            let content = fetch_skill_url(request, url, usage).await?;
            let mut rewritten = object.clone();
            rewritten.remove("url");
            rewritten.insert("content".to_string(), Value::String(content));
            Ok(Value::Object(rewritten))
        }
        _ => Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        )),
    }
}

async fn fetch_skill_url(
    request: &FirstPartyCapabilityRequest,
    url: &str,
    usage: &mut ResourceUsage,
) -> Result<String, FirstPartyCapabilityError> {
    validate_skill_url(url)?;
    let egress = request
        .services
        .runtime_http_egress
        .as_ref()
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::NetworkDenied))?
        .clone();
    let http_request = RuntimeHttpEgressRequest {
        runtime: RuntimeKind::FirstParty,
        scope: request.scope.clone(),
        capability_id: request.capability_id.clone(),
        method: NetworkMethod::Get,
        url: url.to_string(),
        headers: Vec::new(),
        body: Vec::new(),
        network_policy: NetworkPolicy::default(),
        credential_injections: Vec::new(),
        response_body_limit: Some(SKILL_URL_RESPONSE_BODY_LIMIT_BYTES),
        timeout_ms: Some(SKILL_URL_FETCH_TIMEOUT_MS),
    };
    let response = tokio::task::spawn_blocking(move || egress.execute(http_request))
        .await
        .map_err(|error| {
            if error.is_panic() {
                tracing::error!("skill URL fetch egress worker panicked");
            }
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
        })?
        .map_err(|error| skill_url_fetch_error(error, usage))?;
    usage.network_egress_bytes = usage
        .network_egress_bytes
        .saturating_add(response.request_bytes);
    if !(200..300).contains(&response.status) {
        return Err(
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                .with_usage(usage.clone()),
        );
    }
    String::from_utf8(response.body).map_err(|_| {
        FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
            .with_usage(usage.clone())
    })
}

fn validate_skill_url(url: &str) -> Result<(), FirstPartyCapabilityError> {
    let parsed = url::Url::parse(url)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    Ok(())
}

fn skill_url_fetch_error(
    error: RuntimeHttpEgressError,
    usage: &mut ResourceUsage,
) -> FirstPartyCapabilityError {
    usage.network_egress_bytes = usage
        .network_egress_bytes
        .saturating_add(error.request_bytes());
    let kind = match error.reason_code() {
        RuntimeHttpEgressReasonCode::CredentialUnavailable => RuntimeDispatchErrorKind::Client,
        RuntimeHttpEgressReasonCode::RequestDenied => RuntimeDispatchErrorKind::InputEncode,
        RuntimeHttpEgressReasonCode::PolicyDenied => RuntimeDispatchErrorKind::PolicyDenied,
        RuntimeHttpEgressReasonCode::NetworkError => RuntimeDispatchErrorKind::NetworkDenied,
        RuntimeHttpEgressReasonCode::ResponseError => RuntimeDispatchErrorKind::OperationFailed,
        RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded => {
            RuntimeDispatchErrorKind::OutputTooLarge
        }
    };
    FirstPartyCapabilityError::new(kind).with_usage(usage.clone())
}

fn skill_management_error(error: SkillManagementCapabilityError) -> FirstPartyCapabilityError {
    tracing::debug!(
        runtime_dispatch_error_kind = %error.kind(),
        "skill management error mapped to first-party capability error"
    );
    FirstPartyCapabilityError::new(error.kind())
}
