use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountProjection, ProviderScope,
};
use ironclaw_host_api::{ExtensionId, InvocationId, ResourceScope};
use uuid::Uuid;

use crate::{
    LifecycleExtensionCredentialRequirement, LifecycleExtensionCredentialSetup,
    LifecyclePackageRef, RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind,
    WebUiAuthenticatedCaller,
};

use super::{ExtensionCredentialSetupService, ExtensionCredentialStatusRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExtensionCredentialReadiness {
    NotRequired,
    Configured,
    MissingRequired,
    Unknown,
}

enum RequirementCredentialReadiness {
    Configured,
    Missing,
    Unknown,
}

pub(super) fn credential_scope(
    caller: &WebUiAuthenticatedCaller,
    package_ref: &LifecyclePackageRef,
) -> AuthProductScope {
    let seed = format!(
        "webui-v2-extension-setup:{}:{}:{}:{}:{}",
        caller.tenant_id.as_str(),
        caller.user_id.as_str(),
        caller.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        caller
            .project_id
            .as_ref()
            .map(|id| id.as_str())
            .unwrap_or(""),
        package_ref.id.as_str()
    );
    AuthProductScope::new(
        ResourceScope {
            tenant_id: caller.tenant_id.clone(),
            user_id: caller.user_id.clone(),
            agent_id: caller.agent_id.clone(),
            project_id: caller.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::from_uuid(Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                seed.as_bytes(),
            )),
        },
        AuthSurface::Web,
    )
}

pub(super) fn unique_requirements<'a>(
    requirements: impl IntoIterator<Item = &'a LifecycleExtensionCredentialRequirement>,
) -> Vec<LifecycleExtensionCredentialRequirement> {
    let mut unique = Vec::new();
    for requirement in requirements {
        if unique
            .iter()
            .any(|seen: &LifecycleExtensionCredentialRequirement| seen.name == requirement.name)
        {
            continue;
        }
        unique.push(requirement.clone());
    }
    unique
}

pub(super) async fn readiness_for_requirements(
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirements: &[LifecycleExtensionCredentialRequirement],
) -> Result<ExtensionCredentialReadiness, RebornServicesError> {
    let requirements = unique_requirements(requirements);
    if requirements.is_empty() {
        return Ok(ExtensionCredentialReadiness::NotRequired);
    }
    let Some(service) = extension_credentials else {
        return Ok(ExtensionCredentialReadiness::Unknown);
    };
    let mut saw_unknown = false;
    for requirement in requirements
        .iter()
        .filter(|requirement| requirement.required)
    {
        match credential_readiness_for_requirement(
            service,
            scope.clone(),
            extension_id,
            requirement,
        )
        .await?
        {
            RequirementCredentialReadiness::Configured => {}
            RequirementCredentialReadiness::Missing => {
                return Ok(ExtensionCredentialReadiness::MissingRequired);
            }
            RequirementCredentialReadiness::Unknown => {
                saw_unknown = true;
            }
        }
    }
    if saw_unknown {
        return Ok(ExtensionCredentialReadiness::Unknown);
    }
    Ok(ExtensionCredentialReadiness::Configured)
}

async fn credential_readiness_for_requirement(
    service: &dyn ExtensionCredentialSetupService,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirement: &LifecycleExtensionCredentialRequirement,
) -> Result<RequirementCredentialReadiness, RebornServicesError> {
    let request = credential_status_request(scope, extension_id, requirement)?;
    match service.credential_status(request).await {
        Ok(Some(_)) => Ok(RequirementCredentialReadiness::Configured),
        Ok(None) => Ok(RequirementCredentialReadiness::Missing),
        Err(error) if is_retryable_status_failure(&error) => {
            warn_retryable_status_failure(
                extension_id,
                requirement,
                &error,
                "readiness_projection",
            );
            Ok(RequirementCredentialReadiness::Unknown)
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn credential_status_for_requirement(
    service: &dyn ExtensionCredentialSetupService,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirement: &LifecycleExtensionCredentialRequirement,
) -> Result<Option<CredentialAccountProjection>, RebornServicesError> {
    let request = credential_status_request(scope, extension_id, requirement)?;
    match service.credential_status(request).await {
        Ok(account) => Ok(account),
        Err(error) if is_retryable_status_failure(&error) => {
            warn_retryable_status_failure(extension_id, requirement, &error, "setup_projection");
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn credential_status_for_requirement_strict(
    service: &dyn ExtensionCredentialSetupService,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirement: &LifecycleExtensionCredentialRequirement,
) -> Result<Option<CredentialAccountProjection>, RebornServicesError> {
    let request = credential_status_request(scope, extension_id, requirement)?;
    service.credential_status(request).await
}

fn credential_status_request(
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirement: &LifecycleExtensionCredentialRequirement,
) -> Result<ExtensionCredentialStatusRequest, RebornServicesError> {
    Ok(ExtensionCredentialStatusRequest {
        scope,
        provider: provider_for_requirement(requirement)?,
        provider_scopes: provider_scopes_for_requirement(requirement)?,
        requester_extension: extension_id.clone(),
    })
}

pub(super) fn provider_for_requirement(
    requirement: &LifecycleExtensionCredentialRequirement,
) -> Result<AuthProviderId, RebornServicesError> {
    AuthProviderId::new(requirement.provider.as_str())
        .map_err(|_| RebornServicesError::internal_invariant())
}

fn provider_scopes_for_requirement(
    requirement: &LifecycleExtensionCredentialRequirement,
) -> Result<Vec<ProviderScope>, RebornServicesError> {
    let LifecycleExtensionCredentialSetup::OAuth { scopes } = &requirement.setup else {
        return Ok(Vec::new());
    };
    scopes
        .iter()
        .map(|scope| {
            ProviderScope::new(scope.clone()).map_err(|_| RebornServicesError::internal_invariant())
        })
        .collect()
}

fn is_retryable_status_failure(error: &RebornServicesError) -> bool {
    error.retryable
        && (error.code == RebornServicesErrorCode::Unavailable
            || error.kind == RebornServicesErrorKind::ServiceUnavailable)
}

fn warn_retryable_status_failure(
    extension_id: &ExtensionId,
    requirement: &LifecycleExtensionCredentialRequirement,
    error: &RebornServicesError,
    usage: &'static str,
) {
    tracing::warn!(
        target: "ironclaw::reborn::extension_credentials",
        extension_id = %extension_id.as_str(),
        provider = %requirement.provider,
        requirement = %requirement.name,
        usage,
        code = ?error.code,
        kind = ?error.kind,
        status_code = error.status_code,
        retryable = error.retryable,
        "credential status unavailable during extension credential projection"
    );
}
