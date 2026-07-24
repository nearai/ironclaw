use std::collections::{BTreeMap, HashMap};

use ironclaw_auth::{AuthProductScope, CredentialAccountStatus, CredentialAccountUpdateBinding};
use ironclaw_host_api::{ExtensionId, ProductSurfaceError, ProductSurfaceValidationCode};
use secrecy::SecretString;
use serde::Deserialize;

use crate::{
    LifecycleExtensionCredentialRequirement, LifecycleExtensionCredentialSetup,
    LifecycleProductPayload, LifecycleProductResponse, ProductSetupExtensionRequest,
    RebornExtensionCredentialSetup, RebornExtensionSetupSecret,
};

use super::{
    ExtensionCredentialSetupService, ExtensionCredentialSubmitRequest,
    extension_credentials::{
        ExtensionCredentialReadiness, RequirementCredentialReadiness,
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
) -> Result<
    (
        Vec<RebornExtensionSetupSecret>,
        ExtensionCredentialReadiness,
    ),
    ProductSurfaceError,
> {
    let mut secrets = Vec::with_capacity(requirements.len());
    let mut missing_required = false;
    let mut unknown_required = false;
    for requirement in requirements {
        let (account, readiness) = match extension_credentials {
            Some(service) => {
                credential_status_for_requirement(service, scope.clone(), extension_id, requirement)
                    .await?
            }
            None => (None, RequirementCredentialReadiness::Unknown),
        };
        if requirement.required {
            match readiness {
                RequirementCredentialReadiness::Configured => {}
                RequirementCredentialReadiness::Missing => missing_required = true,
                RequirementCredentialReadiness::Unknown => unknown_required = true,
            }
        }
        secrets.push(RebornExtensionSetupSecret {
            name: requirement.name.clone(),
            provider: requirement.provider.clone(),
            prompt: credential_prompt(requirement),
            optional: !requirement.required,
            provided: account
                .as_ref()
                .is_some_and(|account| account.status == CredentialAccountStatus::Configured),
            setup: setup_projection(&scope, extension_id, requirement),
            credential_ref: account.map(|account| account.id.to_string()),
        });
    }
    secrets.sort_by_key(|secret| !secret.provided);
    let readiness = if requirements.is_empty() {
        ExtensionCredentialReadiness::NotRequired
    } else if missing_required {
        ExtensionCredentialReadiness::MissingRequired
    } else if unknown_required {
        ExtensionCredentialReadiness::Unknown
    } else {
        ExtensionCredentialReadiness::Configured
    };
    Ok((secrets, readiness))
}

/// Parse the setup submit payload (`secrets` + `fields` maps). Shared by the
/// credential-submission path below and the channel-config routing in
/// `lifecycle_setup`, so the wire shape is decoded exactly once.
pub(super) fn parse_submit_payload(
    request: ProductSetupExtensionRequest,
) -> Result<SetupSubmitPayload, ProductSurfaceError> {
    let payload = request
        .payload
        .ok_or_else(|| validation_error("payload", ProductSurfaceValidationCode::MissingField))?;
    serde_json::from_value::<SetupSubmitPayload>(payload)
        .map_err(|_| validation_error("payload", ProductSurfaceValidationCode::InvalidValue))
}

pub(super) async fn submit_manual_tokens(
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirements: &[LifecycleExtensionCredentialRequirement],
    secrets: BTreeMap<String, String>,
) -> Result<(), ProductSurfaceError> {
    let service =
        extension_credentials.ok_or_else(|| ProductSurfaceError::service_unavailable(true))?;
    let by_name = requirements
        .iter()
        .map(|requirement| (requirement.name.as_str(), requirement))
        .collect::<HashMap<_, _>>();

    for submitted_name in secrets.keys() {
        let Some(requirement) = by_name.get(submitted_name.as_str()) else {
            return Err(validation_error(
                "secrets",
                ProductSurfaceValidationCode::InvalidValue,
            ));
        };
        if !matches!(
            requirement.setup,
            LifecycleExtensionCredentialSetup::ManualToken
        ) {
            return Err(validation_error(
                "secrets",
                ProductSurfaceValidationCode::InvalidValue,
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
            secrets.get(&requirement.name),
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
) -> Result<(), ProductSurfaceError> {
    let provider = provider_for_requirement(requirement)?;
    let existing =
        credential_status_for_requirement_strict(service, scope.clone(), extension_id, requirement)
            .await?;
    let configured = existing
        .as_ref()
        .is_some_and(|account| account.status == CredentialAccountStatus::Configured);
    let Some(raw_secret) = raw_secret else {
        if requirement.required && !configured {
            return Err(validation_error(
                "secrets",
                ProductSurfaceValidationCode::MissingField,
            ));
        }
        return Ok(());
    };
    let trimmed = raw_secret.trim();
    if trimmed.is_empty() {
        if requirement.required && !configured {
            return Err(validation_error(
                "secrets",
                ProductSurfaceValidationCode::Blank,
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
        LifecycleExtensionCredentialSetup::Pairing => RebornExtensionCredentialSetup::Pairing,
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

/// The decoded setup submit payload: credential/channel secret values by
/// name plus non-secret channel-config field values by handle.
#[derive(Debug, Default, Deserialize)]
pub(super) struct SetupSubmitPayload {
    #[serde(default)]
    pub(super) secrets: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) fields: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use ironclaw_auth::{AuthSurface, CredentialAccountId, CredentialAccountProjection};
    use ironclaw_host_api::{
        InvocationId, ProductSurfaceErrorCode, ProductSurfaceErrorKind, ResourceScope, UserId,
    };

    use crate::ExtensionCredentialStatusRequest;

    use super::*;

    #[tokio::test]
    async fn project_preserves_retryable_unavailable_credential_status_as_unknown() {
        let service = FailingCredentialSetupService {
            error: ProductSurfaceError {
                code: ProductSurfaceErrorCode::Unavailable,
                kind: ProductSurfaceErrorKind::ServiceUnavailable,
                status_code: 503,
                retryable: true,
                field: None,
                validation_code: None,
            },
        };
        let extension_id = ExtensionId::new("google-docs").expect("extension id");

        let (secrets, readiness) = project(
            Some(&service),
            test_scope(),
            &extension_id,
            &[oauth_requirement()],
        )
        .await
        .expect("setup projection should render when credential status is unavailable");

        assert_eq!(readiness, ExtensionCredentialReadiness::Unknown);
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
            error: ProductSurfaceError {
                code: ProductSurfaceErrorCode::InvalidRequest,
                kind: ProductSurfaceErrorKind::Validation,
                status_code: 400,
                retryable: false,
                field: Some("provider".to_string()),
                validation_code: Some(ProductSurfaceValidationCode::InvalidValue),
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

        assert_eq!(error.code, ProductSurfaceErrorCode::InvalidRequest);
        assert_eq!(error.kind, ProductSurfaceErrorKind::Validation);
        assert_eq!(error.field.as_deref(), Some("provider"));
    }

    #[tokio::test]
    async fn project_preserves_non_retryable_unavailable_credential_errors() {
        let service = FailingCredentialSetupService {
            error: ProductSurfaceError {
                code: ProductSurfaceErrorCode::Unavailable,
                kind: ProductSurfaceErrorKind::ServiceUnavailable,
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

        assert_eq!(error.code, ProductSurfaceErrorCode::Unavailable);
        assert_eq!(error.kind, ProductSurfaceErrorKind::ServiceUnavailable);
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn submit_manual_tokens_preserves_retryable_status_unavailable() {
        let service = FailingCredentialSetupService {
            error: ProductSurfaceError {
                code: ProductSurfaceErrorCode::Unavailable,
                kind: ProductSurfaceErrorKind::ServiceUnavailable,
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
            parse_submit_payload(ProductSetupExtensionRequest {
                client_action_id: None,
                action: Some("submit".to_string()),
                payload: Some(serde_json::json!({ "secrets": {} })),
            })
            .expect("payload parses")
            .secrets,
        )
        .await
        .expect_err("submit should surface credential status outages");

        assert_eq!(error.code, ProductSurfaceErrorCode::Unavailable);
        assert_eq!(error.kind, ProductSurfaceErrorKind::ServiceUnavailable);
        assert!(error.retryable);
        assert!(error.field.is_none());
    }

    struct FailingCredentialSetupService {
        error: ProductSurfaceError,
    }

    #[async_trait]
    impl ExtensionCredentialSetupService for FailingCredentialSetupService {
        async fn credential_status(
            &self,
            _request: ExtensionCredentialStatusRequest,
        ) -> Result<Option<CredentialAccountProjection>, ProductSurfaceError> {
            Err(self.error.clone())
        }

        async fn submit_manual_token(
            &self,
            _request: ExtensionCredentialSubmitRequest,
        ) -> Result<CredentialAccountId, ProductSurfaceError> {
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
