use std::collections::{BTreeMap, HashMap};

use ironclaw_auth::{AuthProductScope, CredentialAccountUpdateBinding};
use ironclaw_host_api::ExtensionId;
use secrecy::SecretString;
use serde::Deserialize;

use crate::{
    LifecycleExtensionCredentialRequirement, LifecycleExtensionCredentialSetup,
    LifecycleProductPayload, LifecycleProductResponse, RebornExtensionCredentialSetup,
    RebornExtensionSetupSecret, RebornServicesError, WebUiInboundValidationCode,
    WebUiSetupExtensionRequest,
};

use super::{
    ExtensionCredentialSetupService, ExtensionCredentialSubmitRequest,
    extension_credentials::{
        credential_status_for_requirement, credential_status_for_requirement_strict,
        provider_for_requirement, unique_requirements,
    },
    lifecycle_setup::validation_error,
};

pub(super) fn requirements(
    lifecycle: &LifecycleProductResponse,
) -> Vec<LifecycleExtensionCredentialRequirement> {
    let Some(LifecycleProductPayload::ExtensionList { extensions, .. }) = &lifecycle.payload else {
        return Vec::new();
    };
    unique_requirements(
        extensions
            .iter()
            .flat_map(|extension| extension.summary.credential_requirements.iter()),
    )
}

pub(super) async fn project(
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirements: &[LifecycleExtensionCredentialRequirement],
) -> Result<Vec<RebornExtensionSetupSecret>, RebornServicesError> {
    let mut secrets = Vec::with_capacity(requirements.len());
    for requirement in requirements {
        let account = match extension_credentials {
            Some(service) => {
                credential_status_for_requirement(service, scope.clone(), extension_id, requirement)
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
    secrets.sort_by_key(|secret| !secret.provided);
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
    let provider = provider_for_requirement(requirement)?;
    let existing =
        credential_status_for_requirement_strict(service, scope.clone(), extension_id, requirement)
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
    let base = format!("{} {}", extension_id.as_str(), requirement.provider);
    if requirement.name.contains("__") {
        format!("{base} {}", requirement.name)
    } else {
        base
    }
}

#[derive(Debug, Default, Deserialize)]
struct SetupSubmitPayload {
    #[serde(default)]
    secrets: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use ironclaw_auth::{AuthSurface, CredentialAccountId, CredentialAccountProjection};
    use ironclaw_host_api::{InvocationId, ResourceScope, UserId};

    use crate::{
        ExtensionCredentialStatusRequest, RebornServicesErrorCode, RebornServicesErrorKind,
    };

    use super::*;

    #[tokio::test]
    async fn project_treats_retryable_unavailable_credential_status_as_unconfigured() {
        let service = FailingCredentialSetupService {
            error: RebornServicesError {
                code: RebornServicesErrorCode::Unavailable,
                kind: RebornServicesErrorKind::ServiceUnavailable,
                status_code: 503,
                retryable: true,
                field: None,
                validation_code: None,
            },
        };
        let extension_id = ExtensionId::new("google-docs").expect("extension id");

        let secrets = project(
            Some(&service),
            test_scope(),
            &extension_id,
            &[oauth_requirement()],
        )
        .await
        .expect("setup projection should render when credential status is unavailable");

        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].name, "google_oauth");
        assert_eq!(secrets[0].provider, "google");
        assert!(!secrets[0].provided);
        assert!(secrets[0].credential_ref.is_none());
        assert!(matches!(
            secrets[0].setup,
            RebornExtensionCredentialSetup::OAuth { .. }
        ));
    }

    #[tokio::test]
    async fn project_preserves_non_status_credential_errors() {
        let service = FailingCredentialSetupService {
            error: RebornServicesError {
                code: RebornServicesErrorCode::InvalidRequest,
                kind: RebornServicesErrorKind::Validation,
                status_code: 400,
                retryable: false,
                field: Some("provider".to_string()),
                validation_code: Some(WebUiInboundValidationCode::InvalidValue),
            },
        };
        let extension_id = ExtensionId::new("google-docs").expect("extension id");

        let error = project(
            Some(&service),
            test_scope(),
            &extension_id,
            &[oauth_requirement()],
        )
        .await
        .expect_err("validation errors should not be hidden by setup projection");

        assert_eq!(error.code, RebornServicesErrorCode::InvalidRequest);
        assert_eq!(error.kind, RebornServicesErrorKind::Validation);
        assert_eq!(error.field.as_deref(), Some("provider"));
    }

    #[tokio::test]
    async fn project_preserves_non_retryable_unavailable_credential_errors() {
        let service = FailingCredentialSetupService {
            error: RebornServicesError {
                code: RebornServicesErrorCode::Unavailable,
                kind: RebornServicesErrorKind::ServiceUnavailable,
                status_code: 503,
                retryable: false,
                field: None,
                validation_code: None,
            },
        };
        let extension_id = ExtensionId::new("google-docs").expect("extension id");

        let error = project(
            Some(&service),
            test_scope(),
            &extension_id,
            &[oauth_requirement()],
        )
        .await
        .expect_err("non-retryable unavailable errors should stay visible");

        assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
        assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn submit_manual_tokens_preserves_retryable_status_unavailable() {
        let service = FailingCredentialSetupService {
            error: RebornServicesError {
                code: RebornServicesErrorCode::Unavailable,
                kind: RebornServicesErrorKind::ServiceUnavailable,
                status_code: 503,
                retryable: true,
                field: None,
                validation_code: None,
            },
        };
        let extension_id = ExtensionId::new("github").expect("extension id");

        let error = submit_manual_tokens(
            Some(&service),
            test_scope(),
            &extension_id,
            &[manual_requirement()],
            WebUiSetupExtensionRequest {
                action: Some("submit".to_string()),
                payload: Some(serde_json::json!({ "secrets": {} })),
            },
        )
        .await
        .expect_err("submit should surface credential status outages");

        assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
        assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
        assert!(error.retryable);
        assert!(error.field.is_none());
    }

    struct FailingCredentialSetupService {
        error: RebornServicesError,
    }

    #[async_trait]
    impl ExtensionCredentialSetupService for FailingCredentialSetupService {
        async fn credential_status(
            &self,
            _request: ExtensionCredentialStatusRequest,
        ) -> Result<Option<CredentialAccountProjection>, RebornServicesError> {
            Err(self.error.clone())
        }

        async fn submit_manual_token(
            &self,
            _request: ExtensionCredentialSubmitRequest,
        ) -> Result<CredentialAccountId, RebornServicesError> {
            Ok(CredentialAccountId::new())
        }
    }

    fn oauth_requirement() -> LifecycleExtensionCredentialRequirement {
        LifecycleExtensionCredentialRequirement {
            name: "google_oauth".to_string(),
            provider: "google".to_string(),
            required: true,
            setup: LifecycleExtensionCredentialSetup::OAuth {
                scopes: vec!["https://www.googleapis.com/auth/documents".to_string()],
            },
        }
    }

    fn manual_requirement() -> LifecycleExtensionCredentialRequirement {
        LifecycleExtensionCredentialRequirement {
            name: "github_runtime_token".to_string(),
            provider: "github".to_string(),
            required: true,
            setup: LifecycleExtensionCredentialSetup::ManualToken,
        }
    }

    fn test_scope() -> AuthProductScope {
        AuthProductScope::new(
            ResourceScope::local_default(
                UserId::new("user-alpha").expect("user id"),
                InvocationId::new(),
            )
            .expect("resource scope"),
            AuthSurface::Web,
        )
    }
}
