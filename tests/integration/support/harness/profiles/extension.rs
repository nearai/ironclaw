//! Extension domain tools profiles.

use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel, CredentialAccountStatus,
    CredentialOwnership, NewCredentialAccount, ProviderScope, SLACK_PERSONAL_PROVIDER_ID,
};
use ironclaw_host_api::{
    AgentId, InvocationId, MountView, ProjectId, ResourceScope, SecretHandle, TenantId, UserId,
};

use std::sync::Arc;

use ironclaw_network::NetworkHttpEgress;

use super::super::super::extension_surface::{
    BUNDLED_EXTENSION_CAPABILITY_IDS, EXTENSION_LIFECYCLE_CAPABILITY_IDS,
};
use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{
    HarnessResult, HostRuntimeCapabilityHarness, RecordingNetworkHttpEgress,
    bundled_extension_provider_trust, capability_ids_from_strs, local_dev_all_effects,
    wildcard_test_policy,
};

pub(crate) fn extension_lifecycle_tools_profile() -> HarnessResult<ToolsProfile> {
    extension_lifecycle_tools_profile_for_user("reborn-e2e-extension-lifecycle-user")
}

/// Same profile as [`extension_lifecycle_tools_profile`], but seeds
/// credentials and provider trust under a caller-supplied `user_id` instead
/// of the fixed test constant. Callers that align the built harness's
/// dispatch scope to a real turn's binding subject (`HostRuntimeCapabilityHarness::with_user_id`,
/// e.g. `group_constructors.rs`'s `build_group_capability_with_base` and
/// `RebornBinaryE2EHarness::with_host_runtime_extension_lifecycle_capabilities`)
/// must also seed under that SAME aligned user — `with_user_id` only
/// re-points dispatch scope, not the extension-credential rows seeded during
/// `.build()`, so a mismatched seed user leaves credentialed extensions
/// (e.g. `github`) `BlockedAuth` for the aligned caller.
pub(crate) fn extension_lifecycle_tools_profile_for_user(
    user_id: &str,
) -> HarnessResult<ToolsProfile> {
    let mut capability_ids = capability_ids_from_strs(EXTENSION_LIFECYCLE_CAPABILITY_IDS)?;
    capability_ids.extend(capability_ids_from_strs(BUNDLED_EXTENSION_CAPABILITY_IDS)?);
    // Hermetic guard: without a test egress, `build_local_runtime` defaults to
    // a REAL `ReqwestNetworkTransport`, and this profile's scenarios dispatch a
    // bundled extension capability post-activation, which crosses HTTP.
    let network_egress: Arc<dyn NetworkHttpEgress> =
        Arc::new(RecordingNetworkHttpEgress::with_body(
            br#"{"ok":true,"channels":[],"messages":[],"resultSizeEstimate":0,"response_metadata":{"next_cursor":""}}"#.to_vec(),
        ));
    Ok(ToolsProfile {
        capability_ids,
        effect_kinds: local_dev_all_effects(),
        options: HostRuntimeHarnessOptions::new(
            MountView::default(),
            Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                true,
            )?),
        )
        .with_seed_extension_credentials()
        .with_network_http_egress_for_test(network_egress),
        network_policy_override: Some(wildcard_test_policy()),
        provider_trust_override: Some(bundled_extension_provider_trust()?),
        auto_approve_default: Some(true),
        ..ToolsProfile::new("reborn-e2e-extension-lifecycle-tools", user_id)?
    })
}

/// [`extension_lifecycle_tools_profile`], plus a composition-time Google
/// OAuth backend (the "config set" + restart arm of the provider-instance
/// readiness map) — the no-false-positive counterpart proving the
/// readiness-map check clears
/// once an operator configures the instance, and the run falls through to
/// the ordinary per-account credential gate instead.
pub(crate) fn extension_lifecycle_tools_profile_google_oauth_configured()
-> HarnessResult<ToolsProfile> {
    let mut profile = extension_lifecycle_tools_profile()?;
    profile.options = profile.options.with_google_oauth_backend_for_test();
    Ok(profile)
}

/// [`extension_lifecycle_tools_profile_google_oauth_configured`], seeded under a
/// caller-supplied `user_id` — the same fixed-user/aligned-user split
/// [`extension_lifecycle_tools_profile_for_user`] documents. Callers that align
/// the harness's dispatch scope to a real turn's binding subject (the
/// `RebornBinaryE2EHarness` extension-lifecycle constructor) need BOTH the
/// aligned seed user and the configured-instance signal, which neither
/// single-axis constructor above provides on its own.
pub(crate) fn extension_lifecycle_tools_profile_google_oauth_configured_for_user(
    user_id: &str,
) -> HarnessResult<ToolsProfile> {
    let mut profile = extension_lifecycle_tools_profile_for_user(user_id)?;
    profile.options = profile.options.with_google_oauth_backend_for_test();
    Ok(profile)
}

pub(crate) async fn extension_lifecycle_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    extension_lifecycle_tools_profile()?.build().await
}

/// Model-visible capability of the visibility-probe fixture extension.
pub(crate) const VISIBILITY_PROBE_MODEL_CAPABILITY_ID: &str = "visprobe.search";
/// `host_internal` sibling in the SAME package — must never be advertised to
/// the model even though it is granted and registry-published.
pub(crate) const VISIBILITY_PROBE_HOST_INTERNAL_CAPABILITY_ID: &str = "visprobe.audit";

/// Two-capability fixture manifest: one `model`-visible capability and one
/// `host_internal` sibling. Parsed by the production manifest parser with the
/// HostBundled source — the same loader path bundled manifests use — so the
/// visibility vocabulary under test is the real manifest schema, not a
/// hand-built descriptor.
const VISIBILITY_PROBE_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "visprobe"
name = "Visibility Probe"
version = "0.1.0"
description = "Surface-visibility probe fixture"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/visprobe.wasm"

[[capabilities]]
id = "visprobe.search"
description = "Model-visible probe capability"
effects = ["network"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"

[[capabilities]]
id = "visprobe.audit"
description = "Host-internal probe capability"
effects = ["network", "external_write"]
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/audit.input.json"
output_schema_ref = "schemas/audit.output.json"
"#;

fn visibility_probe_package() -> HarnessResult<ironclaw_extensions::ExtensionPackage> {
    let manifest = ironclaw_extensions::ExtensionManifest::parse(
        VISIBILITY_PROBE_MANIFEST,
        ironclaw_extensions::ManifestSource::HostBundled,
        &ironclaw_host_api::host_port::HostPortCatalog::empty(),
    )?;
    Ok(ironclaw_extensions::ExtensionPackage::from_manifest(
        manifest,
        ironclaw_host_api::VirtualPath::new("/system/extensions/visprobe")?,
    )?)
}

/// Harness for the HostInternal surface-hiding probe: the fixture package is
/// published into the active-extension registry at construction (the same
/// publish step activation uses) and BOTH its capabilities are granted — so
/// the ONLY thing that can keep `visprobe.audit` off the model surface is the
/// registry-level visibility filter, not grant absence or non-publication.
pub(crate) fn extension_visibility_probe_tools_profile() -> HarnessResult<ToolsProfile> {
    let package = visibility_probe_package()?;
    Ok(ToolsProfile {
        capability_ids: capability_ids_from_strs(&[
            VISIBILITY_PROBE_MODEL_CAPABILITY_ID,
            VISIBILITY_PROBE_HOST_INTERNAL_CAPABILITY_ID,
        ])?,
        effect_kinds: local_dev_all_effects(),
        options: HostRuntimeHarnessOptions::new(
            MountView::default(),
            Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                true,
            )?),
        )
        .with_activated_bundled_extension(package),
        network_policy_override: Some(wildcard_test_policy()),
        provider_trust_override: Some(vec![(
            ironclaw_host_api::ExtensionId::new("visprobe")?,
            local_dev_all_effects(),
        )]),
        // Surface resolution reads each advertised capability's
        // `input_schema_ref` off the mounted filesystem under the package
        // root; without the fixture schemas host creation fails.
        post_construct_asset_copy: Some((
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/extension_visibility"),
            std::path::PathBuf::from("local-dev/system/extensions/visprobe"),
        )),
        auto_approve_default: Some(true),
        ..ToolsProfile::new(
            "reborn-e2e-extension-visibility-probe",
            "reborn-e2e-extension-visibility-user",
        )?
    })
}

pub(crate) async fn extension_visibility_probe_tools() -> HarnessResult<HostRuntimeCapabilityHarness>
{
    extension_visibility_probe_tools_profile()?.build().await
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
        ExtensionLifecycleCredentialSeed {
            provider: SLACK_PERSONAL_PROVIDER_ID,
            label: "qa slack",
            secret_handle: "qa_slack_personal_access",
            scopes: &[
                "search:read",
                "channels:history",
                "groups:history",
                "im:history",
                "mpim:history",
                "channels:read",
                "groups:read",
                "im:read",
                "mpim:read",
                "users:read",
                "chat:write",
            ],
        },
    ]
}
