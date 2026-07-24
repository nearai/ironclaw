use std::sync::Arc;

use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccount, CredentialAccountStatus,
    CredentialAccountUpdateBinding,
};
use ironclaw_host_api::{
    ExtensionId, InvocationId, ProductSurfaceError, ProductSurfaceErrorCode,
    ProductSurfaceErrorKind, ResourceScope,
};
use ironclaw_operator::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig;
pub(crate) use ironclaw_operator::llm_admin::nearai_mcp::{
    NearAiMcpBootstrapOutcome, NearAiMcpEndpoint, durable_product_auth_storage_enabled,
    nearai_mcp_bootstrap_config_from_llm_config, nearai_mcp_endpoint_from_base,
    nearai_mcp_endpoint_from_env,
};
use ironclaw_product::{
    ExtensionCredentialSetupService, ExtensionCredentialSubmitRequest, LifecyclePackageKind,
    LifecyclePackageRef, LifecycleProductPayload, LifecyclePublicState,
};

use crate::extension_host::extension_activation_credentials::RuntimeExtensionActivationCredentialGate;
use crate::extension_host::extension_lifecycle::{
    ExtensionActivationMode, ExtensionManagementPort,
};
use crate::extension_host::webui_extension_credentials::ProductAuthExtensionCredentialSetup;
use crate::{RebornBuildError, RebornProductAuthServices};

pub(crate) async fn bootstrap_nearai_mcp(
    config: Option<NearAiMcpBootstrapConfig>,
    product_auth: &Arc<RebornProductAuthServices>,
    extension_management: &Arc<ExtensionManagementPort>,
    owner_scope: ResourceScope,
) -> Result<NearAiMcpBootstrapOutcome, RebornBuildError> {
    let Some(config) = config else {
        return Ok(NearAiMcpBootstrapOutcome::NotConfigured);
    };
    if !durable_product_auth_storage_enabled() {
        tracing::debug!(
            "NEAR AI MCP credentials are present, but durable product-auth secret storage is not enabled; skipping auto-activation"
        );
        return Ok(NearAiMcpBootstrapOutcome::SkippedUnsupportedStorage);
    }
    config
        .endpoint()
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("NEAR AI MCP auto-enable skipped: invalid NEARAI_BASE_URL: {error}"),
        })?;

    let package_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "nearai").map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("NEAR AI MCP package ref is invalid: {error}"),
            }
        })?;
    let resource_scope = ResourceScope {
        invocation_id: InvocationId::new(),
        ..owner_scope.without_thread_and_mission()
    };
    let credential_gate = RuntimeExtensionActivationCredentialGate::new(
        resource_scope.clone(),
        product_auth.runtime_credential_account_selection_service(),
    );
    let projection = extension_management
        .project(
            package_ref.clone(),
            &owner_scope.user_id,
            Some(&credential_gate),
        )
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("NEAR AI MCP extension projection failed: {error}"),
        })?;
    let phase = projection.phase;
    // `install_scope` is present exactly when the caller has a visible
    // installation; the projected `phase` is a resting state only for an
    // installed package (a not-installed projection carries a neutral phase).
    let installed = matches!(
        projection.payload.as_ref(),
        Some(LifecycleProductPayload::ExtensionList { extensions, .. })
            if extensions.first().and_then(|extension| extension.install_scope).is_some()
    );
    if installed {
        match phase {
            LifecyclePublicState::Active | LifecyclePublicState::SetupNeeded => {}
            other => {
                tracing::debug!(
                    phase = ?other,
                    "NEAR AI MCP credentials are present, but the extension is not in an auto-activatable state"
                );
                return Ok(NearAiMcpBootstrapOutcome::SkippedNonActivatable);
            }
        }
    }

    let scope = AuthProductScope::new(resource_scope.clone(), AuthSurface::Api);
    let provider =
        AuthProviderId::new("nearai").map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("NEAR AI MCP provider id is invalid: {error}"),
        })?;
    let requester_extension =
        ExtensionId::new("nearai").map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("NEAR AI MCP requester extension id is invalid: {error}"),
        })?;
    let existing_accounts = product_auth
        .credential_account_record_source()
        .accounts_for_owner(&scope)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("NEAR AI MCP product-auth lookup failed: {error}"),
        })?
        .into_iter()
        .filter(|account| account.provider == provider)
        .collect::<Vec<_>>();

    let credential_decision =
        nearai_mcp_bootstrap_existing_credential_decision(&existing_accounts, &requester_extension);
    let mut submitted_credential = false;
    match credential_decision {
        NearAiMcpBootstrapExistingCredentialDecision::ReuseUsable => {
            tracing::debug!(
                "NEAR AI MCP credential already exists; skipping boot-time token update"
            );
        }
        NearAiMcpBootstrapExistingCredentialDecision::Submit { existing_account } => {
            let credential_submit =
                ProductAuthExtensionCredentialSetup::new(Arc::clone(product_auth))
                    .submit_manual_token(ExtensionCredentialSubmitRequest {
                        scope,
                        provider,
                        label: "NEAR AI API key".to_string(),
                        requester_extension,
                        existing_account,
                        secret: config.into_api_key(),
                    })
                    .await;
            if let Err(error) = credential_submit {
                if is_nearai_mcp_disabled_or_removed(&error) {
                    tracing::debug!(
                        "NEAR AI MCP credentials are present, but the extension participant is disabled or removed; preserving explicit operator state"
                    );
                    return Ok(NearAiMcpBootstrapOutcome::SkippedDisabled);
                }
                if is_nearai_mcp_product_auth_temporarily_unavailable(&error) {
                    tracing::debug!(
                        error = ?error,
                        "NEAR AI MCP credential bootstrap is temporarily unavailable; continuing without auto-activating the extension"
                    );
                    return Ok(NearAiMcpBootstrapOutcome::SkippedUnavailable);
                }
                return Err(RebornBuildError::InvalidConfig {
                    reason: format!("NEAR AI MCP product-auth credential submit failed: {error:?}"),
                });
            }
            submitted_credential = true;
        }
    }

    // Bootstrap installs for the owner named by the runtime scope. Extension
    // membership remains per user even when that owner is also the operator.
    let bootstrap_caller = resource_scope.user_id.clone();
    if !installed {
        extension_management
            .install(package_ref.clone(), &bootstrap_caller)
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("NEAR AI MCP extension install failed: {error}"),
            })?;
    }
    // A not-installed (just installed above) or an installed-but-inactive
    // extension activates; an already-active one only reports its credential
    // outcome.
    if !installed || phase == LifecyclePublicState::SetupNeeded {
        extension_management
            .activate_with_credential_gate(
                package_ref,
                ExtensionActivationMode::Static,
                RuntimeExtensionActivationCredentialGate::new(
                    resource_scope,
                    product_auth.runtime_credential_account_selection_service(),
                ),
                &bootstrap_caller,
            )
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("NEAR AI MCP extension activation failed: {error}"),
            })?;
        return Ok(NearAiMcpBootstrapOutcome::Activated);
    }
    // Installed and already active.
    if submitted_credential {
        Ok(NearAiMcpBootstrapOutcome::SubmittedCredential)
    } else {
        Ok(NearAiMcpBootstrapOutcome::ReusedCredential)
    }
}

fn nearai_mcp_bootstrap_account_is_usable(
    account: &CredentialAccount,
    requester_extension: &ExtensionId,
) -> bool {
    account.status == CredentialAccountStatus::Configured
        && account.access_secret.is_some()
        && account.is_authorized_for_requester(Some(requester_extension))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NearAiMcpBootstrapExistingCredentialDecision {
    ReuseUsable,
    Submit {
        existing_account: Option<CredentialAccountUpdateBinding>,
    },
}

fn nearai_mcp_bootstrap_existing_credential_decision(
    existing_accounts: &[CredentialAccount],
    requester_extension: &ExtensionId,
) -> NearAiMcpBootstrapExistingCredentialDecision {
    if existing_accounts
        .iter()
        .any(|account| nearai_mcp_bootstrap_account_is_usable(account, requester_extension))
    {
        return NearAiMcpBootstrapExistingCredentialDecision::ReuseUsable;
    }

    let existing_account = existing_accounts
        .iter()
        .max_by_key(|account| (account.updated_at, account.created_at, account.id))
        .map(|account| CredentialAccountUpdateBinding {
            account_id: account.id,
            ownership: account.ownership,
            owner_extension: account.owner_extension.clone(),
            granted_extensions: account.granted_extensions.clone(),
        });
    NearAiMcpBootstrapExistingCredentialDecision::Submit { existing_account }
}

fn is_nearai_mcp_disabled_or_removed(error: &ProductSurfaceError) -> bool {
    error.code == ProductSurfaceErrorCode::Forbidden
        && error.kind == ProductSurfaceErrorKind::ParticipantDenied
}

fn is_nearai_mcp_product_auth_temporarily_unavailable(error: &ProductSurfaceError) -> bool {
    error.code == ProductSurfaceErrorCode::Unavailable
        && error.kind == ProductSurfaceErrorKind::ServiceUnavailable
        && error.retryable
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{InvocationId, UserId};

    fn account_for_bootstrap_decision(
        requester_extension: &ExtensionId,
        status: CredentialAccountStatus,
        access_secret: Option<&str>,
        updated_at_secs: i64,
    ) -> CredentialAccount {
        let user_id = UserId::new("nearai-account-user").expect("user");
        CredentialAccount {
            id: ironclaw_auth::CredentialAccountId::new(),
            scope: AuthProductScope::new(
                ResourceScope::local_default(user_id, InvocationId::new()).expect("scope"),
                AuthSurface::Api,
            ),
            provider: AuthProviderId::new("nearai").expect("provider"),
            label: ironclaw_auth::CredentialAccountLabel::new("NEAR AI").expect("label"),
            status,
            ownership: ironclaw_auth::CredentialOwnership::ExtensionOwned,
            owner_extension: Some(requester_extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: access_secret
                .map(|handle| ironclaw_host_api::SecretHandle::new(handle).expect("secret handle")),
            refresh_secret: None,
            scopes: Vec::new(),
            provider_identity: None,
            created_at: chrono::DateTime::from_timestamp(updated_at_secs, 0).expect("timestamp"),
            updated_at: chrono::DateTime::from_timestamp(updated_at_secs, 0).expect("timestamp"),
        }
    }

    #[test]
    fn bootstrap_existing_credential_decision_reuses_any_usable_account() {
        let requester_extension = ExtensionId::new("nearai").expect("extension");
        let older_usable = account_for_bootstrap_decision(
            &requester_extension,
            CredentialAccountStatus::Configured,
            Some("nearai-access-secret"),
            10,
        );
        let newer_unusable = account_for_bootstrap_decision(
            &requester_extension,
            CredentialAccountStatus::PendingSetup,
            None,
            20,
        );

        let decision = nearai_mcp_bootstrap_existing_credential_decision(
            &[newer_unusable, older_usable],
            &requester_extension,
        );

        assert_eq!(
            decision,
            NearAiMcpBootstrapExistingCredentialDecision::ReuseUsable
        );
    }

    #[test]
    fn bootstrap_existing_credential_decision_updates_newest_when_none_are_usable() {
        let requester_extension = ExtensionId::new("nearai").expect("extension");
        let older_unusable = account_for_bootstrap_decision(
            &requester_extension,
            CredentialAccountStatus::PendingSetup,
            None,
            10,
        );
        let newer_unusable = account_for_bootstrap_decision(
            &requester_extension,
            CredentialAccountStatus::PendingSetup,
            None,
            20,
        );
        let expected_account_id = newer_unusable.id;

        let decision = nearai_mcp_bootstrap_existing_credential_decision(
            &[newer_unusable, older_unusable],
            &requester_extension,
        );

        let NearAiMcpBootstrapExistingCredentialDecision::Submit {
            existing_account: Some(existing_account),
        } = decision
        else {
            panic!("expected newest account update binding, got {decision:?}");
        };
        assert_eq!(existing_account.account_id, expected_account_id);
    }
}
