//! Linked-account (device-link) domain tools profile.
//!
//! Mounts the **real bundled Telegram package** — its shipped `manifest.toml`,
//! which declares a `[channel]` surface, an `[auth.telegram] method =
//! "device_link"` surface, and fifteen `standard_op` `[[tools]]` bound to the
//! user's own linked account — and satisfies its `[admin_configuration]`
//! (including the MTProto `telegram_api_id` / `telegram_api_hash` fields the
//! linked-device surface needs) so a scenario can reach that tool surface.
//!
//! # What is faked, and why exactly this much
//!
//! Two adapters are scripted; nothing else is:
//!
//! * [`ScriptedDeviceLinkAdapter`] — the vendor half of a device link. The real
//!   `TelegramDeviceLinkAdapter` speaks MTProto over a raw socket with no
//!   injectable seam, so it cannot run offline; this walks the same
//!   `DeviceLinkAdapter` contract (display → awaiting → input-required →
//!   completed) with no network.
//! * [`LinkedAccountFixtureToolAdapter`] — the linked-account tool half. The
//!   real `TelegramLinkedToolAdapter` needs a live `SessionPool` (MTProto) and
//!   an implementation of its `LinkedAccountResolver` port, which **no crate in
//!   the workspace provides** (the port's own doc records that the host seam is
//!   not built yet). This one answers the canonical standard-op contracts and
//!   echoes the caller scope so a test can prove *which* actor a dispatch ran
//!   as.
//!
//! Everything between a scripted model tool call and those two adapters is
//! production: manifest parse → registry → install/activate through
//! `builtin.extension_install` → the binding rule (`check_binding`) → the
//! published active snapshot → authorization → the credential-account
//! requirement derived from `[[tools.credentials]]` + the device-link recipe →
//! `CapabilityHost` → `ToolAdapter::invoke` → standard-op output validation.
//!
//! # Why the factory binds three surfaces
//!
//! `check_binding` refuses a partial binding in both directions: a manifest
//! that declares tools and a device-link auth surface must bind a tool adapter
//! and a device-link adapter, and an adapter for a surface the manifest never
//! declared is refused too. The bundled Telegram manifest declares all three
//! surfaces, so [`TelegramLinkedFixtureFactory`] binds all three — the shape
//! `ironclaw_cli::runtime::native_extensions` must also take before the
//! shipping binary can activate this package.
//!
//! # What this profile reaches, and what it deliberately does not
//!
//! The device-link **handshake** (`begin -> poll -> submit_input -> completed`)
//! IS drivable from this tier as of 2026-08-10, and
//! `scenario_handshake_mints_and_serves` drives it. The four seams that used to
//! make it unreachable have all landed: composition constructs the
//! `DeviceLinkFlowDriver` and the extension host's `SnapshotDeviceLinkDriver`
//! and attaches them to the product-auth bundle; custody resolves to the
//! credential-service-backed store rather than `LinkedSessionStore::unavailable()`;
//! completion mints the credential account through
//! `CredentialAccountService::complete_linked_device_link` (ownership pinned
//! there); and `LinkedAccountResolver` is implemented host-side, so the tool
//! half resolves the caller to the account the handshake minted.
//!
//! Two things remain out of reach at every Rust tier, and no scenario here
//! pretends otherwise:
//!
//! 1. **Real MTProto.** The vendor half is scripted
//!    ([`ScriptedDeviceLinkAdapter`] and [`LinkedAccountFixtureToolAdapter`])
//!    because the shipped one speaks a binary protocol over a socket with no
//!    injectable seam. QR acceptance, datacenter migration, 2FA and flood-wait
//!    behaviour are therefore unexercised anywhere in CI.
//! 2. **"A failed session must not reconnect until `link_revision` changes."**
//!    The pool keys on the revision and custody gates on it, but nothing
//!    records a session-level *failure* or refuses reconnection until the
//!    revision moves — there is no production behavior to assert. Tracked as a
//!    gap row in `tests/CLAUDE.md` §7.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_extension_contracts::device_link::{
    DeviceLinkAdapter, DeviceLinkContext, DeviceLinkDisplayKind, DeviceLinkError,
    DeviceLinkErrorCode, DeviceLinkInput, DeviceLinkInputKind, DeviceLinkMode, DeviceLinkPayload,
    DeviceLinkStep,
};
use ironclaw_extension_contracts::tool_adapter::{
    ToolAdapter, ToolCall, ToolError, ToolPorts, ToolResult,
};
use ironclaw_host_api::dispatch::{DispatchFailureDetail, RuntimeDispatchErrorKind};
use ironclaw_host_api::ids::{CapabilityId, ExtensionId};
use ironclaw_host_api::messaging::StandardMessagingOp;
use ironclaw_host_api::safe_summary::SafeSummary;
use serde_json::{Value, json};

use super::super::options::ToolsProfile;
use super::super::{HarnessResult, standalone_all_effects};
use super::extension::extension_lifecycle_tools_profile_for_user;

/// The bundled package this profile mounts.
pub(crate) const LINKED_EXTENSION_ID: &str = "telegram";

/// The credential-authority namespace the linked tools' `[[tools.credentials]]`
/// blocks name (`vendor = "telegram"`), and therefore the provider a seeded
/// credential account must carry for a linked-account call to resolve.
pub(crate) const LINKED_VENDOR_ID: &str = "telegram";

/// The package's `[admin_configuration] group_id`.
pub(crate) const LINKED_ADMIN_GROUP_ID: &str = "extension.telegram";

/// The package's `runtime.service` id, matched by the native factory below.
pub(crate) const LINKED_RUNTIME_SERVICE: &str = "telegram.extension/v1";

/// Capability id of the cheapest linked-account read — the one every scenario
/// uses to ask "does this caller have a usable linked account?".
pub(crate) const LINKED_WHOAMI_CAPABILITY_ID: &str = "telegram.whoami";

/// Capability id of a linked-account listing read.
pub(crate) const LINKED_LIST_CONVERSATIONS_CAPABILITY_ID: &str = "telegram.list_conversations";

/// Deployment `[admin_configuration]` values, complete enough that the
/// linked-device surface is configured rather than fail-closed: the four
/// bot-channel fields the manifest marks `required`, PLUS the two MTProto
/// application-identity fields (`required = false` on purpose, so an existing
/// bot-only deployment keeps booting) the linked surface needs.
pub(crate) fn linked_admin_configuration_values() -> Value {
    json!([
        {"handle": "telegram_bot_token", "value": "itest-device-link-bot-token"},
        {"handle": "telegram_webhook_secret", "value": "itest-device-link-webhook-secret"},
        {"handle": "telegram_webhook_url", "value": "https://hooks.example.test/webhooks/extensions/telegram/updates"},
        {"handle": "bot_username", "value": "itest_device_link_bot"},
        {"handle": "telegram_api_id", "value": "1234567"},
        {"handle": "telegram_api_hash", "value": "itest-device-link-api-hash"}
    ])
}

/// Every capability id the bundled package's shipped `manifest.toml` declares,
/// parsed from the real asset (never a hand-transcribed list) so a tool added
/// to or removed from the manifest cannot silently drop out of the granted set.
pub(crate) fn linked_capability_ids() -> HarnessResult<Vec<CapabilityId>> {
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/extensions/packages")
        .join(LINKED_EXTENSION_ID)
        .join("manifest.toml");
    let record = ironclaw_extension_registry::ExtensionManifestRecord::from_toml(
        std::fs::read_to_string(&manifest_path)?,
        ironclaw_extension_registry::ManifestSource::HostBundled,
        &ironclaw_host_api::host_port::default_host_port_catalog()?,
        None,
        &ironclaw_extension_registry::default_host_api_contract_registry()?,
        None,
    )?;
    let manifest =
        ironclaw_extension_registry::ExtensionManifest::try_from(record.manifest().clone())?;
    let package = ironclaw_extension_registry::ExtensionPackage::from_manifest(
        manifest,
        ironclaw_host_api::path::VirtualPath::new(format!(
            "/system/extensions/{LINKED_EXTENSION_ID}"
        ))?,
    )?;
    let mut registry = ironclaw_extension_registry::ExtensionRegistry::new();
    registry.insert(package)?;
    Ok(registry
        .capabilities()
        .map(|descriptor| descriptor.id.clone())
        .collect())
}

// ── The scripted vendor half of a device link ──────────────────────────────

/// How many polls the scripted adapter answers "not yet" before it asks the
/// user for their account password. Small so a scenario walks the whole
/// sequence without sleeping.
const POLLS_BEFORE_INPUT: u32 = 1;

/// Where one scripted link stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkPhase {
    /// A payload is on screen; `remaining` polls still answer "not yet".
    Displayed {
        remaining: u32,
    },
    /// The vendor asked for the account password and waits on the human.
    AwaitingPassword,
    Completed,
    Abandoned,
}

/// One call the scripted adapter served, in order. A scenario reads this to
/// prove the host did *not* re-invoke a transition that already ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScriptedDeviceLinkCall {
    Begin(String, DeviceLinkMode),
    Poll(String),
    SubmitInput(String, DeviceLinkInputKind),
    Finalize(String),
    Cancel(String),
    Revoke(String),
}

#[derive(Default)]
struct ScriptedDeviceLinkState {
    phases: HashMap<String, LinkPhase>,
    calls: Vec<ScriptedDeviceLinkCall>,
    payloads: u32,
}

/// A [`DeviceLinkAdapter`] that walks a real multi-step handshake with no
/// network.
///
/// It models rather than stubs for the same reason the auth crate's driver
/// double does: the risk surface of a device link is *sequencing* — a poll that
/// advances state instead of reading it, a duplicated transition, a submit
/// against a frame that already moved — and a double answering every call with
/// a canned step would make all three invisible.
///
/// It also touches the custody handle the host hands it, so a scenario can tell
/// a deployment that wired linked-session custody from one that did not: the
/// blob write is attempted on completion and its outcome recorded, never
/// silently swallowed.
#[derive(Default)]
pub(crate) struct ScriptedDeviceLinkAdapter {
    state: Mutex<ScriptedDeviceLinkState>,
    custody: Mutex<Vec<String>>,
}

impl ScriptedDeviceLinkAdapter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Every call served, in order.
    pub(crate) fn calls(&self) -> Vec<ScriptedDeviceLinkCall> {
        self.lock_state().calls.clone()
    }

    /// One entry per attempted custody write: `"ok"` or the typed custody
    /// failure's `Display`. A deployment with no linked-session material store
    /// records the fail-closed "custody is not wired" reason here rather than
    /// pretending the session was persisted.
    pub(crate) fn custody_outcomes(&self) -> Vec<String> {
        self.custody
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, ScriptedDeviceLinkState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn record(&self, call: ScriptedDeviceLinkCall) {
        self.lock_state().calls.push(call);
    }

    /// Persist the "session" through the pre-scoped custody handle the host
    /// supplied. The bytes are a fixture marker, never vendor material.
    async fn persist(&self, ctx: &DeviceLinkContext<'_>) {
        use ironclaw_extension_contracts::linked_session::{LinkedSessionVersion, SessionBytes};

        let outcome = match SessionBytes::new(b"itest-linked-session-blob".to_vec()) {
            Ok(blob) => match ctx.session.save(LinkedSessionVersion::absent(), blob).await {
                Ok(_) => "ok".to_string(),
                Err(error) => error.to_string(),
            },
            Err(error) => error.to_string(),
        };
        self.custody
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(outcome);
    }

    fn completed_step() -> DeviceLinkStep {
        DeviceLinkStep::Completed {
            account_label: "Linked personal account".to_string(),
            vendor_user_ref: "424242".to_string(),
        }
    }
}

#[async_trait]
impl DeviceLinkAdapter for ScriptedDeviceLinkAdapter {
    async fn begin(
        &self,
        ctx: &DeviceLinkContext<'_>,
        mode: DeviceLinkMode,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        let flow = ctx.flow_id.to_string();
        let payload = {
            let mut state = self.lock_state();
            state
                .calls
                .push(ScriptedDeviceLinkCall::Begin(flow.clone(), mode));
            state.payloads = state.payloads.saturating_add(1);
            state.phases.insert(
                flow,
                LinkPhase::Displayed {
                    remaining: POLLS_BEFORE_INPUT,
                },
            );
            format!("itest-device-link-token-{}", state.payloads)
        };
        Ok(DeviceLinkStep::Display {
            kind: DeviceLinkDisplayKind::QrCode,
            payload: DeviceLinkPayload::new(payload).map_err(|_| DeviceLinkError::Internal {
                reason: "fixture payload is not a valid device-link payload",
            })?,
            expires_in: Duration::from_secs(30),
        })
    }

    async fn poll(&self, ctx: &DeviceLinkContext<'_>) -> Result<DeviceLinkStep, DeviceLinkError> {
        let flow = ctx.flow_id.to_string();
        let phase = {
            let mut state = self.lock_state();
            state.calls.push(ScriptedDeviceLinkCall::Poll(flow.clone()));
            let Some(phase) = state.phases.get_mut(&flow) else {
                return Err(DeviceLinkError::UnknownFlow);
            };
            match *phase {
                LinkPhase::Displayed { remaining: 0 } => {
                    *phase = LinkPhase::AwaitingPassword;
                    LinkPhase::AwaitingPassword
                }
                LinkPhase::Displayed { remaining } => {
                    *phase = LinkPhase::Displayed {
                        remaining: remaining - 1,
                    };
                    LinkPhase::Displayed { remaining }
                }
                other => other,
            }
        };
        match phase {
            LinkPhase::AwaitingPassword => Ok(DeviceLinkStep::InputRequired {
                kind: DeviceLinkInputKind::Password,
                label: "Account password".to_string(),
                hint: None,
            }),
            LinkPhase::Displayed { .. } => Ok(DeviceLinkStep::AwaitingVendor {
                retry_in: Duration::from_secs(3),
            }),
            LinkPhase::Completed => Ok(Self::completed_step()),
            LinkPhase::Abandoned => Err(DeviceLinkError::UnknownFlow),
        }
    }

    async fn submit_input(
        &self,
        ctx: &DeviceLinkContext<'_>,
        input: DeviceLinkInput,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        let flow = ctx.flow_id.to_string();
        let kind = input.kind();
        let accepted = {
            let mut state = self.lock_state();
            state
                .calls
                .push(ScriptedDeviceLinkCall::SubmitInput(flow.clone(), kind));
            let Some(phase) = state.phases.get_mut(&flow) else {
                return Err(DeviceLinkError::UnknownFlow);
            };
            match (*phase, kind) {
                (LinkPhase::AwaitingPassword, DeviceLinkInputKind::Password) => {
                    *phase = LinkPhase::Completed;
                    true
                }
                _ => false,
            }
        };
        if !accepted {
            // Fail closed on a value the phase did not ask for, so a host that
            // routed the wrong input cannot pass by accident.
            return Err(DeviceLinkError::Vendor {
                code: DeviceLinkErrorCode::InvalidInput,
                restartable: true,
            });
        }
        self.persist(ctx).await;
        Ok(Self::completed_step())
    }

    async fn finalize(&self, ctx: &DeviceLinkContext<'_>) {
        let flow = ctx.flow_id.to_string();
        let mut state = self.lock_state();
        state
            .calls
            .push(ScriptedDeviceLinkCall::Finalize(flow.clone()));
        state.phases.remove(&flow);
    }

    async fn cancel(&self, ctx: &DeviceLinkContext<'_>) -> Result<(), DeviceLinkError> {
        let flow = ctx.flow_id.to_string();
        let mut state = self.lock_state();
        state
            .calls
            .push(ScriptedDeviceLinkCall::Cancel(flow.clone()));
        state.phases.insert(flow, LinkPhase::Abandoned);
        Ok(())
    }

    async fn revoke(&self, ctx: &DeviceLinkContext<'_>) -> Result<(), DeviceLinkError> {
        let flow = ctx.flow_id.to_string();
        let mut state = self.lock_state();
        state
            .calls
            .push(ScriptedDeviceLinkCall::Revoke(flow.clone()));
        state.phases.insert(flow, LinkPhase::Abandoned);
        Ok(())
    }
}

// ── The scripted linked-account tool half ──────────────────────────────────

/// A [`ToolAdapter`] for the package's `standard_op` tool surface that answers
/// the canonical output contracts without a vendor connection.
///
/// Every answer echoes the dispatching scope's user id into a field the
/// canonical schema allows (`user_ref`, `vendor`), which is what lets a
/// cross-actor scenario prove a call ran as the *second* actor rather than
/// silently reusing the first actor's resolved account.
#[derive(Default)]
pub(crate) struct LinkedAccountFixtureToolAdapter {
    calls: Mutex<Vec<(String, String)>>,
    /// The host-supplied bind-time resolver — the same seam the real
    /// `TelegramLinkedToolAdapter` consumes. Filled at every `bind`.
    resolver:
        Mutex<Option<Arc<dyn ironclaw_extension_contracts::linked_session::LinkedAccountResolver>>>,
}

impl LinkedAccountFixtureToolAdapter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// `(capability_id, scope.user_id)` per invocation, in order.
    pub(crate) fn calls(&self) -> Vec<(String, String)> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(crate) fn attach_resolver(
        &self,
        resolver: Arc<dyn ironclaw_extension_contracts::linked_session::LinkedAccountResolver>,
    ) {
        *self
            .resolver
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(resolver);
    }

    fn resolver(
        &self,
    ) -> Option<Arc<dyn ironclaw_extension_contracts::linked_session::LinkedAccountResolver>> {
        self.resolver
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl ToolAdapter for LinkedAccountFixtureToolAdapter {
    async fn invoke(
        &self,
        call: ToolCall,
        _ports: &ToolPorts<'_>,
    ) -> Result<ToolResult, ToolError> {
        let capability_id = call.capability_id.as_str().to_string();
        let caller = call.scope.user_id.as_str().to_string();
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((capability_id.clone(), caller.clone()));

        let Some(op_name) = capability_id.strip_prefix("telegram.") else {
            return Err(undeclared(&capability_id));
        };
        let Some(op) = StandardMessagingOp::ALL
            .iter()
            .copied()
            .find(|op| op.op_name() == op_name)
        else {
            return Err(undeclared(&capability_id));
        };
        let mut output =
            canonical_output(op, &caller, &call.input).ok_or_else(|| undeclared(&capability_id))?;
        // Resolve through the host's bind-time resolver — the production seam
        // — and echo the grant so a scenario can assert the call acted as the
        // MINTED account. Soft on purpose: scenarios that seed accounts
        // through the manual-token path (no linked device) keep working, and
        // the handshake scenario asserts the echo is present.
        if let Some(resolver) = self.resolver()
            && let Ok(grant) = resolver.resolve(&call.scope).await
            && let Some(vendor) = output
                .get_mut("vendor")
                .and_then(|value| value.as_object_mut())
        {
            vendor.insert(
                "linked_account_ref".to_string(),
                serde_json::Value::String(grant.account().as_str().to_string()),
            );
            vendor.insert(
                "link_revision".to_string(),
                serde_json::Value::from(grant.link_revision()),
            );
        }
        let output_bytes = serde_json::to_vec(&output)
            .map(|bytes| bytes.len() as u64)
            .unwrap_or_default();
        Ok(ToolResult {
            output,
            display_preview: None,
            output_bytes,
        })
    }
}

fn undeclared(capability_id: &str) -> ToolError {
    ToolError::Rejected {
        kind: RuntimeDispatchErrorKind::UndeclaredCapability,
        diagnostic: None,
        detail: Some(DispatchFailureDetail::HostSummary {
            summary: SafeSummary::new(format!(
                "the linked-account fixture does not implement the capability {capability_id}"
            ))
            .unwrap(),
            detail: None,
        }),
    }
}

/// Canonical standard-op output for the ops this fixture answers. The host
/// validates every one of these against `standard:messaging/<op>.output.<v>`,
/// so a shape drift here fails the dispatch rather than passing silently.
fn canonical_output(op: StandardMessagingOp, caller: &str, input: &Value) -> Option<Value> {
    let conversation = input
        .get("conversation")
        .and_then(Value::as_str)
        .unwrap_or("itest-linked-conversation")
        .to_string();
    Some(match op {
        StandardMessagingOp::Whoami => json!({
            "user_ref": caller,
            "display_name": "Linked fixture account",
            "vendor": {"linked_caller": caller},
        }),
        StandardMessagingOp::ListConversations => json!({
            "conversations": [{
                "conversation": conversation,
                "kind": "dm",
                "name": "Saved Messages",
                "member": true,
                "vendor": {"linked_caller": caller},
            }],
        }),
        StandardMessagingOp::OpenDm => json!({
            "conversation": "itest-linked-dm",
            "vendor": {"linked_caller": caller},
        }),
        _ => return None,
    })
}

// ── The native factory (binary-parity binding shape) ───────────────────────

/// Native factory for the bundled Telegram package, binding all three surfaces
/// its shipped manifest declares.
///
/// The binary's own factory
/// (`ironclaw_cli::runtime::native_extensions::TelegramExtensionEntrypoint`)
/// binds the same three surfaces with the real Telegram implementations; this
/// fixture substitutes only the live MTProto edges so the contract remains
/// hermetic.
pub(crate) struct TelegramLinkedFixtureFactory {
    device_link: Arc<ScriptedDeviceLinkAdapter>,
    tools: Arc<LinkedAccountFixtureToolAdapter>,
}

impl TelegramLinkedFixtureFactory {
    pub(crate) fn new(
        device_link: Arc<ScriptedDeviceLinkAdapter>,
        tools: Arc<LinkedAccountFixtureToolAdapter>,
    ) -> Self {
        Self { device_link, tools }
    }
}

#[async_trait::async_trait]
impl ironclaw_extension_host::NativeExtensionFactory for TelegramLinkedFixtureFactory {
    fn service(&self) -> &str {
        LINKED_RUNTIME_SERVICE
    }

    async fn load(
        &self,
        _ctx: &ironclaw_extension_host::LoadContext,
    ) -> Result<
        Box<dyn ironclaw_extension_host::ExtensionEntrypoint>,
        ironclaw_extension_host::BindError,
    > {
        Ok(Box::new(TelegramLinkedFixtureEntrypoint {
            device_link: Arc::clone(&self.device_link),
            tools: Arc::clone(&self.tools),
        }))
    }
}

struct TelegramLinkedFixtureEntrypoint {
    device_link: Arc<ScriptedDeviceLinkAdapter>,
    tools: Arc<LinkedAccountFixtureToolAdapter>,
}

impl ironclaw_extension_host::ExtensionEntrypoint for TelegramLinkedFixtureEntrypoint {
    fn bind(
        &self,
        ctx: ironclaw_extension_host::BindContext,
    ) -> Result<ironclaw_extension_host::ExtensionBindings, ironclaw_extension_host::BindError>
    {
        self.tools.attach_resolver(Arc::clone(&ctx.linked_accounts));
        Ok(ironclaw_extension_host::ExtensionBindings {
            tools: Some(Arc::clone(&self.tools) as Arc<dyn ToolAdapter>),
            channel: {
                let adapter =
                    Arc::new(ironclaw_telegram_extension::TelegramChannelAdapter::default());
                ironclaw_extension_contracts::channel_adapter::ChannelSurfaces::default()
                    .with_ingress(adapter.clone())
                    .with_reply(adapter.clone())
                    .with_delivery(adapter)
            },
            device_link: Some(Arc::clone(&self.device_link) as Arc<dyn DeviceLinkAdapter>),
        })
    }
}

// ── The profile ────────────────────────────────────────────────────────────

/// Handles a scenario reads back after driving a turn.
#[derive(Clone)]
pub(crate) struct LinkedFixtureHandles {
    pub(crate) device_link: Arc<ScriptedDeviceLinkAdapter>,
    pub(crate) tools: Arc<LinkedAccountFixtureToolAdapter>,
}

/// The linked-account profile: the extension-lifecycle profile plus the
/// bundled Telegram package's own capability ids, provider trust, and the
/// three-surface native factory.
pub(crate) fn device_link_tools_profile_for_user(
    user_id: &str,
) -> HarnessResult<(ToolsProfile, LinkedFixtureHandles)> {
    let handles = LinkedFixtureHandles {
        device_link: Arc::new(ScriptedDeviceLinkAdapter::new()),
        tools: Arc::new(LinkedAccountFixtureToolAdapter::new()),
    };
    let mut profile = extension_lifecycle_tools_profile_for_user(user_id)?;
    profile.capability_ids.extend(linked_capability_ids()?);
    if let Some(trust) = profile.provider_trust_override.as_mut() {
        trust.push((
            ExtensionId::new(LINKED_EXTENSION_ID)?,
            standalone_all_effects(),
        ));
    }
    profile.options =
        profile
            .options
            .with_native_extension_factory(Arc::new(TelegramLinkedFixtureFactory::new(
                Arc::clone(&handles.device_link),
                Arc::clone(&handles.tools),
            )));
    Ok((profile, handles))
}
