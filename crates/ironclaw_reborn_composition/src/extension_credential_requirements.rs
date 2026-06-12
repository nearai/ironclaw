use std::collections::BTreeSet;

use ironclaw_extensions::ExtensionPackage;
use ironclaw_host_api::{
    RuntimeCredentialAccountProviderId, RuntimeCredentialAccountSetup,
    RuntimeCredentialAuthRequirement, RuntimeCredentialRequirementSource,
};
use ironclaw_product_workflow::LifecycleExtensionCredentialSetup;

pub(crate) fn package_runtime_credential_auth_requirements(
    package: &ExtensionPackage,
) -> Vec<RuntimeCredentialAuthRequirement> {
    let mut requirements: Vec<RuntimeCredentialAuthRequirement> = Vec::new();
    for capability in &package.manifest.capabilities {
        for credential in &capability.runtime_credentials {
            if !credential.required {
                continue;
            }
            let Some(requirement) =
                credential.product_auth_requirement_for(package.manifest.id.clone())
            else {
                continue;
            };
            let requirement = RuntimeCredentialAuthRequirement {
                provider_scopes: normalized_provider_scopes(&requirement.provider_scopes),
                setup: normalized_runtime_credential_setup(requirement.setup),
                ..requirement
            };
            if let Some(seen) = requirements
                .iter_mut()
                .find(|seen| can_merge_runtime_credential_auth_requirement(seen, &requirement))
            {
                merge_runtime_credential_auth_requirement(seen, requirement);
                continue;
            }
            requirements.push(requirement);
        }
    }
    requirements
}

pub(crate) fn lifecycle_credential_setup(
    setup: &RuntimeCredentialAccountSetup,
) -> LifecycleExtensionCredentialSetup {
    match setup {
        RuntimeCredentialAccountSetup::ManualToken => {
            LifecycleExtensionCredentialSetup::ManualToken
        }
        RuntimeCredentialAccountSetup::OAuth { scopes } => {
            LifecycleExtensionCredentialSetup::OAuth {
                scopes: normalized_provider_scopes(scopes),
            }
        }
    }
}

pub(crate) fn product_auth_credential_source(
    credential: &ironclaw_host_api::RuntimeCredentialRequirement,
) -> Option<(
    RuntimeCredentialAccountProviderId,
    LifecycleExtensionCredentialSetup,
)> {
    let RuntimeCredentialRequirementSource::ProductAuthAccount { provider, setup } =
        &credential.source
    else {
        return None;
    };
    Some((provider.clone(), lifecycle_credential_setup(setup)))
}

pub(crate) fn can_merge_lifecycle_credential_setup(
    existing: &LifecycleExtensionCredentialSetup,
    candidate: &LifecycleExtensionCredentialSetup,
) -> bool {
    matches!(
        (existing, candidate),
        (
            LifecycleExtensionCredentialSetup::ManualToken,
            LifecycleExtensionCredentialSetup::ManualToken
        ) | (
            LifecycleExtensionCredentialSetup::OAuth { .. },
            LifecycleExtensionCredentialSetup::OAuth { .. }
        )
    )
}

pub(crate) fn merge_lifecycle_credential_setup(
    existing: &mut LifecycleExtensionCredentialSetup,
    candidate: LifecycleExtensionCredentialSetup,
) {
    if let (
        LifecycleExtensionCredentialSetup::OAuth { scopes: existing },
        LifecycleExtensionCredentialSetup::OAuth { scopes: candidate },
    ) = (existing, candidate)
    {
        *existing = merged_provider_scopes(existing.iter().cloned().chain(candidate));
    }
}

fn can_merge_runtime_credential_auth_requirement(
    existing: &RuntimeCredentialAuthRequirement,
    candidate: &RuntimeCredentialAuthRequirement,
) -> bool {
    existing.provider == candidate.provider
        && existing.requester_extension == candidate.requester_extension
        && can_merge_runtime_credential_setup(&existing.setup, &candidate.setup)
}

fn can_merge_runtime_credential_setup(
    existing: &RuntimeCredentialAccountSetup,
    candidate: &RuntimeCredentialAccountSetup,
) -> bool {
    matches!(
        (existing, candidate),
        (
            RuntimeCredentialAccountSetup::ManualToken,
            RuntimeCredentialAccountSetup::ManualToken
        ) | (
            RuntimeCredentialAccountSetup::OAuth { .. },
            RuntimeCredentialAccountSetup::OAuth { .. }
        )
    )
}

fn merge_runtime_credential_auth_requirement(
    existing: &mut RuntimeCredentialAuthRequirement,
    candidate: RuntimeCredentialAuthRequirement,
) {
    existing.provider_scopes = merged_provider_scopes(
        existing
            .provider_scopes
            .iter()
            .cloned()
            .chain(candidate.provider_scopes),
    );
    merge_runtime_credential_setup(&mut existing.setup, candidate.setup);
}

fn merge_runtime_credential_setup(
    existing: &mut RuntimeCredentialAccountSetup,
    candidate: RuntimeCredentialAccountSetup,
) {
    if let (
        RuntimeCredentialAccountSetup::OAuth { scopes: existing },
        RuntimeCredentialAccountSetup::OAuth { scopes: candidate },
    ) = (existing, candidate)
    {
        *existing = merged_provider_scopes(existing.iter().cloned().chain(candidate));
    }
}

fn normalized_runtime_credential_setup(
    setup: RuntimeCredentialAccountSetup,
) -> RuntimeCredentialAccountSetup {
    match setup {
        RuntimeCredentialAccountSetup::OAuth { scopes } => RuntimeCredentialAccountSetup::OAuth {
            scopes: normalized_provider_scopes(&scopes),
        },
        RuntimeCredentialAccountSetup::ManualToken => RuntimeCredentialAccountSetup::ManualToken,
    }
}

fn normalized_provider_scopes(scopes: &[String]) -> Vec<String> {
    merged_provider_scopes(scopes.iter().cloned())
}

fn merged_provider_scopes(scopes: impl IntoIterator<Item = String>) -> Vec<String> {
    scopes
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
