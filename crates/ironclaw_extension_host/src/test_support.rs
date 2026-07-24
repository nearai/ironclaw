//! Shared test fixtures: resolved-manifest builders and scripted adapters.
//!
//! Available to this crate's own tests and to downstream integration tests
//! (behind the crate's default build — these are lightweight fakes, not a
//! feature-gated seam) so the acme fixture and the state-machine contract
//! tests share one construction path.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_extensions::{ExtensionManifestRecord, ManifestSource, ResolvedExtensionManifest};
use ironclaw_host_api::{
    HOST_RUNTIME_HTTP_EGRESS_PORT_ID, HostPortCatalog, HostPortCatalogEntry, HostPortId,
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
    ToolAdapter, ToolCall, ToolError, ToolPorts, ToolResult,
};
use ironclaw_product::{
    ChannelAdapter, ChannelContext, ChannelError, DeliveryReport, InboundOutcome, OutboundEnvelope,
    VerifiedInbound,
};

use crate::entrypoint::{BindContext, BindError, ExtensionBindings, ExtensionEntrypoint};
use crate::lifecycle::{DrainController, EgressFactory, HookError};
use crate::loaders::{ExtensionLoader, LoadContext, LoadedExtension};

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

const CHANNEL_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acme-chat"
name = "Acme Chat"
version = "0.1.0"
description = "fixture: channel-only extension"
trust = "third_party"

[admin_configuration]
group_id = "extension.acme-chat"
display_name = "Acme Chat deployment configuration"
fields = [ { handle = "acme_chat_signing_secret", label = "Signing secret", secret = true, required = false } ]

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

[[channel.egress]]
scheme = "https"
host = "api.acme.example"
methods = ["post"]
"#;

const TOOL_AND_CHANNEL_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "acme"
name = "Acme"
version = "0.1.0"
description = "fixture: tool + channel + auth"
trust = "third_party"

[admin_configuration]
group_id = "extension.acme"
display_name = "Acme deployment configuration"
fields = [ { handle = "acme_signing_secret", label = "Signing secret", secret = true, required = false } ]

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
        let mut registry = ironclaw_extensions::HostApiContractRegistry::new();
        registry
            .register(Arc::new(
                ironclaw_extensions::CapabilityProviderHostApiContract::new().unwrap(),
            ))
            .unwrap();
        registry
    };
    // These channel fixtures declare an `[admin_configuration]` group to back
    // their channel signing secrets — a deployment-owned surface only a
    // host-bundled (first-party) manifest may declare (see `parse_v3`'s trust
    // gate). Real channel extensions (Slack, Telegram) are host-bundled, so
    // resolve fixtures with that source. Their `trust = "third_party"` ceiling
    // is unaffected: the resolved manifest is source-independent for a
    // third-party trust class.
    ExtensionManifestRecord::from_toml(
        toml,
        ManifestSource::HostBundled,
        &catalog(),
        None,
        &contracts,
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

/// A tool + channel + auth resolved manifest.
pub fn tool_and_channel_manifest() -> ResolvedExtensionManifest {
    resolve(TOOL_AND_CHANNEL_MANIFEST)
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
        _declared: &[ironclaw_host_api::ChannelEgressDescriptor],
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
