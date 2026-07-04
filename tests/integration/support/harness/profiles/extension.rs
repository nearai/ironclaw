//! Extension domain tools profiles.

use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel, CredentialAccountStatus,
    CredentialOwnership, NewCredentialAccount, ProviderScope,
};
use ironclaw_host_api::{
    AgentId, InvocationId, MountView, ProjectId, ResourceScope, SecretHandle, TenantId, UserId,
};

use super::super::super::extension_surface::{
    BUNDLED_EXTENSION_CAPABILITY_IDS, EXTENSION_LIFECYCLE_CAPABILITY_IDS,
};
use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{
    HarnessResult, HostRuntimeCapabilityHarness, bundled_extension_provider_trust,
    capability_ids_from_strs, local_dev_all_effects, wildcard_test_policy,
};

pub(crate) fn extension_lifecycle_tools_profile() -> HarnessResult<ToolsProfile> {
    let mut capability_ids = capability_ids_from_strs(EXTENSION_LIFECYCLE_CAPABILITY_IDS)?;
    capability_ids.extend(capability_ids_from_strs(BUNDLED_EXTENSION_CAPABILITY_IDS)?);
    Ok(ToolsProfile {
        capability_ids,
        effect_kinds: local_dev_all_effects(),
        options: HostRuntimeHarnessOptions::new(
            MountView::default(),
            Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                true,
            )?),
        )
        .with_seed_extension_credentials(),
        network_policy_override: Some(wildcard_test_policy()),
        provider_trust_override: Some(bundled_extension_provider_trust()?),
        auto_approve_default: Some(true),
        ..ToolsProfile::new(
            "reborn-e2e-extension-lifecycle-tools",
            "reborn-e2e-extension-lifecycle-user",
        )?
    })
}

pub(crate) async fn extension_lifecycle_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    extension_lifecycle_tools_profile()?.build().await
}

pub(crate) async fn seed_extension_lifecycle_credentials(
    services: &ironclaw_reborn_composition::RebornServices,
    user_id: &UserId,
) -> HarnessResult<()> {
    let product_auth = services
        .product_auth
        .as_ref()
        .ok_or("extension lifecycle harness missing product auth")?;
    let scope = AuthProductScope::credential_owner(
        &ResourceScope {
            tenant_id: TenantId::new("tenant-e2e")?,
            user_id: user_id.clone(),
            agent_id: Some(AgentId::new("agent-e2e")?),
            project_id: Some(ProjectId::new("project-e2e")?),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        },
        AuthSurface::Api,
    );
    let accounts = product_auth.credential_account_service();
    for seed in extension_lifecycle_credential_seeds() {
        accounts
            .create_account(NewCredentialAccount {
                scope: scope.clone(),
                provider: AuthProviderId::new(seed.provider)?,
                label: CredentialAccountLabel::new(seed.label)?,
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new(seed.secret_handle)?),
                refresh_secret: None,
                scopes: seed
                    .scopes
                    .iter()
                    .map(|scope| ProviderScope::new(*scope))
                    .collect::<Result<Vec<_>, _>>()?,
            })
            .await?;
    }
    Ok(())
}

struct ExtensionLifecycleCredentialSeed {
    provider: &'static str,
    label: &'static str,
    secret_handle: &'static str,
    scopes: &'static [&'static str],
}

fn extension_lifecycle_credential_seeds() -> &'static [ExtensionLifecycleCredentialSeed] {
    &[
        ExtensionLifecycleCredentialSeed {
            provider: "github",
            label: "qa github",
            secret_handle: "qa_github_access",
            scopes: &[],
        },
        ExtensionLifecycleCredentialSeed {
            provider: "google",
            label: "qa google",
            secret_handle: "qa_google_access",
            scopes: &[
                "https://www.googleapis.com/auth/calendar.events",
                "https://www.googleapis.com/auth/calendar.readonly",
                "https://www.googleapis.com/auth/documents",
                "https://www.googleapis.com/auth/documents.readonly",
                "https://www.googleapis.com/auth/drive",
                "https://www.googleapis.com/auth/drive.readonly",
                "https://www.googleapis.com/auth/gmail.modify",
                "https://www.googleapis.com/auth/gmail.readonly",
                "https://www.googleapis.com/auth/gmail.send",
                "https://www.googleapis.com/auth/presentations",
                "https://www.googleapis.com/auth/presentations.readonly",
                "https://www.googleapis.com/auth/spreadsheets",
                "https://www.googleapis.com/auth/spreadsheets.readonly",
            ],
        },
        ExtensionLifecycleCredentialSeed {
            provider: "nearai",
            label: "qa nearai",
            secret_handle: "qa_nearai_access",
            scopes: &[],
        },
        ExtensionLifecycleCredentialSeed {
            provider: "notion",
            label: "qa notion",
            secret_handle: "qa_notion_access",
            scopes: &[],
        },
    ]
}
