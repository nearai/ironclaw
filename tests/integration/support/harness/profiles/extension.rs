//! Extension domain tools profiles.

use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel, CredentialAccountStatus,
    CredentialOwnership, NewCredentialAccount, ProviderScope,
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
    let mut capability_ids = capability_ids_from_strs(EXTENSION_LIFECYCLE_CAPABILITY_IDS)?;
    capability_ids.extend(capability_ids_from_strs(BUNDLED_EXTENSION_CAPABILITY_IDS)?);
    // Hermetic guard: without a test egress, `build_local_runtime` defaults to
    // a REAL `ReqwestNetworkTransport`, and this profile's scenarios dispatch a
    // bundled extension capability post-activation, which crosses HTTP. The
    // typed recorder is retained so tests can assert on the recorded wire
    // (`captured_network_requests`).
    let network_egress = Arc::new(RecordingNetworkHttpEgress::with_body(
        br#"{"ok":true,"messages":[],"resultSizeEstimate":0}"#.to_vec(),
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
        .with_recording_network_egress(network_egress),
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

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "visprobe.search"
description = "Model-visible probe capability"
effects = ["network"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"

[[capability_provider.tools.capabilities]]
id = "visprobe.audit"
description = "Host-internal probe capability"
effects = ["network", "external_write"]
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/audit.input.json"
output_schema_ref = "schemas/audit.output.json"
"#;

fn visibility_probe_package() -> HarnessResult<(
    ironclaw_extensions::ExtensionPackage,
    ironclaw_extensions::ResolvedExtensionManifest,
)> {
    let record = ironclaw_extensions::ExtensionManifestRecord::from_toml(
        VISIBILITY_PROBE_MANIFEST,
        ironclaw_extensions::ManifestSource::HostBundled,
        &ironclaw_host_api::host_port::HostPortCatalog::empty(),
        None,
        &capability_provider_contracts(),
    )?;
    let manifest = ironclaw_extensions::ExtensionManifest::try_from(record.manifest().clone())?;
    Ok((
        ironclaw_extensions::ExtensionPackage::from_manifest(
            manifest,
            ironclaw_host_api::VirtualPath::new("/system/extensions/visprobe")?,
        )?,
        record.resolved().clone(),
    ))
}

/// Harness for the HostInternal surface-hiding probe: the fixture package is
/// published into the active-extension registry at construction (the same
/// publish step activation uses) and BOTH its capabilities are granted — so
/// the ONLY thing that can keep `visprobe.audit` off the model surface is the
/// registry-level visibility filter, not grant absence or non-publication.
pub(crate) fn extension_visibility_probe_tools_profile() -> HarnessResult<ToolsProfile> {
    let (package, resolved) = visibility_probe_package()?;
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
        .with_activated_bundled_extension_resolved(package, resolved),
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
    ]
}

fn capability_provider_contracts() -> ironclaw_extensions::HostApiContractRegistry {
    let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
    contracts
        .register(std::sync::Arc::new(
            ironclaw_extensions::CapabilityProviderHostApiContract::new()
                .expect("capability provider contract"),
        ))
        .expect("register capability provider contract");
    contracts
}

// ── Invented-vendor fixture (extension-runtime P2, overview §8) ─────────────

/// The fixture's native `runtime.service` id, from
/// `tests/fixtures/extensions/acme-messenger/manifest.toml`.
pub(crate) const ACME_FIXTURE_SERVICE: &str = "acme-messenger.extension/v1";
pub(crate) const ACME_SEND_NOTE_CAPABILITY_ID: &str = "acme-messenger.send_note";

fn acme_fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/extensions/acme-messenger")
}

/// The binary-assembled native factory for the fixture: binds the tool
/// adapter (routes `send_note`) plus the scripted channel adapter the
/// binding rule requires for the declared `[channel]`.
struct AcmeFixtureFactory;

impl ironclaw_extension_host::NativeExtensionFactory for AcmeFixtureFactory {
    fn service(&self) -> &str {
        ACME_FIXTURE_SERVICE
    }

    fn load(
        &self,
        _ctx: &ironclaw_extension_host::LoadContext,
    ) -> Result<
        Box<dyn ironclaw_extension_host::ExtensionEntrypoint>,
        ironclaw_extension_host::BindError,
    > {
        Ok(Box::new(AcmeFixtureEntrypoint))
    }
}

struct AcmeFixtureEntrypoint;

impl ironclaw_extension_host::ExtensionEntrypoint for AcmeFixtureEntrypoint {
    fn bind(
        &self,
        _ctx: ironclaw_extension_host::BindContext,
    ) -> Result<ironclaw_extension_host::ExtensionBindings, ironclaw_extension_host::BindError>
    {
        Ok(ironclaw_extension_host::ExtensionBindings {
            tools: Some(Arc::new(AcmeFixtureToolAdapter)),
            channel: Some(Arc::new(
                ironclaw_extension_host::test_support::FakeChannelAdapter::default(),
            )),
        })
    }
}

struct AcmeFixtureToolAdapter;

#[async_trait::async_trait]
impl ironclaw_host_api::ToolAdapter for AcmeFixtureToolAdapter {
    async fn invoke(
        &self,
        call: ironclaw_host_api::ToolCall,
        _ports: &ironclaw_host_api::ToolPorts<'_>,
    ) -> Result<ironclaw_host_api::ToolResult, ironclaw_host_api::ToolError> {
        match call.capability_id.as_str() {
            ACME_SEND_NOTE_CAPABILITY_ID => {
                let text = call
                    .input
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let output =
                    serde_json::json!({"delivered": true, "note_id": "note-1", "text": text});
                let output_bytes = serde_json::to_vec(&output)
                    .map(|bytes| bytes.len() as u64)
                    .unwrap_or_default();
                Ok(ironclaw_host_api::ToolResult {
                    output,
                    display_preview: None,
                    output_bytes,
                })
            }
            _ => Err(ironclaw_host_api::ToolError::Failed {
                kind: ironclaw_host_api::RuntimeDispatchErrorKind::UndeclaredCapability,
                safe_summary: None,
            }),
        }
    }
}

/// The extension-lifecycle profile extended with the invented-vendor fixture:
/// its assets copied into the storage root pre-build (the catalog discovers
/// them), its native factory assembled into the composition input, its tool
/// granted, and its provider trusted — the acme lifecycle then runs through
/// the REAL facade (install → activate → dispatch-from-snapshot → remove).
pub(crate) fn extension_runtime_acme_tools_profile() -> HarnessResult<ToolsProfile> {
    let mut profile = extension_lifecycle_tools_profile()?;
    profile
        .capability_ids
        .push(ironclaw_host_api::CapabilityId::new(
            ACME_SEND_NOTE_CAPABILITY_ID,
        )?);
    // The real Slack package's five tools (TOOL-7 drives them through the
    // generic dispatcher post-activation).
    for slack_tool in [
        "slack.search_messages",
        "slack.list_conversations",
        "slack.get_conversation_history",
        "slack.get_user_info",
        "slack.send_message",
    ] {
        profile
            .capability_ids
            .push(ironclaw_host_api::CapabilityId::new(slack_tool)?);
    }
    if let Some(trust) = profile.provider_trust_override.as_mut() {
        trust.push((
            ironclaw_host_api::ExtensionId::new("acme-messenger")?,
            local_dev_all_effects(),
        ));
        trust.push((
            ironclaw_host_api::ExtensionId::new("slack")?,
            local_dev_all_effects(),
        ));
    }
    profile.options = profile
        .options
        .with_fixture_extension_dir(acme_fixture_dir(), "acme-messenger")
        .with_native_extension_factory(Arc::new(AcmeFixtureFactory));
    Ok(profile)
}

pub(crate) async fn extension_runtime_acme_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    extension_runtime_acme_tools_profile()?.build().await
}
