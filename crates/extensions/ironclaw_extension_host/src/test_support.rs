//! Shared test fixtures: resolved-manifest builders and scripted adapters.
//!
//! Available to this crate's own tests and to downstream integration tests
//! (behind the crate's default build — these are lightweight fakes, not a
//! feature-gated seam) so the acme fixture and the state-machine contract
//! tests share one construction path.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_extension_contracts::channel_adapter::ChannelAdapter;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelContext, ChannelError, DeliveryReport, InboundOutcome, OutboundEnvelope, VerifiedInbound,
};
use ironclaw_extension_contracts::device_link::{
    DeviceLinkAdapter, DeviceLinkContext, DeviceLinkError, DeviceLinkFlowId, DeviceLinkInput,
    DeviceLinkInputKind, DeviceLinkMode, DeviceLinkStep,
};
use ironclaw_extension_contracts::linked_session::{
    LinkedAccountGrant, LinkedSessionVersion, SessionBytes,
};
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
    ToolAdapter, ToolCall, ToolError, ToolPorts, ToolResult,
};
use ironclaw_extension_registry::{
    ExtensionManifestRecord, ManifestSource, ResolvedExtensionManifest,
};
use ironclaw_host_api::host_port::{
    HOST_RUNTIME_HTTP_EGRESS_PORT_ID, HostPortCatalog, HostPortCatalogEntry, HostPortId,
};
use ironclaw_host_api::ids::{ExtensionId, UserId};

use crate::entrypoint::{BindContext, BindError, ExtensionBindings, ExtensionEntrypoint};
use crate::lifecycle::{DrainController, EgressFactory, HookError};
use crate::loaders::{ExtensionLoader, LoadContext, LoadedExtension};

#[cfg(feature = "test-support")]
pub mod first_party_registrars;

/// Opaque test-support handle carrying the Reborn local extension-management
/// port without forcing composition harness structs to define extension-host
/// wrapper types locally.
#[cfg(feature = "test-support")]
pub struct ExtensionManagementTestHandle {
    extension_management: Arc<crate::extension_lifecycle::RebornLocalExtensionManagementPort>,
}

#[cfg(feature = "test-support")]
impl ExtensionManagementTestHandle {
    /// Build a test-support handle over the local extension-management port.
    pub fn new(
        extension_management: Arc<crate::extension_lifecycle::RebornLocalExtensionManagementPort>,
    ) -> Self {
        Self {
            extension_management,
        }
    }

    /// Return the wrapped local extension-management port.
    pub fn extension_management(
        &self,
    ) -> Arc<crate::extension_lifecycle::RebornLocalExtensionManagementPort> {
        self.extension_management.clone()
    }
}

const MCP_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acme-tools"
name = "Acme Tools"
version = "0.1.0"
description = "fixture: hosted MCP tools"
trust = "third_party"

[mcp]
server = "https://mcp.acme.example/mcp"
namespace = "acme-tools"
max_tools = 32
default_permission = "ask"
effects = ["network", "use_secret"]

[[mcp.credentials]]
handle = "acme_tools_account"
vendor = "acme-tools"
scopes = ["read"]
injection = { type = "header", name = "authorization", prefix = "Bearer " }

[auth.acme-tools]
method = "oauth2_code"
display_name = "Acme Tools account"
authorization_endpoint = "https://auth.acme.example/authorize"
token_endpoint = "https://auth.acme.example/token"
scopes = ["read"]
client_credentials = { client_id_handle = "acme_tools_client_id" }

[auth.acme-tools.token_response]
access_token = "/access_token"
"#;

const OUTBOUND_ONLY_CHANNEL_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acme-push"
name = "Acme Push"
version = "0.1.0"
description = "fixture: outbound-only channel extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/acme_push.wasm"

[channel]
id = "notifications"
display_name = "Acme push"
inbound = false
outbound = true
conversation_model = "continuous"
"#;

const CHANNEL_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acme-chat"
name = "Acme Chat"
version = "0.1.0"
description = "fixture: channel-only extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/acme_chat.wasm"

[channel]
id = "messages"
display_name = "Acme chat"
inbound = true
outbound = true
conversation_model = "continuous"

[channel.ingress]
route_suffix = "events"
method = "post"
body_limit_bytes = 1048576

[channel.ingress.verification]
kind = "hmac_sha256"
secret_handle = "acme_chat_signing_secret"
signature_header = "X-Acme-Signature"
signed_payload = [ { body = true } ]

[admin_configuration]
group_id = "acme.chat"
display_name = "Acme Chat channel"
fields = [ { handle = "acme_chat_signing_secret", label = "Signing secret", secret = true } ]

[[channel.egress]]
scheme = "https"
host = "api.acme.example"
methods = ["post"]
"#;

const DEVICE_LINK_CHANNEL_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acme-link"
name = "Acme Link"
version = "0.1.0"
description = "fixture: channel + device-link auth"
trust = "third_party"

[runtime]
kind = "first_party"
service = "acme-link"

[[tools]]
id = "acme-link.whoami"
description = "Report the linked account."
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/acme-link/whoami.input.v1.json"

[[tools.credentials]]
handle = "acme_link_session"
vendor = "acme-link"
scopes = ["session"]
audience = { scheme = "https", host = "api.acme.example" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }

[channel]
id = "messages"
display_name = "Acme link messages"
inbound = false
outbound = true
conversation_model = "continuous"

[auth.acme-link]
method = "device_link"
display_name = "Acme personal account"
default_mode_label = "Scan a code"
instructions = "Open Acme on your phone and scan the code."
"#;

const TOOL_AND_CHANNEL_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acme"
name = "Acme"
version = "0.1.0"
description = "fixture: tool + channel + auth"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/acme.wasm"

[[tools]]
id = "acme.ping"
description = "Ping the vendor."
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/acme/ping.input.v1.json"

[[tools.credentials]]
handle = "acme_token"
vendor = "acme"
scopes = ["ping"]
audience = { scheme = "https", host = "api.acme.example" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }

[channel]
id = "messages"
display_name = "Acme messages"
inbound = true
outbound = true
conversation_model = "continuous"

[channel.ingress]
route_suffix = "hooks"
method = "post"
body_limit_bytes = 1048576

[channel.ingress.verification]
kind = "hmac_sha256"
secret_handle = "acme_signing_secret"
signature_header = "X-Acme-Signature"
signed_payload = [ { body = true } ]

[admin_configuration]
group_id = "acme.channel"
display_name = "Acme channel"
fields = [ { handle = "acme_signing_secret", label = "Signing secret", secret = true } ]

[[channel.egress]]
scheme = "https"
host = "api.acme.example"
methods = ["post"]

[auth.acme]
method = "oauth2_code"
display_name = "Acme account"
authorization_endpoint = "https://auth.acme.example/authorize"
token_endpoint = "https://auth.acme.example/token"
scopes = ["ping"]
client_credentials = { client_id_handle = "acme_client_id" }

[auth.acme.token_response]
access_token = "/access_token"
"#;

fn catalog() -> HostPortCatalog {
    HostPortCatalog::new(vec![HostPortCatalogEntry::new(
        HostPortId::new(HOST_RUNTIME_HTTP_EGRESS_PORT_ID).unwrap(),
    )])
    .unwrap()
}

/// Resolve an arbitrary v2/v3 manifest through the production parser (test
/// fixtures that need a shape the canned manifests below don't cover).
pub fn resolve_manifest_toml(toml: &str) -> ResolvedExtensionManifest {
    resolve(toml)
}

fn resolve(toml: &str) -> ResolvedExtensionManifest {
    let contracts = {
        let mut registry = ironclaw_extension_registry::HostApiContractRegistry::new();
        registry
            .register(Arc::new(
                ironclaw_extension_registry::CapabilityProviderHostApiContract::new().unwrap(),
            ))
            .unwrap();
        registry
    };
    ExtensionManifestRecord::from_toml(
        toml,
        ManifestSource::HostBundled,
        &catalog(),
        None,
        &contracts,
        None,
    )
    .expect("fixture manifest parses")
    .resolved()
    .clone()
}

/// A hosted-MCP (tools-only) resolved manifest.
pub fn mcp_manifest() -> ResolvedExtensionManifest {
    resolve(MCP_MANIFEST)
}

/// A channel-only resolved manifest.
pub fn channel_only_manifest() -> ResolvedExtensionManifest {
    resolve(CHANNEL_MANIFEST)
}

/// An outbound-only channel manifest (no ingress section) — the web-push
/// deployment shape: nothing to mount, everything to deliver.
pub fn outbound_only_channel_manifest() -> ResolvedExtensionManifest {
    resolve(OUTBOUND_ONLY_CHANNEL_MANIFEST)
}

/// A tool + channel + auth resolved manifest.
pub fn tool_and_channel_manifest() -> ResolvedExtensionManifest {
    resolve(TOOL_AND_CHANNEL_MANIFEST)
}

/// A channel + `device_link` auth resolved manifest — the one auth shape whose
/// mechanics an extension binds rather than declares. Its recipe declares no
/// alternate mode, so the host-side mode check has something to refuse.
pub fn device_link_channel_manifest() -> ResolvedExtensionManifest {
    resolve(DEVICE_LINK_CHANNEL_MANIFEST)
}

#[cfg(any(test, feature = "test-support"))]
pub fn first_party_bundles_from_inventory() -> Vec<crate::FirstPartyPackageBundle> {
    use crate::{FirstPartyPackageAsset, FirstPartyPackageBundle, FirstPartyPackageOnboarding};
    use ironclaw_extension_support::is_gsuite_extension_id;
    use ironclaw_extension_support::packages::{PackageAssetContent, bundled_packages};
    use ironclaw_host_api::ids::ExtensionId;

    bundled_packages()
        .into_iter()
        .map(|bundle| {
            let assets = bundle
                .assets
                .into_iter()
                .map(|asset| {
                    let PackageAssetContent::Bytes(bytes) = asset.content;
                    FirstPartyPackageAsset {
                        path: asset.path,
                        bytes,
                    }
                })
                .collect();
            let search_aliases = if ExtensionId::new(bundle.id)
                .map(|id| is_gsuite_extension_id(&id))
                .unwrap_or(false)
            {
                [
                    "google",
                    "gsuite",
                    "g suite",
                    "workspace",
                    "google workspace",
                ]
                .into_iter()
                .map(str::to_string)
                .collect()
            } else {
                Vec::new()
            };
            FirstPartyPackageBundle {
                id: bundle.id.to_string(),
                display_name: bundle.display_name.to_string(),
                manifest_toml: bundle.manifest_toml.into_owned(),
                assets,
                onboarding: bundle.onboarding.map(|copy| FirstPartyPackageOnboarding {
                    instructions: copy.instructions,
                    credential_instructions: copy.credential_instructions,
                    setup_url: copy.setup_url,
                    credential_next_step: copy.credential_next_step,
                }),
                oauth_setup: None,
                trust_effects: bundle.trust_effects,
                search_aliases,
            }
        })
        .collect()
}

/// A no-op tool adapter.
#[derive(Default)]
pub struct FakeToolAdapter;

#[async_trait]
impl ToolAdapter for FakeToolAdapter {
    async fn invoke(
        &self,
        _call: ToolCall,
        _ports: &ToolPorts<'_>,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult {
            output: serde_json::json!({"ok": true}),
            display_preview: None,
            output_bytes: 0,
        })
    }
}

/// A channel adapter that records its activate/cleanup calls and never wires
/// a real vendor.
#[derive(Default)]
pub struct FakeChannelAdapter {
    pub activate_calls: Arc<AtomicUsize>,
    pub cleanup_calls: Arc<AtomicUsize>,
    /// When set, `activate` fails (to test activation abort).
    pub fail_activate: bool,
    /// When set, `cleanup` fails (to test `RemovalPending`).
    pub fail_cleanup: bool,
}

#[async_trait]
impl ChannelAdapter for FakeChannelAdapter {
    async fn activate(
        &self,
        _ctx: &ChannelContext<'_>,
        _egress: &dyn RestrictedEgress,
    ) -> Result<(), ChannelError> {
        self.activate_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_activate {
            Err(ChannelError::VendorWiring {
                reason: "scripted activate failure".to_string(),
            })
        } else {
            Ok(())
        }
    }

    async fn cleanup(
        &self,
        _ctx: &ChannelContext<'_>,
        _egress: &dyn RestrictedEgress,
    ) -> Result<(), ChannelError> {
        self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_cleanup {
            Err(ChannelError::VendorWiring {
                reason: "scripted cleanup failure".to_string(),
            })
        } else {
            Ok(())
        }
    }

    fn inbound(&self, _request: VerifiedInbound<'_>) -> Result<InboundOutcome, ChannelError> {
        Ok(InboundOutcome::Ignore)
    }

    async fn deliver(
        &self,
        _envelope: OutboundEnvelope,
        _egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        Ok(DeliveryReport { parts: Vec::new() })
    }
}

/// Which adapter method a [`FakeDeviceLinkAdapter`] call recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeDeviceLinkCallKind {
    Begin(DeviceLinkMode),
    Poll,
    SubmitInput(DeviceLinkInputKind),
    Cancel,
    Revoke,
}

/// One recorded device-link adapter call, with everything the host scoped it
/// to. Secrets are deliberately not recorded — only the input's kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeDeviceLinkCall {
    pub kind: FakeDeviceLinkCallKind,
    pub flow_id: DeviceLinkFlowId,
    pub extension_id: ExtensionId,
    pub user_id: UserId,
    pub account: Option<LinkedAccountGrant>,
    /// Whether the pre-scoped custody handle answered a `load`. Proves the
    /// host wired a usable handle without the adapter naming an account.
    pub session_loaded: bool,
}

/// A scripted [`DeviceLinkAdapter`] that records every call and never speaks a
/// vendor protocol.
///
/// Deliberately *not* self-limiting: it answers as fast as it is asked, so a
/// test that sees a poll floor or a TTL observed is seeing host enforcement.
#[derive(Default)]
pub struct FakeDeviceLinkAdapter {
    pub calls: Arc<Mutex<Vec<FakeDeviceLinkCall>>>,
    /// Steps handed out in order; exhausted scripts answer `AwaitingVendor`.
    pub steps: Arc<Mutex<VecDeque<DeviceLinkStep>>>,
    /// When set, every call fails with this error instead.
    pub fail_with: Arc<Mutex<Option<DeviceLinkError>>>,
    /// When set, a scripted `Completed` step is returned WITHOUT persisting a
    /// session blob first — the adapter-contract violation the engine must
    /// refuse (a completion the custody store cannot back).
    pub skip_completion_persist: Arc<Mutex<bool>>,
}

impl FakeDeviceLinkAdapter {
    /// A fake whose calls answer `steps` in order.
    pub fn scripted(steps: impl IntoIterator<Item = DeviceLinkStep>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            steps: Arc::new(Mutex::new(steps.into_iter().collect())),
            fail_with: Arc::new(Mutex::new(None)),
            skip_completion_persist: Arc::new(Mutex::new(false)),
        }
    }

    pub fn recorded(&self) -> Vec<FakeDeviceLinkCall> {
        self.calls.lock().expect("fake device-link calls").clone()
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().expect("fake device-link calls").len()
    }

    async fn record(
        &self,
        kind: FakeDeviceLinkCallKind,
        ctx: &DeviceLinkContext<'_>,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        let session_loaded = ctx.session.load().await.is_ok();
        self.calls
            .lock()
            .expect("fake device-link calls")
            .push(FakeDeviceLinkCall {
                kind,
                flow_id: ctx.flow_id.clone(),
                extension_id: ctx.extension_id.clone(),
                user_id: ctx.user_id.clone(),
                account: ctx.account.cloned(),
                session_loaded,
            });
        if let Some(error) = self
            .fail_with
            .lock()
            .expect("fake device-link failure")
            .clone()
        {
            return Err(error);
        }
        let step = self
            .steps
            .lock()
            .expect("fake device-link steps")
            .pop_front()
            .unwrap_or(DeviceLinkStep::AwaitingVendor {
                retry_in: Duration::from_millis(1),
            });
        // The adapter contract: custody is durable before completion is
        // reported (store blob → mint → report). Mirror the real adapter by
        // persisting through the pre-scoped handle before a `Completed` step —
        // unless a test explicitly scripts the violation.
        if matches!(step, DeviceLinkStep::Completed { .. })
            && !*self
                .skip_completion_persist
                .lock()
                .expect("fake device-link persist flag")
        {
            let expected = match ctx.session.load().await {
                Ok(Some(snapshot)) => snapshot.version,
                _ => LinkedSessionVersion::absent(),
            };
            let blob = SessionBytes::new(b"fake-linked-session".to_vec())
                .expect("fixture blob satisfies bounds");
            if let Err(error) = ctx.session.save(expected, blob).await {
                return Err(DeviceLinkError::Custody(error));
            }
        }
        Ok(step)
    }
}

#[async_trait]
impl DeviceLinkAdapter for FakeDeviceLinkAdapter {
    async fn begin(
        &self,
        ctx: &DeviceLinkContext<'_>,
        mode: DeviceLinkMode,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        self.record(FakeDeviceLinkCallKind::Begin(mode), ctx).await
    }

    async fn poll(&self, ctx: &DeviceLinkContext<'_>) -> Result<DeviceLinkStep, DeviceLinkError> {
        self.record(FakeDeviceLinkCallKind::Poll, ctx).await
    }

    async fn submit_input(
        &self,
        ctx: &DeviceLinkContext<'_>,
        input: DeviceLinkInput,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        self.record(FakeDeviceLinkCallKind::SubmitInput(input.kind()), ctx)
            .await
    }

    async fn cancel(&self, ctx: &DeviceLinkContext<'_>) -> Result<(), DeviceLinkError> {
        self.record(FakeDeviceLinkCallKind::Cancel, ctx).await?;
        Ok(())
    }

    async fn revoke(&self, ctx: &DeviceLinkContext<'_>) -> Result<(), DeviceLinkError> {
        self.record(FakeDeviceLinkCallKind::Revoke, ctx).await?;
        Ok(())
    }
}

/// An entrypoint that binds a fixed set of adapters.
pub struct FakeEntrypoint {
    pub bindings: ExtensionBindings,
}

impl ExtensionEntrypoint for FakeEntrypoint {
    fn bind(&self, _ctx: BindContext) -> Result<ExtensionBindings, BindError> {
        Ok(self.bindings.clone())
    }
}

/// A loader that returns a fixed entrypoint; records load calls.
pub struct FakeLoader {
    pub bindings: ExtensionBindings,
    pub load_calls: Arc<AtomicUsize>,
    /// When set, `load` fails (to test skip-invalid-at-restore).
    pub fail_load: bool,
}

#[async_trait]
impl ExtensionLoader for FakeLoader {
    async fn load(&self, _ctx: &LoadContext) -> Result<LoadedExtension, BindError> {
        self.load_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_load {
            return Err(BindError::Load {
                reason: "scripted load failure".to_string(),
            });
        }
        Ok(LoadedExtension::new(Box::new(FakeEntrypoint {
            bindings: self.bindings.clone(),
        })))
    }
}

/// A drain controller that records drains.
#[derive(Default)]
pub struct RecordingDrain {
    pub drained: Arc<tokio::sync::Mutex<Vec<String>>>,
}

#[async_trait]
impl DrainController for RecordingDrain {
    async fn drain(&self, extension_id: &str, _deadline: Duration) -> Result<(), HookError> {
        self.drained.lock().await.push(extension_id.to_string());
        Ok(())
    }
}

/// An egress factory yielding a deny-all restricted egress (fixtures never
/// perform real network calls).
#[derive(Default)]
pub struct FakeEgressFactory;

impl EgressFactory for FakeEgressFactory {
    fn egress_for_channel(
        &self,
        _extension_id: &str,
        _installation_id: &str,
        _declared: &[ironclaw_extension_contracts::channel::ChannelEgressDescriptor],
    ) -> Arc<dyn RestrictedEgress> {
        Arc::new(DenyAllEgress)
    }
}

struct DenyAllEgress;

#[async_trait]
impl RestrictedEgress for DenyAllEgress {
    async fn send(
        &self,
        _request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        Err(RestrictedEgressError::PolicyDenied)
    }
}

/// Records pairing outcomes the generic sink observes. An ordinary double now
/// that the observer is a trait; shared so the sink contract tests and the
/// composition-side pairing-service tests assert against one implementation.
pub struct RecordingPairingOutcomeObserver {
    pub outcomes: Arc<std::sync::Mutex<Vec<crate::channel_pairing::ChannelPairingConsumeOutcome>>>,
}

#[async_trait]
impl crate::extension_ingress::ChannelPairingOutcomeObserver for RecordingPairingOutcomeObserver {
    async fn observe_pairing_outcome(
        &self,
        _conversation: ironclaw_extension_contracts::external::ExternalConversationRef,
        _event_id: ironclaw_extension_contracts::external::ExternalEventId,
        outcome: crate::channel_pairing::ChannelPairingConsumeOutcome,
    ) {
        match self.outcomes.lock() {
            Ok(mut outcomes) => outcomes.push(outcome),
            Err(poisoned) => poisoned.into_inner().push(outcome),
        }
    }
}
