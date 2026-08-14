//! Extension domain tools profiles.

use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel, CredentialAccountStatus,
    CredentialOwnership, NewCredentialAccount, ProviderScope,
};
use ironclaw_extension_contracts::test_support::conformance::ScriptedVendorServer;
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgress, RestrictedEgressRequest, RestrictedEgressResponse, ToolAdapter, ToolCall,
    ToolError, ToolPorts, ToolResult,
};
use ironclaw_host_api::{
    action::NetworkMethod,
    dispatch::RuntimeDispatchErrorKind,
    ids::{AgentId, InvocationId, ProjectId, SecretHandle, TenantId, UserId},
    messaging::StandardMessagingErrorCode,
    mount::{MountPermissions, MountView},
    resource::ResourceScope,
};

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use super::super::super::extension_surface::{
    EXTENSION_LIFECYCLE_CAPABILITY_IDS, bundled_extension_manifest_capability_ids,
};
use super::super::super::github;
use super::super::options::{HostRuntimeHarnessOptions, ToolsProfile};
use super::super::{
    HarnessResult, HostRuntimeCapabilityHarness, RecordingNetworkHttpEgress, VendorResponseRouter,
    bundled_extension_provider_trust, capability_ids_from_strs, standalone_all_effects,
    wildcard_test_policy, workspace_mounts,
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
    capability_ids.extend(github::capability_ids()?);
    capability_ids.extend(bundled_extension_manifest_capability_ids()?);
    // Hermetic guard: without a test egress, `build_local_runtime` defaults to
    // a REAL `ReqwestNetworkTransport`, and this profile's scenarios dispatch a
    // bundled extension capability post-activation, which crosses HTTP. The
    // typed recorder is retained so tests can assert on the recorded wire
    // (`captured_network_requests`).
    let network_egress = Arc::new(
        RecordingNetworkHttpEgress::with_body(
            br#"{"ok":true,"channels":[],"messages":[],"resultSizeEstimate":0,"response_metadata":{"next_cursor":""}}"#.to_vec(),
        )
        .with_vendor_router(Arc::new(hosted_mcp_discovery_fixture_response)),
    );
    Ok(ToolsProfile {
        capability_ids,
        effect_kinds: standalone_all_effects(),
        options: HostRuntimeHarnessOptions::new(
            MountView::default(),
            Some(ironclaw_composition::standalone_unrestricted_runtime_policy(true)?),
        )
        .with_durable_capability_io()
        .with_seed_extension_credentials()
        .with_recording_network_egress(network_egress),
        network_policy_override: Some(wildcard_test_policy()),
        provider_trust_override: Some(bundled_extension_provider_trust()?),
        auto_approve_default: Some(true),
        ..ToolsProfile::new("reborn-e2e-extension-lifecycle-tools", user_id)?
    })
}

/// Hermetic hosted-MCP handshake for lifecycle scenarios that need a real
/// post-auth activation. The production path still performs the complete
/// initialize -> initialized -> tools/list exchange through mediated network
/// egress; only the external server response is replaced here.
fn hosted_mcp_discovery_fixture_response(
    request: &ironclaw_network::NetworkHttpRequest,
) -> Option<(u16, Vec<u8>)> {
    let body: serde_json::Value = serde_json::from_slice(&request.body).ok()?;
    let method = body.get("method")?.as_str()?;
    let is_nearai = request.url.contains(".near.ai/");
    let result = match method {
        "initialize" => serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "extension-lifecycle-test", "version": "1.0.0"}
        }),
        "notifications/initialized" => serde_json::json!({}),
        "tools/list" => serde_json::json!({
            "tools": [{
                "name": if is_nearai { "web_search" } else { "live-search" },
                "description": "Hermetic hosted MCP search tool",
                "inputSchema": {
                    "type": "object",
                    "properties": {"query": {"type": "string"}},
                    "required": ["query"]
                },
                "annotations": {"readOnlyHint": true}
            }]
        }),
        "tools/call" if is_nearai => {
            let is_empty_query = body
                .pointer("/params/arguments/query")
                .and_then(serde_json::Value::as_str)
                == Some("NEARAI_EMPTY_PROVIDER_RESULT");
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": if is_empty_query {
                        "[]"
                    } else {
                        "REBORN_NEARAI_WEB_SEARCH_RESULT"
                    }
                }]
            })
        }
        _ => return None,
    };
    let response = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": body.get("id").cloned().unwrap_or(serde_json::Value::Null),
        "result": result,
    }))
    .ok()?;
    Some((200, response))
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

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
origin_gate_matrix = { loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }
id = "visprobe.search"
description = "Model-visible probe capability"
effects = ["network"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"

[[capability_provider.tools.capabilities]]
origin_gate_matrix = { loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }
id = "visprobe.audit"
description = "Host-internal probe capability"
effects = ["network", "external_write"]
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/audit.input.json"
output_schema_ref = "schemas/audit.output.json"
"#;

fn visibility_probe_package() -> HarnessResult<(
    ironclaw_extension_registry::ExtensionPackage,
    ironclaw_extension_registry::ResolvedExtensionManifest,
)> {
    let root = ironclaw_host_api::path::VirtualPath::new("/system/extensions/visprobe")?;
    let record = ironclaw_extension_registry::ExtensionManifestRecord::from_toml(
        VISIBILITY_PROBE_MANIFEST,
        ironclaw_extension_registry::ManifestSource::HostBundled,
        &ironclaw_host_api::host_port::HostPortCatalog::empty(),
        None,
        &capability_provider_contracts(),
        Some(root.clone()),
    )?;
    let manifest =
        ironclaw_extension_registry::ExtensionManifest::try_from(record.manifest().clone())?;
    Ok((
        ironclaw_extension_registry::ExtensionPackage::from_manifest(manifest, root)?,
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
        effect_kinds: standalone_all_effects(),
        options: HostRuntimeHarnessOptions::new(
            MountView::default(),
            Some(ironclaw_composition::standalone_unrestricted_runtime_policy(true)?),
        )
        .with_activated_bundled_extension_resolved(package, resolved),
        network_policy_override: Some(wildcard_test_policy()),
        provider_trust_override: Some(vec![(
            ironclaw_host_api::ids::ExtensionId::new("visprobe")?,
            standalone_all_effects(),
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

/// Authentication vocabulary from the Attio incident. It is ordinary prompt
/// data regardless of package provenance; actual credential values are
/// handled by the provider-bound redaction pass.
pub(crate) const AUTH_VOCABULARY_DESCRIPTION: &str =
    "Authenticated with a workspace API key presented as a Bearer header";

const VERIFIED_PROMPT_DESCRIPTION_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "verifiedprompt"
name = "Verified Prompt Description Probe"
version = "0.1.0"
description = "Verified prompt-description probe fixture"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/verifiedprompt.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
origin_gate_matrix = { loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }
id = "verifiedprompt.invoke"
description = "Authenticated with a workspace API key presented as a Bearer header"
effects = ["network"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/invoke.input.json"
output_schema_ref = "schemas/invoke.output.json"
"#;

const LOCAL_PROMPT_DESCRIPTION_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "localprompt"
name = "Local Prompt Description Probe"
version = "0.1.0"
description = "Local prompt-description probe fixture"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/localprompt.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
origin_gate_matrix = { loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }
id = "localprompt.unsafe"
description = "Authenticated with a workspace API key presented as a Bearer header"
effects = ["network"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/probe.input.json"
output_schema_ref = "schemas/probe.output.json"

[[capability_provider.tools.capabilities]]
origin_gate_matrix = { loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }
id = "localprompt.healthy"
description = "Healthy local capability remains available"
effects = ["network"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/probe.input.json"
output_schema_ref = "schemas/probe.output.json"
"#;

fn prompt_description_files(
    manifest_toml: &str,
    package_id: &str,
    module_name: &str,
    schema_names: &[&str],
) -> HarnessResult<Vec<(String, Vec<u8>)>> {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture_root = repo_root
        .join("tests/fixtures/extension_prompt_trust")
        .join(package_id);
    let module =
        std::fs::read(repo_root.join("crates/extensions/packages/github/wasm/github_tool.wasm"))?;
    let mut files = vec![
        (
            "manifest.toml".to_string(),
            manifest_toml.as_bytes().to_vec(),
        ),
        (format!("wasm/{module_name}"), module),
    ];
    for schema_name in schema_names {
        files.push((
            format!("schemas/{schema_name}"),
            std::fs::read(fixture_root.join("schemas").join(schema_name))?,
        ));
    }
    Ok(files)
}

fn verified_prompt_description_package() -> HarnessResult<(
    ironclaw_extension_registry::ExtensionPackage,
    ironclaw_extension_registry::ResolvedExtensionManifest,
)> {
    let available = ironclaw_extension_host::registry_extension_package(
        prompt_description_files(
            VERIFIED_PROMPT_DESCRIPTION_MANIFEST,
            "verifiedprompt",
            "verifiedprompt.wasm",
            &["invoke.input.json", "invoke.output.json"],
        )?,
        &[],
    )?;
    let resolved = available.resolved_manifest.as_ref().clone();
    Ok((available.package, resolved))
}

fn local_prompt_description_package() -> HarnessResult<(
    ironclaw_extension_registry::ExtensionPackage,
    ironclaw_extension_registry::ResolvedExtensionManifest,
)> {
    let available = ironclaw_extension_host::imported_extension_package(
        prompt_description_files(
            LOCAL_PROMPT_DESCRIPTION_MANIFEST,
            "localprompt",
            "localprompt.wasm",
            &["probe.input.json", "probe.output.json"],
        )?,
        &[],
    )?;
    let resolved = available.resolved_manifest.as_ref().clone();
    Ok((available.package, resolved))
}

/// Real-turn prompt-description trust probe. Both fixture manifests go
/// through the production registry/local import boundaries and the
/// active-extension publisher. Those boundaries assign the manifest source,
/// which is the production input that derives `CapabilityDescriptionTrust`.
pub(crate) fn extension_prompt_description_trust_probe_tools_profile() -> HarnessResult<ToolsProfile>
{
    let (verified_package, verified_resolved) = verified_prompt_description_package()?;
    let (local_package, local_resolved) = local_prompt_description_package()?;
    Ok(ToolsProfile {
        capability_ids: capability_ids_from_strs(&[
            "verifiedprompt.invoke",
            "localprompt.unsafe",
            "localprompt.healthy",
        ])?,
        effect_kinds: standalone_all_effects(),
        options: HostRuntimeHarnessOptions::new(
            MountView::default(),
            Some(ironclaw_composition::standalone_unrestricted_runtime_policy(true)?),
        )
        .with_activated_bundled_extension_resolved(verified_package, verified_resolved)
        .with_activated_bundled_extension_resolved(local_package, local_resolved),
        network_policy_override: Some(wildcard_test_policy()),
        provider_trust_override: Some(vec![
            (
                ironclaw_host_api::ids::ExtensionId::new("verifiedprompt")?,
                standalone_all_effects(),
            ),
            (
                ironclaw_host_api::ids::ExtensionId::new("localprompt")?,
                standalone_all_effects(),
            ),
        ]),
        post_construct_asset_copy: Some((
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/extension_prompt_trust"),
            std::path::PathBuf::from("local-dev/system/extensions"),
        )),
        auto_approve_default: Some(true),
        ..ToolsProfile::new(
            "reborn-e2e-extension-prompt-description-trust",
            "reborn-e2e-extension-prompt-description-trust-user",
        )?
    })
}

pub(crate) async fn extension_prompt_description_trust_probe_tools()
-> HarnessResult<HostRuntimeCapabilityHarness> {
    extension_prompt_description_trust_probe_tools_profile()?
        .build()
        .await
}

pub(crate) async fn seed_extension_lifecycle_credentials(
    services: &ironclaw_composition::RebornRuntime,
    user_id: &UserId,
) -> HarnessResult<()> {
    let product_auth = services.product_auth_for_test();
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
            provider: "slack",
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
                "reactions:read",
                "reactions:write",
                "im:write",
            ],
        },
    ]
}

fn capability_provider_contracts() -> ironclaw_extension_registry::HostApiContractRegistry {
    let mut contracts = ironclaw_extension_registry::HostApiContractRegistry::new();
    contracts
        .register(std::sync::Arc::new(
            ironclaw_extension_registry::CapabilityProviderHostApiContract::new()
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
pub(crate) const ACME_CREDENTIAL_SCOPES: &[&str] = &["notes:write", "notes:read"];

/// Every standard-op capability id the acme fixture's manifest binds
/// (standardized messaging framework, task 7), in
/// [`ironclaw_host_api::messaging::StandardMessagingOp::ALL`] core-op order. Pushed into
/// the harness profile's granted capability set alongside
/// [`ACME_SEND_NOTE_CAPABILITY_ID`] so the standard ops dispatch through the
/// same active-surface path the bespoke tool already proves.
const ACME_STANDARD_OP_CAPABILITY_IDS: &[&str] = &[
    "acme-messenger.send_message",
    "acme-messenger.edit_message",
    "acme-messenger.delete_message",
    "acme-messenger.add_reaction",
    "acme-messenger.remove_reaction",
    "acme-messenger.open_dm",
    "acme-messenger.list_conversations",
    "acme-messenger.get_conversation_info",
    "acme-messenger.get_conversation_history",
    "acme-messenger.get_thread_replies",
    "acme-messenger.get_message",
    "acme-messenger.search_messages",
    "acme-messenger.get_user_info",
    "acme-messenger.resolve_user",
    "acme-messenger.list_members",
    "acme-messenger.whoami",
];

fn acme_fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/extensions/acme-messenger")
}

// ── Acme vendor egress fallback (standardized messaging framework, task 7
//    fix round) ─────────────────────────────────────────────────────────────
//
// `ToolPorts.egress` is `None` on every production dispatch path today (no
// first-party adapter has ever had a live consumer — see
// `ironclaw_extension_host::resolver::SnapshotBoundCapability::dispatch_json`
// and `ironclaw_capabilities::registry`, both of which hardcode
// `ToolPorts { egress: None }` unconditionally). Wiring a REAL
// `RestrictedEgress` into that production seam for a test-only fixture is out
// of scope (YAGNI — the framework's host-mediated egress story for a real
// extension is the WASM/MCP staged pipeline, exercised by Slack in task 9).
// Instead `AcmeFixtureToolAdapter` carries its OWN constructor-held scripted
// vendor egress and `post_acme` falls back to it whenever `ports.egress` is
// `None`, so the fixture dispatches end to end through the real turn/dispatch
// pipeline (Task 8's through-the-stack scenarios) exactly as it does when a
// test drives `invoke` directly with `ports.egress` supplied.

/// One scripted outcome for one acme vendor op: either a canned success body
/// or a canned vendor failure code (mapped through
/// [`acme_error_to_standard_code`] the same way a real non-2xx response is).
#[derive(Clone)]
enum AcmeVendorOutcome {
    Body(serde_json::Value),
    VendorError(String),
}

/// Shared, mutable per-op script for the acme vendor fixture's constructor-
/// held fallback egress. `Clone` + `Arc`-backed: a scenario builds one,
/// scripts whatever ops it needs (`respond`/`fail`), hands it to
/// [`extension_runtime_acme_tools_profile_with_vendor_script`], and reads the
/// returned `ScriptedVendorServer::requests()` back after driving its turn —
/// the same "only the external server response is replaced here" idiom
/// `hosted_mcp_discovery_fixture_response` and `delivery_vendor_router` use
/// for the runtime HTTP egress lane, mirrored here for the restricted-egress
/// lane. Every op not explicitly scripted still answers with a working
/// built-in default (see [`default_acme_vendor_response`]), so a scenario
/// that scripts nothing still gets a successful dispatch.
#[derive(Clone, Default)]
pub(crate) struct AcmeVendorScript {
    overrides: Arc<Mutex<HashMap<String, AcmeVendorOutcome>>>,
}

impl AcmeVendorScript {
    /// Script `op_name`'s response body, replacing its built-in default.
    pub(crate) fn respond(&self, op_name: &str, body: serde_json::Value) {
        self.overrides
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(op_name.to_string(), AcmeVendorOutcome::Body(body));
    }

    /// Script `op_name` to fail with the given acme vendor error code
    /// (e.g. `"conversation_missing"`), replacing its built-in default.
    pub(crate) fn fail(&self, op_name: &str, vendor_code: &str) {
        self.overrides
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                op_name.to_string(),
                AcmeVendorOutcome::VendorError(vendor_code.to_string()),
            );
    }

    fn outcome_for(&self, op_name: &str) -> AcmeVendorOutcome {
        self.overrides
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(op_name)
            .cloned()
            .unwrap_or_else(|| AcmeVendorOutcome::Body(default_acme_vendor_response(op_name)))
    }
}

/// Built-in happy-path vendor response for one op, keyed by op name — the
/// same shapes the adapter's own conformance test scripts by hand
/// (`acme_standard_ops_satisfy_canonical_contracts`), so a scenario that
/// never calls [`AcmeVendorScript::respond`]/[`AcmeVendorScript::fail`] still
/// gets a working dispatch for every one of the 16 core ops. Ops the adapter
/// only checks for a 2xx status (edit/delete/react) share the generic
/// `{"ok": true}` fallback.
fn default_acme_vendor_response(op_name: &str) -> serde_json::Value {
    match op_name {
        "send_message" => serde_json::json!({ "id": "AMSG-1" }),
        "open_dm" => serde_json::json!({ "conversation": "ACME-C-DM-1" }),
        "list_conversations" => serde_json::json!({
            "conversations": [
                { "conversation": "ACME-C-1", "kind": "channel", "name": "general", "member": true },
            ],
        }),
        "get_conversation_info" => serde_json::json!({
            "conversation": "ACME-C-1", "kind": "channel", "name": "general", "member": true,
        }),
        "get_conversation_history" | "get_thread_replies" => serde_json::json!({
            "messages": [
                { "conversation": "ACME-C-1", "message_id": "AMSG-1", "author_ref": "U1",
                  "author_name": "Ann", "text": "hi", "ts": "2026-07-27T00:00:00Z", "self": false },
            ],
        }),
        "get_message" => serde_json::json!({
            "conversation": "ACME-C-1", "message_id": "AMSG-1", "author_ref": "U1",
            "text": "hi", "self": true,
        }),
        "search_messages" => serde_json::json!({
            "matches": [
                { "conversation": "ACME-C-1", "message_id": "AMSG-1", "author_ref": "U1",
                  "text": "hi", "self": true },
            ],
        }),
        "get_user_info" => serde_json::json!({
            "user_ref": "U1", "name": "Ann", "bot": false, "presence": "active",
        }),
        "resolve_user" => serde_json::json!({ "matches": [{ "user_ref": "U1", "name": "Ann" }] }),
        "list_members" => serde_json::json!({ "members": [{ "user_ref": "U1", "name": "Ann" }] }),
        "whoami" => serde_json::json!({ "user_ref": "U-SELF", "name": "Acme Bot" }),
        _ => serde_json::json!({ "ok": true }),
    }
}

/// Builds the constructor-held fallback egress: a [`ScriptedVendorServer`]
/// (the SAME double the adapter's own conformance tests hand-build) that
/// extracts the op name from the request URL's final path segment and
/// consults `script` for its outcome.
fn acme_scripted_vendor_egress(script: AcmeVendorScript) -> ScriptedVendorServer {
    ScriptedVendorServer::new(Arc::new(move |request: &RestrictedEgressRequest| {
        let op_name = request.url.rsplit('/').next().unwrap_or_default();
        match script.outcome_for(op_name) {
            AcmeVendorOutcome::Body(body) => RestrictedEgressResponse {
                status: 200,
                body: serde_json::to_vec(&body).unwrap_or_default(),
            },
            AcmeVendorOutcome::VendorError(vendor_code) => RestrictedEgressResponse {
                status: 400,
                body: serde_json::to_vec(&serde_json::json!({ "error": vendor_code }))
                    .unwrap_or_default(),
            },
        }
    }))
}

/// The binary-assembled native factory for the fixture: binds the tool
/// adapter (routes `send_note` and the 16 standard ops) plus the scripted
/// channel adapter the binding rule requires for the declared `[channel]`.
/// Carries the constructor-held fallback egress
/// [`AcmeFixtureToolAdapter::post_acme`] uses whenever a dispatch arrives
/// with `ports.egress = None` (every production path today).
struct AcmeFixtureFactory {
    fallback_egress: Arc<ScriptedVendorServer>,
}

#[async_trait::async_trait]
impl ironclaw_extension_host::NativeExtensionFactory for AcmeFixtureFactory {
    fn service(&self) -> &str {
        ACME_FIXTURE_SERVICE
    }

    async fn load(
        &self,
        _ctx: &ironclaw_extension_host::LoadContext,
    ) -> Result<
        Box<dyn ironclaw_extension_host::ExtensionEntrypoint>,
        ironclaw_extension_host::BindError,
    > {
        Ok(Box::new(AcmeFixtureEntrypoint {
            fallback_egress: Arc::clone(&self.fallback_egress),
        }))
    }
}

struct AcmeFixtureEntrypoint {
    fallback_egress: Arc<ScriptedVendorServer>,
}

impl ironclaw_extension_host::ExtensionEntrypoint for AcmeFixtureEntrypoint {
    fn bind(
        &self,
        ctx: ironclaw_extension_host::BindContext,
    ) -> Result<ironclaw_extension_host::ExtensionBindings, ironclaw_extension_host::BindError>
    {
        Ok(ironclaw_extension_host::ExtensionBindings {
            tools: Some(Arc::new(AcmeFixtureToolAdapter {
                fallback_egress: Arc::clone(&self.fallback_egress),
            })),
            channel: {
                let adapter = Arc::new(AcmeFixtureChannelAdapter);
                ironclaw_extension_contracts::channel_adapter::ChannelSurfaces::default()
                    .with_ingress(adapter.clone())
                    .with_reply(adapter.clone())
                    .with_delivery(adapter)
            },
            // Bound only when the installed manifest declares a device_link
            // recipe: `check_binding` proves agreement per axis, and the
            // stock acme-messenger manifest declares oauth2_code only — an
            // unconditional adapter fails every acme bind with
            // `UndeclaredDeviceLinkAdapter`, so activation never completes
            // and install turns record no capability results.
            device_link: ironclaw_extension_host::declared_device_link_recipe(&ctx.resolved)
                .is_some()
                .then(|| {
                    Arc::new(super::device_link::ScriptedDeviceLinkAdapter::new())
                        as Arc<dyn ironclaw_extension_contracts::device_link::DeviceLinkAdapter>
                }),
        })
    }
}

/// The fixture's REAL channel adapter: pure protocol parsing of the invented
/// vendor's wire shape for the generic ingress router (extension-runtime P4).
///
/// Wire shape: `{"type":"message","event_id":..,"conversation":..,"user":..,
/// "text":..}` normalizes to one message; `{"type":"challenge",
/// "challenge":..}` echoes the challenge; any other authenticated payload is
/// an ignored no-op.
pub(crate) struct AcmeFixtureChannelAdapter;

#[async_trait::async_trait]
impl ironclaw_extension_contracts::channel_adapter::ChannelIngress for AcmeFixtureChannelAdapter {
    async fn receive(
        &self,
        request: ironclaw_extension_contracts::channel_adapter::VerifiedInbound<'_>,
        _egress: &dyn ironclaw_extension_contracts::tool_adapter::RestrictedEgress,
    ) -> Result<
        ironclaw_extension_contracts::channel_adapter::InboundOutcome,
        ironclaw_extension_contracts::channel_adapter::ChannelError,
    > {
        use ironclaw_extension_contracts::channel_adapter::NormalizedInboundMessage;
        use ironclaw_extension_contracts::channel_adapter::{
            ChannelError, ImmediateResponse, InboundOutcome, ProductTriggerReason,
        };
        use ironclaw_extension_contracts::external::{
            ExternalActorRef, ExternalConversationRef, ExternalEventId,
        };
        let parse = |reason: String| ChannelError::Parse { reason };
        let value: serde_json::Value =
            serde_json::from_slice(request.body).map_err(|error| parse(error.to_string()))?;
        match value.get("type").and_then(serde_json::Value::as_str) {
            Some("challenge") => {
                let challenge = value
                    .get("challenge")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| parse("missing challenge".to_string()))?;
                Ok(InboundOutcome::Respond(ImmediateResponse {
                    status: 200,
                    content_type: Some("text/plain".to_string()),
                    body: challenge.as_bytes().to_vec(),
                }))
            }
            Some("message") => {
                let field = |name: &str| {
                    value
                        .get(name)
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                        .ok_or_else(|| parse(format!("missing {name}")))
                };
                Ok(InboundOutcome::Messages(vec![NormalizedInboundMessage {
                    actor: ExternalActorRef::new("acme_user", field("user")?, None::<&str>)
                        .map_err(|error| parse(error.to_string()))?,
                    conversation: ExternalConversationRef::new(
                        None,
                        field("conversation")?,
                        None,
                        None,
                    )
                    .map_err(|error| parse(error.to_string()))?,
                    event_id: ExternalEventId::new(field("event_id")?)
                        .map_err(|error| parse(error.to_string()))?,
                    text: field("text")?,
                    trigger: ProductTriggerReason::DirectChat,
                    attachments: Vec::new(),
                    conversation_context: None,
                    reply_context: Some(b"acme-reply-route".to_vec()),
                }]))
            }
            _ => Ok(InboundOutcome::Ignore),
        }
    }
}

/// One vendor mechanism serves both output axes for this fixture, exactly as
/// it does for a real conversational vendor.
impl AcmeFixtureChannelAdapter {
    /// Minimal real outbound: one vendor POST per text part. Proves the
    /// generic delivery path (coordinator → adapter → restricted egress)
    /// needs no real product, and gives the conformance suite a deliverable
    /// fixture.
    async fn send(
        &self,
        envelope: ironclaw_extension_contracts::channel_adapter::OutboundEnvelope,
        egress: &dyn ironclaw_extension_contracts::tool_adapter::RestrictedEgress,
    ) -> Result<
        ironclaw_extension_contracts::channel_adapter::DeliveryReport,
        ironclaw_extension_contracts::channel_adapter::ChannelError,
    > {
        use ironclaw_extension_contracts::channel_adapter::{
            ChannelError, OutboundPart, PartDeliveryOutcome,
        };
        if envelope.parts.is_empty() {
            return Err(ChannelError::Render {
                reason: "outbound envelope carries no parts".to_string(),
            });
        }
        let mut parts = Vec::new();
        for part in &envelope.parts {
            let outcome =
                match part {
                    OutboundPart::Text(text) => {
                        let body = serde_json::json!({
                            "conversation": envelope.target.conversation.conversation_id(),
                            "text": text,
                        });
                        let response = egress
                        .send(ironclaw_extension_contracts::tool_adapter::RestrictedEgressRequest {
                            method: ironclaw_host_api::action::NetworkMethod::Post,
                            url: "https://api.acme.example/messages".to_string(),
                            headers: vec![(
                                "content-type".to_string(),
                                "application/json".to_string(),
                            )],
                            body: serde_json::to_vec(&body).ok(),
                            credential: None,
                            body_credentials: Vec::new(),
                        })
                        .await;
                        match response {
                            Ok(response) if (200..300).contains(&response.status) => {
                                PartDeliveryOutcome::Sent {
                                    vendor_message_ref: None,
                                }
                            }
                            Ok(response) => PartDeliveryOutcome::Permanent {
                                reason: format!("acme vendor returned status {}", response.status),
                            },
                            Err(error) => PartDeliveryOutcome::Retryable {
                                reason: error.to_string(),
                            },
                        }
                    }
                    _ => PartDeliveryOutcome::Permanent {
                        reason: "the acme fixture delivers text parts only".to_string(),
                    },
                };
            let sent = matches!(outcome, PartDeliveryOutcome::Sent { .. });
            parts.push(outcome);
            if !sent {
                break;
            }
        }
        Ok(ironclaw_extension_contracts::channel_adapter::DeliveryReport::from_parts(parts))
    }
}

#[async_trait::async_trait]
impl ironclaw_extension_contracts::channel_adapter::ChannelReply for AcmeFixtureChannelAdapter {
    async fn send_reply(
        &self,
        envelope: ironclaw_extension_contracts::channel_adapter::OutboundEnvelope,
        egress: &dyn ironclaw_extension_contracts::tool_adapter::RestrictedEgress,
    ) -> Result<
        ironclaw_extension_contracts::channel_adapter::DeliveryReport,
        ironclaw_extension_contracts::channel_adapter::ChannelError,
    > {
        self.send(envelope, egress).await
    }
}

#[async_trait::async_trait]
impl ironclaw_extension_contracts::channel_adapter::ChannelDelivery for AcmeFixtureChannelAdapter {
    async fn deliver(
        &self,
        envelope: ironclaw_extension_contracts::channel_adapter::OutboundEnvelope,
        egress: &dyn ironclaw_extension_contracts::tool_adapter::RestrictedEgress,
    ) -> Result<
        ironclaw_extension_contracts::channel_adapter::DeliveryReport,
        ironclaw_extension_contracts::channel_adapter::ChannelError,
    > {
        self.send(envelope, egress).await
    }
}

/// Binds `send_note` plus the 16 standard messaging ops. Carries a
/// constructor-held scripted vendor egress (`fallback_egress`) that
/// [`Self::post_acme`] falls back to whenever `ports.egress` is `None` —
/// every production dispatch path today, since no first-party adapter has a
/// live `RestrictedEgress` consumer wired (see the module doc above
/// `AcmeVendorOutcome`).
struct AcmeFixtureToolAdapter {
    fallback_egress: Arc<ScriptedVendorServer>,
}

#[async_trait::async_trait]
impl ToolAdapter for AcmeFixtureToolAdapter {
    async fn invoke(&self, call: ToolCall, ports: &ToolPorts<'_>) -> Result<ToolResult, ToolError> {
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
                Ok(ironclaw_extension_contracts::tool_adapter::ToolResult {
                    output,
                    display_preview: None,
                    output_bytes,
                })
            }
            // ── standardized messaging framework: the 16 core ops ──────────
            // Every arm: parse the canonical input, POST to the invented acme
            // vendor API (one op per path segment, scripted in tests), map
            // the vendor response onto the canonical output. Vendor failures
            // (non-2xx) are mapped to a `StandardMessagingErrorCode` inside
            // `post_acme`/`acme_vendor_error` before they reach here.
            "acme-messenger.send_message" => {
                let conversation = input_str(&call.input, "conversation")?;
                let text = input_str(&call.input, "text")?;
                // W3/W4 (pre-merge amendment wave): `reply_to` (quotes one
                // specific message) is distinct from `thread` (posts into a
                // thread/topic container); acme has only one vendor mechanism
                // so both forward to the vendor, and both echo back on the
                // output — when supplied — so a silent drop is checkable.
                // `.filter(!is_empty)`: unlike `reply_to` (whose nested
                // conversation/message_id both carry canonical minLength: 1),
                // the standalone `thread` string has none pre-dispatch —
                // treat an empty string the same as absent rather than echo
                // it back and trip the output schema's own minLength: 1.
                let thread = call
                    .input
                    .get("thread")
                    .and_then(serde_json::Value::as_str)
                    .filter(|thread| !thread.is_empty());
                let reply_to = call.input.get("reply_to");
                let response = self
                    .post_acme(
                        "send_message",
                        serde_json::json!({
                            "conversation": conversation,
                            "text": text,
                            "thread": thread,
                            "reply_to": reply_to,
                        }),
                        ports,
                    )
                    .await?;
                let message_id = vendor_str(&response, "id")?;
                let mut output = serde_json::json!({
                    "message_ref": { "conversation": conversation, "message_id": message_id }
                });
                if let Some(thread) = thread {
                    output["thread"] = serde_json::json!(thread);
                }
                if let Some(reply_to) = reply_to {
                    output["reply_to"] = reply_to.clone();
                }
                Ok(tool_result(output))
            }
            "acme-messenger.edit_message" => {
                let message_ref = input_object(&call.input, "message_ref")?;
                let text = input_str(&call.input, "text")?;
                self.post_acme(
                    "edit_message",
                    serde_json::json!({ "message_ref": message_ref, "text": text }),
                    ports,
                )
                .await?;
                Ok(tool_result(
                    serde_json::json!({ "message_ref": message_ref }),
                ))
            }
            "acme-messenger.delete_message" => {
                let message_ref = input_object(&call.input, "message_ref")?;
                self.post_acme(
                    "delete_message",
                    serde_json::json!({ "message_ref": message_ref }),
                    ports,
                )
                .await?;
                Ok(tool_result(serde_json::json!({
                    "deleted": true,
                    "message_ref": message_ref
                })))
            }
            "acme-messenger.add_reaction" => {
                let message_ref = input_object(&call.input, "message_ref")?;
                let emoji = input_str(&call.input, "emoji")?;
                self.post_acme(
                    "add_reaction",
                    serde_json::json!({ "message_ref": message_ref, "emoji": emoji }),
                    ports,
                )
                .await?;
                Ok(tool_result(serde_json::json!({
                    "message_ref": message_ref,
                    "emoji": emoji
                })))
            }
            "acme-messenger.remove_reaction" => {
                let message_ref = input_object(&call.input, "message_ref")?;
                // W5 (pre-merge amendment wave): `emoji` is optional on
                // remove — absent means "remove the connected account's own
                // reaction(s)". acme continues echoing it back when given.
                let emoji = call.input.get("emoji").and_then(serde_json::Value::as_str);
                self.post_acme(
                    "remove_reaction",
                    serde_json::json!({ "message_ref": message_ref, "emoji": emoji }),
                    ports,
                )
                .await?;
                let mut output = serde_json::json!({ "message_ref": message_ref });
                if let Some(emoji) = emoji {
                    output["emoji"] = serde_json::json!(emoji);
                }
                Ok(tool_result(output))
            }
            "acme-messenger.open_dm" => {
                let user_ref = input_str(&call.input, "user_ref")?;
                let response = self
                    .post_acme(
                        "open_dm",
                        serde_json::json!({ "user_ref": user_ref }),
                        ports,
                    )
                    .await?;
                let conversation = vendor_str(&response, "conversation")?;
                Ok(tool_result(
                    serde_json::json!({ "conversation": conversation }),
                ))
            }
            "acme-messenger.list_conversations" => {
                let response = self
                    .post_acme(
                        "list_conversations",
                        serde_json::json!({
                            "kinds": call.input.get("kinds"),
                            "limit": call.input.get("limit"),
                            "cursor": call.input.get("cursor"),
                        }),
                        ports,
                    )
                    .await?;
                let conversations = response
                    .get("conversations")
                    .and_then(serde_json::Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .map(conversation_info_from_vendor)
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                let mut output = serde_json::json!({ "conversations": conversations });
                if let Some(cursor) = response
                    .get("next_cursor")
                    .and_then(serde_json::Value::as_str)
                {
                    output["next_cursor"] = serde_json::json!(cursor);
                }
                Ok(tool_result(output))
            }
            "acme-messenger.get_conversation_info" => {
                let conversation = input_str(&call.input, "conversation")?;
                let response = self
                    .post_acme(
                        "get_conversation_info",
                        serde_json::json!({ "conversation": conversation }),
                        ports,
                    )
                    .await?;
                Ok(tool_result(conversation_info_from_vendor(&response)?))
            }
            "acme-messenger.get_conversation_history" => {
                let conversation = input_str(&call.input, "conversation")?;
                let response = self
                    .post_acme(
                        "get_conversation_history",
                        serde_json::json!({
                            "conversation": conversation,
                            "limit": call.input.get("limit"),
                            "cursor": call.input.get("cursor"),
                        }),
                        ports,
                    )
                    .await?;
                Ok(tool_result(messages_output_from_vendor(
                    &response, "messages",
                )?))
            }
            "acme-messenger.get_thread_replies" => {
                let conversation = input_str(&call.input, "conversation")?;
                let thread = input_str(&call.input, "thread")?;
                let response = self
                    .post_acme(
                        "get_thread_replies",
                        serde_json::json!({
                            "conversation": conversation,
                            "thread": thread,
                            "limit": call.input.get("limit"),
                            "cursor": call.input.get("cursor"),
                        }),
                        ports,
                    )
                    .await?;
                Ok(tool_result(messages_output_from_vendor(
                    &response, "messages",
                )?))
            }
            "acme-messenger.get_message" => {
                let message_ref = input_object(&call.input, "message_ref")?;
                let response = self
                    .post_acme(
                        "get_message",
                        serde_json::json!({ "message_ref": message_ref }),
                        ports,
                    )
                    .await?;
                Ok(tool_result(serde_json::json!({
                    "message": message_from_vendor(&response)?
                })))
            }
            "acme-messenger.search_messages" => {
                let query = input_str(&call.input, "query")?;
                let response = self
                    .post_acme(
                        "search_messages",
                        serde_json::json!({
                            "query": query,
                            "limit": call.input.get("limit"),
                            "cursor": call.input.get("cursor"),
                        }),
                        ports,
                    )
                    .await?;
                let mut output = messages_output_from_vendor(&response, "matches")?;
                if let Some(total) = response.get("total").and_then(serde_json::Value::as_u64) {
                    output["total"] = serde_json::json!(total);
                }
                Ok(tool_result(output))
            }
            "acme-messenger.get_user_info" => {
                let user_ref = input_str(&call.input, "user_ref")?;
                let response = self
                    .post_acme(
                        "get_user_info",
                        serde_json::json!({ "user_ref": user_ref }),
                        ports,
                    )
                    .await?;
                Ok(tool_result(user_info_from_vendor(&response)?))
            }
            "acme-messenger.resolve_user" => {
                let query = input_str(&call.input, "query")?;
                let response = self
                    .post_acme(
                        "resolve_user",
                        serde_json::json!({
                            "query": query,
                            "limit": call.input.get("limit"),
                            "cursor": call.input.get("cursor"),
                        }),
                        ports,
                    )
                    .await?;
                Ok(tool_result(user_ref_list_from_vendor(
                    &response, "matches",
                )?))
            }
            "acme-messenger.list_members" => {
                let conversation = input_str(&call.input, "conversation")?;
                let response = self
                    .post_acme(
                        "list_members",
                        serde_json::json!({
                            "conversation": conversation,
                            "limit": call.input.get("limit"),
                            "cursor": call.input.get("cursor"),
                        }),
                        ports,
                    )
                    .await?;
                Ok(tool_result(user_ref_list_from_vendor(
                    &response, "members",
                )?))
            }
            "acme-messenger.whoami" => {
                let response = self
                    .post_acme("whoami", serde_json::json!({}), ports)
                    .await?;
                Ok(tool_result(user_ref_entry_from_vendor(&response)?))
            }

            _ => Err(ToolError::Failed {
                kind: RuntimeDispatchErrorKind::UndeclaredCapability,
                safe_summary: None,
                model_visible_cause: None,
            }),
        }
    }
}

impl AcmeFixtureToolAdapter {
    /// One POST per standard op at `https://api.acme.example/<op_name>` —
    /// the invented acme vendor API (standardized messaging framework, task
    /// 7). Uses `ports.egress` when the dispatch supplied one (the shape a
    /// test driving `invoke` directly hands in, e.g. this adapter's own
    /// conformance tests), otherwise falls back to the constructor-held
    /// [`Self::fallback_egress`] — every production dispatch path today, since
    /// no first-party adapter has a live `RestrictedEgress` consumer wired
    /// (see the module doc above `AcmeVendorOutcome`). Mirrors the egress
    /// pattern [`AcmeFixtureChannelAdapter::deliver`] uses for real outbound
    /// delivery. A non-2xx vendor response maps its `error` code onto the
    /// standard messaging error taxonomy via [`acme_vendor_error`] before the
    /// caller ever sees it.
    async fn post_acme(
        &self,
        op_name: &str,
        body: serde_json::Value,
        ports: &ToolPorts<'_>,
    ) -> Result<serde_json::Value, ToolError> {
        let egress: &dyn RestrictedEgress = ports
            .egress
            .unwrap_or(self.fallback_egress.as_ref() as &dyn RestrictedEgress);
        let credential = SecretHandle::new("acme_user_token").map_err(|error| {
            acme_tool_error(
                RuntimeDispatchErrorKind::Manifest,
                format!("acme credential handle is invalid: {error}"),
            )
        })?;
        let response = egress
            .send(RestrictedEgressRequest {
                method: NetworkMethod::Post,
                url: format!("https://api.acme.example/{op_name}"),
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                body: serde_json::to_vec(&body).ok(),
                credential: Some(credential),
                body_credentials: Vec::new(),
            })
            .await
            .map_err(|error| {
                acme_tool_error(
                    RuntimeDispatchErrorKind::Backend,
                    format!("acme vendor request failed: {error}"),
                )
            })?;
        let payload: serde_json::Value =
            serde_json::from_slice(&response.body).unwrap_or(serde_json::Value::Null);
        if (200..300).contains(&response.status) {
            Ok(payload)
        } else {
            let vendor_code = payload
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            Err(acme_vendor_error(vendor_code))
        }
    }
}

/// Builds a successful [`ToolResult`] from a canonical
/// output value — the shared envelope every standard-op arm (and send_note)
/// returns through.
fn tool_result(output: serde_json::Value) -> ToolResult {
    let output_bytes = serde_json::to_vec(&output)
        .map(|bytes| bytes.len() as u64)
        .unwrap_or_default();
    ToolResult {
        output,
        display_preview: None,
        output_bytes,
    }
}

fn acme_tool_error(kind: RuntimeDispatchErrorKind, safe_summary: String) -> ToolError {
    ToolError::Failed {
        kind,
        safe_summary: Some(safe_summary),
        model_visible_cause: None,
    }
}

fn missing_input_field(field: &str) -> ToolError {
    acme_tool_error(
        RuntimeDispatchErrorKind::Unknown,
        format!("acme adapter input missing required field: {field}"),
    )
}

/// A required top-level string field from canonical (already
/// schema-validated) input.
fn input_str<'a>(input: &'a serde_json::Value, field: &str) -> Result<&'a str, ToolError> {
    input
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| missing_input_field(field))
}

/// A required top-level object field from canonical input — used for
/// `message_ref`, which every write/read-by-ref op forwards to the vendor
/// and echoes back verbatim rather than decomposing.
fn input_object<'a>(
    input: &'a serde_json::Value,
    field: &str,
) -> Result<&'a serde_json::Value, ToolError> {
    input
        .get(field)
        .filter(|value| value.is_object())
        .ok_or_else(|| missing_input_field(field))
}

/// A required string field from a vendor response — fails loud (rather than
/// silently embedding `null`) when the scripted/real vendor body is
/// malformed, so a bad fixture surfaces as a test failure instead of an
/// invalid canonical output.
fn vendor_str<'a>(response: &'a serde_json::Value, field: &str) -> Result<&'a str, ToolError> {
    response
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            acme_tool_error(
                RuntimeDispatchErrorKind::OutputDecode,
                format!("acme vendor response missing or empty field: {field}"),
            )
        })
}

/// Maps one acme vendor error code onto the closed standard messaging error
/// taxonomy (standardized messaging framework spec §8); an unmapped vendor
/// code falls back to `VendorError`, matching every other adapter's
/// contract for the day the vendor adds a new failure mode.
fn acme_error_to_standard_code(vendor_code: &str) -> StandardMessagingErrorCode {
    use StandardMessagingErrorCode::*;
    match vendor_code {
        "conversation_missing" => UnknownConversation,
        "message_missing" => UnknownMessage,
        "user_missing" => UnknownUser,
        "not_member" => NotAMember,
        "forbidden" => PermissionDenied,
        "dm_closed" => CannotMessageUser,
        "window_closed" => OutsideMessagingWindow,
        "too_long" => MessageTooLong,
        "bad_content" => UnsupportedContent,
        "slow_down" => RateLimited,
        "edit_locked" => EditNotAllowed,
        _ => VendorError,
    }
}

/// Builds the adapter error for a non-2xx vendor response: maps the vendor
/// code and puts the standard code string in the safe summary — the same
/// error path `send_note` surfaces through today (`ToolError::Failed`'s
/// `safe_summary`), which is the channel the standard messaging error
/// taxonomy is documented to ride
/// (`ironclaw_host_api::messaging::StandardMessagingErrorCode`).
fn acme_vendor_error(vendor_code: &str) -> ToolError {
    let code = acme_error_to_standard_code(vendor_code);
    acme_tool_error(
        RuntimeDispatchErrorKind::OperationFailed,
        format!("acme vendor rejected the request: {}", code.as_str()),
    )
}

/// One canonical `message` object (spec appendix) from one vendor-shaped
/// message row. Shared by `get_message`, `get_conversation_history`,
/// `get_thread_replies`, and `search_messages` — the vendor's row shape
/// (`author_ref`/`author_name`/`ts`/`self`) is deliberately distinct from
/// the canonical field names to prove this is a real mapping, not a
/// passthrough.
fn message_from_vendor(value: &serde_json::Value) -> Result<serde_json::Value, ToolError> {
    let conversation = vendor_str(value, "conversation")?;
    let message_id = vendor_str(value, "message_id")?;
    let author_ref = vendor_str(value, "author_ref")?;
    let text = vendor_str(value, "text")?;
    let mut author = serde_json::json!({
        "user_ref": author_ref,
    });
    if let Some(name) = value.get("author_name").and_then(serde_json::Value::as_str) {
        author["display_name"] = serde_json::json!(name);
    }
    let mut message = serde_json::json!({
        "message_ref": {
            "conversation": conversation,
            "message_id": message_id,
        },
        "author": author,
        "text": text,
        "is_self": value.get("self").and_then(serde_json::Value::as_bool).unwrap_or(false),
    });
    if let Some(ts) = value.get("ts").and_then(serde_json::Value::as_str) {
        message["timestamp"] = serde_json::json!(ts);
    }
    if let Some(thread) = value.get("thread").and_then(serde_json::Value::as_str) {
        let mut thread_obj = serde_json::json!({ "thread": thread });
        if let Some(count) = value.get("reply_count").and_then(serde_json::Value::as_u64) {
            thread_obj["reply_count"] = serde_json::json!(count);
        }
        message["thread"] = thread_obj;
    }
    Ok(message)
}

/// The canonical `{ "<list_key>": [message, ...], "next_cursor"? }` envelope
/// shared by `get_conversation_history`/`get_thread_replies`
/// (`list_key = "messages"`) and `search_messages`
/// (`list_key = "matches"`).
fn messages_output_from_vendor(
    response: &serde_json::Value,
    list_key: &str,
) -> Result<serde_json::Value, ToolError> {
    let messages = response
        .get(list_key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(message_from_vendor)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let mut output = serde_json::json!({ list_key: messages });
    if let Some(cursor) = response
        .get("next_cursor")
        .and_then(serde_json::Value::as_str)
    {
        output["next_cursor"] = serde_json::json!(cursor);
    }
    Ok(output)
}

/// One canonical `conversation_info`-shaped object (used both as
/// `get_conversation_info`'s top-level output and per-item in
/// `list_conversations`) from one vendor-shaped conversation row.
fn conversation_info_from_vendor(
    value: &serde_json::Value,
) -> Result<serde_json::Value, ToolError> {
    let conversation = vendor_str(value, "conversation")?;
    let kind = vendor_str(value, "kind")?;
    if !matches!(kind, "dm" | "group_dm" | "channel" | "other") {
        return Err(acme_tool_error(
            RuntimeDispatchErrorKind::OutputDecode,
            format!("acme vendor response has unknown conversation kind: {kind}"),
        ));
    }
    let mut info = serde_json::json!({
        "conversation": conversation,
        "kind": kind,
    });
    if let Some(name) = value.get("name").and_then(serde_json::Value::as_str) {
        info["display_name"] = serde_json::json!(name);
    }
    if let Some(member) = value.get("member").and_then(serde_json::Value::as_bool) {
        info["is_member"] = serde_json::json!(member);
    }
    if value.get("counterpart_ref").is_some() {
        let counterpart_ref = vendor_str(value, "counterpart_ref")?;
        let mut counterpart = serde_json::json!({ "user_ref": counterpart_ref });
        if let Some(name) = value
            .get("counterpart_name")
            .and_then(serde_json::Value::as_str)
        {
            counterpart["display_name"] = serde_json::json!(name);
        }
        info["counterpart"] = counterpart;
    } else if kind == "dm" {
        return Err(acme_tool_error(
            RuntimeDispatchErrorKind::OutputDecode,
            "acme vendor dm response missing counterpart_ref".to_string(),
        ));
    }
    Ok(info)
}

/// One canonical `{ "user_ref", "display_name"? }` entry from one
/// vendor-shaped user row — shared by `resolve_user`/`list_members`'s
/// per-item shape and `whoami`'s top-level output.
fn user_ref_entry_from_vendor(value: &serde_json::Value) -> Result<serde_json::Value, ToolError> {
    let user_ref = vendor_str(value, "user_ref")?;
    let mut entry = serde_json::json!({
        "user_ref": user_ref,
    });
    if let Some(name) = value.get("name").and_then(serde_json::Value::as_str) {
        entry["display_name"] = serde_json::json!(name);
    }
    Ok(entry)
}

/// The canonical `{ "<list_key>": [{user_ref, display_name?}, ...],
/// "next_cursor"? }` envelope shared by `resolve_user`
/// (`list_key = "matches"`) and `list_members` (`list_key = "members"`).
fn user_ref_list_from_vendor(
    response: &serde_json::Value,
    list_key: &str,
) -> Result<serde_json::Value, ToolError> {
    let entries = response
        .get(list_key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(user_ref_entry_from_vendor)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let mut output = serde_json::json!({ list_key: entries });
    if let Some(cursor) = response
        .get("next_cursor")
        .and_then(serde_json::Value::as_str)
    {
        output["next_cursor"] = serde_json::json!(cursor);
    }
    Ok(output)
}

/// The canonical `get_user_info`/`whoami`-adjacent user-info shape from one
/// vendor-shaped user record; unlike [`user_ref_entry_from_vendor`] this
/// carries the fuller profile fields `get_user_info` alone exposes.
fn user_info_from_vendor(value: &serde_json::Value) -> Result<serde_json::Value, ToolError> {
    let user_ref = vendor_str(value, "user_ref")?;
    let mut info = serde_json::json!({
        "user_ref": user_ref,
    });
    for (vendor_key, canonical_key) in [
        ("name", "display_name"),
        ("real_name", "real_name"),
        ("status_text", "status_text"),
        ("status_emoji", "status_emoji"),
        ("tz", "timezone"),
        ("title", "title"),
    ] {
        if let Some(field_value) = value.get(vendor_key).and_then(serde_json::Value::as_str) {
            info[canonical_key] = serde_json::json!(field_value);
        }
    }
    if let Some(bot) = value.get("bot").and_then(serde_json::Value::as_bool) {
        info["is_bot"] = serde_json::json!(bot);
    }
    if let Some(presence) = value.get("presence").and_then(serde_json::Value::as_str) {
        info["presence"] = serde_json::json!(presence);
    }
    Ok(info)
}

/// The extension-lifecycle profile extended with the invented-vendor fixture:
/// its assets copied into the storage root pre-build (the catalog discovers
/// them), its native factory assembled into the composition input, its tool
/// granted, and its provider trusted — the acme lifecycle then runs through
/// the REAL service (install → activate → dispatch-from-snapshot → remove).
///
/// Builds with a fresh, all-default [`AcmeVendorScript`] — every scenario
/// gets a working built-in response for all 16 standard ops, but cannot
/// script a failure or read the fallback egress's recorded requests. Use
/// [`extension_runtime_acme_tools_profile_with_vendor_script`] for that.
pub(crate) fn extension_runtime_acme_tools_profile() -> HarnessResult<ToolsProfile> {
    Ok(extension_runtime_acme_tools_profile_with_vendor_script(AcmeVendorScript::default())?.0)
}

/// [`extension_runtime_acme_tools_profile`], but the caller supplies (and
/// retains, via the returned handle) the acme vendor script: script per-op
/// responses/failures with [`AcmeVendorScript::respond`]/`fail` before
/// building, then read `ScriptedVendorServer::requests()` (method, URL path,
/// body) off the returned handle after driving a turn — e.g. Task 8's "the
/// send hit api.acme.example/send_message" proofs. This is the SAME
/// `ScriptedVendorServer` `AcmeFixtureToolAdapter::post_acme` falls back to
/// when a real dispatch supplies no `ports.egress` (every production path
/// today), so the recorded requests it captures are the real ones the
/// install → activate → dispatch-from-snapshot pipeline made.
pub(crate) fn extension_runtime_acme_tools_profile_with_vendor_script(
    script: AcmeVendorScript,
) -> HarnessResult<(ToolsProfile, Arc<ScriptedVendorServer>)> {
    let fallback_egress = Arc::new(acme_scripted_vendor_egress(script));
    let mut profile = extension_lifecycle_tools_profile()?;
    profile
        .capability_ids
        .push(ironclaw_host_api::ids::CapabilityId::new(
            ACME_SEND_NOTE_CAPABILITY_ID,
        )?);
    for standard_op_capability_id in ACME_STANDARD_OP_CAPABILITY_IDS {
        profile
            .capability_ids
            .push(ironclaw_host_api::ids::CapabilityId::new(
                *standard_op_capability_id,
            )?);
    }
    // The real Slack package's five tools (TOOL-7 drives them through the
    // generic dispatcher after the install reaches `active`).
    for slack_tool in [
        "slack.search_messages",
        "slack.list_conversations",
        "slack.get_conversation_history",
        "slack.get_user_info",
        "slack.send_message",
    ] {
        profile
            .capability_ids
            .push(ironclaw_host_api::ids::CapabilityId::new(slack_tool)?);
    }
    if let Some(trust) = profile.provider_trust_override.as_mut() {
        trust.push((
            ironclaw_host_api::ids::ExtensionId::new("acme-messenger")?,
            standalone_all_effects(),
        ));
        trust.push((
            ironclaw_host_api::ids::ExtensionId::new("slack")?,
            standalone_all_effects(),
        ));
    }
    profile.options = profile
        .options
        .with_fixture_extension_dir(acme_fixture_dir(), "acme-messenger")
        .with_native_extension_factory(Arc::new(AcmeFixtureFactory {
            fallback_egress: Arc::clone(&fallback_egress),
        }));
    Ok((profile, fallback_egress))
}

pub(crate) async fn extension_runtime_acme_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    extension_runtime_acme_tools_profile()?.build().await
}

// ── Delivery-proof profile (extension-runtime P5, §5.4 / DEL-10) ───────────

/// The bundled telegram manifest's `runtime.service` id — the same native
/// binding the binary assembles (`ironclaw_cli::runtime::native_extensions`).
pub(crate) const TELEGRAM_FIXTURE_SERVICE: &str = "telegram.extension/v1";

/// Native factory for the bundled telegram package: binds the REAL
/// `TelegramChannelAdapter` as its channel surface, exactly like the binary
/// assembly in `crates/app/ironclaw_cli/src/runtime/native_extensions.rs`
/// (mirrored here because the integration harness composes its own runtime
/// and cannot depend on the CLI crate).
struct TelegramFixtureFactory;

/// Hermetic native factory for WebUI/lifecycle tests that install the bundled
/// Telegram package outside the full capability-harness profile.
pub(crate) fn telegram_fixture_factory() -> Arc<dyn ironclaw_extension_host::NativeExtensionFactory>
{
    Arc::new(TelegramFixtureFactory)
}

#[async_trait::async_trait]
impl ironclaw_extension_host::NativeExtensionFactory for TelegramFixtureFactory {
    fn service(&self) -> &str {
        TELEGRAM_FIXTURE_SERVICE
    }

    async fn load(
        &self,
        _ctx: &ironclaw_extension_host::LoadContext,
    ) -> Result<
        Box<dyn ironclaw_extension_host::ExtensionEntrypoint>,
        ironclaw_extension_host::BindError,
    > {
        Ok(Box::new(TelegramFixtureEntrypoint))
    }
}

struct TelegramFixtureEntrypoint;

impl ironclaw_extension_host::ExtensionEntrypoint for TelegramFixtureEntrypoint {
    fn bind(
        &self,
        ctx: ironclaw_extension_host::BindContext,
    ) -> Result<ironclaw_extension_host::ExtensionBindings, ironclaw_extension_host::BindError>
    {
        let tools = Arc::new(super::device_link::LinkedAccountFixtureToolAdapter::new());
        tools.attach_resolver(Arc::clone(&ctx.linked_accounts));
        Ok(ironclaw_extension_host::ExtensionBindings {
            tools: Some(tools as Arc<dyn ironclaw_extension_contracts::tool_adapter::ToolAdapter>),
            channel: {
                let adapter =
                    Arc::new(ironclaw_telegram_extension::TelegramChannelAdapter::default());
                ironclaw_extension_contracts::channel_adapter::ChannelSurfaces::default()
                    .with_ingress(adapter.clone())
                    .with_reply(adapter.clone())
                    .with_delivery(adapter)
            },
            device_link: Some(
                Arc::new(super::device_link::ScriptedDeviceLinkAdapter::new())
                    as Arc<dyn ironclaw_extension_contracts::device_link::DeviceLinkAdapter>,
            ),
        })
    }
}

/// Slack conversation id reserved by the delivery user journeys to script a
/// permanent vendor rejection (`channel_not_found`, mirroring the real
/// vendor error `ironclaw_slack_extension::channel` maps to
/// `PartDeliveryOutcome::Permanent`) instead of the happy-path
/// `chat.postMessage` body below — proves a partial-failure leg without a
/// second, per-test HTTP double. Mirrored (not imported — a separate test
/// binary) as `PARTIAL_FAILING_DM_CHANNEL` in `delivery_user_journeys.rs`.
const DELIVERY_VENDOR_PERMANENT_FAILURE_CHANNEL: &str = "D-JOURNEY-VENDOR-REJECT";

/// Vendor-shaped scripted responses for the delivery proofs: the Slack Web
/// API and the Telegram Bot API answer their happy-path bodies (the adapters
/// parse these for vendor message refs), everything else falls back to the
/// profile's default recorder body.
fn delivery_vendor_router(
    request: &ironclaw_network::NetworkHttpRequest,
) -> Option<(u16, Vec<u8>)> {
    if request.url.ends_with("/api/chat.postMessage") {
        let channel = serde_json::from_slice::<serde_json::Value>(&request.body)
            .ok()
            .and_then(|body| {
                body.get("channel")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "D0000000000".to_string());
        if channel == DELIVERY_VENDOR_PERMANENT_FAILURE_CHANNEL {
            let body = serde_json::json!({
                "ok": false,
                "error": "channel_not_found",
            });
            return Some((200, serde_json::to_vec(&body).ok()?));
        }
        let body = serde_json::json!({
            "ok": true,
            "channel": channel,
            "ts": "1710000200.000001",
        });
        return Some((200, serde_json::to_vec(&body).ok()?));
    }
    if request.url.ends_with("/api/conversations.open") {
        return Some((
            200,
            br#"{"ok":true,"channel":{"id":"D0000000000"}}"#.to_vec(),
        ));
    }
    if request.url.contains("api.telegram.org") {
        if request.url.contains("api.telegram.org/file/") {
            // The manifest's path-prefixed download target streams raw bytes.
            return Some((200, b"DATA".to_vec()));
        }
        let body: &[u8] = if request.url.ends_with("/sendMessage") {
            br#"{"ok":true,"result":{"message_id":4242}}"#
        } else if request.url.ends_with("/getFile") {
            br#"{"ok":true,"result":{"file_path":"documents/report.pdf","file_size":4}}"#
        } else {
            // setWebhook / deleteWebhook and friends return a bool result.
            br#"{"ok":true,"result":true}"#
        };
        return Some((200, body.to_vec()));
    }
    None
}

/// The delivery router plus a scripted transient failure on the FIRST
/// Telegram `getFile` lookup, so the attachment journey can prove the
/// retryable-release-then-refetch ledger semantics on the production mount.
fn delivery_vendor_router_with_flaky_get_file() -> Arc<VendorResponseRouter> {
    let get_file_calls = std::sync::atomic::AtomicUsize::new(0);
    Arc::new(move |request: &ironclaw_network::NetworkHttpRequest| {
        if request.url.contains("api.telegram.org")
            && request.url.ends_with("/getFile")
            && get_file_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0
        {
            return Some((500, br#"{"ok":false,"error_code":500}"#.to_vec()));
        }
        delivery_vendor_router(request)
    })
}

/// The acme runtime profile extended for the §5.4 delivery proofs: the
/// bundled telegram package's native channel factory is assembled (DEL-10
/// activates the REAL bundled manifest through the generic host), telegram's
/// provider is trusted, and the recording network egress answers
/// vendor-shaped bodies so the real adapters can parse delivery responses.
pub(crate) fn extension_delivery_tools_profile() -> HarnessResult<ToolsProfile> {
    let mut profile = extension_runtime_acme_tools_profile()?;
    // Explicit model-initiated delivery (`builtin.outbound_deliver`): the
    // delivery user journeys drive it through this profile's real
    // coordinator → adapter → vendor-router wire.
    profile
        .capability_ids
        .push(ironclaw_host_api::ids::CapabilityId::new(
            ironclaw_host_runtime::OUTBOUND_DELIVER_CAPABILITY_ID,
        )?);
    profile
        .capability_ids
        .push(ironclaw_host_api::ids::CapabilityId::new(
            ironclaw_host_runtime::ATTACH_WORKSPACE_FILE_TO_REPLY_CAPABILITY_ID,
        )?);
    profile.options.mounts = workspace_mounts(MountPermissions::read_only())?;
    if let Some(trust) = profile.provider_trust_override.as_mut() {
        trust.push((
            ironclaw_host_api::ids::ExtensionId::new("telegram")?,
            standalone_all_effects(),
        ));
    }
    let network_egress = Arc::new(
        RecordingNetworkHttpEgress::with_body(br#"{"ok":true}"#.to_vec())
            .with_vendor_router(delivery_vendor_router_with_flaky_get_file()),
    );
    profile.options = profile
        .options
        .with_native_extension_factory(Arc::new(TelegramFixtureFactory))
        .with_channel_extension_binding(slack_channel_extension_binding())
        .with_channel_extension_binding(telegram_channel_extension_binding())
        .with_recording_network_egress(network_egress);
    Ok(profile)
}

/// [`extension_delivery_tools_profile`] PLUS `builtin.write_file`, so a
/// scripted run can raise a REAL `BlockedApproval` gate while the whole
/// channel/coordinator surface (Slack + Telegram adapters, recording vendor
/// egress, delivery coordinator) stays wired.
///
/// This is the only profile where a BACKGROUND run's gate prompt can fan out
/// over real notification channels: `extension_delivery` alone has channels
/// but nothing gateable, and `live_approvals`/`triggers_with_gated_write` have
/// gates but no delivery coordinator.
///
/// Auto-approve stays ON (as everywhere in the delivery chain) so extension
/// lifecycle and delivery verbs dispatch gate-free; the scenario gates ONLY
/// the write with `set_ask_each_time_override_for_test`, which beats global
/// auto-approve (#4776 precedence) — the same mechanism
/// `trigger_management_with_gated_write_profile` uses.
///
/// `options` is MUTATED, never rebuilt: rebuilding it would drop the channel
/// extension bindings, native factories, and recording vendor egress the
/// delivery profile installed.
pub(crate) fn extension_delivery_with_gated_write_tools_profile() -> HarnessResult<ToolsProfile> {
    let mut profile = extension_delivery_tools_profile()?;
    profile
        .capability_ids
        .push(ironclaw_host_api::ids::CapabilityId::new(
            ironclaw_host_runtime::WRITE_FILE_CAPABILITY_ID,
        )?);
    // The lifecycle chain this profile inherits from mounts nothing;
    // `write_file` needs a workspace to write into.
    profile.options.mounts = super::super::workspace_mounts(
        ironclaw_host_api::mount::MountPermissions::read_write_list_delete(),
    )?;
    Ok(profile)
}

/// Slack's channel-adapter binding, mirrored from the binary assembly
/// (`ironclaw_cli::runtime::native_extensions::bundled_channel_extension_bindings`)
/// the same way [`TelegramFixtureFactory`] mirrors the native factory: the
/// harness composes its own runtime and cannot depend on the CLI crate.
/// Slack's WASM-runtime package cannot ride a native factory, so without
/// this binding composition serves its `[channel]` surface with the
/// transitional `HostServedChannelBridge`, which rejects every verified
/// inbound request.
fn slack_channel_extension_binding() -> ironclaw_composition::ChannelExtensionBinding {
    ironclaw_composition::ChannelExtensionBinding {
        extension_id: ironclaw_host_api::ids::ExtensionId::from_trusted("slack".to_string()),
        surfaces: {
            let adapter = Arc::new(ironclaw_slack_extension::SlackChannelAdapter);
            ironclaw_extension_contracts::channel_adapter::ChannelSurfaces::default()
                .with_ingress(adapter.clone())
                .with_reply(adapter.clone())
                .with_delivery(adapter)
        },
        preference_target_codec: Some(Arc::new(
            ironclaw_slack_extension::SlackPreferenceTargetCodec,
        )),
        outbound_target_provider: None,
        first_party_initializer: None,
        registration_document_path: None,
    }
}

fn telegram_channel_extension_binding() -> ironclaw_composition::ChannelExtensionBinding {
    ironclaw_composition::ChannelExtensionBinding {
        extension_id: ironclaw_host_api::ids::ExtensionId::from_trusted("telegram".to_string()),
        surfaces: {
            let adapter = Arc::new(ironclaw_telegram_extension::TelegramChannelAdapter::default());
            ironclaw_extension_contracts::channel_adapter::ChannelSurfaces::default()
                .with_ingress(adapter.clone())
                .with_reply(adapter.clone())
                .with_delivery(adapter)
        },
        // Explicit model-initiated delivery (`builtin.outbound_deliver`)
        // decodes a Telegram target's conversation through THIS codec
        // (`CoordinatedModelChannelDelivery`'s `CodecChannelTargetResolver`
        // is built from every binding's `preference_target_codec` —
        // the production assembly's `target_codecs` filter). Without it a
        // Telegram target's binding ref is undecodable and the tool fails
        // closed as `Internal` before ever reaching the coordinator.
        preference_target_codec: Some(Arc::new(
            ironclaw_telegram_extension::TelegramPreferenceTargetCodec,
        )),
        outbound_target_provider: None,
        first_party_initializer: None,
        registration_document_path: None,
    }
}

pub(crate) async fn extension_delivery_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    extension_delivery_tools_profile()?.build().await
}

/// Push-service endpoint token the web-app delivery journeys script a
/// vendor `410 Gone` for — the "this subscription no longer exists" arm the
/// adapter must prune on. Mirrored (not imported — a separate test binary)
/// in `delivery_user_journeys.rs`.
const WEB_APP_GONE_ENDPOINT_TOKEN: &str = "gone-subscription-token";

/// [`extension_delivery_with_gated_write_tools_profile`] PLUS the complete
/// web-app channel: the deployment binding (adapter + codec + catalog
/// provider), its generic first-party initializer, the bundled package
/// manifest, and the vendor router extended so push-service POSTs answer `201
/// Created` (`410 Gone` for the reserved endpoint token above).
pub(crate) fn extension_delivery_with_web_app_tools_profile() -> HarnessResult<ToolsProfile> {
    let mut profile = extension_delivery_with_gated_write_tools_profile()?;
    let network_egress = Arc::new(
        RecordingNetworkHttpEgress::with_body(br#"{"ok":true}"#.to_vec())
            .with_vendor_router(web_app_delivery_vendor_router()),
    );
    profile.options = profile
        .options
        .with_web_app_channel_extension()
        .with_recording_network_egress(network_egress);
    Ok(profile)
}

/// The delivery vendor router extended for push services: any POST to an
/// endpoint on the web-app manifest's declared hosts answers the way a
/// real push service does — `201 Created` with an empty body (RFC 8030), or
/// `410 Gone` for the reserved dead-subscription token.
fn web_app_delivery_vendor_router() -> Arc<VendorResponseRouter> {
    Arc::new(move |request: &ironclaw_network::NetworkHttpRequest| {
        if request.url.starts_with("https://fcm.googleapis.com/") {
            if request.url.contains(WEB_APP_GONE_ENDPOINT_TOKEN) {
                return Some((410, Vec::new()));
            }
            return Some((201, Vec::new()));
        }
        delivery_vendor_router(request)
    })
}

// ── Standard messaging op conformance (standardized messaging framework,
//    task 7) ────────────────────────────────────────────────────────────────
//
// Drives `AcmeFixtureToolAdapter::invoke` directly (bypassing the generic
// dispatch pipeline entirely — no registry, no resolver, no authorization),
// against a scripted vendor server standing in for `api.acme.example` behind
// `ToolPorts.egress`. This is the adapter's own conformance/error-taxonomy
// proof; Task 8's integration scenarios separately drive these ops through
// the full turn/dispatch pipeline.
#[cfg(test)]
pub(crate) mod standard_op_contract_tests {
    use ironclaw_extension_contracts::tool_adapter::{
        RestrictedEgressRequest, RestrictedEgressResponse, ToolAdapter, ToolCall, ToolError,
        ToolPorts, ToolResult,
    };
    use ironclaw_host_api::action::NetworkMethod;
    use ironclaw_host_api::ids::{
        AgentId, CapabilityId, InvocationId, ProjectId, TenantId, UserId,
    };
    use ironclaw_host_api::messaging::{StandardMessagingErrorCode, StandardMessagingOp};
    use ironclaw_host_api::resource::ResourceScope;
    use ironclaw_host_api::test_support::messaging_conformance::{
        assert_canonical_input_accepted, assert_canonical_output, message_ref_from_output,
    };
    use serde_json::json;
    use std::sync::Arc;

    use super::*;

    fn test_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-acme-standard-ops").expect("tenant id"),
            user_id: UserId::new("user-acme-standard-ops").expect("user id"),
            agent_id: Some(AgentId::new("agent-acme-standard-ops").expect("agent id")),
            project_id: Some(ProjectId::new("project-acme-standard-ops").expect("project id")),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    /// A scripted vendor that always answers `200` with `body`, regardless of
    /// which op it was asked for — one per happy-path call in these tests.
    fn vendor_ok(body: serde_json::Value) -> ScriptedVendorServer {
        ScriptedVendorServer::new(Arc::new(move |_request: &RestrictedEgressRequest| {
            RestrictedEgressResponse {
                status: 200,
                body: serde_json::to_vec(&body).expect("fixture body serializes"),
            }
        }))
    }

    /// A scripted vendor that always answers a non-2xx `{"error": vendor_code}`
    /// — the acme vendor's invented failure shape.
    fn vendor_err(vendor_code: &'static str) -> ScriptedVendorServer {
        ScriptedVendorServer::new(Arc::new(move |_request: &RestrictedEgressRequest| {
            RestrictedEgressResponse {
                status: 400,
                body: serde_json::to_vec(&json!({ "error": vendor_code }))
                    .expect("fixture body serializes"),
            }
        }))
    }

    /// A fresh adapter with a throwaway constructor-held fallback egress —
    /// irrelevant to every test here except the fallback proof below (which
    /// builds its own adapter directly, so it controls the fallback), since
    /// `invoke_acme` always supplies `ports.egress = Some(vendor)`.
    fn test_adapter() -> AcmeFixtureToolAdapter {
        AcmeFixtureToolAdapter {
            fallback_egress: Arc::new(acme_scripted_vendor_egress(AcmeVendorScript::default())),
        }
    }

    async fn invoke_acme(
        op_name: &str,
        input: serde_json::Value,
        vendor: &ScriptedVendorServer,
    ) -> Result<ToolResult, ToolError> {
        let capability_id =
            CapabilityId::new(format!("acme-messenger.{op_name}")).expect("capability id");
        let call = ToolCall::new(capability_id, test_scope(), input);
        let ports = ToolPorts {
            egress: Some(vendor),
        };
        test_adapter().invoke(call, &ports).await
    }

    /// The 16-op conformance loop: for each core op, build the canonical
    /// happy-path input (asserted accepted by the Task 1 registry first),
    /// invoke the adapter against a scripted vendor response, then assert the
    /// output against the canonical output schema (Task 6 helpers). The
    /// evidence loop — send's `message_ref` feeds edit/delete/react — is
    /// exercised explicitly rather than inventing a fresh ref per op.
    pub(crate) async fn acme_standard_ops_satisfy_canonical_contracts() {
        let send_input = json!({ "conversation": "ACME-C-1", "text": "hello" });
        assert_canonical_input_accepted(StandardMessagingOp::SendMessage, &send_input);
        let vendor = vendor_ok(json!({ "id": "AMSG-1" }));
        let send_output = invoke_acme("send_message", send_input, &vendor)
            .await
            .expect("send_message succeeds")
            .output;
        assert_canonical_output(StandardMessagingOp::SendMessage, &send_output);
        // One POST, to the op-named path, real teeth on `post_acme`'s URL
        // construction (a scripted vendor answers any URL the same way, so
        // only reading back the recorded request proves this).
        let requests = vendor.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "https://api.acme.example/send_message");
        assert!(matches!(requests[0].method, NetworkMethod::Post));
        let message_ref = message_ref_from_output(&send_output);

        // W3/W4 (pre-merge amendment wave): `thread` (a thread/topic
        // container) and `reply_to` (a quoted message_ref) are distinct,
        // both accepted, both forwarded to the vendor, and both echoed back
        // on the output when supplied — so a silent drop is checkable.
        let reply_to_ref = json!({ "conversation": "ACME-C-1", "message_id": "AMSG-QUOTED-1" });
        let send_with_thread_and_reply_to_input = json!({
            "conversation": "ACME-C-1",
            "text": "hello again",
            "thread": "AMSG-THREAD-1",
            "reply_to": reply_to_ref,
        });
        assert_canonical_input_accepted(
            StandardMessagingOp::SendMessage,
            &send_with_thread_and_reply_to_input,
        );
        let vendor = vendor_ok(json!({ "id": "AMSG-2" }));
        let send_with_thread_and_reply_to_output =
            invoke_acme("send_message", send_with_thread_and_reply_to_input, &vendor)
                .await
                .expect("send_message with thread+reply_to succeeds")
                .output;
        assert_canonical_output(
            StandardMessagingOp::SendMessage,
            &send_with_thread_and_reply_to_output,
        );
        assert_eq!(
            send_with_thread_and_reply_to_output["thread"],
            json!("AMSG-THREAD-1"),
            "thread must echo back on the output: {send_with_thread_and_reply_to_output}"
        );
        assert_eq!(
            send_with_thread_and_reply_to_output["reply_to"], reply_to_ref,
            "reply_to must echo back on the output: {send_with_thread_and_reply_to_output}"
        );
        let forward_request = vendor
            .requests()
            .into_iter()
            .find(|request| request.url.contains("send_message"))
            .expect("send_message must call the vendor");
        let forward_body: serde_json::Value =
            serde_json::from_slice(forward_request.body.as_deref().expect("request body"))
                .expect("request body is JSON");
        assert_eq!(
            forward_body["thread"],
            json!("AMSG-THREAD-1"),
            "thread must forward to the vendor: {forward_body}"
        );
        assert_eq!(
            forward_body["reply_to"], reply_to_ref,
            "reply_to must forward to the vendor: {forward_body}"
        );

        let edit_input = json!({ "message_ref": message_ref.clone(), "text": "hello, edited" });
        assert_canonical_input_accepted(StandardMessagingOp::EditMessage, &edit_input);
        let vendor = vendor_ok(json!({ "ok": true }));
        let edit_output = invoke_acme("edit_message", edit_input, &vendor)
            .await
            .expect("edit_message succeeds")
            .output;
        assert_canonical_output(StandardMessagingOp::EditMessage, &edit_output);

        let delete_input = json!({ "message_ref": message_ref.clone() });
        assert_canonical_input_accepted(StandardMessagingOp::DeleteMessage, &delete_input);
        let vendor = vendor_ok(json!({ "ok": true }));
        let delete_output = invoke_acme("delete_message", delete_input, &vendor)
            .await
            .expect("delete_message succeeds")
            .output;
        assert_canonical_output(StandardMessagingOp::DeleteMessage, &delete_output);

        for (op_name, op) in [
            ("add_reaction", StandardMessagingOp::AddReaction),
            ("remove_reaction", StandardMessagingOp::RemoveReaction),
        ] {
            let input = json!({ "message_ref": message_ref.clone(), "emoji": "tada" });
            assert_canonical_input_accepted(op, &input);
            let vendor = vendor_ok(json!({ "ok": true }));
            let output = invoke_acme(op_name, input, &vendor)
                .await
                .unwrap_or_else(|error| panic!("{op_name} succeeds: {error:?}"))
                .output;
            assert_canonical_output(op, &output);
        }

        // W5 (pre-merge amendment wave): remove_reaction's `emoji` is
        // optional — absent means "remove the connected account's own
        // reaction(s)" (some vendors cannot name the emoji on removal). The
        // canonical output must omit `emoji` entirely rather than fabricate
        // one when it wasn't supplied. `add_reaction` is unaffected — its
        // `emoji` stays required (asserted in the loop above).
        let remove_reaction_without_emoji_input = json!({ "message_ref": message_ref.clone() });
        assert_canonical_input_accepted(
            StandardMessagingOp::RemoveReaction,
            &remove_reaction_without_emoji_input,
        );
        let vendor = vendor_ok(json!({ "ok": true }));
        let remove_reaction_without_emoji_output = invoke_acme(
            "remove_reaction",
            remove_reaction_without_emoji_input,
            &vendor,
        )
        .await
        .expect("remove_reaction without emoji succeeds")
        .output;
        assert_canonical_output(
            StandardMessagingOp::RemoveReaction,
            &remove_reaction_without_emoji_output,
        );
        assert!(
            remove_reaction_without_emoji_output.get("emoji").is_none(),
            "emoji must be omitted, not fabricated, when absent on input: {remove_reaction_without_emoji_output}"
        );

        let open_dm_input = json!({ "user_ref": "U1" });
        assert_canonical_input_accepted(StandardMessagingOp::OpenDm, &open_dm_input);
        let vendor = vendor_ok(json!({ "conversation": "ACME-C-DM-1" }));
        let open_dm_output = invoke_acme("open_dm", open_dm_input, &vendor)
            .await
            .expect("open_dm succeeds")
            .output;
        assert_canonical_output(StandardMessagingOp::OpenDm, &open_dm_output);

        let list_conversations_input = json!({});
        assert_canonical_input_accepted(
            StandardMessagingOp::ListConversations,
            &list_conversations_input,
        );
        let vendor = vendor_ok(json!({
            "conversations": [
                { "conversation": "ACME-C-1", "kind": "channel", "name": "general", "member": true },
                { "conversation": "ACME-C-DM-1", "kind": "dm", "counterpart_ref": "U1", "counterpart_name": "Ann" },
            ],
            "next_cursor": "cursor-1",
        }));
        let list_conversations_output =
            invoke_acme("list_conversations", list_conversations_input, &vendor)
                .await
                .expect("list_conversations succeeds")
                .output;
        assert_canonical_output(
            StandardMessagingOp::ListConversations,
            &list_conversations_output,
        );

        let get_conversation_info_input = json!({ "conversation": "ACME-C-1" });
        assert_canonical_input_accepted(
            StandardMessagingOp::GetConversationInfo,
            &get_conversation_info_input,
        );
        let vendor = vendor_ok(
            json!({ "conversation": "ACME-C-1", "kind": "channel", "name": "general", "member": true }),
        );
        let get_conversation_info_output = invoke_acme(
            "get_conversation_info",
            get_conversation_info_input,
            &vendor,
        )
        .await
        .expect("get_conversation_info succeeds")
        .output;
        assert_canonical_output(
            StandardMessagingOp::GetConversationInfo,
            &get_conversation_info_output,
        );

        let get_conversation_history_input = json!({ "conversation": "ACME-C-1" });
        assert_canonical_input_accepted(
            StandardMessagingOp::GetConversationHistory,
            &get_conversation_history_input,
        );
        let vendor = vendor_ok(json!({
            "messages": [
                { "conversation": "ACME-C-1", "message_id": "AMSG-1", "author_ref": "U1",
                  "author_name": "Ann", "text": "hi", "ts": "2026-07-27T00:00:00Z", "self": false },
            ],
        }));
        let get_conversation_history_output = invoke_acme(
            "get_conversation_history",
            get_conversation_history_input,
            &vendor,
        )
        .await
        .expect("get_conversation_history succeeds")
        .output;
        assert_canonical_output(
            StandardMessagingOp::GetConversationHistory,
            &get_conversation_history_output,
        );

        let get_thread_replies_input = json!({ "conversation": "ACME-C-1", "thread": "AMSG-1" });
        assert_canonical_input_accepted(
            StandardMessagingOp::GetThreadReplies,
            &get_thread_replies_input,
        );
        let vendor = vendor_ok(json!({
            "messages": [
                { "conversation": "ACME-C-1", "message_id": "AMSG-2", "author_ref": "U2",
                  "text": "reply", "self": true, "thread": "AMSG-1", "reply_count": 1 },
            ],
        }));
        let get_thread_replies_output =
            invoke_acme("get_thread_replies", get_thread_replies_input, &vendor)
                .await
                .expect("get_thread_replies succeeds")
                .output;
        assert_canonical_output(
            StandardMessagingOp::GetThreadReplies,
            &get_thread_replies_output,
        );

        let get_message_input = json!({ "message_ref": message_ref.clone() });
        assert_canonical_input_accepted(StandardMessagingOp::GetMessage, &get_message_input);
        let vendor = vendor_ok(json!({
            "conversation": "ACME-C-1", "message_id": "AMSG-1", "author_ref": "U1",
            "text": "hi", "self": true,
        }));
        let get_message_output = invoke_acme("get_message", get_message_input, &vendor)
            .await
            .expect("get_message succeeds")
            .output;
        assert_canonical_output(StandardMessagingOp::GetMessage, &get_message_output);

        let search_messages_input = json!({ "query": "hi" });
        assert_canonical_input_accepted(
            StandardMessagingOp::SearchMessages,
            &search_messages_input,
        );
        let vendor = vendor_ok(json!({
            "matches": [
                { "conversation": "ACME-C-1", "message_id": "AMSG-1", "author_ref": "U1",
                  "text": "hi", "self": true },
            ],
            "total": 1,
        }));
        let search_messages_output = invoke_acme("search_messages", search_messages_input, &vendor)
            .await
            .expect("search_messages succeeds")
            .output;
        assert_canonical_output(StandardMessagingOp::SearchMessages, &search_messages_output);

        let get_user_info_input = json!({ "user_ref": "U1" });
        assert_canonical_input_accepted(StandardMessagingOp::GetUserInfo, &get_user_info_input);
        let vendor = vendor_ok(
            json!({ "user_ref": "U1", "name": "Ann", "bot": false, "presence": "active" }),
        );
        let get_user_info_output = invoke_acme("get_user_info", get_user_info_input, &vendor)
            .await
            .expect("get_user_info succeeds")
            .output;
        assert_canonical_output(StandardMessagingOp::GetUserInfo, &get_user_info_output);

        let resolve_user_input = json!({ "query": "ann" });
        assert_canonical_input_accepted(StandardMessagingOp::ResolveUser, &resolve_user_input);
        let vendor = vendor_ok(json!({ "matches": [{ "user_ref": "U1", "name": "Ann" }] }));
        let resolve_user_output = invoke_acme("resolve_user", resolve_user_input, &vendor)
            .await
            .expect("resolve_user succeeds")
            .output;
        assert_canonical_output(StandardMessagingOp::ResolveUser, &resolve_user_output);

        let list_members_input = json!({ "conversation": "ACME-C-1" });
        assert_canonical_input_accepted(StandardMessagingOp::ListMembers, &list_members_input);
        let vendor = vendor_ok(json!({ "members": [{ "user_ref": "U1", "name": "Ann" }] }));
        let list_members_output = invoke_acme("list_members", list_members_input, &vendor)
            .await
            .expect("list_members succeeds")
            .output;
        assert_canonical_output(StandardMessagingOp::ListMembers, &list_members_output);

        let whoami_input = json!({});
        assert_canonical_input_accepted(StandardMessagingOp::Whoami, &whoami_input);
        let vendor = vendor_ok(json!({ "user_ref": "U-SELF", "name": "Acme Bot" }));
        let whoami_output = invoke_acme("whoami", whoami_input, &vendor)
            .await
            .expect("whoami succeeds")
            .output;
        assert_canonical_output(StandardMessagingOp::Whoami, &whoami_output);
    }

    /// All 12 standard messaging error codes, via a table of (scripted
    /// vendor code -> expected code) driven through one representative op
    /// (`send_message`) — `acme_error_to_standard_code`/`post_acme`'s
    /// mapping is shared verbatim by all 16 arms, so this proves the shared
    /// logic once rather than 16 times.
    pub(crate) async fn acme_standard_ops_emit_canonical_error_codes() {
        let cases: &[(&str, StandardMessagingErrorCode)] = &[
            (
                "conversation_missing",
                StandardMessagingErrorCode::UnknownConversation,
            ),
            (
                "message_missing",
                StandardMessagingErrorCode::UnknownMessage,
            ),
            ("user_missing", StandardMessagingErrorCode::UnknownUser),
            ("not_member", StandardMessagingErrorCode::NotAMember),
            ("forbidden", StandardMessagingErrorCode::PermissionDenied),
            ("dm_closed", StandardMessagingErrorCode::CannotMessageUser),
            (
                "window_closed",
                StandardMessagingErrorCode::OutsideMessagingWindow,
            ),
            ("too_long", StandardMessagingErrorCode::MessageTooLong),
            (
                "bad_content",
                StandardMessagingErrorCode::UnsupportedContent,
            ),
            ("slow_down", StandardMessagingErrorCode::RateLimited),
            ("edit_locked", StandardMessagingErrorCode::EditNotAllowed),
            (
                "something_acme_never_documented",
                StandardMessagingErrorCode::VendorError,
            ),
        ];
        assert_eq!(cases.len(), StandardMessagingErrorCode::ALL.len());

        for (vendor_code, expected) in cases {
            let vendor = vendor_err(vendor_code);
            let input = json!({ "conversation": "ACME-C-1", "text": "hello" });
            let error = invoke_acme("send_message", input, &vendor)
                .await
                .expect_err("a non-2xx vendor response must surface as an error");
            let ToolError::Failed { safe_summary, .. } = error else {
                panic!("expected ToolError::Failed for vendor code {vendor_code}");
            };
            let summary = safe_summary.expect("acme vendor errors carry a safe summary");
            assert!(
                summary.contains(expected.as_str()),
                "vendor code {vendor_code}: expected {summary:?} to contain {}",
                expected.as_str()
            );
        }
    }

    /// The fallback proof (fix round: `ToolPorts.egress` is `None` on every
    /// production dispatch path today — see the module doc above
    /// `AcmeVendorOutcome`). Builds an adapter with its OWN constructor-held
    /// scripted vendor egress (not one supplied via `ports`, unlike every
    /// test above), invokes it with `ToolPorts { egress: None }` — the exact
    /// shape a real production dispatch hands in — and asserts both that the
    /// canonical output still comes back (the fallback egress served the
    /// call) and that the SAME fallback instance recorded the request
    /// (method, URL, body all readable off it), proving it is genuinely the
    /// egress the arm used and not a coincidentally-matching default.
    pub(crate) async fn acme_standard_ops_fall_back_to_the_constructor_held_vendor_when_ports_lack_egress()
     {
        let script = AcmeVendorScript::default();
        script.respond("send_message", json!({ "id": "AMSG-FALLBACK-1" }));
        let fallback_egress = Arc::new(acme_scripted_vendor_egress(script));
        let adapter = AcmeFixtureToolAdapter {
            fallback_egress: Arc::clone(&fallback_egress),
        };

        let input = json!({ "conversation": "ACME-C-1", "text": "hello via fallback" });
        assert_canonical_input_accepted(StandardMessagingOp::SendMessage, &input);
        let capability_id =
            CapabilityId::new("acme-messenger.send_message").expect("capability id");
        let call = ToolCall::new(capability_id, test_scope(), input);
        let ports = ToolPorts { egress: None };

        let output = adapter
            .invoke(call, &ports)
            .await
            .expect("the fallback egress serves the call when ports.egress is None")
            .output;
        assert_canonical_output(StandardMessagingOp::SendMessage, &output);
        let message_ref = message_ref_from_output(&output);
        assert_eq!(message_ref["message_id"], "AMSG-FALLBACK-1");

        let requests = fallback_egress.requests();
        assert_eq!(
            requests.len(),
            1,
            "the constructor-held fallback, not some other egress, must have served the call"
        );
        assert_eq!(requests[0].url, "https://api.acme.example/send_message");
        assert!(matches!(requests[0].method, NetworkMethod::Post));
        let body: serde_json::Value =
            serde_json::from_slice(requests[0].body.as_deref().expect("request body"))
                .expect("request body is JSON");
        assert_eq!(body["conversation"], "ACME-C-1");
        assert_eq!(body["text"], "hello via fallback");
    }
}
