//! Extension domain tools profiles.

use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel, CredentialAccountStatus,
    CredentialOwnership, NewCredentialAccount, ProviderScope,
};
use ironclaw_host_api::{
    AgentId, InvocationId, MountView, ProjectId, ResourceScope, SecretHandle, TenantId, UserId,
};

use std::sync::Arc;

use super::super::super::extension_surface::{
    EXTENSION_LIFECYCLE_CAPABILITY_IDS, bundled_extension_manifest_capability_ids,
};
use super::super::super::github;
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
        effect_kinds: local_dev_all_effects(),
        options: HostRuntimeHarnessOptions::new(
            MountView::default(),
            Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                true,
            )?),
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
        "tools/call" if is_nearai => serde_json::json!({
            "content": [{
                "type": "text",
                "text": "REBORN_NEARAI_WEB_SEARCH_RESULT"
            }]
        }),
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
    services: &ironclaw_reborn_composition::RebornRuntime,
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
            ],
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
            channel: Some(Arc::new(AcmeFixtureChannelAdapter)),
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
impl ironclaw_product::ChannelAdapter for AcmeFixtureChannelAdapter {
    fn inbound(
        &self,
        request: ironclaw_product::VerifiedInbound<'_>,
    ) -> Result<ironclaw_product::InboundOutcome, ironclaw_product::ChannelError> {
        use ironclaw_product::{
            ChannelError, ExternalActorRef, ExternalConversationRef, ExternalEventId,
            ImmediateResponse, InboundOutcome, NormalizedInboundMessage, ProductTriggerReason,
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
                    reply_context: Some(b"acme-reply-route".to_vec()),
                }]))
            }
            _ => Ok(InboundOutcome::Ignore),
        }
    }

    /// Minimal real outbound: one vendor POST per text part. Proves the
    /// generic delivery path (coordinator → adapter → restricted egress)
    /// needs no real product, and gives the conformance suite a deliverable
    /// fixture.
    async fn deliver(
        &self,
        envelope: ironclaw_product::OutboundEnvelope,
        egress: &dyn ironclaw_host_api::RestrictedEgress,
    ) -> Result<ironclaw_product::DeliveryReport, ironclaw_product::ChannelError> {
        use ironclaw_product::{ChannelError, OutboundPart, PartDeliveryOutcome};
        if envelope.parts.is_empty() {
            return Err(ChannelError::Render {
                reason: "outbound envelope carries no parts".to_string(),
            });
        }
        let mut parts = Vec::new();
        for part in &envelope.parts {
            let outcome = match part {
                OutboundPart::Text(text) => {
                    let body = serde_json::json!({
                        "conversation": envelope.target.conversation.conversation_id(),
                        "text": text,
                    });
                    let response = egress
                        .send(ironclaw_host_api::RestrictedEgressRequest {
                            method: ironclaw_host_api::NetworkMethod::Post,
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
        Ok(ironclaw_product::DeliveryReport { parts })
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
                model_visible_cause: None,
            }),
        }
    }
}

/// The extension-lifecycle profile extended with the invented-vendor fixture:
/// its assets copied into the storage root pre-build (the catalog discovers
/// them), its native factory assembled into the composition input, its tool
/// granted, and its provider trusted — the acme lifecycle then runs through
/// the REAL facade (install → dispatch-from-snapshot → remove).
pub(crate) fn extension_runtime_acme_tools_profile() -> HarnessResult<ToolsProfile> {
    let mut profile = extension_lifecycle_tools_profile()?;
    profile
        .capability_ids
        .push(ironclaw_host_api::CapabilityId::new(
            ACME_SEND_NOTE_CAPABILITY_ID,
        )?);
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

// ── Delivery-proof profile (extension-runtime P5, §5.4 / DEL-10) ───────────

/// The bundled telegram manifest's `runtime.service` id — the same native
/// binding the binary assembles (`ironclaw_reborn_cli::runtime::native_extensions`).
pub(crate) const TELEGRAM_FIXTURE_SERVICE: &str = "telegram.extension/v1";

/// Native factory for the bundled telegram package: binds the REAL
/// `TelegramChannelAdapter` as its channel surface, exactly like the binary
/// assembly in `crates/ironclaw_reborn_cli/src/runtime/native_extensions.rs`
/// (mirrored here because the integration harness composes its own runtime
/// and cannot depend on the CLI crate).
struct TelegramFixtureFactory;

impl ironclaw_extension_host::NativeExtensionFactory for TelegramFixtureFactory {
    fn service(&self) -> &str {
        TELEGRAM_FIXTURE_SERVICE
    }

    fn load(
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
        _ctx: ironclaw_extension_host::BindContext,
    ) -> Result<ironclaw_extension_host::ExtensionBindings, ironclaw_extension_host::BindError>
    {
        Ok(ironclaw_extension_host::ExtensionBindings {
            tools: None,
            channel: Some(Arc::new(
                ironclaw_telegram_extension::TelegramChannelAdapter::default(),
            )),
        })
    }
}

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
        let body: &[u8] = if request.url.ends_with("/sendMessage") {
            br#"{"ok":true,"result":{"message_id":4242}}"#
        } else {
            // setWebhook / deleteWebhook and friends return a bool result.
            br#"{"ok":true,"result":true}"#
        };
        return Some((200, body.to_vec()));
    }
    None
}

/// The acme runtime profile extended for the §5.4 delivery proofs: the
/// bundled telegram package's native channel factory is assembled (DEL-10
/// activates the REAL bundled manifest through the generic host), telegram's
/// provider is trusted, and the recording network egress answers
/// vendor-shaped bodies so the real adapters can parse delivery responses.
pub(crate) fn extension_delivery_tools_profile() -> HarnessResult<ToolsProfile> {
    let mut profile = extension_runtime_acme_tools_profile()?;
    profile
        .capability_ids
        .push(ironclaw_host_api::CapabilityId::new(
            ironclaw_host_runtime::OUTBOUND_DELIVERY_TARGET_ROUTE_CURRENT_CAPABILITY_ID,
        )?);
    if let Some(trust) = profile.provider_trust_override.as_mut() {
        trust.push((
            ironclaw_host_api::ExtensionId::new("telegram")?,
            local_dev_all_effects(),
        ));
    }
    let network_egress = Arc::new(
        RecordingNetworkHttpEgress::with_body(br#"{"ok":true}"#.to_vec())
            .with_vendor_router(Arc::new(delivery_vendor_router)),
    );
    profile.options = profile
        .options
        .with_native_extension_factory(Arc::new(TelegramFixtureFactory))
        .with_channel_extension_binding(slack_channel_extension_binding())
        .with_channel_extension_binding(telegram_channel_extension_binding())
        .with_recording_network_egress(network_egress);
    Ok(profile)
}

/// Slack's channel-adapter binding, mirrored from the binary assembly
/// (`ironclaw_reborn_cli::runtime::native_extensions::bundled_channel_extension_bindings`)
/// the same way [`TelegramFixtureFactory`] mirrors the native factory: the
/// harness composes its own runtime and cannot depend on the CLI crate.
/// Slack's WASM-runtime package cannot ride a native factory, so without
/// this binding composition serves its `[channel]` surface with the
/// transitional `HostServedChannelBridge`, which rejects every verified
/// inbound request.
fn slack_channel_extension_binding() -> ironclaw_reborn_composition::ChannelExtensionBinding {
    ironclaw_reborn_composition::ChannelExtensionBinding {
        extension_id: "slack".to_string(),
        adapter: Arc::new(ironclaw_slack_extension::SlackChannelAdapter),
        preference_target_codec: Some(Arc::new(
            ironclaw_slack_extension::SlackPreferenceTargetCodec,
        )),
    }
}

fn telegram_channel_extension_binding() -> ironclaw_reborn_composition::ChannelExtensionBinding {
    ironclaw_reborn_composition::ChannelExtensionBinding {
        extension_id: "telegram".to_string(),
        adapter: Arc::new(ironclaw_telegram_extension::TelegramChannelAdapter::default()),
        preference_target_codec: None,
    }
}

pub(crate) async fn extension_delivery_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    extension_delivery_tools_profile()?.build().await
}
