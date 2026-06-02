use std::collections::{BTreeMap, HashMap};

use ironclaw_auth::{AuthProductScope, AuthProviderId, CredentialAccountUpdateBinding};
use ironclaw_host_api::ExtensionId;
use secrecy::SecretString;
use serde::Deserialize;

use crate::{
    LifecycleExtensionCredentialRequirement, LifecycleExtensionCredentialSetup,
    LifecycleProductPayload, LifecycleProductResponse, RebornExtensionCredentialSetup,
    RebornExtensionSetupSecret, RebornServicesError, WebUiInboundValidationCode,
    WebUiInboundValidationError, WebUiSetupExtensionRequest,
};

use super::{
    ExtensionCredentialSetupService, ExtensionCredentialStatusRequest,
    ExtensionCredentialSubmitRequest,
};

pub(super) fn requirements(
    lifecycle: &LifecycleProductResponse,
) -> Vec<LifecycleExtensionCredentialRequirement> {
    let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = &lifecycle.payload else {
        return Vec::new();
    };
    let mut requirements = Vec::new();
    for extension in extensions {
        for requirement in &extension.summary.credential_requirements {
            if requirements
                .iter()
                .any(|seen: &LifecycleExtensionCredentialRequirement| seen.name == requirement.name)
            {
                continue;
            }
            requirements.push(requirement.clone());
        }
    }
    requirements
}

pub(super) async fn project(
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirements: &[LifecycleExtensionCredentialRequirement],
) -> Result<Vec<RebornExtensionSetupSecret>, RebornServicesError> {
    let mut secrets = Vec::with_capacity(requirements.len());
    for requirement in requirements {
        let provider = AuthProviderId::new(requirement.provider.as_str())
            .map_err(|_| RebornServicesError::internal_invariant())?;
        let account = match extension_credentials {
            Some(service) => {
                service
                    .credential_status(ExtensionCredentialStatusRequest {
                        scope: scope.clone(),
                        provider,
                        requester_extension: extension_id.clone(),
                    })
                    .await?
            }
            None => None,
        };
        secrets.push(RebornExtensionSetupSecret {
            name: requirement.name.clone(),
            provider: requirement.provider.clone(),
            prompt: credential_prompt(requirement),
            optional: !requirement.required,
            provided: account.is_some(),
            setup: setup_projection(&scope, extension_id, requirement),
            credential_ref: account.map(|account| account.id.to_string()),
        });
    }
    Ok(secrets)
}

pub(super) async fn submit_manual_tokens(
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirements: &[LifecycleExtensionCredentialRequirement],
    request: WebUiSetupExtensionRequest,
) -> Result<(), RebornServicesError> {
    let service =
        extension_credentials.ok_or_else(|| RebornServicesError::service_unavailable(true))?;
    let payload = request
        .payload
        .ok_or_else(|| validation_error("payload", WebUiInboundValidationCode::MissingField))?;
    let submit = serde_json::from_value::<SetupSubmitPayload>(payload)
        .map_err(|_| validation_error("payload", WebUiInboundValidationCode::InvalidValue))?;
    let by_name = requirements
        .iter()
        .map(|requirement| (requirement.name.as_str(), requirement))
        .collect::<HashMap<_, _>>();

    for submitted_name in submit.secrets.keys() {
        let Some(requirement) = by_name.get(submitted_name.as_str()) else {
            return Err(validation_error(
                "secrets",
                WebUiInboundValidationCode::InvalidValue,
            ));
        };
        if !matches!(
            requirement.setup,
            LifecycleExtensionCredentialSetup::ManualToken
        ) {
            return Err(validation_error(
                "secrets",
                WebUiInboundValidationCode::InvalidValue,
            ));
        }
    }

    for requirement in requirements.iter().filter(|requirement| {
        matches!(
            requirement.setup,
            LifecycleExtensionCredentialSetup::ManualToken
        )
    }) {
        submit_manual_token_requirement(
            service,
            scope.clone(),
            extension_id,
            requirement,
            submit.secrets.get(&requirement.name),
        )
        .await?;
    }
    Ok(())
}

async fn submit_manual_token_requirement(
    service: &dyn ExtensionCredentialSetupService,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirement: &LifecycleExtensionCredentialRequirement,
    raw_secret: Option<&String>,
) -> Result<(), RebornServicesError> {
    let provider = AuthProviderId::new(requirement.provider.as_str())
        .map_err(|_| RebornServicesError::internal_invariant())?;
    let existing = service
        .credential_status(ExtensionCredentialStatusRequest {
            scope: scope.clone(),
            provider: provider.clone(),
            requester_extension: extension_id.clone(),
        })
        .await?;
    let Some(raw_secret) = raw_secret else {
        if requirement.required && existing.is_none() {
            return Err(validation_error(
                "secrets",
                WebUiInboundValidationCode::MissingField,
            ));
        }
        return Ok(());
    };
    let trimmed = raw_secret.trim();
    if trimmed.is_empty() {
        if requirement.required && existing.is_none() {
            return Err(validation_error(
                "secrets",
                WebUiInboundValidationCode::Blank,
            ));
        }
        return Ok(());
    }
    service
        .submit_manual_token(ExtensionCredentialSubmitRequest {
            scope,
            provider,
            label: credential_label(extension_id, requirement),
            requester_extension: extension_id.clone(),
            existing_account: existing
                .as_ref()
                .map(CredentialAccountUpdateBinding::from_projection),
            secret: SecretString::from(trimmed.to_string()),
        })
        .await?;
    Ok(())
}

fn setup_projection(
    scope: &AuthProductScope,
    extension_id: &ExtensionId,
    requirement: &LifecycleExtensionCredentialRequirement,
) -> RebornExtensionCredentialSetup {
    match &requirement.setup {
        LifecycleExtensionCredentialSetup::ManualToken => {
            RebornExtensionCredentialSetup::ManualToken
        }
        LifecycleExtensionCredentialSetup::OAuth { scopes } => {
            RebornExtensionCredentialSetup::OAuth {
                account_label: credential_label(extension_id, requirement),
                scopes: scopes.clone(),
                invocation_id: scope.resource.invocation_id.to_string(),
            }
        }
    }
}

fn credential_prompt(requirement: &LifecycleExtensionCredentialRequirement) -> String {
    format!("{} credential", requirement.provider)
}

fn credential_label(
    extension_id: &ExtensionId,
    requirement: &LifecycleExtensionCredentialRequirement,
) -> String {
    format!("{} {}", extension_id.as_str(), requirement.provider)
}

fn validation_error(field: &'static str, code: WebUiInboundValidationCode) -> RebornServicesError {
    RebornServicesError::from(WebUiInboundValidationError::new(field, code))
}

#[derive(Debug, Default, Deserialize)]
struct SetupSubmitPayload {
    #[serde(default)]
    secrets: BTreeMap<String, String>,
}
