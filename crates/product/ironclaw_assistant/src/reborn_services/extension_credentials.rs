use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountProjection,
    CredentialAccountStatus, ProviderScope,
};
use ironclaw_host_api::{
    ids::{ExtensionId, InvocationId},
    resource::ResourceScope,
};
use ironclaw_product_contracts::surface::{
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
};
use uuid::Uuid;

use crate::{
    LifecycleExtensionCredentialRequirement, LifecycleExtensionCredentialSetup, LifecyclePackageRef,
};

use super::{ExtensionCredentialSetupService, ExtensionCredentialStatusRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExtensionCredentialReadiness {
    NotRequired,
    Configured,
    MissingRequired,
    Unknown,
}

pub(super) enum RequirementCredentialReadiness {
    Configured,
    Missing,
    Unknown,
}

pub(super) fn credential_scope(
    caller: &ProductSurfaceCaller,
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

/// Presence readiness plus the recipe-scope delta for the caller's
/// configured accounts: the union, across required OAuth requirements, of
/// ceiling scopes the granted account does not hold. Empty when every grant
/// covers its recipe (or nothing is configured).
pub(super) async fn presence_readiness_and_missing_scopes(
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirements: &[LifecycleExtensionCredentialRequirement],
) -> Result<(ExtensionCredentialReadiness, Vec<String>), ProductSurfaceError> {
    let requirements = unique_requirements(requirements);
    if requirements.is_empty() {
        return Ok((ExtensionCredentialReadiness::NotRequired, Vec::new()));
    }
    let Some(service) = extension_credentials else {
        return Ok((ExtensionCredentialReadiness::Unknown, Vec::new()));
    };
    let mut saw_unknown = false;
    let mut missing_scopes: Vec<String> = Vec::new();
    for requirement in requirements
        .iter()
        .filter(|requirement| requirement.required)
    {
        let request = credential_presence_request(scope.clone(), extension_id, requirement)?;
        match service.credential_status(request).await {
            Ok(Some(account)) => match requirement_readiness_for_status(account.status) {
                RequirementCredentialReadiness::Configured => {
                    if let LifecycleExtensionCredentialSetup::OAuth { scopes } = &requirement.setup
                    {
                        let granted: std::collections::BTreeSet<&str> =
                            account.scopes.iter().map(|scope| scope.as_str()).collect();
                        for ceiling in scopes {
                            if !granted.contains(ceiling.as_str())
                                && !missing_scopes.iter().any(|seen| seen == ceiling)
                            {
                                missing_scopes.push(ceiling.clone());
                            }
                        }
                    }
                }
                RequirementCredentialReadiness::Missing => {
                    return Ok((ExtensionCredentialReadiness::MissingRequired, Vec::new()));
                }
                RequirementCredentialReadiness::Unknown => saw_unknown = true,
            },
            Ok(None) => {
                return Ok((ExtensionCredentialReadiness::MissingRequired, Vec::new()));
            }
            Err(error) if is_retryable_status_failure(&error) => {
                warn_retryable_status_failure(
                    extension_id,
                    requirement,
                    &error,
                    "readiness_projection",
                );
                saw_unknown = true;
            }
            Err(error) => return Err(error),
        }
    }
    if saw_unknown {
        return Ok((ExtensionCredentialReadiness::Unknown, missing_scopes));
    }
    Ok((ExtensionCredentialReadiness::Configured, missing_scopes))
}

pub(super) async fn credential_status_for_requirement(
    service: &dyn ExtensionCredentialSetupService,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirement: &LifecycleExtensionCredentialRequirement,
) -> Result<
    (
        Option<CredentialAccountProjection>,
        RequirementCredentialReadiness,
    ),
    ProductSurfaceError,
> {
    let request = credential_status_request(scope, extension_id, requirement)?;
    match service.credential_status(request).await {
        Ok(Some(account)) => {
            let readiness = requirement_readiness_for_status(account.status);
            Ok((Some(account), readiness))
        }
        Ok(None) => Ok((None, RequirementCredentialReadiness::Missing)),
        Err(error) if is_retryable_status_failure(&error) => {
            warn_retryable_status_failure(extension_id, requirement, &error, "setup_projection");
            // A retryable outage is not evidence of a missing credential:
            // report `Unknown` so the caller preserves lifecycle state instead
            // of projecting a spurious setup prompt.
            Ok((None, RequirementCredentialReadiness::Unknown))
        }
        Err(error) => Err(error),
    }
}

/// A durable credential account exists, but only `Configured` means the
/// caller can actually use it. Expired / revoked / refresh-failed / missing /
/// inactive / pending-setup accounts are rows, not readiness -- treating the
/// row's existence as readiness makes a broken integration look callable and
/// hides the reconnect affordance.
pub(super) fn requirement_readiness_for_status(
    status: CredentialAccountStatus,
) -> RequirementCredentialReadiness {
    match status {
        CredentialAccountStatus::Configured => RequirementCredentialReadiness::Configured,
        CredentialAccountStatus::Inactive
        | CredentialAccountStatus::Missing
        | CredentialAccountStatus::Expired
        | CredentialAccountStatus::RefreshFailed
        | CredentialAccountStatus::Revoked
        | CredentialAccountStatus::PendingSetup => RequirementCredentialReadiness::Missing,
    }
}

pub(super) async fn credential_status_for_requirement_strict(
    service: &dyn ExtensionCredentialSetupService,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirement: &LifecycleExtensionCredentialRequirement,
) -> Result<Option<CredentialAccountProjection>, ProductSurfaceError> {
    let request = credential_status_request(scope, extension_id, requirement)?;
    service.credential_status(request).await
}

/// The presence question: does the caller hold an account for this
/// requirement at all? Deliberately built WITHOUT the recipe-ceiling scopes —
/// a grant that predates a recipe widening still answers "yes". The widened
/// ceiling surfaces as the wire's `missing_recipe_scopes` delta instead of
/// flipping the whole card back to `setup_needed` (#7660); enforcement for
/// newly-scoped tools stays on the runtime dispatch gate, which checks the
/// per-tool scopes on every call.
fn credential_presence_request(
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirement: &LifecycleExtensionCredentialRequirement,
) -> Result<ExtensionCredentialStatusRequest, ProductSurfaceError> {
    Ok(ExtensionCredentialStatusRequest {
        scope,
        provider: provider_for_requirement(requirement)?,
        setup: requirement.setup.clone(),
        provider_scopes: Vec::new(),
        requester_extension: extension_id.clone(),
    })
}

fn credential_status_request(
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirement: &LifecycleExtensionCredentialRequirement,
) -> Result<ExtensionCredentialStatusRequest, ProductSurfaceError> {
    Ok(ExtensionCredentialStatusRequest {
        scope,
        provider: provider_for_requirement(requirement)?,
        setup: requirement.setup.clone(),
        provider_scopes: provider_scopes_for_requirement(requirement)?,
        requester_extension: extension_id.clone(),
    })
}

pub(super) fn provider_for_requirement(
    requirement: &LifecycleExtensionCredentialRequirement,
) -> Result<AuthProviderId, ProductSurfaceError> {
    AuthProviderId::new(requirement.provider.as_str())
        .map_err(|_| ProductSurfaceError::internal_invariant())
}

fn provider_scopes_for_requirement(
    requirement: &LifecycleExtensionCredentialRequirement,
) -> Result<Vec<ProviderScope>, ProductSurfaceError> {
    let LifecycleExtensionCredentialSetup::OAuth { scopes } = &requirement.setup else {
        return Ok(Vec::new());
    };
    scopes
        .iter()
        .map(|scope| {
            ProviderScope::new(scope.clone()).map_err(|_| ProductSurfaceError::internal_invariant())
        })
        .collect()
}

fn is_retryable_status_failure(error: &ProductSurfaceError) -> bool {
    error.retryable
        && (error.code == ProductSurfaceErrorCode::Unavailable
            || error.kind == ProductSurfaceErrorKind::ServiceUnavailable)
}

fn warn_retryable_status_failure(
    extension_id: &ExtensionId,
    requirement: &LifecycleExtensionCredentialRequirement,
    error: &ProductSurfaceError,
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
