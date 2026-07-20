// arch-exempt: large_file, shared e2e harness builders pending test-support extraction, plan #6310
//! Lane 7 end-to-end coverage for the WebChat v2 HTTP surface.
//!
//! Unlike [`webui_v2_serve`], which drives the composed router against a
//! stub `RebornServicesApi`, this test stands up a real local-dev
//! `RebornRuntime`, overrides its LLM gateway with a scripted
//! tool-calling fake, composes the v2 router through
//! [`build_webui_services`] + [`webui_v2_app`], and exercises it from
//! the browser side over HTTP (`tower::ServiceExt::oneshot`).
//!
//! The point is to prove the full chain — bearer auth → caller scope →
//! product workflow → turn coordinator → agent loop → capability host
//! (`builtin.echo`) → durable transcript → WebChat v2 SSE/timeline
//! endpoints — works end-to-end without anything mocked above the LLM
//! boundary.

#![cfg(feature = "test-support")]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use ironclaw_auth::{
    AuthProductScope, AuthSurface, CredentialAccountLabel, CredentialAccountStatus,
    CredentialOwnership, NewCredentialAccount, ProviderScope,
};
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_host_api::{
    AgentId, CapabilityId, InvocationId, ProviderToolName, ResourceScope, SecretHandle, TenantId,
    UserId,
};
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
    HostManagedModelStreamSink,
};
use ironclaw_reborn_composition::{
    OAuthClientConfig, PollSettings, RebornBuildInput, RebornRuntime, RebornRuntimeIdentity,
    RebornRuntimeInput, build_reborn_runtime, build_webui_services,
};
use ironclaw_turns::run_profile::{
    CapabilityCallCandidate, LoopCapabilityPort, ProviderToolCall, RegisterProviderToolCallRequest,
};
use ironclaw_webui::{WebuiAuthentication, WebuiAuthenticator, WebuiServeConfig, webui_v2_app};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tower::ServiceExt;

// ─── identities ───────────────────────────────────────────────────────

const VALID_TOKEN: &str = "valid-e2e-token";
const USER_B_TOKEN: &str = "valid-e2e-token-b";
const TENANT: &str = "e2e-tenant";
const USER: &str = "e2e-owner";
const USER_B: &str = "e2e-viewer-b";
const AGENT: &str = "e2e-agent";
const SENSITIVE_TOOL_SENTINEL: &str = "sk-e2e-progress-secret";
const MEMORY_ISOLATION_MARKER: &str = "webui-memory-owner-a-secret-5460";

// ─── auth stub ────────────────────────────────────────────────────────

struct ValidTokenForUser {
    user_id: UserId,
}

impl ValidTokenForUser {
    fn new(user_id: &str) -> Self {
        Self {
            user_id: UserId::new(user_id).expect("valid user id"),
        }
    }
}

#[async_trait]
impl WebuiAuthenticator for ValidTokenForUser {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        if token == VALID_TOKEN {
            Some(WebuiAuthentication::user(self.user_id.clone()))
        } else {
            None
        }
    }
}

struct TwoUserTokens;

#[async_trait]
impl WebuiAuthenticator for TwoUserTokens {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        match token {
            VALID_TOKEN => Some(WebuiAuthentication::user(
                UserId::new(USER).expect("valid user id"),
            )),
            USER_B_TOKEN => Some(WebuiAuthentication::user(
                UserId::new(USER_B).expect("valid user id"),
            )),
            _ => None,
        }
    }
}

// ─── runtime policy ───────────────────────────────────────────────────

fn local_dev_effective_policy() -> EffectiveRuntimePolicy {
    // Mirrors the policy the in-mod runtime tests use. Avoids the
    // public `local_dev_runtime_policy()` helper because that returns a
    // `ResolvedRuntimePolicy` shape; `RebornBuildInput::with_runtime_policy`
    // takes the `EffectiveRuntimePolicy` shape and the two are not
    // interchangeable in this direction yet.
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}

/// Local trusted-laptop policy with minimal approvals, so an in-workspace
/// `write_file` auto-proceeds instead of parking on a destructive-write gate.
/// Used by the file-production test, which is about download — not approval.
fn local_yolo_effective_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        requested_profile: RuntimeProfile::LocalYolo,
        resolved_profile: RuntimeProfile::LocalYolo,
        approval_policy: ApprovalPolicy::Minimal,
        ..local_dev_effective_policy()
    }
}

// ─── scripted tool-calling gateway ────────────────────────────────────

/// Two-step LLM stand-in:
///
/// 1. First call: register a provider tool call against `builtin.echo`
///    with arguments containing a secret-like sentinel and return
///    that as a `CapabilityCalls` response so the agent loop dispatches
///    the tool.
/// 2. Second call (after tool execution): assert the tool result is
///    visible in the request transcript, then return a plain assistant
///    reply that the timeline endpoint will surface as the final user-
///    visible message.
#[derive(Debug)]
struct ToolCallingGateway {
    call_count: StdMutex<usize>,
    tool_message: String,
    // When true, the scripted tool argument embeds `SENSITIVE_TOOL_SENTINEL`
    // (secret-shaped text), which the durable model-observation validator
    // (`normalized_model_observation` in
    // `ironclaw_threads::tool_result_reference`) is designed to strip from
    // the inline preview while preserving the opaque result reference. The
    // follow-up assertion below flips accordingly: it checks the sentinel
    // was NOT hydrated back to the model, instead of checking that it was.
    expect_redacted_preview: bool,
}

impl Default for ToolCallingGateway {
    fn default() -> Self {
        Self {
            call_count: StdMutex::new(0),
            tool_message: "hello from e2e tool".to_string(),
            expect_redacted_preview: false,
        }
    }
}

impl ToolCallingGateway {
    /// Scripted tool argument embeds a secret-shaped sentinel so the caller
    /// can assert the sensitive text never reaches the model's own tool
    /// result content or any user-facing SSE/timeline surface, matching the
    /// intended graceful preview-drop behavior.
    fn with_sensitive_tool_message() -> Self {
        Self {
            tool_message: format!("hello from e2e tool {SENSITIVE_TOOL_SENTINEL}"),
            expect_redacted_preview: true,
            ..Self::default()
        }
    }
}

#[async_trait]
impl HostManagedModelGateway for ToolCallingGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        // The capability-aware entrypoint is the right one for this
        // flow; the bare `stream_model` exists for non-tool-calling
        // gateways and should never be reached here. Surfacing an
        // explicit error makes a routing regression fail loudly.
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "ToolCallingGateway requires the capability-aware model path",
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let call_index = {
            let mut count = self
                .call_count
                .lock()
                .expect("tool gateway call lock poisoned");
            let index = *count;
            *count += 1;
            index
        };

        if call_index > 0 {
            let tool_result = request
                .messages
                .iter()
                .find(|m| m.role == HostManagedModelMessageRole::ToolResult)
                .expect("follow-up model call must include a tool_result message");
            if self.expect_redacted_preview {
                assert!(
                    !tool_result.content.contains(SENSITIVE_TOOL_SENTINEL),
                    "follow-up model call must not see the redacted secret-shaped preview, got: {}",
                    tool_result.content,
                );
            } else {
                assert!(
                    tool_result.content.contains(&self.tool_message),
                    "follow-up model call should see hydrated echo output, got: {}",
                    tool_result.content,
                );
            }
            return Ok(HostManagedModelResponse::assistant_reply("e2e tool ok"));
        }

        let echo_id = CapabilityId::new("builtin.echo").expect("echo capability id");
        let echo_tool = capabilities
            .tool_definitions()
            .map_err(|err| {
                HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidRequest,
                    format!("tool_definitions failed: {err}"),
                )
            })?
            .into_iter()
            .find(|def| def.capability_id == echo_id)
            .expect("builtin.echo must be visible in local-dev capability surface");

        let candidate = capabilities
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(ProviderToolCall {
                provider_id: "e2e-provider".to_string(),
                provider_model_id: "e2e-model".to_string(),
                turn_id: Some("e2e-turn-1".to_string()),
                id: "e2e-call-1".to_string(),
                name: echo_tool.name,
                arguments: json!({"message": self.tool_message.clone()}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }))
            .await
            .map_err(|err| {
                HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidRequest,
                    format!("register_provider_tool_call failed: {err}"),
                )
            })?;

        Ok(HostManagedModelResponse::capability_calls(
            vec![candidate],
            "",
        ))
    }
}

#[derive(Debug)]
struct DelayedStreamingTextGateway {
    first_update: StdMutex<Option<oneshot::Sender<()>>>,
    release: StdMutex<Option<oneshot::Receiver<()>>>,
}

impl DelayedStreamingTextGateway {
    fn new(first_update: oneshot::Sender<()>, release: oneshot::Receiver<()>) -> Self {
        Self {
            first_update: StdMutex::new(Some(first_update)),
            release: StdMutex::new(Some(release)),
        }
    }

    async fn stream_with_progress(
        &self,
        sink: Arc<dyn HostManagedModelStreamSink>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        sink.safe_text_update("Hel".to_string()).await;
        if let Some(first_update) = self
            .first_update
            .lock()
            .expect("first update lock poisoned")
            .take()
        {
            let _ = first_update.send(());
        }

        let release = self
            .release
            .lock()
            .expect("release lock poisoned")
            .take()
            .expect("delayed streaming gateway should be called once");
        let _ = release.await;

        sink.safe_text_update("Hello".to_string()).await;
        Ok(HostManagedModelResponse::assistant_reply("Hello"))
    }
}

#[async_trait]
impl HostManagedModelGateway for DelayedStreamingTextGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "DelayedStreamingTextGateway requires a progress sink",
        ))
    }

    async fn stream_model_with_progress(
        &self,
        _request: HostManagedModelRequest,
        sink: Arc<dyn HostManagedModelStreamSink>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.stream_with_progress(sink).await
    }

    async fn stream_model_with_capabilities_and_progress(
        &self,
        _request: HostManagedModelRequest,
        _capabilities: Arc<dyn LoopCapabilityPort>,
        sink: Arc<dyn HostManagedModelStreamSink>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.stream_with_progress(sink).await
    }
}

// ─── file-producing gateway ───────────────────────────────────────────

const CSV_PATH: &str = "/workspace/report.csv";
const PDF_PATH: &str = "/workspace/report.pdf";
const CSV_BODY: &str = "name,score\nalice,90\nbob,85\n";
// A minimal, byte-stable PDF. The download mime is derived from the `.pdf`
// extension (not content sniffing), so the bytes only need to round-trip
// write -> read exactly; they are not rendered.
const PDF_BODY: &str = "%PDF-1.4\n1 0 obj<</Type/Catalog>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";

/// Scripted gateway that drives the agent to produce two downloadable files —
/// a CSV then a PDF — via `builtin.write_file`, then emit a final reply that
/// references both `/workspace` paths. Exercises the full "agent produces a
/// file the user can download" flow against the real loop + capability host.
#[derive(Debug, Default)]
struct WriteFileGateway {
    call_count: StdMutex<usize>,
}

#[derive(Debug, Default)]
struct MemoryWriteGateway {
    call_count: StdMutex<usize>,
}

async fn register_write(
    capabilities: &Arc<dyn LoopCapabilityPort>,
    tool_name: ProviderToolName,
    call_id: &str,
    path: &str,
    content: &str,
) -> Result<CapabilityCallCandidate, HostManagedModelError> {
    capabilities
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(ProviderToolCall {
            provider_id: "e2e-provider".to_string(),
            provider_model_id: "e2e-model".to_string(),
            turn_id: Some("e2e-write-turn".to_string()),
            id: call_id.to_string(),
            name: tool_name,
            arguments: json!({"path": path, "content": content}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }))
        .await
        .map_err(|err| {
            HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidRequest,
                format!("register_provider_tool_call(write_file) failed: {err}"),
            )
        })
}

#[async_trait]
impl HostManagedModelGateway for WriteFileGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "WriteFileGateway requires the capability-aware model path",
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        _request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let call_index = {
            let mut count = self
                .call_count
                .lock()
                .expect("write gateway call lock poisoned");
            let index = *count;
            *count += 1;
            index
        };

        let write_id = CapabilityId::new("builtin.write_file").expect("write_file capability id");
        let write_tool = capabilities
            .tool_definitions()
            .map_err(|err| {
                HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidRequest,
                    format!("tool_definitions failed: {err}"),
                )
            })?
            .into_iter()
            .find(|def| def.capability_id == write_id)
            .expect("builtin.write_file must be visible in local-dev capability surface");

        // One tool round writes both files (mirrors the single-round shape the
        // echo gateway proves), then the follow-up call emits the final reply.
        if call_index == 0 {
            let csv = register_write(
                &capabilities,
                write_tool.name.clone(),
                "e2e-write-csv",
                CSV_PATH,
                CSV_BODY,
            )
            .await?;
            let pdf = register_write(
                &capabilities,
                write_tool.name,
                "e2e-write-pdf",
                PDF_PATH,
                PDF_BODY,
            )
            .await?;
            return Ok(HostManagedModelResponse::capability_calls(
                vec![csv, pdf],
                "",
            ));
        }
        Ok(HostManagedModelResponse::assistant_reply(format!(
            "Saved {CSV_PATH} and {PDF_PATH} — both are ready to download."
        )))
    }
}

#[async_trait]
impl HostManagedModelGateway for MemoryWriteGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::InvalidRequest,
            "MemoryWriteGateway requires the capability-aware model path",
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let call_index = {
            let mut count = self
                .call_count
                .lock()
                .expect("memory gateway call lock poisoned");
            let index = *count;
            *count += 1;
            index
        };
        if call_index > 0 {
            let tool_result = request
                .messages
                .iter()
                .find(|message| message.role == HostManagedModelMessageRole::ToolResult)
                .expect("follow-up model call must include memory_write result");
            assert!(
                tool_result.content.contains("written")
                    && tool_result.content.contains("MEMORY.md"),
                "memory_write must persist MEMORY.md before the browser isolation assertion, got: {}",
                tool_result.content
            );
            return Ok(HostManagedModelResponse::assistant_reply("memory saved"));
        }

        let memory_write_id =
            CapabilityId::new("builtin.memory_write").expect("memory_write capability id");
        let memory_write_tool = capabilities
            .tool_definitions()
            .map_err(|err| {
                HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidRequest,
                    format!("tool_definitions failed: {err}"),
                )
            })?
            .into_iter()
            .find(|def| def.capability_id == memory_write_id)
            .expect("builtin.memory_write must be visible in local-dev capability surface");
        let write = capabilities
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(ProviderToolCall {
                provider_id: "e2e-provider".to_string(),
                provider_model_id: "e2e-model".to_string(),
                turn_id: Some("e2e-memory-turn".to_string()),
                id: "e2e-memory-write".to_string(),
                name: memory_write_tool.name,
                arguments: json!({
                    "target": "memory",
                    "content": format!("owner A private memory: {MEMORY_ISOLATION_MARKER}"),
                    "append": false
                }),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }))
            .await
            .map_err(|err| {
                HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidRequest,
                    format!("register_provider_tool_call(memory_write) failed: {err}"),
                )
            })?;
        Ok(HostManagedModelResponse::capability_calls(vec![write], ""))
    }
}

// ─── harness ──────────────────────────────────────────────────────────

struct Harness {
    runtime: RebornRuntime,
    router: axum::Router,
    _root: Option<tempfile::TempDir>,
}

async fn build_harness() -> Harness {
    build_harness_with_gateway(Arc::new(ToolCallingGateway::default())).await
}

async fn build_harness_with_gateway(gateway: Arc<dyn HostManagedModelGateway>) -> Harness {
    build_harness_with_gateway_and_policy(gateway, local_dev_effective_policy()).await
}

async fn build_harness_with_gateway_and_policy(
    gateway: Arc<dyn HostManagedModelGateway>,
    policy: EffectiveRuntimePolicy,
) -> Harness {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    build_harness_at(storage_root, Some(root), gateway, policy).await
}

// Only consumed by `webui_v2_beta_acceptance_stream_replay_restart_and_redaction`
// (initial build and post-restart reopen), which needs the sensitive-message
// gateway variant to exercise `assert_no_sensitive_payload`.
async fn build_harness_on_storage(storage_root: impl AsRef<Path>) -> Harness {
    build_harness_at(
        storage_root.as_ref().to_path_buf(),
        None,
        Arc::new(ToolCallingGateway::with_sensitive_tool_message()),
        local_dev_effective_policy(),
    )
    .await
}

async fn build_harness_at(
    storage_root: PathBuf,
    root: Option<tempfile::TempDir>,
    gateway: Arc<dyn HostManagedModelGateway>,
    policy: EffectiveRuntimePolicy,
) -> Harness {
    build_harness_at_with_runtime_owner_and_auth_user(
        storage_root,
        root,
        gateway,
        policy,
        USER,
        USER,
    )
    .await
}

/// Harness with NO Google OAuth backend configured at composition time, so
/// the provider-instance readiness map's `google` entry stays populated (see
/// `provider_instance_readiness_map`) and a google-family extension
/// activation fails closed with the sanitized 400 before it ever reaches the
/// per-account credential gate. Backs
/// `webui_v2_extension_activate_returns_400_when_provider_instance_not_configured`.
async fn build_harness_without_google_oauth_backend() -> Harness {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    build_harness_at_with_runtime_owner_auth_user_and_google_oauth_backend(
        storage_root,
        Some(root),
        Arc::new(ToolCallingGateway::default()),
        local_dev_effective_policy(),
        USER,
        USER,
        None,
    )
    .await
}

/// Dummy but well-formed Google OAuth backend for harnesses that need
/// Gmail/GSuite setup+activation to reach the per-account credential gate
/// instead of failing closed at the provider-instance readiness map.
fn test_google_oauth_backend() -> OAuthClientConfig {
    OAuthClientConfig::new(
        "e2e-google-client-id.apps.googleusercontent.com",
        "http://127.0.0.1/oauth/callback/google",
        None,
    )
    .expect("valid test google oauth client config")
}

async fn build_harness_at_with_runtime_owner_and_auth_user(
    storage_root: PathBuf,
    root: Option<tempfile::TempDir>,
    gateway: Arc<dyn HostManagedModelGateway>,
    policy: EffectiveRuntimePolicy,
    runtime_owner_id: &str,
    authenticated_user_id: &str,
) -> Harness {
    // This shared harness backs WebUI e2e tests that exercise Gmail/GSuite
    // setup+activation over the real v2 router, not the provider-instance
    // readiness map — without a configured Google OAuth backend,
    // `webui_v2_gmail_oauth_setup_complete_allows_activation` and any other
    // google-family activation test here fails closed with the
    // readiness-map's 400 before it ever reaches the per-account gate under
    // test. `webui_v2_extension_activate_returns_400_when_provider_instance_not_configured`
    // is the dedicated test for that 400 path and uses
    // `build_harness_without_google_oauth_backend` instead.
    build_harness_at_with_runtime_owner_auth_user_and_google_oauth_backend(
        storage_root,
        root,
        gateway,
        policy,
        runtime_owner_id,
        authenticated_user_id,
        Some(test_google_oauth_backend()),
    )
    .await
}

/// Harness variant with composition-time Google OAuth configuration under the
/// caller's control. `google_oauth_backend: None` leaves the provider-instance
/// readiness map's `google` entry populated, so a google-family extension
/// activation fails closed with the sanitized 400 before it ever reaches the
/// per-account credential gate — the shape
/// `webui_v2_extension_activate_returns_400_when_provider_instance_not_configured`
/// needs to drive the real HTTP activate route through that path.
async fn build_harness_at_with_runtime_owner_auth_user_and_google_oauth_backend(
    storage_root: PathBuf,
    root: Option<tempfile::TempDir>,
    gateway: Arc<dyn HostManagedModelGateway>,
    policy: EffectiveRuntimePolicy,
    runtime_owner_id: &str,
    authenticated_user_id: &str,
    google_oauth_backend: Option<OAuthClientConfig>,
) -> Harness {
    let mut build_input =
        RebornBuildInput::local_dev(runtime_owner_id, storage_root).with_runtime_policy(policy);
    if let Some(google_oauth_backend) = google_oauth_backend {
        build_input = build_input.with_google_oauth_backend(google_oauth_backend);
    }
    let input = RebornRuntimeInput::from_services(build_input)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: TENANT.to_string(),
            agent_id: AGENT.to_string(),
            source_binding_id: "e2e-source".to_string(),
            reply_target_binding_id: "e2e-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(10),
        })
        .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    // The Tools-settings global auto-approve switch is authoritative for
    // first-party tool dispatch; enable it for the e2e dispatch scope so
    // scripted tool calls complete instead of parking on the per-tool approval
    // gate (which would otherwise leave the turn without an assistant reply).
    runtime
        .services()
        .local_dev_auto_approve_settings_for_test()
        .expect("local-dev exposes auto-approve settings for test")
        .set(ironclaw_approvals::AutoApproveSettingInput {
            updated_by: ironclaw_host_api::Principal::User(UserId::new(USER).expect("user")),
            scope: ResourceScope {
                tenant_id: TenantId::new(TENANT).expect("tenant"),
                user_id: UserId::new(USER).expect("user"),
                agent_id: Some(AgentId::new(AGENT).expect("agent")),
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            enabled: true,
        })
        .await
        .expect("enable global auto-approve for e2e dispatch");
    let bundle = build_webui_services(&runtime, None).expect("webui bundle");
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(ValidTokenForUser::new(authenticated_user_id)),
        // CORS allowlist is unused in oneshot tests (no Origin header
        // is set), but the WebuiServeConfig constructor rejects an
        // empty Vec to keep production deployments fail-closed. Any
        // throwaway origin satisfies the type without affecting these
        // tests.
        vec![HeaderValue::from_static("http://localhost:0")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"));
    let router = webui_v2_app(bundle, config).expect("webui v2 app");

    Harness {
        runtime,
        router,
        _root: root,
    }
}

async fn build_two_user_harness(
    gateway: Arc<dyn HostManagedModelGateway>,
    policy: EffectiveRuntimePolicy,
) -> Harness {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev(USER, storage_root).with_runtime_policy(policy),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: TENANT.to_string(),
        agent_id: AGENT.to_string(),
        source_binding_id: "e2e-source".to_string(),
        reply_target_binding_id: "e2e-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(10),
    })
    .with_model_gateway_override(gateway);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    runtime
        .services()
        .local_dev_auto_approve_settings_for_test()
        .expect("local-dev exposes auto-approve settings for test")
        .set(ironclaw_approvals::AutoApproveSettingInput {
            updated_by: ironclaw_host_api::Principal::User(UserId::new(USER).expect("user")),
            scope: ResourceScope {
                tenant_id: TenantId::new(TENANT).expect("tenant"),
                user_id: UserId::new(USER).expect("user"),
                agent_id: Some(AgentId::new(AGENT).expect("agent")),
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            enabled: true,
        })
        .await
        .expect("enable global auto-approve for user A");
    let bundle = build_webui_services(&runtime, None).expect("webui bundle");
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(TwoUserTokens),
        vec![HeaderValue::from_static("http://localhost:0")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"));
    let router = webui_v2_app(bundle, config).expect("webui v2 app");

    Harness {
        runtime,
        router,
        _root: Some(root),
    }
}

async fn read_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 256 * 1024)
        .await
        .expect("response body within 256 KiB cap");
    serde_json::from_slice(&bytes).expect("response body is valid JSON")
}

fn bearer_post(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("bearer POST request")
}

fn bearer_get(uri: &str) -> Request<Body> {
    bearer_get_with_last_event_id(uri, None)
}

fn bearer_get_with_token(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .expect("bearer GET request")
}

fn bearer_get_with_last_event_id(uri: &str, last_event_id: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"));
    if let Some(last_event_id) = last_event_id {
        builder = builder.header("Last-Event-ID", last_event_id);
    }
    builder.body(Body::empty()).expect("bearer GET request")
}

#[derive(Default, Debug)]
struct ParsedSseEvent {
    event: Option<String>,
    id: Option<String>,
    data: Option<String>,
}

fn parse_sse_events(bytes: &[u8]) -> Vec<ParsedSseEvent> {
    let text = String::from_utf8_lossy(bytes);
    let mut events = Vec::new();
    let mut parsed = ParsedSseEvent::default();
    let mut has_fields = false;
    for line in text.lines() {
        if line.is_empty() {
            if has_fields {
                events.push(parsed);
                parsed = ParsedSseEvent::default();
                has_fields = false;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            parsed.event = Some(rest.trim_start().to_string());
            has_fields = true;
        } else if let Some(rest) = line.strip_prefix("id:") {
            parsed.id = Some(rest.trim_start().to_string());
            has_fields = true;
        } else if let Some(rest) = line.strip_prefix("data:") {
            parsed.data = Some(rest.trim_start().to_string());
            has_fields = true;
        }
    }
    events
}

async fn collect_sse_until<F>(body: &mut Body, timeout: Duration, mut done: F) -> Vec<u8>
where
    F: FnMut(&[u8]) -> bool,
{
    let deadline = Instant::now() + timeout;
    let mut buf = Vec::new();
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, body.frame()).await {
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    buf.extend_from_slice(data.as_ref());
                    if done(&buf) {
                        return buf;
                    }
                }
            }
            Ok(_) | Err(_) => return buf,
        }
    }
    buf
}

async fn open_sse(
    router: &axum::Router,
    thread_id: &str,
    last_event_id: Option<&str>,
) -> axum::response::Response {
    let response = router
        .clone()
        .oneshot(bearer_get_with_last_event_id(
            &format!("/api/webchat/v2/threads/{thread_id}/events"),
            last_event_id,
        ))
        .await
        .expect("SSE oneshot");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "SSE stream must open successfully"
    );
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(
        content_type.starts_with("text/event-stream"),
        "SSE content type expected, got: {content_type}"
    );
    response
}

async fn create_thread(router: &axum::Router, client_action_id: &str) -> String {
    let create = router
        .clone()
        .oneshot(bearer_post(
            "/api/webchat/v2/threads",
            json!({"client_action_id": client_action_id}),
        ))
        .await
        .expect("create_thread oneshot");
    assert_eq!(
        create.status(),
        StatusCode::OK,
        "create_thread must succeed against the real bundle"
    );
    let create_body = read_json(create).await;
    create_body["thread"]["thread_id"]
        .as_str()
        .expect("create_thread response must carry thread.thread_id")
        .to_string()
}

async fn send_message(router: &axum::Router, thread_id: &str, client_action_id: &str) {
    let send = router
        .clone()
        .oneshot(bearer_post(
            &format!("/api/webchat/v2/threads/{thread_id}/messages"),
            json!({
                "client_action_id": client_action_id,
                "content": "please call the echo tool",
            }),
        ))
        .await
        .expect("send_message oneshot");
    assert_eq!(
        send.status(),
        StatusCode::OK,
        "send_message must accept the queued turn"
    );
}

async fn wait_for_final_timeline(router: &axum::Router, thread_id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let response = router
            .clone()
            .oneshot(bearer_get(&format!(
                "/api/webchat/v2/threads/{thread_id}/timeline"
            )))
            .await
            .expect("timeline oneshot");
        assert_eq!(response.status(), StatusCode::OK);
        let timeline = read_json(response).await;
        let messages = timeline["messages"]
            .as_array()
            .expect("timeline.messages must be an array");
        if messages.iter().any(|message| {
            extract_assistant_text(message).is_some_and(|text| text.contains("e2e tool ok"))
        }) {
            return timeline;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timeline never surfaced an assistant message containing 'e2e tool ok' within 10s");
}

fn assert_timeline_has_tool_result_reference(timeline: &Value) {
    let messages = timeline["messages"]
        .as_array()
        .expect("timeline.messages must be an array");
    let tool_result_seen = messages.iter().any(|message| {
        message.get("kind").and_then(Value::as_str) == Some("tool_result_reference")
            && message
                .get("tool_result_ref")
                .and_then(Value::as_str)
                .is_some_and(|reference| reference.starts_with("result:"))
    });
    assert!(
        tool_result_seen,
        "timeline must include a tool_result_reference message for the builtin.echo invocation, \
         but the messages array was: {messages:#?}",
    );
}

fn assert_timeline_has_capability_display_preview(timeline: &Value) {
    let messages = timeline["messages"]
        .as_array()
        .expect("timeline.messages must be an array");
    let preview_seen = messages.iter().any(|message| {
        message.get("kind").and_then(Value::as_str) == Some("capability_display_preview")
            && message
                .get("tool_result_ref")
                .and_then(Value::as_str)
                .is_some_and(|reference| reference.starts_with("result:"))
    });
    assert!(
        preview_seen,
        "timeline must include a durable capability display preview for the builtin.echo \
         invocation, but the messages array was: {messages:#?}",
    );
}

fn assert_no_sensitive_payload(label: &str, bytes_or_json: impl AsRef<[u8]>) {
    let text = String::from_utf8_lossy(bytes_or_json.as_ref());
    assert!(
        !text.contains(SENSITIVE_TOOL_SENTINEL),
        "{label} leaked sensitive tool input sentinel: {text}"
    );
}

fn event_ids(events: &[ParsedSseEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| event.id.clone())
        .collect::<Vec<_>>()
}

fn event_payload_without_cursor(event: &ParsedSseEvent) -> Value {
    let mut payload = event_payload_json(event);
    if let Some(object) = payload.as_object_mut() {
        object.remove("cursor");
    }
    payload
}

fn replay_payload_signatures<'a>(
    events: impl Iterator<Item = &'a ParsedSseEvent>,
) -> Vec<(Option<String>, Value)> {
    events
        .filter(|event| event.id.is_some())
        .map(|event| (event.event.clone(), event_payload_without_cursor(event)))
        .collect()
}

fn has_browser_visible_progress(events: &[ParsedSseEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event.event.as_deref(),
            Some("accepted")
                | Some("running")
                | Some("capability_progress")
                | Some("capability_activity")
                | Some("capability_display_preview")
                | Some("projection_snapshot")
                | Some("projection_update")
                | Some("final_reply")
        )
    })
}

fn events_include_error(bytes: &[u8]) -> bool {
    parse_sse_events(bytes)
        .iter()
        .any(|event| event.event.as_deref() == Some("error"))
}

fn events_include_final_reply(bytes: &[u8]) -> bool {
    parse_sse_events(bytes)
        .iter()
        .any(|event| event.event.as_deref() == Some("final_reply"))
}

fn event_has_assistant_text_update(event: &ParsedSseEvent, needle: &str) -> bool {
    if event.event.as_deref() != Some("projection_update") {
        return false;
    }
    let payload = event_payload_json(event);
    payload["state"]["items"].as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item["text"]["body"]
                .as_str()
                .is_some_and(|body| body.contains(needle))
        })
    })
}

fn event_has_exact_assistant_text_update(event: &ParsedSseEvent, expected: &str) -> bool {
    if event.event.as_deref() != Some("projection_update") {
        return false;
    }
    let payload = event_payload_json(event);
    payload["state"]["items"].as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item["text"]["body"]
                .as_str()
                .is_some_and(|body| body == expected)
        })
    })
}

fn event_has_completed_run_status(event: &ParsedSseEvent) -> bool {
    if !matches!(
        event.event.as_deref(),
        Some("projection_update") | Some("projection_snapshot")
    ) {
        return false;
    }
    let payload = event_payload_json(event);
    payload["state"]["items"].as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item["run_status"]["status"]
                .as_str()
                .is_some_and(|status| matches!(status, "completed" | "succeeded"))
        })
    })
}

fn events_include_completed_status_and_assistant_text(bytes: &[u8], needle: &str) -> bool {
    let events = parse_sse_events(bytes);
    events.iter().any(event_has_completed_run_status)
        && events
            .iter()
            .any(|event| event_has_assistant_text_update(event, needle))
}

fn cursor_scopes_thread(cursor: &str, thread_id: &str) -> bool {
    let Ok(cursor) = serde_json::from_str::<Value>(cursor) else {
        return false;
    };
    let cursor = match cursor.as_str() {
        Some(encoded) => match serde_json::from_str::<Value>(encoded) {
            Ok(decoded) => decoded,
            Err(_) => return false,
        },
        None => cursor,
    };
    cursor["runtime"]["scope"]["read_scope"]["thread_id"].as_str() == Some(thread_id)
        || cursor["live"]["scope"]["read_scope"]["thread_id"].as_str() == Some(thread_id)
        || cursor["turn"]["scope"]["thread_id"].as_str() == Some(thread_id)
}

fn first_scoped_cursor_before_later_id(events: &[ParsedSseEvent], thread_id: &str) -> String {
    events
        .iter()
        .enumerate()
        .find_map(|(index, event)| {
            let id = event.id.as_deref()?;
            if !cursor_scopes_thread(id, thread_id) {
                return None;
            }
            events[index + 1..]
                .iter()
                .any(|later| later.id.is_some())
                .then(|| id.to_string())
        })
        .unwrap_or_else(|| {
            panic!(
                "SSE stream must include a scoped cursor with replayable events after it, got ids: {:?}",
                event_ids(events)
            )
        })
}

fn assert_only_fail_closed_error(label: &str, events: &[ParsedSseEvent]) -> Value {
    assert_eq!(
        events.len(),
        1,
        "{label} must fail closed before emitting replay/projection frames, got: {events:?}"
    );
    let error_event = events
        .first()
        .expect("event count checked")
        .event
        .as_deref();
    assert_eq!(
        error_event,
        Some("error"),
        "{label} must emit only an error event, got: {events:?}"
    );
    event_payload_json(events.first().expect("event count checked"))
}

fn event_payload_json(event: &ParsedSseEvent) -> Value {
    serde_json::from_str(event.data.as_deref().expect("SSE event data is present"))
        .expect("SSE data is JSON")
}

fn serialize_json(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("JSON value serializes")
}

fn webui_extension_setup_scope(extension_id: &str) -> AuthProductScope {
    let seed = format!("webui-v2-extension-setup:{TENANT}:{USER}:{AGENT}::{extension_id}");
    let resource = ResourceScope {
        tenant_id: TenantId::new(TENANT).expect("tenant"),
        user_id: UserId::new(USER).expect("user"),
        agent_id: Some(AgentId::new(AGENT).expect("agent")),
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::from_uuid(uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_OID,
            seed.as_bytes(),
        )),
    };
    AuthProductScope::new(resource, AuthSurface::Callback)
}

// ─── tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn webui_v2_http_list_automations_uses_composed_runtime_facade() {
    let harness = build_harness().await;

    let response = harness
        .router
        .clone()
        .oneshot(bearer_get("/api/webchat/v2/automations?limit=10"))
        .await
        .expect("list automations oneshot");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "list_automations must be wired through the real composed bundle"
    );
    let body = read_json(response).await;
    assert!(
        body["automations"].as_array().is_some(),
        "list_automations response must include an automations array, got: {body:#?}"
    );

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");
}

#[tokio::test]
async fn webui_v2_timeline_persists_display_preview_under_authenticated_owner() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    let harness = build_harness_at_with_runtime_owner_and_auth_user(
        storage_root,
        Some(root),
        Arc::new(ToolCallingGateway::default()),
        local_dev_effective_policy(),
        "e2e-runtime-owner",
        USER,
    )
    .await;

    let thread_id = create_thread(&harness.router, "e2e-preview-owner-create").await;
    send_message(&harness.router, &thread_id, "e2e-preview-owner-send").await;

    let timeline = wait_for_final_timeline(&harness.router, &thread_id).await;
    assert_timeline_has_capability_display_preview(&timeline);

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");
}

/// Beta scoreboard acceptance for issue #3613: drive the WebUI/WebChat
/// v2 API from the browser side, stream live Reborn projections over
/// SSE, replay with `Last-Event-ID`, verify final durable timeline
/// state, reject a cross-thread cursor as a redacted SSE error, and
/// reopen the same local-dev stores to prove the transcript survives
/// runtime restart.
#[tokio::test]
async fn webui_v2_beta_acceptance_stream_replay_restart_and_redaction() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    let harness = build_harness_on_storage(&storage_root).await;

    let thread_id = create_thread(&harness.router, "e2e-create-1").await;
    let response = open_sse(&harness.router, &thread_id, None).await;
    let mut body = response.into_body();

    send_message(&harness.router, &thread_id, "e2e-send-1").await;

    let sse_bytes = collect_sse_until(
        &mut body,
        Duration::from_secs(10),
        events_include_final_reply,
    )
    .await;
    drop(body);

    assert_no_sensitive_payload("live SSE stream", &sse_bytes);
    let events = parse_sse_events(&sse_bytes);
    assert!(
        has_browser_visible_progress(&events),
        "SSE stream must surface browser-visible progress, got: {events:?}; raw: {}",
        String::from_utf8_lossy(&sse_bytes)
    );
    let ids = event_ids(&events);
    assert!(
        ids.len() >= 2,
        "SSE stream must emit at least two cursor ids for replay coverage, got: {events:?}; raw: {}",
        String::from_utf8_lossy(&sse_bytes)
    );

    let replay_from = first_scoped_cursor_before_later_id(&events, &thread_id);
    let replay_response = open_sse(&harness.router, &thread_id, Some(&replay_from)).await;
    let mut replay_body = replay_response.into_body();
    let replay_bytes = collect_sse_until(
        &mut replay_body,
        Duration::from_secs(5),
        events_include_final_reply,
    )
    .await;
    drop(replay_body);

    assert_no_sensitive_payload("replayed SSE stream", &replay_bytes);
    let replay_events = parse_sse_events(&replay_bytes);
    let replay_ids = event_ids(&replay_events);
    assert!(
        !replay_ids.is_empty(),
        "Last-Event-ID replay must return cursor-addressed events after {replay_from}, got: {replay_events:?}; raw: {}",
        String::from_utf8_lossy(&replay_bytes)
    );
    assert!(
        replay_ids.iter().all(|id| id != &replay_from),
        "Last-Event-ID replay must resume after the provided cursor, got: {replay_ids:?}"
    );
    let original_replayable_payloads = replay_payload_signatures(events.iter().skip(1));
    let replayed_payloads = replay_payload_signatures(replay_events.iter());
    assert!(
        replayed_payloads
            .iter()
            .any(|replayed| original_replayable_payloads
                .iter()
                .any(|seen| seen == replayed)),
        "Last-Event-ID replay should include a payload already observed after the first cursor; \
         original ids: {ids:?}, replay ids: {replay_ids:?}, \
         original payloads: {original_replayable_payloads:?}, replay payloads: {replayed_payloads:?}"
    );

    let timeline = wait_for_final_timeline(&harness.router, &thread_id).await;
    assert_timeline_has_tool_result_reference(&timeline);
    assert_no_sensitive_payload("durable timeline", serialize_json(&timeline));

    let other_thread_id = create_thread(&harness.router, "e2e-create-foreign-cursor").await;
    let foreign_response = open_sse(&harness.router, &other_thread_id, Some(&replay_from)).await;
    let mut foreign_body = foreign_response.into_body();
    let foreign_bytes = collect_sse_until(
        &mut foreign_body,
        Duration::from_secs(5),
        events_include_error,
    )
    .await;
    drop(foreign_body);

    assert_no_sensitive_payload("foreign-cursor SSE error", &foreign_bytes);
    let foreign_events = parse_sse_events(&foreign_bytes);
    let error_json = assert_only_fail_closed_error("foreign cursor", &foreign_events);
    assert_eq!(
        error_json["error"], "invalid_request",
        "foreign cursor error should be redacted and non-successful: {error_json}"
    );
    assert_eq!(error_json["retryable"], false);

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");

    let reopened = build_harness_on_storage(&storage_root).await;
    let reopened_timeline = wait_for_final_timeline(&reopened.router, &thread_id).await;
    assert_timeline_has_tool_result_reference(&reopened_timeline);
    assert_no_sensitive_payload(
        "reopened durable timeline",
        serialize_json(&reopened_timeline),
    );

    reopened
        .runtime
        .shutdown()
        .await
        .expect("reopened runtime shutdown clean");
}

#[tokio::test]
async fn webui_v2_sse_streams_assistant_text_before_terminal_completion() {
    let harness = build_harness().await;

    let thread_id = create_thread(&harness.router, "e2e-streaming-text-create").await;
    let response = open_sse(&harness.router, &thread_id, None).await;
    let mut body = response.into_body();

    send_message(&harness.router, &thread_id, "e2e-streaming-text-send").await;

    let sse_bytes = collect_sse_until(&mut body, Duration::from_secs(10), |bytes| {
        events_include_completed_status_and_assistant_text(bytes, "e2e tool ok")
    })
    .await;
    drop(body);

    assert_no_sensitive_payload("assistant text SSE stream", &sse_bytes);
    let events = parse_sse_events(&sse_bytes);
    let text_index = events
        .iter()
        .position(|event| event_has_assistant_text_update(event, "e2e tool ok"))
        .unwrap_or_else(|| {
            panic!(
                "SSE stream never emitted assistant text projection before timeout; events={events:?}; raw={}",
                String::from_utf8_lossy(&sse_bytes)
            )
        });
    let completed_index = events
        .iter()
        .position(event_has_completed_run_status)
        .unwrap_or_else(|| {
            panic!(
                "SSE stream never emitted terminal completed status before timeout; events={events:?}; raw={}",
                String::from_utf8_lossy(&sse_bytes)
            )
        });
    assert!(
        text_index < completed_index,
        "assistant text projection must be observable before terminal completion; events={events:?}; raw={}",
        String::from_utf8_lossy(&sse_bytes)
    );

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");
}

#[tokio::test]
async fn webui_v2_sse_streams_first_assistant_text_update_before_model_completion() {
    let (first_update_tx, first_update_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let harness = build_harness_with_gateway(Arc::new(DelayedStreamingTextGateway::new(
        first_update_tx,
        release_rx,
    )))
    .await;

    let thread_id = create_thread(&harness.router, "e2e-first-token-create").await;
    let response = open_sse(&harness.router, &thread_id, None).await;
    let mut body = response.into_body();

    send_message(&harness.router, &thread_id, "e2e-first-token-send").await;
    tokio::time::timeout(Duration::from_secs(20), first_update_rx)
        .await
        .expect("model gateway should emit its first text update")
        .expect("first text update signal should be sent");

    let first_bytes = collect_sse_until(&mut body, Duration::from_secs(5), |bytes| {
        parse_sse_events(bytes)
            .iter()
            .any(|event| event_has_exact_assistant_text_update(event, "Hel"))
    })
    .await;
    let first_events = parse_sse_events(&first_bytes);
    assert!(
        first_events
            .iter()
            .any(|event| event_has_exact_assistant_text_update(event, "Hel")),
        "SSE stream must expose the first assistant text update before the model completes; events={first_events:?}; raw={}",
        String::from_utf8_lossy(&first_bytes)
    );
    assert!(
        !first_events
            .iter()
            .any(|event| event_has_exact_assistant_text_update(event, "Hello")),
        "SSE stream delivered the final accumulated text before the model was released; events={first_events:?}; raw={}",
        String::from_utf8_lossy(&first_bytes)
    );
    assert!(
        !first_events.iter().any(event_has_completed_run_status),
        "SSE stream delivered terminal completion before the model was released; events={first_events:?}; raw={}",
        String::from_utf8_lossy(&first_bytes)
    );

    release_tx.send(()).expect("release delayed model");
    let final_bytes = collect_sse_until(&mut body, Duration::from_secs(10), |bytes| {
        events_include_completed_status_and_assistant_text(bytes, "Hello")
    })
    .await;
    drop(body);
    let final_events = parse_sse_events(&final_bytes);
    assert!(
        final_events
            .iter()
            .any(|event| event_has_exact_assistant_text_update(event, "Hello")),
        "SSE stream must expose the accumulated assistant text after the model completes; events={final_events:?}; raw={}",
        String::from_utf8_lossy(&final_bytes)
    );
    assert!(
        final_events.iter().any(event_has_completed_run_status),
        "SSE stream must still finalize the run after the model completes; events={final_events:?}; raw={}",
        String::from_utf8_lossy(&final_bytes)
    );

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");
}

#[tokio::test]
async fn webui_v2_gmail_oauth_setup_complete_allows_activation() {
    let harness = build_harness().await;
    let product_auth = harness
        .runtime
        .services()
        .product_auth
        .as_ref()
        .expect("local-dev runtime wires product auth");
    product_auth
        .credential_account_service()
        .create_account(NewCredentialAccount {
            scope: webui_extension_setup_scope("gmail"),
            provider: ironclaw_first_party_extensions::google_provider_id()
                .expect("google provider id"),
            label: CredentialAccountLabel::new("work google").expect("label"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google-access").expect("secret handle")),
            refresh_secret: None,
            scopes: vec![
                ProviderScope::new("https://www.googleapis.com/auth/gmail.readonly")
                    .expect("gmail scope"),
                ProviderScope::new("https://www.googleapis.com/auth/gmail.send")
                    .expect("gmail scope"),
                ProviderScope::new("https://www.googleapis.com/auth/gmail.modify")
                    .expect("gmail scope"),
            ],
        })
        .await
        .expect("seed callback-surface Google OAuth account");

    let package_ref = json!({"kind": "extension", "id": "gmail"});
    let install = harness
        .router
        .clone()
        .oneshot(bearer_post(
            "/api/webchat/v2/extensions/install",
            json!({"package_ref": package_ref}),
        ))
        .await
        .expect("install Gmail oneshot");
    assert_eq!(install.status(), StatusCode::OK);
    let install_body = read_json(install).await;
    assert_eq!(
        install_body["success"], true,
        "install body: {install_body}"
    );

    let setup = harness
        .router
        .clone()
        .oneshot(bearer_get("/api/webchat/v2/extensions/gmail/setup"))
        .await
        .expect("setup Gmail oneshot");
    assert_eq!(setup.status(), StatusCode::OK);
    let setup_body = read_json(setup).await;
    assert_eq!(
        setup_body["secrets"][0]["provided"], true,
        "setup should see the completed Google OAuth account so the UI can offer Activate: {setup_body}"
    );

    let activate = harness
        .router
        .clone()
        .oneshot(bearer_post(
            "/api/webchat/v2/extensions/gmail/activate",
            json!({}),
        ))
        .await
        .expect("activate Gmail oneshot");
    assert_eq!(activate.status(), StatusCode::OK);
    let activate_body = read_json(activate).await;
    assert_eq!(
        activate_body["success"], true,
        "activation should succeed after setup completion: {activate_body}"
    );
    assert_eq!(activate_body["activated"], true);
}

/// The provider-instance readiness map's 400 path
/// (`ProviderInstanceNotConfigured` -> `map_lifecycle_error` -> sanitized
/// `InvalidRequest`) had crate-tier coverage for the port contract
/// (`google_family_activation_fails_closed_when_provider_instance_not_configured`
/// in `extension_lifecycle.rs`) and for the WebUI mapping function
/// (`lifecycle_setup::provider_instance_not_configured_maps_to_sanitized_400`),
/// but nothing drove the real HTTP activate route end-to-end. This proves the
/// whole chain: a composition build with NO Google OAuth backend configured
/// -> the readiness map's `google` entry -> `activation_credential_requirements`
/// failing closed -> the real `POST .../activate` handler returning 400 with
/// the sanitized wire body (no remediation text, no `reason` field at all).
#[tokio::test]
async fn webui_v2_extension_activate_returns_400_when_provider_instance_not_configured() {
    let harness = build_harness_without_google_oauth_backend().await;

    let package_ref = json!({"kind": "extension", "id": "gmail"});
    let install = harness
        .router
        .clone()
        .oneshot(bearer_post(
            "/api/webchat/v2/extensions/install",
            json!({"package_ref": package_ref}),
        ))
        .await
        .expect("install Gmail oneshot");
    assert_eq!(install.status(), StatusCode::OK);
    let install_body = read_json(install).await;
    assert_eq!(
        install_body["success"], true,
        "install body: {install_body}"
    );

    let activate = harness
        .router
        .clone()
        .oneshot(bearer_post(
            "/api/webchat/v2/extensions/gmail/activate",
            json!({}),
        ))
        .await
        .expect("activate Gmail oneshot");
    assert_eq!(
        activate.status(),
        StatusCode::BAD_REQUEST,
        "activation must fail closed before any per-account credential gate when the \
         operator never configured this instance's Google OAuth backend at all"
    );
    let activate_body = read_json(activate).await;
    assert_eq!(activate_body["error"], "invalid_request");
    assert_eq!(activate_body["kind"], "validation");
    assert_eq!(activate_body["retryable"], false);
    assert!(
        activate_body.get("field").is_none(),
        "sanitized 400 body must carry no field hint: {activate_body}"
    );
    assert!(
        activate_body.get("validation_code").is_none(),
        "sanitized 400 body must carry no validation code: {activate_body}"
    );
    let body_text = activate_body.to_string().to_ascii_lowercase();
    for leaked in [
        "config set",
        "client_secret",
        "client_id",
        "console.cloud.google.com",
    ] {
        assert!(
            !body_text.contains(leaked),
            "sanitized 400 body must not leak remediation text: {activate_body}"
        );
    }

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");
}

#[tokio::test]
async fn webui_v2_google_docs_setup_projects_oauth_before_install() {
    let harness = build_harness().await;

    let setup = harness
        .router
        .clone()
        .oneshot(bearer_get("/api/webchat/v2/extensions/google-docs/setup"))
        .await
        .expect("setup Google Docs oneshot");
    assert_eq!(setup.status(), StatusCode::OK);
    let setup_body = read_json(setup).await;
    assert_eq!(setup_body["package_ref"]["id"], "google-docs");
    assert_eq!(setup_body["phase"], "discovered");

    let secrets = setup_body["secrets"]
        .as_array()
        .expect("setup secrets should be an array");
    let mut google_oauth_scopes = secrets
        .iter()
        .filter(|secret| secret["provider"] == "google")
        .map(|secret| {
            let setup = &secret["setup"];
            assert_eq!(setup["kind"], "oauth", "secret should be OAuth: {secret}");
            let mut scopes = setup["scopes"]
                .as_array()
                .expect("OAuth scopes should be an array")
                .iter()
                .map(|scope| scope.as_str().expect("scope string").to_string())
                .collect::<Vec<_>>();
            scopes.sort();
            scopes
        })
        .collect::<Vec<_>>();
    google_oauth_scopes.sort();

    assert_eq!(
        google_oauth_scopes,
        vec![vec![
            "https://www.googleapis.com/auth/documents".to_string(),
            "https://www.googleapis.com/auth/documents.readonly".to_string(),
        ]],
        "Google Docs setup should expose one OAuth credential with the WASM-declared scope union before install: {setup_body}"
    );

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");
}

#[tokio::test]
async fn webui_v2_google_drive_oauth_setup_coalesces_operation_scopes() {
    let harness = build_harness().await;

    let package_ref = json!({"kind": "extension", "id": "google-drive"});
    let install = harness
        .router
        .clone()
        .oneshot(bearer_post(
            "/api/webchat/v2/extensions/install",
            json!({"package_ref": package_ref}),
        ))
        .await
        .expect("install Google Drive oneshot");
    assert_eq!(install.status(), StatusCode::OK);

    let setup = harness
        .router
        .clone()
        .oneshot(bearer_get("/api/webchat/v2/extensions/google-drive/setup"))
        .await
        .expect("setup Google Drive oneshot");
    assert_eq!(setup.status(), StatusCode::OK);
    let setup_body = read_json(setup).await;
    let secrets = setup_body["secrets"]
        .as_array()
        .expect("setup secrets should be an array");
    let google_oauth_scopes = secrets
        .iter()
        .filter(|secret| secret["provider"] == "google")
        .map(|secret| {
            let setup = &secret["setup"];
            assert_eq!(setup["kind"], "oauth", "secret should be OAuth: {secret}");
            let mut scopes = setup["scopes"]
                .as_array()
                .expect("OAuth scopes should be an array")
                .iter()
                .map(|scope| scope.as_str().expect("scope string").to_string())
                .collect::<Vec<_>>();
            scopes.sort();
            scopes
        })
        .collect::<Vec<_>>();

    assert_eq!(
        google_oauth_scopes.len(),
        1,
        "Google Drive setup should coalesce read-only and write OAuth scopes into one credential: {setup_body}"
    );
    assert_eq!(
        google_oauth_scopes[0],
        vec![
            "https://www.googleapis.com/auth/drive".to_string(),
            "https://www.googleapis.com/auth/drive.readonly".to_string(),
        ],
        "Google Drive setup should expose one OAuth credential with the full scope union: {setup_body}"
    );

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");
}

/// SECURITY (review on #4673): a browser request body must not be able to set
/// the caller scope. The v2 request DTOs carry no `tenant_id`/`user_id`/`scope`
/// field and the caller scope is host-minted, so injecting the reserved system
/// sentinel into the JSON body is inert — the thread is created under the
/// authenticated caller and stays readable by them. If the injected sentinel
/// had diverted creation to the system scope (or another tenant), this caller
/// could not read the resulting thread's timeline.
#[tokio::test]
async fn untrusted_request_body_cannot_inject_system_scope() {
    let harness = build_harness().await;
    let sentinel = "\u{1f}SYSTEM\u{1f}";

    let malicious = json!({
        "client_action_id": "inject-scope-1",
        "tenant_id": sentinel,
        "user_id": sentinel,
        "scope": { "tenant_id": sentinel, "user_id": sentinel },
    });
    let create = harness
        .router
        .clone()
        .oneshot(bearer_post("/api/webchat/v2/threads", malicious))
        .await
        .expect("create oneshot");
    assert_eq!(
        create.status(),
        StatusCode::OK,
        "injected scope fields must be ignored (unknown fields), not honored or errored"
    );
    let body = read_json(create).await;
    let thread_id = body["thread"]["thread_id"]
        .as_str()
        .expect("thread_id")
        .to_string();

    let timeline = harness
        .router
        .clone()
        .oneshot(bearer_get(&format!(
            "/api/webchat/v2/threads/{thread_id}/timeline"
        )))
        .await
        .expect("timeline oneshot");
    assert_eq!(
        timeline.status(),
        StatusCode::OK,
        "the thread must belong to the authenticated caller — the body could not set scope"
    );

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");
}

// ─── operator LLM-config smoke (issue #4673) ──────────────────────────
//
// Stands up the same real local-dev runtime as the chat e2e, but with a boot
// config (so the WebUI facade composes the operator LLM-config service) and an
// operator-scoped authenticator (so the `/api/webchat/v2/llm/providers` routes
// mount). Saving the built-in NEAR AI provider stores its API key under the
// system scope; the regression was that the system-scoped secret serialized but
// failed to deserialize, so the very next read-back (snapshot metadata, or the
// previous-key read on a second save) returned `service_unavailable`.

mod operator_llm_config {
    use super::*;
    use ironclaw_reborn_config::{RebornBootConfig, RebornHome, RebornProfile};

    struct OperatorToken;

    #[async_trait]
    impl WebuiAuthenticator for OperatorToken {
        async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
            if token == VALID_TOKEN {
                Some(WebuiAuthentication::operator(
                    UserId::new(USER).expect("user id"),
                ))
            } else {
                None
            }
        }

        fn mounts_operator_webui_config_routes(&self) -> bool {
            true
        }
    }

    async fn build_operator_harness() -> Harness {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().join("local-dev");
        let home = RebornHome::resolve_from_env_parts(
            Some(root.path().join("reborn-home").into_os_string()),
            None,
            None,
        )
        .expect("valid reborn home");
        let boot = RebornBootConfig::new(home, RebornProfile::LocalDev);

        let gateway = Arc::new(ToolCallingGateway::default());
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(USER, storage_root)
                .with_runtime_policy(local_dev_effective_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: TENANT.to_string(),
            agent_id: AGENT.to_string(),
            source_binding_id: "e2e-source".to_string(),
            reply_target_binding_id: "e2e-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(10),
        })
        .with_model_gateway_override(gateway)
        .with_boot_config(boot);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let bundle = build_webui_services(&runtime, None).expect("webui bundle");
        let config = WebuiServeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            Arc::new(OperatorToken),
            vec![HeaderValue::from_static("http://localhost:0")],
        )
        .with_default_agent_id(AgentId::new(AGENT).expect("agent"));
        let router = webui_v2_app(bundle, config).expect("webui v2 app");

        Harness {
            runtime,
            router,
            _root: Some(root),
        }
    }

    fn nearai_save_payload() -> Value {
        json!({
            "id": "nearai",
            "name": "NEAR AI",
            "adapter": "near_ai",
            "base_url": "https://cloud-api.near.ai",
            "default_model": "deepseek-ai/DeepSeek-V4-Flash",
            "api_key": "sk-e2e-operator-key",
            "set_active": true,
            "model": "deepseek-ai/DeepSeek-V4-Flash"
        })
    }

    fn find_nearai(snapshot: &Value) -> &Value {
        snapshot["providers"]
            .as_array()
            .expect("providers array")
            .iter()
            .find(|provider| provider["id"] == "nearai")
            .expect("nearai provider in snapshot")
    }

    #[tokio::test]
    async fn nearai_provider_save_persists_key_and_survives_resave() {
        let harness = build_operator_harness().await;

        // First save: persists the operator's NEAR AI key under the system scope
        // and selects it active.
        let response = harness
            .router
            .clone()
            .oneshot(bearer_post(
                "/api/webchat/v2/llm/providers",
                nearai_save_payload(),
            ))
            .await
            .expect("save request");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "saving the NEAR AI provider must succeed"
        );
        let body = read_json(response).await;
        assert_eq!(body["active"]["provider_id"], "nearai");
        assert_eq!(body["active"]["model"], "deepseek-ai/DeepSeek-V4-Flash");
        assert_eq!(
            find_nearai(&body)["api_key_set"],
            true,
            "the stored system-scope key must read back (regression #4673)"
        );

        // Re-saving reads the previous key back first — the exact op that
        // returned service_unavailable before the system-scope deserialize fix.
        let resave = harness
            .router
            .clone()
            .oneshot(bearer_post(
                "/api/webchat/v2/llm/providers",
                nearai_save_payload(),
            ))
            .await
            .expect("resave request");
        assert_eq!(
            resave.status(),
            StatusCode::OK,
            "re-saving an already-configured provider must not 503"
        );

        // The welcome-screen read-back path: GET must report NEAR AI as active
        // with its key set, not as still requiring setup.
        let get = harness
            .router
            .clone()
            .oneshot(bearer_get("/api/webchat/v2/llm/providers"))
            .await
            .expect("get request");
        assert_eq!(get.status(), StatusCode::OK);
        let snapshot = read_json(get).await;
        assert_eq!(snapshot["active"]["provider_id"], "nearai");
        assert_eq!(find_nearai(&snapshot)["api_key_set"], true);

        harness
            .runtime
            .shutdown()
            .await
            .expect("runtime shutdown clean");
    }
}

fn header_str(headers: &axum::http::HeaderMap, name: header::HeaderName) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

async fn read_body_bytes(response: axum::response::Response) -> Vec<u8> {
    to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("download body within 1 MiB cap")
        .to_vec()
}

fn files_uri(thread_id: &str, suffix: &str, path: &str) -> String {
    // test-only: `path` is interpolated raw into the query string, so paths used
    // here must not contain URL-special characters (`&`, `#`, `?`, spaces). All
    // current callers pass fixed `/workspace/...` constants that satisfy this; a
    // future test needing special chars should URL-encode `path` first.
    format!("/api/webchat/v2/threads/{thread_id}/files{suffix}?path={path}")
}

async fn download_file(
    router: &axum::Router,
    thread_id: &str,
    path: &str,
) -> axum::response::Response {
    router
        .clone()
        .oneshot(bearer_get(&files_uri(thread_id, "/content", path)))
        .await
        .expect("download oneshot")
}

/// Wait until the agent's turn has finalized by polling the timeline for an
/// assistant reply containing `needle`. Polls at a cadence under the read-route
/// rate limit (120/min) so the wait itself never trips a 429.
async fn wait_for_assistant_reply(router: &axum::Router, thread_id: &str, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut timeline = Value::Null;
    while Instant::now() < deadline {
        let response = router
            .clone()
            .oneshot(bearer_get(&format!(
                "/api/webchat/v2/threads/{thread_id}/timeline"
            )))
            .await
            .expect("timeline oneshot");
        assert_eq!(response.status(), StatusCode::OK, "timeline read");
        timeline = read_json(response).await;
        let found = timeline["messages"].as_array().is_some_and(|messages| {
            messages.iter().any(|message| {
                extract_assistant_text(message).is_some_and(|text| text.contains(needle))
            })
        });
        if found {
            return;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    panic!("turn never produced an assistant reply containing {needle:?}; timeline={timeline:#?}");
}

#[tokio::test]
async fn webui_filesystem_memory_mount_is_scoped_to_authenticated_user() {
    let harness = build_two_user_harness(
        Arc::new(MemoryWriteGateway::default()),
        local_dev_effective_policy(),
    )
    .await;
    let router = &harness.router;

    let thread_id = create_thread(router, "e2e-memory-isolation-create").await;
    send_message(router, &thread_id, "e2e-memory-isolation-save").await;
    wait_for_assistant_reply(router, &thread_id, "memory saved").await;

    let owner_raw_memory_path =
        format!("tenants/{TENANT}/users/{USER}/agents/{AGENT}/projects/_none/MEMORY.md");
    let response = router
        .clone()
        .oneshot(bearer_get_with_token(
            &format!("/api/webchat/v2/fs/content?mount=memory&path={owner_raw_memory_path}"),
            USER_B_TOKEN,
        ))
        .await
        .expect("user B memory read oneshot");
    let status = response.status();
    let body = String::from_utf8_lossy(&read_body_bytes(response).await).to_string();
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "user B must not read user A's memory document through the WebUI filesystem browser; body={body}"
    );
    assert!(
        !body.contains(MEMORY_ISOLATION_MARKER),
        "user B response body leaked user A's memory marker: {body}"
    );

    let response = router
        .clone()
        .oneshot(bearer_get_with_token(
            "/api/webchat/v2/fs/list?mount=memory",
            USER_B_TOKEN,
        ))
        .await
        .expect("user B memory list oneshot");
    assert_eq!(response.status(), StatusCode::OK, "user B memory listing");
    let listing = read_json(response).await;
    let entries = listing["entries"]
        .as_array()
        .expect("memory list response carries entries");
    assert!(
        entries.iter().all(|entry| {
            let name = entry["name"].as_str().unwrap_or_default();
            let path = entry["path"].as_str().unwrap_or_default();
            name != "tenants" && name != "MEMORY.md" && !path.contains(USER)
        }),
        "user B memory root must not expose user A's document or the raw memory tree: {listing:#?}"
    );

    let response = router
        .clone()
        .oneshot(bearer_get_with_token(
            "/api/webchat/v2/fs/list?mount=memory",
            VALID_TOKEN,
        ))
        .await
        .expect("user A memory list oneshot");
    assert_eq!(response.status(), StatusCode::OK, "user A memory listing");
    let owner_listing = read_json(response).await;
    assert!(
        owner_listing["entries"].as_array().is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry["name"].as_str() == Some("MEMORY.md")
                    && entry["path"].as_str() == Some("MEMORY.md")
            })
        }),
        "user A should see her own memory document: {owner_listing:#?}"
    );

    let response = router
        .clone()
        .oneshot(bearer_get_with_token(
            "/api/webchat/v2/fs/content?mount=memory&path=MEMORY.md",
            VALID_TOKEN,
        ))
        .await
        .expect("user A memory read oneshot");
    assert_eq!(response.status(), StatusCode::OK, "user A memory read");
    let body = String::from_utf8_lossy(&read_body_bytes(response).await).to_string();
    assert!(
        body.contains(MEMORY_ISOLATION_MARKER),
        "user A should read her own memory marker through the WebUI filesystem browser: {body}"
    );

    let response = router
        .clone()
        .oneshot(bearer_get_with_token(
            "/api/webchat/v2/fs/content?mount=memory&path=MEMORY.md",
            USER_B_TOKEN,
        ))
        .await
        .expect("user B canonical memory read oneshot");
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "user B must not read user A's canonical memory document"
    );
    let body = String::from_utf8_lossy(&read_body_bytes(response).await).to_string();
    assert!(
        !body.contains(MEMORY_ISOLATION_MARKER),
        "user B canonical memory response body leaked user A's memory marker: {body}"
    );

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");
}

/// End-to-end: the agent writes a CSV and a PDF into its project workspace via
/// `write_file`, and both are then listable and downloadable through the v2
/// project-filesystem endpoints with the right mime, attachment disposition,
/// and exact bytes — while a path outside the workspace is refused.
#[tokio::test]
async fn agent_produced_workspace_files_are_listable_and_downloadable() {
    let harness = build_harness_with_gateway_and_policy(
        Arc::new(WriteFileGateway::default()),
        local_yolo_effective_policy(),
    )
    .await;
    let router = &harness.router;

    let thread_id = create_thread(router, "e2e-files-thread").await;
    send_message(router, &thread_id, "e2e-files-send").await;

    // Wait for the turn to finalize (both files written), then download once.
    wait_for_assistant_reply(router, &thread_id, "ready to download").await;

    // CSV: registry-derived mime, attachment disposition + nosniff, exact bytes.
    let csv = download_file(router, &thread_id, CSV_PATH).await;
    assert_eq!(csv.status(), StatusCode::OK);
    let csv_headers = csv.headers().clone();
    assert!(
        header_str(&csv_headers, header::CONTENT_TYPE).starts_with("text/csv"),
        "csv content-type: {:?}",
        csv_headers.get(header::CONTENT_TYPE)
    );
    let disposition = header_str(&csv_headers, header::CONTENT_DISPOSITION);
    assert!(
        disposition.contains("attachment") && disposition.contains("report.csv"),
        "csv content-disposition: {disposition}"
    );
    assert_eq!(
        header_str(&csv_headers, header::X_CONTENT_TYPE_OPTIONS),
        "nosniff"
    );
    assert_eq!(read_body_bytes(csv).await, CSV_BODY.as_bytes());

    // PDF: application/pdf mime + exact bytes.
    let pdf = download_file(router, &thread_id, PDF_PATH).await;
    assert_eq!(pdf.status(), StatusCode::OK);
    assert!(
        header_str(pdf.headers(), header::CONTENT_TYPE).starts_with("application/pdf"),
        "pdf content-type: {:?}",
        pdf.headers().get(header::CONTENT_TYPE)
    );
    assert_eq!(read_body_bytes(pdf).await, PDF_BODY.as_bytes());

    // Directory listing surfaces both files under the workspace root.
    let listing = router
        .clone()
        .oneshot(bearer_get(&format!(
            "/api/webchat/v2/threads/{thread_id}/files?path=/workspace"
        )))
        .await
        .expect("list oneshot");
    assert_eq!(listing.status(), StatusCode::OK);
    let listing = read_json(listing).await;
    let names: Vec<&str> = listing["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .collect();
    assert!(
        names.contains(&"report.csv") && names.contains(&"report.pdf"),
        "workspace listing must include both files, got: {names:?}"
    );

    // Stat reports the written size.
    let stat = router
        .clone()
        .oneshot(bearer_get(&files_uri(&thread_id, "/stat", CSV_PATH)))
        .await
        .expect("stat oneshot");
    assert_eq!(stat.status(), StatusCode::OK);
    let stat = read_json(stat).await;
    assert_eq!(
        stat["stat"]["size_bytes"].as_u64(),
        Some(CSV_BODY.len() as u64)
    );
    // The extension-derived MIME drives the WebUI preview mode and mirrors the
    // download Content-Type.
    assert_eq!(stat["stat"]["mime_type"].as_str(), Some("text/csv"));

    // A path outside the workspace mount is refused.
    let denied = download_file(router, &thread_id, "/secrets/master.key").await;
    assert!(
        matches!(
            denied.status(),
            StatusCode::FORBIDDEN | StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND
        ),
        "out-of-workspace path must be refused, got {}",
        denied.status()
    );

    harness
        .runtime
        .shutdown()
        .await
        .expect("runtime shutdown clean");
}

/// Walks a `ThreadMessageRecord` JSON object and returns the rendered
/// text if it is an assistant reply with content. Done as a free
/// function so the polling loop above can stay readable.
fn extract_assistant_text(message: &Value) -> Option<String> {
    let kind = message.get("kind")?.as_str()?;
    if kind != "assistant" {
        return None;
    }
    message
        .get("content")?
        .as_str()
        .map(std::string::ToString::to_string)
}
