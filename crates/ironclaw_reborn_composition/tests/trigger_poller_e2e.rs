//! Full-path integration test for the composition-owned trigger poller.
//!
//! Drives a real `RebornRuntime` with the trigger poller enabled, seeds a
//! due `TriggerRecord` via the in-memory repository, and asserts that the
//! spawned background task (a) mutates the record and (b) causes the LLM
//! gateway to receive a request whose content includes the trigger prompt.

#![cfg(feature = "test-support")]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use ironclaw_conversations::{AdapterInstallationId, AdapterKind, ExternalActorRef};
use ironclaw_host_api::{
    AdapterInstallationId as ProductAdapterInstallationId, AgentId, CapabilityGrant,
    CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind, ExecutionContext, ExtensionId,
    GrantConstraints, MountView, NetworkPolicy, Principal, ProviderToolName, ResourceEstimate,
    RunId, RuntimeKind, TenantId, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    RuntimeCapabilityOutcome, TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_PAUSE_CAPABILITY_ID,
    TRIGGER_REMOVE_CAPABILITY_ID, TRIGGER_RESUME_CAPABILITY_ID,
};
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};
use ironclaw_outbound::{
    CommunicationPreferenceRecord, DeliveryDefaultScope, TriggeredRunDeliveryOutcomeKind,
    TriggeredRunDeliveryStore,
};
use ironclaw_product::{LifecyclePackageKind, LifecyclePackageRef, RebornOutboundDeliveryTargetId};
use ironclaw_reborn_composition::{
    ChannelExtensionBinding, RebornCompositionProfile, RebornRuntime, RebornRuntimeIdentity,
    RebornRuntimeInput, RebornRuntimeProfileOptions, TriggerPollerSettings, build_reborn_runtime,
    local_runtime_build_input_with_options,
};
use ironclaw_runner::runtime::ToolDisclosureMode;
use ironclaw_triggers::{
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerDeliveryTargetId, TriggerId,
    TriggerPollerWorkerConfig, TriggerRecord, TriggerRepository, TriggerRunStatus, TriggerSchedule,
    TriggerSourceKind, TriggerState,
};
use ironclaw_turns::run_profile::{
    LoopCapabilityPort, ProviderToolCall, RegisterProviderToolCallRequest,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};
use serde_json::{Value, json};
use tokio::sync::Mutex as TokioMutex;

const TENANT: &str = "trigger-e2e-tenant";
const USER: &str = "trigger-e2e-owner";
const AGENT: &str = "trigger-e2e-agent";
const TRIGGER_PROMPT: &str = "trigger-e2e-prompt-marker-do-not-rephrase";
const QA_9B_PROMPT: &str = "QA_9B scheduled health digest";
const QA_9B_RESULT: &str = "QA_9B scheduled health digest complete";
const QA_9D_PROMPT: &str = "QA_9D scheduled release digest";
const QA_9D_RESULT: &str = "QA_9D scheduled release digest complete";
const SLACK_TEAM: &str = "T-TRIGGER-E2E";
const SLACK_USER: &str = "U-TRIGGER-E2E";
const SLACK_DEFAULT_DM: &str = "D-TRIGGER-DEFAULT";
const SLACK_PER_TRIGGER_CHANNEL: &str = "C-TRIGGER-OVERRIDE";
const QA_9B_TARGET_ID: &str = "slack:personal-dm:T-TRIGGER-E2E:trigger-e2e-owner";
const QA_9D_TARGET_ID: &str = "slack:shared-channel:T-TRIGGER-E2E:C-TRIGGER-OVERRIDE";
const TEST_SECRET_MASTER_KEY: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// Name of the trigger the fired run's capability call attempts to create.
/// Issue #5505's fix strips `builtin.trigger_create` from the model-visible
/// surface for a `scheduled_trigger` fire, so this name must NEVER appear in
/// the trigger repository after the fire settles.
const SELF_CREATE_MARKER_TRIGGER_NAME: &str = "self-created-by-fire-should-not-exist";
/// Mirrors the production capability-id -> provider-tool-name transform
/// (`provider_tool_name_base` in `ironclaw_loop_host::capability_port`,
/// private to that crate) so this test tracks the `TRIGGER_*_CAPABILITY_ID`
/// constants automatically instead of hardcoding their mapped results. Only
/// the `.` -> `__` replacement is needed for these capability ids (all of the
/// form `"builtin.trigger_*"`) — the full transform also maps other
/// non-alphanumeric/`_`/`-` characters to `_`, but none of these ids have
/// any. Referenced directly rather than discovered via `tool_definitions()`
/// because on a `scheduled_trigger` surface the fix removes every mutator
/// from `tool_definitions()` entirely — there would be nothing to look up.
fn provider_tool_name_for_capability_id(capability_id: &str) -> String {
    capability_id.replace('.', "__")
}

/// Shared by every scripted `HostManagedModelGateway` test double in this
/// file that records raw requests (`RecordingGateway`,
/// `TriggerMutatorAttemptGateway`) — flattens every captured request's message
/// contents into one list.
async fn captured_message_contents(
    requests: &Arc<TokioMutex<Vec<HostManagedModelRequest>>>,
) -> Vec<String> {
    let guard = requests.lock().await;
    guard
        .iter()
        .flat_map(|req| req.messages.iter().map(|m| m.content.clone()))
        .collect()
}

#[derive(Debug, Default)]
struct RecordingGateway {
    requests: Arc<TokioMutex<Vec<HostManagedModelRequest>>>,
}

impl RecordingGateway {
    async fn captured_message_contents(&self) -> Vec<String> {
        captured_message_contents(&self.requests).await
    }

    async fn request_count_containing(&self, needle: &str) -> usize {
        let snapshot = self.requests.lock().await;
        snapshot
            .iter()
            .filter(|req| req.messages.iter().any(|m| m.content.contains(needle)))
            .count()
    }
}

#[async_trait]
impl HostManagedModelGateway for RecordingGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests.lock().await.push(request);
        Ok(HostManagedModelResponse::assistant_reply(
            "trigger e2e ok".to_string(),
        ))
    }
}

#[derive(Debug, Default)]
struct DeliveryJourneyGateway {
    requests: TokioMutex<Vec<HostManagedModelRequest>>,
}

impl DeliveryJourneyGateway {
    async fn request_count_containing(&self, needle: &str) -> usize {
        self.requests
            .lock()
            .await
            .iter()
            .filter(|request| {
                request
                    .messages
                    .iter()
                    .any(|message| message.content.contains(needle))
            })
            .count()
    }
}

#[async_trait]
impl HostManagedModelGateway for DeliveryJourneyGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let reply = if request
            .messages
            .iter()
            .any(|message| message.content.contains(QA_9B_PROMPT))
        {
            QA_9B_RESULT
        } else if request
            .messages
            .iter()
            .any(|message| message.content.contains(QA_9D_PROMPT))
        {
            QA_9D_RESULT
        } else {
            "unexpected scheduled-trigger prompt"
        };
        self.requests.lock().await.push(request);
        Ok(HostManagedModelResponse::assistant_reply(reply.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SlackWireMessage {
    url: String,
    authorization: Option<String>,
    body: Value,
}

#[derive(Debug, Default)]
struct FakeSlackProvider {
    wire_messages: StdMutex<Vec<SlackWireMessage>>,
    provider_messages: StdMutex<Vec<Value>>,
    next_message_id: AtomicUsize,
}

impl FakeSlackProvider {
    fn wire_messages(&self) -> Vec<SlackWireMessage> {
        self.wire_messages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn provider_messages(&self) -> Vec<Value> {
        self.provider_messages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl NetworkHttpEgress for FakeSlackProvider {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let authorization = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| value.clone());
        let body = serde_json::from_slice::<Value>(&request.body).unwrap_or(Value::Null);
        if request.url.ends_with("/api/chat.postMessage") {
            self.wire_messages
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(SlackWireMessage {
                    url: request.url.clone(),
                    authorization,
                    body: body.clone(),
                });
            self.provider_messages
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(body.clone());
        }
        let message_id = self.next_message_id.fetch_add(1, Ordering::SeqCst);
        let response_body = serde_json::json!({
            "ok": true,
            "channel": body["channel"],
            "ts": format!("1710000001.{message_id:06}"),
        })
        .to_string()
        .into_bytes();
        Ok(NetworkHttpResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            usage: NetworkUsage {
                request_bytes: request.body.len() as u64,
                response_bytes: response_body.len() as u64,
                resolved_ip: None,
            },
            body: response_body,
        })
    }
}

/// Model gateway for T0-5505-E2E: on its FIRST turn of the fired run,
/// attempts to register EVERY scheduled-trigger mutator capability
/// (`builtin.trigger_create`, `builtin.trigger_remove`, `builtin.trigger_pause`,
/// `builtin.trigger_resume`) as real provider tool calls — as a real native
/// provider tool call would — directly against the fired run's actual
/// (composed) capability port via `stream_model_with_capabilities`. This is
/// the same seam a production LLM-provider-backed gateway uses to turn a raw
/// tool-call response into a `CapabilityCallCandidate`
/// (`LoopCapabilityPort::register_provider_tool_call`), so registering
/// through it here exercises the *real* `PerSurfaceCapabilityDenyDecorator` /
/// `CapabilitySurfaceDenyFilter` composition chain, not a stand-in.
///
/// Generalized from a single `trigger_create`-only attempt (PR #5515 review:
/// "production mutator deny set only has create covered end-to-end — the
/// full-path poller test only attempts `builtin.trigger_create`") to cover
/// all four mutators from the SAME first turn:
/// `HostManagedModelResponse::capability_calls` already takes a `Vec`, so all
/// four registration attempts are made, and any that resolve are forwarded,
/// before the turn's single response is returned. This closes the drift risk
/// where the production `SCHEDULED_TRIGGER_DENIED_CAPABILITY_IDS` deny set
/// could accidentally drop `trigger_remove`/`trigger_pause`/`trigger_resume`
/// while only `trigger_create` stayed covered by a full-path (real host
/// composition) test.
///
/// `CapabilitySurfaceDenyFilter::register_provider_tool_call` resolves each
/// provider tool call to a capability id via its OWN
/// `provider_tool_call_capability_ids` override (delegating to `inner` so
/// deferred/disclosed tools still resolve — see #5149's progressive tool
/// disclosure), then scope-checks the resolved id against its deny set
/// before ever building a candidate. All four mutator ids are on the fix's
/// scheduled_trigger deny set, so every scope check fails closed with
/// `AgentLoopHostErrorKind::InvalidInvocation` /
/// "provider tool call targets a disabled capability" — registration never
/// reaches `inner.register_provider_tool_call`. `builtin.trigger_list` stays
/// permitted (verified directly against `tool_definitions()` while
/// developing this test), matching the fix's read-only carve-out.
///
/// Whichever registrations succeed (capability visible + permitted) are
/// forwarded together as one `capability_calls` response, which the loop
/// will actually dispatch — including staging the real JSON input through
/// the run's real `StagedCapabilityIo`, so a genuinely unpatched surface
/// would really create a second trigger and/or remove/pause/resume the
/// target trigger. If every registration is denied (the fixed, expected
/// behavior), there is nothing to dispatch and a plain reply is returned
/// instead so the run still terminates cleanly.
///
/// Every subsequent turn returns a plain assistant reply so the run
/// terminates after at most one capability-call round trip.
#[derive(Default)]
struct TriggerMutatorAttemptGateway {
    requests: Arc<TokioMutex<Vec<HostManagedModelRequest>>>,
    call_count: TokioMutex<usize>,
    /// Set by the test body, before the fire is triggered, to the id of the
    /// already-created legitimate trigger. `trigger_remove`/`trigger_pause`/
    /// `trigger_resume`'s attempts target this record — a real `trigger_id`
    /// is required to shape a realistic input for those capabilities' input
    /// schema, even though the scope-check denial happens before the payload
    /// is ever read.
    mutator_target_trigger_id: TokioMutex<Option<TriggerId>>,
    /// Empty until the first turn runs. Populated with one entry per
    /// attempted mutator capability id: `Ok(())` if the registration was
    /// accepted (capability was visible + permitted on this run's surface —
    /// the pre-fix behavior for that mutator), `Err(safe_summary)` if it was
    /// denied before a candidate could even be built (the fixed behavior).
    registration_outcomes: TokioMutex<BTreeMap<String, Result<(), String>>>,
}

impl TriggerMutatorAttemptGateway {
    async fn captured_message_contents(&self) -> Vec<String> {
        captured_message_contents(&self.requests).await
    }

    async fn set_mutator_target_trigger_id(&self, trigger_id: TriggerId) {
        *self.mutator_target_trigger_id.lock().await = Some(trigger_id);
    }

    async fn registration_outcomes(&self) -> BTreeMap<String, Result<(), String>> {
        self.registration_outcomes.lock().await.clone()
    }

    /// One provider tool call per scheduled-trigger mutator capability id,
    /// paired with a payload shaped for that capability's real input schema
    /// (see `ironclaw_host_runtime::first_party_tools::trigger_management`'s
    /// `TriggerCreateInput`/`TriggerRemoveInput`/`TriggerStateInput`).
    fn mutator_tool_calls(target_trigger_id: TriggerId) -> Vec<(&'static str, ProviderToolCall)> {
        let mutator_payload = json!({ "trigger_id": target_trigger_id.to_string() });
        [
            (
                TRIGGER_CREATE_CAPABILITY_ID,
                json!({
                    "name": SELF_CREATE_MARKER_TRIGGER_NAME,
                    "prompt": "run-time action steps -- this trigger must never be created",
                    "schedule": { "kind": "cron", "expression": "* * * * *", "timezone": "UTC" },
                }),
            ),
            (TRIGGER_REMOVE_CAPABILITY_ID, mutator_payload.clone()),
            (TRIGGER_PAUSE_CAPABILITY_ID, mutator_payload.clone()),
            (TRIGGER_RESUME_CAPABILITY_ID, mutator_payload),
        ]
        .into_iter()
        .map(|(capability_id, arguments)| {
            (
                capability_id,
                ProviderToolCall {
                    provider_id: "trigger-e2e-provider".to_string(),
                    provider_model_id: "trigger-e2e-model".to_string(),
                    turn_id: Some("trigger-e2e-self-create-turn".to_string()),
                    id: format!("trigger-e2e-mutator-call-{capability_id}"),
                    name: ProviderToolName::new(provider_tool_name_for_capability_id(
                        capability_id,
                    ))
                    .expect("mutator provider tool name is valid"),
                    arguments,
                    response_reasoning: None,
                    reasoning: None,
                    signature: None,
                },
            )
        })
        .collect()
    }
}

#[async_trait]
impl HostManagedModelGateway for TriggerMutatorAttemptGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        // Only reachable if the driver host ever omits the capability port
        // from the model request (it should not — the composed loop always
        // wires one). Recording + a plain reply keeps this branch harmless
        // either way instead of silently no-op'ing the test.
        self.requests.lock().await.push(request);
        Ok(HostManagedModelResponse::assistant_reply(
            "trigger e2e ok (no capability port on request)".to_string(),
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests.lock().await.push(request);
        let is_first_call = {
            let mut count = self.call_count.lock().await;
            *count += 1;
            *count == 1
        };

        if !is_first_call {
            return Ok(HostManagedModelResponse::assistant_reply(
                "trigger e2e ok".to_string(),
            ));
        }

        let target_trigger_id = self
            .mutator_target_trigger_id
            .lock()
            .await
            .expect("test sets mutator_target_trigger_id before triggering the fire");

        let mut accepted_candidates = Vec::new();
        for (capability_id, tool_call) in Self::mutator_tool_calls(target_trigger_id) {
            let registration = capabilities
                .register_provider_tool_call(RegisterProviderToolCallRequest::new(tool_call))
                .await;
            let outcome = match registration {
                Ok(candidate) => {
                    accepted_candidates.push(candidate);
                    Ok(())
                }
                Err(error) => Err(error.safe_summary.clone()),
            };
            self.registration_outcomes
                .lock()
                .await
                .insert(capability_id.to_string(), outcome);
        }

        if accepted_candidates.is_empty() {
            // Every mutator registration was rejected before any capability
            // call candidate could exist, so there is nothing to dispatch.
            // Reply plainly so the run still terminates cleanly — this is
            // what proves the fix only blocks the mutators, not the fire
            // itself.
            Ok(HostManagedModelResponse::assistant_reply(
                "capability unavailable; continuing".to_string(),
            ))
        } else {
            // At least one mutator was (incorrectly) permitted — forward it
            // so the loop actually dispatches it, letting the repository-side
            // assertions below catch the regression too.
            Ok(HostManagedModelResponse::capability_calls(
                accepted_candidates,
                "",
            ))
        }
    }
}

/// Poll `repo` until `predicate` returns `true` or `deadline` elapses.
///
/// Returns the last record seen. If the predicate is satisfied before the
/// deadline, the returned record satisfies the predicate. If the deadline
/// elapses, the returned record is the last one read (which may not satisfy
/// the predicate — callers should then let the existing assertions fail with
/// the diagnostic they already carry).
///
/// Used by the happy-path and Recurring tests to wait for the settle writes
/// (step 3: `mark_fire_accepted` / `mark_fire_replayed`) to become visible
/// after the first-pass loop breaks on `record_was_mutated && prompt_seen`.
async fn wait_for_settled<F>(
    repo: &Arc<dyn TriggerRepository>,
    tenant_id: &TenantId,
    trigger_id: TriggerId,
    deadline: Duration,
    mut predicate: F,
) -> TriggerRecord
where
    F: FnMut(&TriggerRecord) -> bool,
{
    let stop = Instant::now() + deadline;
    let mut last: Option<TriggerRecord> = None;
    while Instant::now() < stop {
        let current = repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("get_trigger")
            .expect("record present");
        if predicate(&current) {
            return current;
        }
        last = Some(current);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    last.expect("at least one read should have succeeded in wait_for_settled")
}

async fn wait_for_mutator_registration_outcomes(
    gateway: &TriggerMutatorAttemptGateway,
    deadline: Duration,
) -> (Vec<String>, BTreeMap<String, Result<(), String>>) {
    let stop = Instant::now() + deadline;
    loop {
        let captured_contents = gateway.captured_message_contents().await;
        let outcomes = gateway.registration_outcomes().await;
        if outcomes.len() == 4 || Instant::now() >= stop {
            return (captured_contents, outcomes);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn current_minute_slot() -> chrono::DateTime<Utc> {
    let now_seconds = Utc::now().timestamp();
    let minute_seconds = now_seconds - now_seconds.rem_euclid(60);
    Utc.timestamp_opt(minute_seconds, 0)
        .single()
        .expect("valid minute timestamp")
}

/// Shared runtime builder. Every test passes the `TriggerPollerSettings` it
/// wants; identity, runtime policy, and model-gateway override are shared.
///
/// Generic over the concrete gateway type (rather than
/// `Arc<dyn HostManagedModelGateway>`) so every call site — the existing
/// `RecordingGateway` ones and `TriggerMutatorAttemptGateway`'s — passes its
/// `Arc<Concrete>` unchanged; the unsized coercion to
/// `Arc<dyn HostManagedModelGateway>` happens once, here, instead of at every
/// call site.
async fn build_runtime_with<G: HostManagedModelGateway + 'static>(
    root: &tempfile::TempDir,
    model_gateway: Arc<G>,
    trigger_poller: TriggerPollerSettings,
) -> RebornRuntime {
    seed_test_secret_master_key(root.path());
    let host_home_root = root.path().join("host-home");
    std::fs::create_dir_all(&host_home_root).expect("host home root");
    let input = local_runtime_build_input_with_options(
        RebornCompositionProfile::LocalDevYolo,
        USER,
        root.path().join("local-dev"),
        RebornRuntimeProfileOptions {
            confirm_host_access: true,
        },
    )
    .expect("local-yolo runtime input")
    .with_local_dev_confirmed_host_home_root(host_home_root);

    let input = RebornRuntimeInput::from_build_input(input)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: TENANT.to_string(),
            agent_id: AGENT.to_string(),
            source_binding_id: "trigger-e2e-source".to_string(),
            reply_target_binding_id: "trigger-e2e-reply".to_string(),
        })
        .with_trigger_poller_settings(trigger_poller)
        .with_model_gateway_override(model_gateway);

    build_reborn_runtime(input).await.expect("runtime builds")
}

async fn build_runtime_with_slack_delivery(
    root: &tempfile::TempDir,
    model_gateway: Arc<DeliveryJourneyGateway>,
    slack_provider: Arc<FakeSlackProvider>,
) -> RebornRuntime {
    seed_test_secret_master_key(root.path());
    let host_home_root = root.path().join("host-home");
    std::fs::create_dir_all(&host_home_root).expect("host home root");
    let input = local_runtime_build_input_with_options(
        RebornCompositionProfile::LocalDevYolo,
        USER,
        root.path().join("local-dev"),
        RebornRuntimeProfileOptions {
            confirm_host_access: true,
        },
    )
    .expect("local-yolo runtime input")
    .with_local_dev_confirmed_host_home_root(host_home_root)
    .with_bundled_first_party_for_test()
    .with_network_http_egress_for_test(slack_provider)
    .with_channel_extension_bindings(vec![ChannelExtensionBinding {
        extension_id: "slack".to_string(),
        adapter: Arc::new(ironclaw_slack_extension::SlackChannelAdapter),
        preference_target_codec: Some(Arc::new(
            ironclaw_slack_extension::SlackPreferenceTargetCodec,
        )),
    }]);
    let input = RebornRuntimeInput::from_build_input(input)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: TENANT.to_string(),
            agent_id: AGENT.to_string(),
            source_binding_id: "trigger-delivery-e2e-source".to_string(),
            reply_target_binding_id: "trigger-delivery-e2e-reply".to_string(),
        })
        .with_trigger_poller_settings(
            TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test()
                .with_worker_config(
                    TriggerPollerWorkerConfig::default()
                        .set_poll_interval(Duration::from_millis(20)),
                ),
        )
        .with_model_gateway_override(model_gateway);

    build_reborn_runtime(input).await.expect("runtime builds")
}

async fn configure_and_activate_slack_for_delivery(runtime: &RebornRuntime) {
    let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack")
        .expect("valid Slack package ref");
    runtime
        .install_extension_for_test(package_ref.clone())
        .await
        .expect("install Slack through the production lifecycle port");
    runtime
        .configure_admin_group_for_test(
            "extension.slack",
            vec![
                (
                    "slack_bot_token".to_string(),
                    "xoxb-trigger-e2e".to_string(),
                ),
                (
                    "slack_signing_secret".to_string(),
                    "signing-trigger-e2e".to_string(),
                ),
                ("slack_team_id".to_string(), SLACK_TEAM.to_string()),
                ("slack_api_app_id".to_string(), "A-TRIGGER-E2E".to_string()),
                (
                    "slack_installation_id".to_string(),
                    "I-TRIGGER-E2E".to_string(),
                ),
                (
                    "slack_bot_user_id".to_string(),
                    "U-BOT-TRIGGER-E2E".to_string(),
                ),
                (
                    "slack_oauth_client_id".to_string(),
                    "trigger-e2e-client".to_string(),
                ),
                (
                    "slack_oauth_client_secret".to_string(),
                    "trigger-e2e-client-secret".to_string(),
                ),
            ],
        )
        .await
        .expect("configure Slack through the production admin-configuration resolver");
    runtime
        .activate_extension_for_test(package_ref)
        .await
        .expect("activate Slack through the production lifecycle port");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !runtime
        .active_channel_preference_codec_ids_for_test()
        .iter()
        .any(|extension_id| extension_id == "slack")
    {
        assert!(
            Instant::now() < deadline,
            "Slack activation never became routable through the generic channel host"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn slack_reply_target(channel: &str, actor: Option<&str>) -> ReplyTargetBindingRef {
    let installation =
        ProductAdapterInstallationId::new("slack").expect("valid deployment installation id");
    let agent = AgentId::new(AGENT).expect("valid agent id");
    match actor {
        Some(actor) => ironclaw_slack_extension::slack_personal_dm_reply_target_binding_ref(
            &installation,
            &agent,
            None,
            SLACK_TEAM,
            channel,
            actor,
        )
        .expect("valid Slack DM target"),
        None => ironclaw_slack_extension::slack_shared_channel_reply_target_binding_ref(
            &installation,
            &agent,
            None,
            SLACK_TEAM,
            channel,
        )
        .expect("valid Slack shared-channel target"),
    }
}

async fn configure_delivery_targets(runtime: &RebornRuntime) {
    let tenant_id = TenantId::new(TENANT).expect("valid tenant id");
    let user_id = UserId::new(USER).expect("valid user id");
    register_delivery_targets(runtime);
    runtime
        .local_dev_outbound_preferences_for_test()
        .expect("local runtime exposes outbound preferences")
        .put_communication_preference(CommunicationPreferenceRecord {
            scope: DeliveryDefaultScope::personal(tenant_id, user_id.clone()),
            final_reply_target: Some(slack_reply_target(SLACK_DEFAULT_DM, Some(SLACK_USER))),
            progress_target: None,
            approval_prompt_target: None,
            auth_prompt_target: None,
            default_modality: None,
            updated_at: Utc::now(),
            updated_by: user_id,
        })
        .await
        .expect("seed the creator's default Slack DM");
}

fn register_delivery_targets(runtime: &RebornRuntime) {
    runtime
        .register_static_outbound_delivery_target_for_test(
            "qa-9b-static",
            RebornOutboundDeliveryTargetId::new(QA_9B_TARGET_ID).expect("valid default target id"),
            "slack",
            "QA 9B default DM",
            Some("scheduled-trigger default target"),
            slack_reply_target(SLACK_DEFAULT_DM, Some(SLACK_USER)),
        )
        .expect("register the default Slack DM target");
    runtime
        .register_static_outbound_delivery_target_for_test(
            "qa-9d-static",
            RebornOutboundDeliveryTargetId::new(QA_9D_TARGET_ID)
                .expect("valid per-trigger target id"),
            "slack",
            "QA 9D override",
            Some("scheduled-trigger target override"),
            slack_reply_target(SLACK_PER_TRIGGER_CHANNEL, None),
        )
        .expect("register the per-trigger Slack target");
}

async fn pair_trigger_creator(runtime: &RebornRuntime) {
    runtime
        .trigger_conversation_pairing()
        .expect("trigger poller exposes its conversation pairing service")
        .pair_external_actor(
            TenantId::new(TENANT).expect("valid tenant id"),
            AdapterKind::new(TRIGGER_TRUSTED_ADAPTER_KIND).expect("valid adapter kind"),
            AdapterInstallationId::new(TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID)
                .expect("valid trigger installation id"),
            ExternalActorRef::new(TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, USER)
                .expect("valid trigger actor ref"),
            UserId::new(USER).expect("valid user id"),
        )
        .await
        .expect("pair trigger creator");
}

async fn seed_due_delivery_trigger(
    repository: &Arc<dyn TriggerRepository>,
    prompt: &str,
    delivery_target: Option<&str>,
) -> TriggerId {
    let trigger_id = TriggerId::new();
    let fire_at = Utc::now() - chrono::Duration::seconds(120);
    repository
        .upsert_trigger(TriggerRecord {
            trigger_id,
            tenant_id: TenantId::new(TENANT).expect("valid tenant id"),
            creator_user_id: UserId::new(USER).expect("valid user id"),
            agent_id: Some(AgentId::new(AGENT).expect("valid agent id")),
            project_id: None,
            name: format!("{prompt} trigger"),
            source: TriggerSourceKind::Schedule,
            schedule: TriggerSchedule::once(fire_at, "UTC").expect("valid once schedule"),
            prompt: prompt.to_string(),
            delivery_target: delivery_target.map(|target| {
                TriggerDeliveryTargetId::new(target).expect("valid trigger delivery target")
            }),
            state: TriggerState::Scheduled,
            next_run_at: fire_at,
            last_run_at: None,
            last_fired_slot: None,
            last_status: None,
            active_fire_slot: None,
            active_run_ref: None,
            created_at: Utc::now(),
        })
        .await
        .expect("seed due delivery trigger");
    trigger_id
}

async fn wait_for_delivered_run(
    repository: &Arc<dyn TriggerRepository>,
    delivery_store: &Arc<dyn TriggeredRunDeliveryStore>,
    trigger_id: TriggerId,
) -> TurnRunId {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let history = repository
            .list_trigger_run_history(
                TenantId::new(TENANT).expect("valid tenant id"),
                trigger_id,
                1,
            )
            .await
            .expect("read trigger run history");
        if let Some(run_id) = history.first().and_then(|run| run.run_id)
            && let Some(record) = delivery_store
                .load_triggered_run_delivery(run_id)
                .await
                .expect("read triggered delivery outcome")
        {
            assert_eq!(
                record.outcome,
                TriggeredRunDeliveryOutcomeKind::Delivered,
                "triggered run must reach the provider successfully"
            );
            return run_id;
        }
        assert!(
            Instant::now() < deadline,
            "trigger {trigger_id} did not record a delivery outcome within 15s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Same as [`build_runtime_with`], but with an explicit
/// [`ToolDisclosureMode`] instead of the implicit `Off` default. Kept
/// separate (rather than adding a parameter to `build_runtime_with`) because
/// `build_runtime_with` has 8 existing call sites for concerns unrelated to
/// tool disclosure — churning all of them to thread through a mode they never
/// vary would be unnecessary here.
async fn build_runtime_with_tool_disclosure<G: HostManagedModelGateway + 'static>(
    root: &tempfile::TempDir,
    model_gateway: Arc<G>,
    trigger_poller: TriggerPollerSettings,
    tool_disclosure: ToolDisclosureMode,
) -> RebornRuntime {
    seed_test_secret_master_key(root.path());
    let host_home_root = root.path().join("host-home");
    std::fs::create_dir_all(&host_home_root).expect("host home root");
    let input = local_runtime_build_input_with_options(
        RebornCompositionProfile::LocalDevYolo,
        USER,
        root.path().join("local-dev"),
        RebornRuntimeProfileOptions {
            confirm_host_access: true,
        },
    )
    .expect("local-yolo runtime input")
    .with_local_dev_confirmed_host_home_root(host_home_root);

    let input = RebornRuntimeInput::from_build_input(input)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: TENANT.to_string(),
            agent_id: AGENT.to_string(),
            source_binding_id: "trigger-e2e-source".to_string(),
            reply_target_binding_id: "trigger-e2e-reply".to_string(),
        })
        .with_trigger_poller_settings(trigger_poller)
        .with_model_gateway_override(model_gateway)
        .with_tool_disclosure(tool_disclosure);

    build_reborn_runtime(input).await.expect("runtime builds")
}

/// Keep parallel runtime tests off the ambient OS keychain. The production
/// resolver deliberately prefers this cached dotfile, so this still exercises
/// the real local-dev secret-store construction without process-global env
/// mutation or platform keychain serialization.
fn seed_test_secret_master_key(root: &Path) {
    let local_dev_root = root.join("local-dev");
    std::fs::create_dir_all(&local_dev_root).expect("local-dev root");
    let key_path = local_dev_root.join(".reborn-local-dev-secrets-master-key");
    if !key_path.exists() {
        std::fs::write(key_path, TEST_SECRET_MASTER_KEY).expect("seed test secret master key");
    }
}

async fn invoke_trigger_create(runtime: &RebornRuntime, input: Value) -> Value {
    // The Tools-settings global auto-approve switch is authoritative for
    // first-party tool dispatch; turn it on for the trigger management
    // scope so the create call (and the poller-submitted turn that shares the
    // same tenant/user) exercise the dispatch path instead of stopping at the
    // per-tool approval gate.
    let auto_approve = runtime
        .local_dev_auto_approve_settings_for_test()
        .expect("local-dev exposes auto-approve settings for test");
    let auto_approve_scope = trigger_management_execution_context().resource_scope;
    auto_approve
        .set(ironclaw_approvals::AutoApproveSettingInput {
            updated_by: Principal::User(auto_approve_scope.user_id.clone()),
            scope: auto_approve_scope,
            enabled: true,
        })
        .await
        .expect("enable global auto-approve for trigger management dispatch");

    let host_runtime = runtime
        .host_runtime_for_test()
        .expect("runtime exposes host runtime");
    let outcome = host_runtime
        .invoke_capability((
            trigger_management_execution_context(),
            CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID).expect("capability id"),
            ResourceEstimate::default(),
            input,
        ))
        .await
        .expect("trigger create invocation completes");
    let RuntimeCapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected trigger create to complete, got {outcome:?}");
    };
    completed.output
}

fn trigger_management_execution_context() -> ExecutionContext {
    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");
    let extension_id = ExtensionId::new("trigger-e2e-caller").expect("extension id");
    let mut context = ExecutionContext::local_default(
        user_id.clone(),
        extension_id.clone(),
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID).expect("capability id"),
                grantee: Principal::Extension(extension_id),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![
                        EffectKind::DispatchCapability,
                        EffectKind::ExternalWrite,
                    ],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        },
        MountView::default(),
    )
    .expect("execution context");
    context.tenant_id = tenant_id.clone();
    context.agent_id = Some(agent_id.clone());
    context.project_id = None;
    context.resource_scope.tenant_id = tenant_id;
    context.resource_scope.agent_id = Some(agent_id);
    context.resource_scope.project_id = None;
    context.run_id = Some(RunId::new());
    context
}

#[tokio::test]
async fn trigger_poller_drives_trusted_ingress_for_due_scheduled_trigger() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway {
        requests: Arc::new(TokioMutex::new(Vec::new())),
    });

    let runtime = build_runtime_with(
        &root,
        Arc::clone(&recording_gateway),
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test().with_worker_config(
            TriggerPollerWorkerConfig::default().set_poll_interval(Duration::from_millis(20)),
        ),
    )
    .await;

    let repo = runtime.trigger_repository();
    let pairing = runtime
        .trigger_conversation_pairing()
        .expect("trigger poller runtime exposes conversation pairing service");

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");
    let trigger_id = TriggerId::new();

    // Seed the trigger creator's actor pairing through the production
    // `ConversationActorPairingService` API. The trusted trigger
    // submission path fails closed for unpaired actors by design; in
    // production, onboarding establishes this pairing before any trigger
    // can be created. The adapter kind / installation id / external actor
    // ref must match the trusted trigger constants used for trigger fires.
    pairing
        .pair_external_actor(
            tenant_id.clone(),
            AdapterKind::new(TRIGGER_TRUSTED_ADAPTER_KIND).expect("adapter kind"),
            AdapterInstallationId::new(TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID)
                .expect("installation id"),
            ExternalActorRef::new(TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, user_id.as_str())
                .expect("actor ref"),
            user_id.clone(),
        )
        .await
        .expect("pair external actor for trigger creator");

    let record = TriggerRecord {
        trigger_id,
        tenant_id: tenant_id.clone(),
        creator_user_id: user_id,
        agent_id: Some(agent_id),
        project_id: None,
        name: "trigger-e2e-test".to_string(),
        source: TriggerSourceKind::Schedule,
        // One-shot: fires once, then becomes Completed via clear_active_fire.
        schedule: TriggerSchedule::once(Utc::now() - chrono::Duration::seconds(120), "UTC")
            .expect("valid once schedule"),
        prompt: TRIGGER_PROMPT.to_string(),
        delivery_target: None,
        state: TriggerState::Scheduled,
        next_run_at: Utc::now() - chrono::Duration::seconds(120),
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: Utc::now(),
    };

    repo.upsert_trigger(record.clone())
        .await
        .expect("upsert trigger record");

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut record_was_mutated = false;
    let mut prompt_seen = false;

    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;

        let current = repo
            .get_trigger(tenant_id.clone(), record.trigger_id)
            .await
            .expect("get_trigger")
            .expect("record present");

        let mutated = current.last_fired_slot.is_some()
            || current.last_run_at.is_some()
            || current.last_status.is_some()
            || current.active_fire_slot.is_some()
            || current.state == TriggerState::Completed;

        if mutated {
            record_was_mutated = true;
            // Only poll the gateway after the record was touched.
            let contents = recording_gateway.captured_message_contents().await;
            if contents
                .iter()
                .any(|content| content.contains(TRIGGER_PROMPT))
            {
                prompt_seen = true;
            }
        }

        if record_was_mutated && prompt_seen {
            break;
        }
    }

    // Wait for the settle writes (mark_fire_accepted sets last_fired_slot, last_run_at)
    // and for clear_active_fire to run (which transitions state to Completed for
    // CompleteAfterFirstFire triggers). The first-pass loop breaks as soon as the
    // claim+prompt are seen; the settle may still be in flight.
    let final_record = wait_for_settled(
        &repo,
        &tenant_id,
        record.trigger_id,
        Duration::from_secs(5),
        |r| {
            r.last_fired_slot.is_some()
                && r.last_run_at.is_some()
                && r.state == TriggerState::Completed
        },
    )
    .await;

    runtime.shutdown().await.expect("runtime shutdown");

    // Final snapshot for diagnostics, once. Taken after shutdown so a request
    // submitted between snapshot and shutdown completion cannot be invisible.
    let captured_contents = recording_gateway.captured_message_contents().await;

    assert!(
        record_was_mutated,
        "poller did not mutate trigger record within 15s — record: {final_record:?}"
    );
    assert!(
        prompt_seen,
        "LLM gateway never received a request containing the trigger prompt within 15s \
         — captured_messages: {captured_contents:?}"
    );
    assert!(
        final_record.last_fired_slot.is_some(),
        "once schedule: last_fired_slot should be set after fire — record: {final_record:?}",
    );
    assert!(
        final_record.last_run_at.is_some(),
        "once schedule: last_run_at should be set after fire — record: {final_record:?}",
    );
    assert_eq!(
        final_record.last_status,
        Some(TriggerRunStatus::Ok),
        "once schedule: last_status should be Ok after fire — record: {final_record:?}",
    );
    assert_eq!(
        final_record.state,
        TriggerState::Completed,
        "once schedule: state must be Completed after clear_active_fire — record: {final_record:?}",
    );
}

/// QA-9B + QA-9D whole-path regression:
///
/// due trigger -> trusted ingress -> real Reborn run -> persisted final reply
/// -> generic triggered-delivery hook -> real Slack adapter -> host-mediated
/// credential injection -> fake Slack HTTP boundary.
///
/// The two arms prove that the creator's default DM is used when the trigger
/// has no target and that an explicit per-trigger target overrides that
/// default. Re-polling and rebuilding the runtime over the same durable store
/// must not repeat either provider mutation.
#[tokio::test]
async fn scheduled_trigger_results_reach_exact_slack_targets_once_across_restart() {
    let root = tempfile::tempdir().expect("tempdir");
    let model_gateway = Arc::new(DeliveryJourneyGateway::default());
    let slack_provider = Arc::new(FakeSlackProvider::default());
    let runtime = build_runtime_with_slack_delivery(
        &root,
        Arc::clone(&model_gateway),
        Arc::clone(&slack_provider),
    )
    .await;

    configure_and_activate_slack_for_delivery(&runtime).await;
    configure_delivery_targets(&runtime).await;
    pair_trigger_creator(&runtime).await;

    let repository = runtime.trigger_repository();
    let delivery_store = runtime
        .triggered_run_delivery_store_for_test()
        .expect("local runtime exposes the production triggered-delivery store");
    let default_target_trigger = seed_due_delivery_trigger(&repository, QA_9B_PROMPT, None).await;
    let explicit_target_trigger =
        seed_due_delivery_trigger(&repository, QA_9D_PROMPT, Some(QA_9D_TARGET_ID)).await;

    wait_for_delivered_run(&repository, &delivery_store, default_target_trigger).await;
    wait_for_delivered_run(&repository, &delivery_store, explicit_target_trigger).await;

    let provider_messages = slack_provider.provider_messages();
    assert_eq!(
        provider_messages.len(),
        2,
        "one provider-side message per scheduled trigger: {provider_messages:?}"
    );
    assert_slack_message(
        &provider_messages,
        QA_9B_RESULT,
        SLACK_DEFAULT_DM,
        "QA-9B default delivery",
    );
    assert_slack_message(
        &provider_messages,
        QA_9D_RESULT,
        SLACK_PER_TRIGGER_CHANNEL,
        "QA-9D per-trigger override",
    );

    let wire_messages = slack_provider.wire_messages();
    assert_eq!(
        wire_messages.len(),
        2,
        "exactly two Slack wire mutations: {wire_messages:?}"
    );
    assert!(
        wire_messages.iter().all(|message| {
            message.url == "https://slack.com/api/chat.postMessage"
                && message.authorization.as_deref() == Some("Bearer xoxb-trigger-e2e")
                && provider_messages.contains(&message.body)
        }),
        "each provider mutation must cross the real Slack adapter and host credential boundary: \
         {wire_messages:?}"
    );
    assert_eq!(
        model_gateway.request_count_containing(QA_9B_PROMPT).await,
        1,
        "QA-9B must execute exactly one model run"
    );
    assert_eq!(
        model_gateway.request_count_containing(QA_9D_PROMPT).await,
        1,
        "QA-9D must execute exactly one model run"
    );

    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        slack_provider.provider_messages().len(),
        2,
        "re-polling completed one-shot triggers must not duplicate delivery"
    );
    runtime.shutdown().await.expect("first runtime shutdown");

    let restarted = build_runtime_with_slack_delivery(
        &root,
        Arc::clone(&model_gateway),
        Arc::clone(&slack_provider),
    )
    .await;
    register_delivery_targets(&restarted);
    tokio::time::sleep(Duration::from_millis(300)).await;
    restarted
        .shutdown()
        .await
        .expect("restarted runtime shutdown");

    assert_eq!(
        slack_provider.provider_messages().len(),
        2,
        "restart over the same durable trigger state must not duplicate provider effects"
    );
    assert_eq!(
        model_gateway.request_count_containing(QA_9B_PROMPT).await,
        1,
        "restart must not rerun QA-9B"
    );
    assert_eq!(
        model_gateway.request_count_containing(QA_9D_PROMPT).await,
        1,
        "restart must not rerun QA-9D"
    );
}

fn assert_slack_message(
    messages: &[Value],
    expected_text: &str,
    expected_channel: &str,
    scenario: &str,
) {
    let matching = messages.iter().filter(|message| {
        message["channel"] == expected_channel
            && message["text"]
                .as_str()
                .is_some_and(|text| text.contains(expected_text))
    });
    assert_eq!(
        matching.count(),
        1,
        "{scenario} must create exactly one message in {expected_channel}: {messages:?}"
    );
}

#[tokio::test]
async fn builtin_trigger_create_pairs_creator_and_poller_submits_turn() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway {
        requests: Arc::new(TokioMutex::new(Vec::new())),
    });

    let runtime = build_runtime_with(
        &root,
        Arc::clone(&recording_gateway),
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test().with_worker_config(
            TriggerPollerWorkerConfig::default().set_poll_interval(Duration::from_millis(20)),
        ),
    )
    .await;

    let created = invoke_trigger_create(
        &runtime,
        json!({
            "name": "trigger-e2e-created-by-tool",
            "prompt": TRIGGER_PROMPT,
            "schedule": { "kind": "cron", "expression": "* * * * *", "timezone": "UTC" }
        }),
    )
    .await;
    assert_eq!(
        created["trigger"]["name"],
        json!("trigger-e2e-created-by-tool")
    );
    assert_eq!(created["trigger"]["state"], json!("scheduled"));
    assert!(created["trigger"]["last_status"].is_null());
    assert!(created["trigger"]["prompt"].is_null());
    assert!(created["trigger"]["tenant_id"].is_null());
    assert!(created["trigger"]["creator_user_id"].is_null());

    let repo = runtime.trigger_repository();
    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let trigger_id = TriggerId::parse(
        created["trigger"]["trigger_id"]
            .as_str()
            .expect("created trigger id"),
    )
    .expect("valid trigger id");

    let mut record = repo
        .get_trigger(tenant_id.clone(), trigger_id)
        .await
        .expect("get created trigger")
        .expect("created trigger persisted");
    assert_eq!(record.prompt, TRIGGER_PROMPT);
    assert_eq!(record.creator_user_id, user_id);
    assert_eq!(record.name, "trigger-e2e-created-by-tool");

    let original_next_run_at = record.next_run_at;
    record.next_run_at = Utc::now() - chrono::Duration::seconds(120);
    repo.upsert_trigger(record.clone())
        .await
        .expect("make created trigger due");

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut record_was_mutated = false;
    let mut prompt_seen = false;

    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;

        let current = repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("get trigger")
            .expect("record present");

        let mutated = current.last_fired_slot.is_some()
            || current.last_run_at.is_some()
            || current.last_status.is_some()
            || current.active_fire_slot.is_some();

        if mutated {
            record_was_mutated = true;
            let contents = recording_gateway.captured_message_contents().await;
            if contents
                .iter()
                .any(|content| content.contains(TRIGGER_PROMPT))
            {
                prompt_seen = true;
            }
        }

        if record_was_mutated && prompt_seen {
            break;
        }
    }

    let settled = wait_for_settled(&repo, &tenant_id, trigger_id, Duration::from_secs(5), |r| {
        r.last_fired_slot.is_some() && r.next_run_at > original_next_run_at
    })
    .await;

    runtime.shutdown().await.expect("runtime shutdown");

    let captured_contents = recording_gateway.captured_message_contents().await;
    assert!(
        record_was_mutated,
        "poller did not mutate trigger created through builtin.trigger_create — record: {settled:?}",
    );
    assert!(
        prompt_seen,
        "LLM gateway never received trigger prompt for builtin-created trigger — \
         captured_messages: {captured_contents:?}"
    );
    assert_eq!(settled.last_status, Some(TriggerRunStatus::Ok));
    assert!(
        settled.last_run_at.is_some(),
        "builtin-created trigger should record last_run_at after poller fire"
    );
}

#[tokio::test]
async fn builtin_created_recurring_trigger_fires_again_after_first_run_settles() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway {
        requests: Arc::new(TokioMutex::new(Vec::new())),
    });

    let runtime = build_runtime_with(
        &root,
        Arc::clone(&recording_gateway),
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test().with_worker_config(
            TriggerPollerWorkerConfig::default().set_poll_interval(Duration::from_millis(20)),
        ),
    )
    .await;

    let created = invoke_trigger_create(
        &runtime,
        json!({
            "name": "trigger-e2e-created-by-tool-fires-twice",
            "prompt": TRIGGER_PROMPT,
            "schedule": { "kind": "cron", "expression": "* * * * *", "timezone": "UTC" }
        }),
    )
    .await;

    let repo = runtime.trigger_repository();
    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let trigger_id = TriggerId::parse(
        created["trigger"]["trigger_id"]
            .as_str()
            .expect("created trigger id"),
    )
    .expect("valid trigger id");

    let mut record = repo
        .get_trigger(tenant_id.clone(), trigger_id)
        .await
        .expect("get created trigger")
        .expect("created trigger persisted");
    let first_due_slot = current_minute_slot() - chrono::Duration::minutes(1);
    let second_due_slot = record
        .schedule
        .next_slot_after(first_due_slot)
        .expect("valid recurring schedule")
        .expect("recurring schedule should have a second slot");
    record.next_run_at = first_due_slot;
    repo.upsert_trigger(record.clone())
        .await
        .expect("make first recurring slot due");

    let second = wait_for_settled(
        &repo,
        &tenant_id,
        trigger_id,
        Duration::from_secs(15),
        |r| {
            r.last_fired_slot
                .map(|slot| slot >= second_due_slot)
                .unwrap_or(false)
                && r.last_run_at.is_some()
                && r.last_status == Some(TriggerRunStatus::Ok)
                && r.active_fire_slot.is_none()
                && r.active_run_ref.is_none()
                && r.next_run_at > second_due_slot
        },
    )
    .await;

    runtime.shutdown().await.expect("runtime shutdown");

    let request_count = recording_gateway
        .request_count_containing(TRIGGER_PROMPT)
        .await;
    assert!(
        request_count >= 2,
        "recurring trigger should submit once per due slot — requests containing prompt: {request_count}"
    );
    assert_eq!(
        second.state,
        TriggerState::Scheduled,
        "recurring trigger must remain Scheduled after the second fire — record: {second:?}"
    );
    assert_eq!(
        second.last_status,
        Some(TriggerRunStatus::Ok),
        "second fire should settle successfully — record: {second:?}"
    );
}

#[tokio::test]
async fn trigger_conversation_pairing_returns_none_when_poller_disabled() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway {
        requests: Arc::new(TokioMutex::new(Vec::new())),
    });

    // Use the default settings (enabled: false) — do NOT call
    // with_trigger_poller_settings with enabled: true.
    let runtime = build_runtime_with(
        &root,
        Arc::clone(&recording_gateway),
        TriggerPollerSettings::default(),
    )
    .await;

    // The trigger repository is built regardless of poller state.
    let _repo = runtime.trigger_repository();

    // When the poller is disabled, no conversation pairing service is wired.
    assert!(
        runtime.trigger_conversation_pairing().is_none(),
        "trigger_conversation_pairing should be None when poller is disabled"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn trigger_poller_does_not_fire_trigger_with_future_next_run_at() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway {
        requests: Arc::new(TokioMutex::new(Vec::new())),
    });

    let runtime = build_runtime_with(
        &root,
        Arc::clone(&recording_gateway),
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test().with_worker_config(
            TriggerPollerWorkerConfig::default().set_poll_interval(Duration::from_millis(20)),
        ),
    )
    .await;

    let repo = runtime.trigger_repository();
    let pairing = runtime
        .trigger_conversation_pairing()
        .expect("trigger poller runtime exposes conversation pairing service");

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");
    let trigger_id = TriggerId::new();

    pairing
        .pair_external_actor(
            tenant_id.clone(),
            AdapterKind::new(TRIGGER_TRUSTED_ADAPTER_KIND).expect("adapter kind"),
            AdapterInstallationId::new(TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID)
                .expect("installation id"),
            ExternalActorRef::new(TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, user_id.as_str())
                .expect("actor ref"),
            user_id.clone(),
        )
        .await
        .expect("pair external actor for trigger creator");

    // Seed a trigger that is NOT due — next_run_at is one hour in the future.
    let record = TriggerRecord {
        trigger_id,
        tenant_id: tenant_id.clone(),
        creator_user_id: user_id,
        agent_id: Some(agent_id),
        project_id: None,
        name: "trigger-e2e-future".to_string(),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::cron("* * * * *").expect("valid cron expression"),
        prompt: TRIGGER_PROMPT.to_string(),
        delivery_target: None,
        state: TriggerState::Scheduled,
        next_run_at: Utc::now() + chrono::Duration::seconds(3600),
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: Utc::now(),
    };

    repo.upsert_trigger(record.clone())
        .await
        .expect("upsert trigger record");

    // Sleep for ~500ms — 25 poll cycles at 20ms. Generous margin.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Clone the repo handle before shutdown (Arc is cheap to clone).
    let repo_after = repo.clone();

    runtime.shutdown().await.expect("runtime shutdown");

    // Snapshot captured_contents AFTER shutdown so a request submitted between
    // snapshot and shutdown completion cannot produce a false-green result.
    // The recording_gateway Arc is independent of the runtime.
    let captured_contents = recording_gateway.captured_message_contents().await;

    let current = repo_after
        .get_trigger(tenant_id.clone(), record.trigger_id)
        .await
        .expect("get_trigger")
        .expect("record present");

    assert!(
        current.last_fired_slot.is_none(),
        "poller should not have fired a future trigger — last_fired_slot: {:?}",
        current.last_fired_slot
    );
    assert!(
        current.last_run_at.is_none(),
        "poller should not have run a future trigger — last_run_at: {:?}",
        current.last_run_at
    );
    assert!(
        current.last_status.is_none(),
        "poller should not have set a status on a future trigger — last_status: {:?}",
        current.last_status
    );
    assert!(
        current.active_fire_slot.is_none(),
        "poller should not have set active_fire_slot on a future trigger — active_fire_slot: {:?}",
        current.active_fire_slot
    );
    assert_eq!(
        current.state,
        TriggerState::Scheduled,
        "future trigger should remain Scheduled — state: {:?}",
        current.state
    );
    assert!(
        captured_contents.is_empty(),
        "LLM gateway should not have received any requests for a future trigger"
    );
}

#[tokio::test]
async fn trigger_poller_does_not_submit_turn_for_unpaired_actor() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway {
        requests: Arc::new(TokioMutex::new(Vec::new())),
    });

    let runtime = build_runtime_with(
        &root,
        Arc::clone(&recording_gateway),
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test().with_worker_config(
            TriggerPollerWorkerConfig::default().set_poll_interval(Duration::from_millis(20)),
        ),
    )
    .await;

    let repo = runtime.trigger_repository();

    // Intentionally do NOT call pair_external_actor — the actor is unpaired.

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");
    let trigger_id = TriggerId::new();

    // Seed a past-due one-shot trigger. An unpaired external actor blocks
    // trusted trigger materialization before any turn can be submitted. This
    // is retryable: the trigger records the failed attempt, clears the active
    // claim, and remains Scheduled until the actor is paired.
    let fire_at = Utc::now() - chrono::Duration::seconds(120);
    let record = TriggerRecord {
        trigger_id,
        tenant_id: tenant_id.clone(),
        creator_user_id: user_id,
        agent_id: Some(agent_id),
        project_id: None,
        name: "trigger-e2e-unpaired".to_string(),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::once(fire_at, "UTC").expect("valid once schedule"),
        prompt: TRIGGER_PROMPT.to_string(),
        delivery_target: None,
        state: TriggerState::Scheduled,
        next_run_at: fire_at,
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: Utc::now(),
    };

    repo.upsert_trigger(record.clone())
        .await
        .expect("upsert trigger record");

    // Sleep for ~1s — 50 poll cycles at 20ms — to give the poller multiple chances.
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Clone the repo handle before shutdown (Arc is cheap to clone).
    let repo_after = repo.clone();

    runtime.shutdown().await.expect("runtime shutdown");

    // Snapshot captured_contents AFTER shutdown so a request submitted between
    // snapshot and shutdown completion cannot produce a false-green result.
    // The recording_gateway Arc is independent of the runtime.
    let captured_contents = recording_gateway.captured_message_contents().await;

    let current = repo_after
        .get_trigger(tenant_id.clone(), record.trigger_id)
        .await
        .expect("get_trigger")
        .expect("record present");

    // Safety guarantee: no turn was ever submitted to the LLM gateway.
    assert!(
        captured_contents.is_empty(),
        "LLM gateway should not have received any requests for an unpaired actor — \
         captured: {:?}",
        captured_contents
    );

    // The one-shot trigger records the blocked pre-submit failure and remains
    // retryable instead of completing the already-past slot.
    assert_eq!(
        current.state,
        TriggerState::Scheduled,
        "unpaired one-shot trigger must remain Scheduled after blocked pre-submit failure — \
         state: {:?}, last_status: {:?}",
        current.state,
        current.last_status
    );
    assert_eq!(
        current.last_status,
        Some(TriggerRunStatus::Error),
        "unpaired trigger must record the retryable failure — record: {current:?}"
    );
    assert_eq!(
        current.active_fire_slot, None,
        "blocked failed one-shot trigger must not keep an active fire — record: {current:?}"
    );
    assert_eq!(
        current.active_run_ref, None,
        "blocked failed one-shot trigger must not have an active run — record: {current:?}"
    );
}

#[tokio::test]
async fn trigger_poller_fires_recurring_trigger_and_leaves_it_scheduled() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway {
        requests: Arc::new(TokioMutex::new(Vec::new())),
    });

    let runtime = build_runtime_with(
        &root,
        Arc::clone(&recording_gateway),
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test().with_worker_config(
            TriggerPollerWorkerConfig::default().set_poll_interval(Duration::from_millis(20)),
        ),
    )
    .await;

    let repo = runtime.trigger_repository();
    let pairing = runtime
        .trigger_conversation_pairing()
        .expect("trigger poller runtime exposes conversation pairing service");

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");
    let trigger_id = TriggerId::new();

    // Pair the external actor — same as the happy-path test.
    pairing
        .pair_external_actor(
            tenant_id.clone(),
            AdapterKind::new(TRIGGER_TRUSTED_ADAPTER_KIND).expect("adapter kind"),
            AdapterInstallationId::new(TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID)
                .expect("installation id"),
            ExternalActorRef::new(TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, user_id.as_str())
                .expect("actor ref"),
            user_id.clone(),
        )
        .await
        .expect("pair external actor for trigger creator");

    let original_next_run_at = Utc::now() - chrono::Duration::seconds(120);

    let record = TriggerRecord {
        trigger_id,
        tenant_id: tenant_id.clone(),
        creator_user_id: user_id,
        agent_id: Some(agent_id),
        project_id: None,
        name: "trigger-e2e-recurring".to_string(),
        source: TriggerSourceKind::Schedule,
        // Every minute — recurring cron stays Scheduled after each fire.
        schedule: TriggerSchedule::cron("* * * * *").expect("valid cron expression"),
        prompt: TRIGGER_PROMPT.to_string(),
        delivery_target: None,
        state: TriggerState::Scheduled,
        next_run_at: original_next_run_at,
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: Utc::now(),
    };

    repo.upsert_trigger(record.clone())
        .await
        .expect("upsert trigger record");

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut record_was_mutated = false;
    let mut prompt_seen = false;

    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;

        let current = repo
            .get_trigger(tenant_id.clone(), record.trigger_id)
            .await
            .expect("get_trigger")
            .expect("record present");

        let mutated = current.last_fired_slot.is_some()
            || current.last_run_at.is_some()
            || current.last_status.is_some()
            || current.active_fire_slot.is_some();

        if mutated {
            record_was_mutated = true;
            // Only poll the gateway after the record was touched.
            let contents = recording_gateway.captured_message_contents().await;
            if contents
                .iter()
                .any(|content| content.contains(TRIGGER_PROMPT))
            {
                prompt_seen = true;
            }
        }

        if record_was_mutated && prompt_seen {
            break;
        }
    }

    // Wait for the settle writes (mark_fire_replayed sets last_fired_slot, advances
    // next_run_at) to become visible. The first-pass loop breaks as soon as the
    // claim+prompt are seen; the settle may still be in flight.
    let settled = wait_for_settled(
        &repo,
        &tenant_id,
        record.trigger_id,
        Duration::from_secs(5),
        |r| r.last_fired_slot.is_some() && r.next_run_at > original_next_run_at,
    )
    .await;

    runtime.shutdown().await.expect("runtime shutdown");

    // Final snapshot for diagnostics, once. Taken after shutdown so a request
    // submitted between snapshot and shutdown completion cannot be invisible.
    let captured_contents = recording_gateway.captured_message_contents().await;

    assert!(
        record_was_mutated,
        "poller did not mutate recurring trigger record within 15s — record: {settled:?}"
    );
    assert!(
        prompt_seen,
        "LLM gateway never received a request for recurring trigger within 15s \
         — captured_messages: {captured_contents:?}"
    );

    // Recurring triggers must remain Scheduled (not Completed) after firing.
    assert_eq!(
        settled.state,
        TriggerState::Scheduled,
        "recurring trigger should remain Scheduled after fire — state: {:?}",
        settled.state
    );
    assert!(
        settled.last_fired_slot.is_some(),
        "recurring trigger should have last_fired_slot set after fire"
    );
    assert!(
        settled.last_run_at.is_some(),
        "recurring trigger should have last_run_at set after fire"
    );
    assert!(
        settled.next_run_at > original_next_run_at,
        "recurring trigger next_run_at should have advanced — original: {:?}, current: {:?}",
        original_next_run_at,
        settled.next_run_at
    );
}

/// T0-5505-E2E: end-to-end proof that issue #5505's fix composes through the
/// real Reborn runtime — a scheduled-trigger fire resolves the dedicated
/// `scheduled_trigger` capability surface, and a fired run that tries to
/// create a *second* trigger, or remove/pause/resume the existing one,
/// cannot, because all four mutator capabilities are stripped from that
/// surface (`builtin.trigger_list` and firing itself stay intact).
///
/// Unlike the other tests in this file, the fired run's model gateway
/// (`TriggerMutatorAttemptGateway`) does not just record requests — on the fired
/// run's first turn it registers real `builtin.trigger_create`,
/// `builtin.trigger_remove`, `builtin.trigger_pause`, and
/// `builtin.trigger_resume` provider tool calls against the run's actual
/// composed `LoopCapabilityPort` (the exact seam a native provider tool-call
/// response goes through in production) — one attempting to create a second
/// trigger named `SELF_CREATE_MARKER_TRIGGER_NAME`, the other three
/// targeting the already-created legitimate trigger. See
/// `TriggerMutatorAttemptGateway`'s doc comment for why this exercises the real
/// `PerSurfaceCapabilityDenyDecorator` / `CapabilitySurfaceDenyFilter` chain
/// instead of a stand-in, and for why all four mutators — not just
/// `trigger_create` — are covered here (PR #5515 review comment: a
/// full-path test that only exercised `trigger_create` would not catch the
/// production deny constant accidentally dropping one of the other three).
#[tokio::test]
async fn scheduled_trigger_fire_cannot_invoke_trigger_mutators() {
    scheduled_trigger_denies_mutators_with_tool_disclosure(ToolDisclosureMode::Off).await;
}

/// Same coverage as `scheduled_trigger_fire_cannot_invoke_trigger_mutators`,
/// but with the runtime built under `ToolDisclosureMode::Bridged` instead of
/// the default `Off`.
///
/// This is not a redundant copy of the `Off` variant above. PR #5515
/// self-review: the deny decorator (`PerSurfaceCapabilityDenyDecorator` /
/// `CapabilitySurfaceDenyFilter`) is deliberately wired in `runtime.rs`
/// *after* the conditional `ToolDisclosureCapabilityDecorator` so the
/// mutator denial stays outermost — and therefore still wins — even when
/// bridged tool disclosure is enabled. Before this test, `Bridged` had
/// exactly one usage anywhere in the repo (an unrelated system-prompt test),
/// so nothing exercised that decorator-ordering composition end-to-end; a
/// decorator-order or bridged-disclosure regression could have re-exposed
/// `trigger_create`/`remove`/`pause`/`resume` without any whole-path test
/// failing. Keep this alongside the `Off` variant rather than folding it in
/// — it pins the composition order, not just the deny outcome.
#[tokio::test]
async fn scheduled_trigger_fire_cannot_invoke_trigger_mutators_with_bridged_disclosure() {
    scheduled_trigger_denies_mutators_with_tool_disclosure(ToolDisclosureMode::Bridged).await;
}

async fn scheduled_trigger_denies_mutators_with_tool_disclosure(
    tool_disclosure: ToolDisclosureMode,
) {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(TriggerMutatorAttemptGateway::default());

    let runtime = build_runtime_with_tool_disclosure(
        &root,
        Arc::clone(&gateway),
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test().with_worker_config(
            TriggerPollerWorkerConfig::default().set_poll_interval(Duration::from_millis(20)),
        ),
        tool_disclosure,
    )
    .await;

    // Create the one legitimate trigger through the builtin tool (direct
    // dispatch — bypasses the model, so `gateway` is not invoked yet).
    let created = invoke_trigger_create(
        &runtime,
        json!({
            "name": "trigger-e2e-self-create-guard",
            "prompt": TRIGGER_PROMPT,
            "schedule": { "kind": "cron", "expression": "* * * * *", "timezone": "UTC" }
        }),
    )
    .await;
    assert_eq!(
        created["trigger"]["name"],
        json!("trigger-e2e-self-create-guard")
    );

    let repo = runtime.trigger_repository();
    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let trigger_id = TriggerId::parse(
        created["trigger"]["trigger_id"]
            .as_str()
            .expect("created trigger id"),
    )
    .expect("valid trigger id");

    let mut record = repo
        .get_trigger(tenant_id.clone(), trigger_id)
        .await
        .expect("get created trigger")
        .expect("created trigger persisted");
    record.next_run_at = Utc::now() - chrono::Duration::seconds(120);
    repo.upsert_trigger(record.clone())
        .await
        .expect("make created trigger due");

    // The remove/pause/resume mutator attempts need a real trigger_id to
    // shape a realistic input payload against; give them the already-created
    // legitimate trigger's id before the fire runs.
    gateway.set_mutator_target_trigger_id(trigger_id).await;

    // Wait for the submit settlement first. `mark_fire_accepted` sets
    // `last_status` when the turn is accepted, before the scheduler has
    // necessarily executed the fired run's model turn.
    let settled = wait_for_settled(
        &repo,
        &tenant_id,
        trigger_id,
        Duration::from_secs(15),
        |r| r.last_fired_slot.is_some() && r.last_run_at.is_some() && r.last_status.is_some(),
    )
    .await;

    // This is the model's only turn where a capability call can be attempted
    // (`invoke_trigger_create` above never touched the model). Wait for the
    // gateway-side registration attempts before shutting the runtime down; a
    // shutdown immediately after submit settlement can stop a queued run before
    // it reaches the model.
    let (captured_contents, registration_outcomes) =
        wait_for_mutator_registration_outcomes(gateway.as_ref(), Duration::from_secs(15)).await;

    runtime.shutdown().await.expect("runtime shutdown");

    assert_eq!(
        registration_outcomes.len(),
        4,
        "the fired run must have attempted all 4 scheduled-trigger mutator \
         registrations (create/remove/pause/resume) — captured_messages: \
         {captured_contents:?}, outcomes: {registration_outcomes:?}, \
         settled_record: {settled:?}"
    );

    // Core assertion, mechanism-level: the surface must deny EVERY mutator
    // registration, not just `trigger_create`. Each of these ids is on the
    // scheduled_trigger deny set (the fix's `PerSurfaceCapabilityDenyDecorator`
    // / `CapabilitySurfaceDenyFilter`), so `register_provider_tool_call`'s own
    // scope check on the resolved capability id fails closed before a
    // candidate is ever built — see `TriggerMutatorAttemptGateway`'s doc comment
    // for the exact call chain.
    //
    // GUARD AGAINST FALSE-PASS: pre-fix (or if the production deny constant
    // ever drops one of these ids), the scheduled_trigger capability surface
    // would not deny that mutator, so the scope check above would pass and
    // `capabilities.register_provider_tool_call(...)` inside the gateway
    // would return `Ok(candidate)` for it — with a REAL, run-scoped staged
    // input, because `register_provider_tool_call` is the exact path a
    // native provider tool call uses to stage its arguments through the
    // run's real `StagedCapabilityIo`. The loop would then actually
    // dispatch that mutator against the staged input, and either the marker
    // trigger asserted absent below WOULD exist, or the original trigger's
    // state WOULD have changed. Reverting any one entry of the
    // scheduled_trigger deny map turns that entry's assertion here into a
    // `Some(Ok(()))` and one of the repository-state assertions below into a
    // failure — both catch the regression independently, per mutator.
    const DENIED_SUMMARY: &str = "provider tool call targets a disabled capability";
    assert_eq!(
        registration_outcomes.get(TRIGGER_CREATE_CAPABILITY_ID),
        Some(&Err(DENIED_SUMMARY.to_string())),
        "expected the scheduled_trigger surface to deny the trigger_create \
         registration attempt made from inside the fired run: {registration_outcomes:?}"
    );
    assert_eq!(
        registration_outcomes.get(TRIGGER_REMOVE_CAPABILITY_ID),
        Some(&Err(DENIED_SUMMARY.to_string())),
        "expected the scheduled_trigger surface to deny the trigger_remove \
         registration attempt made from inside the fired run: {registration_outcomes:?}"
    );
    assert_eq!(
        registration_outcomes.get(TRIGGER_PAUSE_CAPABILITY_ID),
        Some(&Err(DENIED_SUMMARY.to_string())),
        "expected the scheduled_trigger surface to deny the trigger_pause \
         registration attempt made from inside the fired run: {registration_outcomes:?}"
    );
    assert_eq!(
        registration_outcomes.get(TRIGGER_RESUME_CAPABILITY_ID),
        Some(&Err(DENIED_SUMMARY.to_string())),
        "expected the scheduled_trigger surface to deny the trigger_resume \
         registration attempt made from inside the fired run: {registration_outcomes:?}"
    );

    // The mutator denials must not otherwise break the fire: the original
    // trigger still settles Ok, exactly like the happy-path tests above.
    assert_eq!(
        settled.last_status,
        Some(TriggerRunStatus::Ok),
        "the original trigger must still settle Ok — the fix blocks only the \
         mutator capabilities, not the fire itself — record: {settled:?}"
    );

    // Belt-and-suspenders behavioral check straight against the repository:
    // regardless of how the denials surfaced, no second trigger was ever
    // persisted, and the only trigger that exists is the original —
    // unmodified (a should-be-denied pause/resume attempt did not flip its
    // state).
    let all_triggers = repo
        .list_triggers(tenant_id)
        .await
        .expect("list triggers after fire settles");
    assert!(
        all_triggers
            .iter()
            .all(|trigger| trigger.name != SELF_CREATE_MARKER_TRIGGER_NAME),
        "a scheduled-trigger fire must not be able to create a second trigger — \
         found triggers: {all_triggers:?}"
    );
    assert_eq!(
        all_triggers.len(),
        1,
        "exactly the original trigger should exist after the fire settles — \
         found triggers: {all_triggers:?}"
    );
    assert_eq!(all_triggers[0].trigger_id, trigger_id);
    assert_eq!(
        all_triggers[0].state,
        TriggerState::Scheduled,
        "a should-be-denied trigger_pause attempt must not have changed the \
         original trigger's state: {:?}",
        all_triggers[0]
    );
}
// arch-exempt: large_file, trigger poller end-to-end coverage remains centralized, plan #6175
