//! `RebornIntegrationHarness` — the integration test tier that runs the full
//! internal Reborn stack and intercepts the model at the vendor-SDK seam.
//!
//! Unlike `RebornBinaryE2EHarness` (which swaps the whole `HostManagedModelGateway`
//! with `RebornTraceReplayModelGateway`), this tier wires the REAL
//! `LlmProviderModelGateway` over the REAL `ironclaw_llm` decorator chain
//! (`apply_decorator_chain`, hermetic passthrough) and only scripts the raw
//! provider underneath via `TraceLlm`. A turn therefore exercises model-profile
//! resolution, `CompletionRequest`/tool-definition assembly, and the
//! retry/routing/circuit/cache decorators for real.
//!
//! Slice 1 scope: InMemory storage, single text reply, `build → submit_turn →
//! assert_reply_contains`.
//! Slice 3: `StorageMode { InMemory, LibSql }` — the builder defaults to
//! `InMemory`; `.storage(StorageMode::LibSql)` selects a real SQLite file in a
//! per-`build()` `TempDir`. Both modes ride **one** `CompositeRootFilesystem`
//! at `/tenants/...` so thread history and turn state share the same backend
//! and the same production path layout.

// Shared integration-test support: not every binary that mounts the
// `reborn_support` tree consumes this module — `support_unit_tests.rs` mounts
// the tree to run the support unit tests but exercises none of the slice-1/2
// integration harness, so its symbols read as dead there under `-D warnings`.
// Module-level allow matches `assertions.rs`/`test_channel.rs`/`live_mission_helpers.rs`.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ironclaw_filesystem::{
    CompositeRootFilesystem, InMemoryBackend, LibSqlRootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, RuntimeHttpEgressRequest, VirtualPath,
};
use ironclaw_llm::testing::provider_chain_over;
use ironclaw_llm::{LlmProvider, SessionConfig, create_session_manager};
use ironclaw_loop_support::{
    EmptyUserProfileSource, HostManagedModelGateway, JsonSpawnSubagentInputCodec,
    SubagentSpawnLimits,
};
use ironclaw_product_adapters::{ProductInboundAck, ProductTriggerReason, ProductWorkflow};
use ironclaw_product_workflow::{
    ConversationBindingService, DefaultInboundTurnService, DefaultProductWorkflow,
    IdempotencyLedger, InboundTurnService, ProductConversationRouteKind, ResolveBindingRequest,
    ResolvedBinding,
};
use ironclaw_reborn::loop_exit_applier::{
    LoopExitEvidencePort, ThreadCheckpointLoopExitEvidencePort,
};
use ironclaw_reborn::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
use ironclaw_reborn::runtime::{
    DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts, RuntimeTurnStateStore,
    build_default_planned_runtime,
};
use ironclaw_reborn::subagent::{
    flavors::StaticSubagentDefinitionResolver, gate_resolution::BoundedSubagentGateResolutionStore,
    goal_store::InMemoryBoundedSubagentGoalStore,
};
use ironclaw_threads::{SessionThreadService, ThreadScope};
use ironclaw_turns::run_profile::{InMemoryLoopHostMilestoneSink, ModelProfileId};
use ironclaw_turns::{
    FilesystemTurnStateStore, GetRunStateRequest, InMemoryCheckpointStateStore,
    LoopCheckpointStore, TurnRunId, TurnScope, TurnStateStore, TurnStatus,
};

use super::harness::{
    EmptyIdentityContextSource, HarnessCapabilityMode, HarnessCapabilityRecorder,
    HarnessTurnBackend, HostRuntimeCapabilityHarness, RecordedCapabilityResult,
    RecordingTestCapabilityPort, test_product_scope,
};
use super::http_matcher::ScriptedHttpResponse;
use super::reply::RebornScriptedReply;
use super::scripted_provider::{SCRIPTED_MODEL_NAME, scripted_trace_llm};
use super::session_thread::RebornThreadHarness;
use super::test_adapter::{RebornTestIngress, RebornTestProductAdapter};

type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The actor/user that submits turns. Reused at binding-probe time and submit
/// time so both resolve to the same conversation binding (and thread).
const HARNESS_ACTOR_ID: &str = "host-user";
/// Model profile the planned runtime requests; the gateway policy permits it.
const INTERACTIVE_MODEL_PROFILE: &str = "interactive_model";

/// Selects the durable storage backend mounted into the integration harness's
/// `CompositeRootFilesystem` (design spec §3.2, §3.8).
///
/// Both modes ride **one** composite at the production path layout
/// `/tenants/<tenant>/users/<user>/...` — the only difference is which
/// `RootFilesystem` is mounted under `/tenants`, `/memory`, and `/events`.
///
/// `InMemory` is the default: it's fast, needs no filesystem, and covers
/// all assertion cases that don't require on-disk durability.
/// `LibSql` creates a real SQLite file in a per-`build()` `TempDir`, runs
/// the full libSQL migration suite, and lets `assert_reply_persists_after_reopen`
/// verify that data survived serialization to disk (design §3.8 guardrail).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StorageMode {
    /// In-memory backend: fast, no filesystem I/O, default.
    #[default]
    InMemory,
    /// Real SQLite on a per-test `TempDir`: full SQL + migrations + CAS.
    /// Enables `assert_reply_persists_after_reopen`.
    LibSql,
}

/// Provider id prefix used by every mock-MCP test capability and assertion.
/// One owner for the string — the `MockMcp` variant and `assert_mcp_tool_called`
/// both derive their ids from this constant.
const MOCK_MCP_PROVIDER_ID: &str = "mock-mcp";

/// Selects the capability backend the integration harness wires.
enum RebornCapabilityBackend {
    /// Echo recorder: records capability invocations, executes nothing. Default —
    /// a text-only turn invokes no tool.
    Echo,
    /// Real first-party tool runtime (`builtin.http` + friends) with the recording
    /// `RuntimeHttpEgress` (scripted body, no network) — the §3.7 Tier-2 capture.
    BuiltinHttpTools,
    /// Real MCP runtime wired to a loopback mock MCP server (slice 6 §3.6).
    /// Uses `LoopbackMcpRuntimeHttpEgress` which makes real HTTP connections to
    /// the mock server; no real credentials or network policy are required.
    MockMcp { mcp_url: String },
}

/// Builder for [`RebornIntegrationHarness`]. The script is fixed at build time
/// (no post-build mutation), matching the existing harness's construction-time
/// queue.
pub struct RebornIntegrationHarnessBuilder {
    conversation_id: String,
    replies: Vec<RebornScriptedReply>,
    capability: RebornCapabilityBackend,
    keyed_http_responses: Vec<ScriptedHttpResponse>,
    storage: StorageMode,
    /// Slice 5: when `true`, the `BuiltinHttpTools` backend uses the real
    /// `LocalHostProcessPort` instead of the inert `RecordingProcessPort`.
    live_shell: bool,
}

impl RebornIntegrationHarnessBuilder {
    /// Set the scripted model replies (consumed in order at the raw-provider seam).
    pub fn script(mut self, replies: impl IntoIterator<Item = RebornScriptedReply>) -> Self {
        self.replies = replies.into_iter().collect();
        self
    }

    /// Select the durable storage backend for this harness.
    ///
    /// Defaults to [`StorageMode::InMemory`]. Pass [`StorageMode::LibSql`] to
    /// test on-disk durability via `assert_reply_persists_after_reopen`.
    pub fn storage(mut self, mode: StorageMode) -> Self {
        self.storage = mode;
        self
    }

    /// Use the real first-party tool runtime so scripted tool calls execute through
    /// `RuntimeHttpEgress`, captured at the recording egress (no network). Required
    /// for tool-calling tests; a text-only turn needs only the default echo backend.
    pub fn with_builtin_http_tools(mut self) -> Self {
        self.capability = RebornCapabilityBackend::BuiltinHttpTools;
        self
    }

    /// Opt-in to real shell execution for this harness (slice 5). By default the
    /// `BuiltinHttpTools` backend injects an inert `RecordingProcessPort` so that
    /// `builtin.shell` turns record the command without spawning any OS process.
    ///
    /// Call `.with_live_shell()` only when the test genuinely needs to observe the
    /// output of a real command (e.g. `echo hello`). The command must be hermetic —
    /// no network, no external state, reproducible on any developer machine.
    ///
    /// Implies [`with_builtin_http_tools`](Self::with_builtin_http_tools).
    pub fn with_live_shell(mut self) -> Self {
        self.capability = RebornCapabilityBackend::BuiltinHttpTools;
        self.live_shell = true;
        self
    }

    /// Install URL/method/capability-keyed scripted HTTP responses over the
    /// recording `RuntimeHttpEgress` (§3.6 P1 ergonomics) and switch on the real
    /// first-party tool runtime. For multi-step tool-HTTP flows where each
    /// `builtin.http` call to a different URL must get a different scripted body;
    /// requests that match no scripted response fall back to the default body.
    /// Implies [`with_builtin_http_tools`](Self::with_builtin_http_tools).
    pub fn with_keyed_http_responses(
        mut self,
        responses: impl IntoIterator<Item = ScriptedHttpResponse>,
    ) -> Self {
        self.capability = RebornCapabilityBackend::BuiltinHttpTools;
        self.keyed_http_responses = responses.into_iter().collect();
        self
    }

    /// Wire the real MCP runtime backed by a loopback mock MCP server (slice 6).
    ///
    /// `mcp_url` is the full mock endpoint URL (e.g. `server.mcp_url()`). The
    /// harness registers a single MCP capability `"<provider>.search"` (where
    /// provider = `"mock-mcp"`) and wires it via `LoopbackMcpRuntimeHttpEgress`
    /// — real HTTP connections to the mock server on a loopback port, with an
    /// injected Bearer token so the mock's OAuth gate passes.
    ///
    /// Script the model with `RebornScriptedReply::tool_call("mock-mcp.search", json!({}))`.
    /// Assert via `assert_mcp_tool_called("search")`.
    pub fn with_mock_mcp(mut self, mcp_url: impl Into<String>) -> Self {
        self.capability = RebornCapabilityBackend::MockMcp {
            mcp_url: mcp_url.into(),
        };
        self
    }

    /// Build the harness: apply hermetic env, wire the real model gateway over
    /// the scripted provider, and start the planned runtime.
    pub async fn build(self) -> HarnessResult<RebornIntegrationHarness> {
        apply_hermetic_env();

        // --- product workflow + binding -------------------------------------
        let adapter = RebornTestProductAdapter::new("reborn-itest", "itest-install")?;
        let ingress = RebornTestIngress::new(adapter);
        let scope = test_product_scope(
            "tenant-itest",
            "host-user",
            "agent-itest",
            Some("project-itest"),
        );
        let product_harness =
            super::product_workflow::RebornProductWorkflowHarness::filesystem_temp(scope)?;

        let probe = ingress.verified_text_envelope_with_trigger(
            "binding-probe",
            HARNESS_ACTOR_ID,
            &self.conversation_id,
            "hi",
            ProductTriggerReason::DirectChat,
        )?;
        let binding = product_harness
            .binding_service()?
            .resolve_binding(binding_request(&probe))
            .await?;
        let thread_scope = thread_scope_from_binding(&binding)?;
        let turn_scope = TurnScope::new_with_owner(
            binding.tenant_id.clone(),
            binding.agent_id.clone(),
            binding.project_id.clone(),
            binding.thread_id.clone(),
            binding.subject_user_id.clone(),
        );

        // --- one composite for threads + turns (slice 3) -------------------
        // `_turn_root` keeps the TempDir alive for the harness's lifetime.
        // Post-migration: this TempDir is the durable root for the whole
        // composite (thread history + turn state), not just turns. The libSQL
        // `.db` file lives in this directory; InMemory ignores the path.
        let turn_root = Arc::new(tempfile::tempdir()?);
        let (composite, libsql_db_path) =
            build_storage_composite(self.storage, turn_root.path()).await?;

        let thread_harness = RebornThreadHarness::filesystem_shared_composite(
            thread_scope.clone(),
            Arc::clone(&composite),
            Arc::clone(&turn_root),
        )?;
        let turn_store: Arc<FilesystemTurnStateStore<HarnessTurnBackend>> =
            Arc::new(FilesystemTurnStateStore::new(scoped_turns_fs_composite(
                Arc::clone(&composite),
                &binding,
            )?));
        let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
        let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> = turn_store.clone();
        let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());

        // --- real model gateway over the scripted raw provider --------------
        let raw: Arc<dyn LlmProvider> = Arc::new(scripted_trace_llm(self.replies));
        let session = create_session_manager(SessionConfig {
            session_path: turn_root.path().join("session.json"),
            ..SessionConfig::default()
        })
        .await;
        let llm_config = ironclaw_llm::testing::nearai_test_config(SCRIPTED_MODEL_NAME);
        let provider = provider_chain_over(raw, &llm_config, session).await?;
        let model_profile_id = ModelProfileId::new(INTERACTIVE_MODEL_PROFILE)
            .map_err(|reason| format!("invalid model profile id: {reason}"))?;
        let policy = LlmModelProfilePolicy::new().allow_model_profile(model_profile_id, None);
        let model_gateway: Arc<dyn HostManagedModelGateway> =
            Arc::new(LlmProviderModelGateway::new(provider, policy));

        // --- capability surface ---------------------------------------------
        // Echo by default (records, executes nothing — a text reply invokes no
        // tool). `with_builtin_http_tools` swaps in the real first-party tool
        // runtime so tool calls execute through `RuntimeHttpEgress`, captured at
        // the recording egress (§3.6/§3.7). Both backends flow through the shared
        // `HarnessCapabilityMode::into_parts` wiring (single mechanism). The echo
        // arm surfaces the port's own allowlist (not `CapabilityAllowSet::All`);
        // benign because a text-only turn invokes no tool.
        let capability_mode = match self.capability {
            RebornCapabilityBackend::Echo => {
                HarnessCapabilityMode::Recording(RecordingTestCapabilityPort::echo())
            }
            RebornCapabilityBackend::BuiltinHttpTools => {
                // Slice 5: `.with_live_shell()` opts into the real LocalHostProcessPort;
                // the default recording path uses the inert RecordingProcessPort.
                let host_runtime = if self.live_shell {
                    HostRuntimeCapabilityHarness::core_builtin_tools_with_live_shell().await?
                } else {
                    HostRuntimeCapabilityHarness::core_builtin_tools().await?
                };
                host_runtime.install_http_responses(self.keyed_http_responses)?;
                HarnessCapabilityMode::HostRuntime(Arc::new(host_runtime))
            }
            RebornCapabilityBackend::MockMcp { mcp_url } => {
                // Slice 6: wire the real MCP runtime backed by the loopback mock server.
                let host_runtime = HostRuntimeCapabilityHarness::mock_mcp_tools(
                    &mcp_url,
                    MOCK_MCP_PROVIDER_ID,
                    &format!("{MOCK_MCP_PROVIDER_ID}.search"),
                )
                .await?;
                HarnessCapabilityMode::HostRuntime(Arc::new(host_runtime))
            }
        };
        let (
            capability_factory,
            capability_surface_resolver,
            capability_input_resolver,
            capability_result_writer,
            capability_recorder,
        ) = capability_mode.into_parts(milestone_sink.clone())?;

        // --- loop-exit evidence (plain; no gates/blocks in slice 1) ---------
        let turn_state_for_evidence: Arc<dyn TurnStateStore> = turn_store.clone();
        let loop_exit_evidence: Arc<dyn LoopExitEvidencePort> =
            Arc::new(ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
                thread_harness.service.clone(),
                turn_state_for_evidence,
                Arc::clone(&loop_checkpoint_store),
                thread_scope.clone(),
            ));

        // --- planned runtime composition ------------------------------------
        // NOTE: this `DefaultPlannedRuntimeParts` literal overlaps the one in
        // `RebornBinaryE2EHarness` — but the two harnesses differ in three places
        // (model_gateway, loop_exit_evidence type, identity source) plus their
        // upstream binding/thread-scope/storage wiring, so the shared core is
        // mostly the 23 default `None` extension-point fields below. Extracting a
        // shared builder now would be a 20+-param bag over a struct that already
        // is the container. Deliberately kept duplicated; extract into a shared
        // `build_harness_planned_runtime(...)` when a THIRD harness copies this,
        // or when these fields start diverging between the two harnesses.
        let turn_state_for_runtime: Arc<dyn RuntimeTurnStateStore> = turn_store.clone();
        let composition = build_default_planned_runtime(DefaultPlannedRuntimeParts {
            turn_state: turn_state_for_runtime,
            thread_service: thread_harness.service.clone() as Arc<dyn SessionThreadService>,
            thread_scope: thread_scope.clone(),
            model_gateway,
            checkpoint_state_store,
            loop_checkpoint_store,
            milestone_sink,
            capability_factory,
            capability_surface_resolver,
            capability_result_writer,
            subagent_goal_store: Arc::new(InMemoryBoundedSubagentGoalStore::new()),
            subagent_gate_store: Arc::new(BoundedSubagentGateResolutionStore::new()),
            subagent_definition_resolver: Arc::new(StaticSubagentDefinitionResolver),
            subagent_spawn_input_codec: Arc::new(JsonSpawnSubagentInputCodec::new(
                capability_input_resolver,
            )),
            subagent_spawn_limits: SubagentSpawnLimits::default(),
            loop_exit_evidence,
            config: DefaultPlannedRuntimeConfig {
                poll_interval: Duration::from_millis(10),
                ..DefaultPlannedRuntimeConfig::default()
            },
            model_route_resolver: None,
            cancellation_factory: None,
            skill_context_source: None,
            input_queue: None,
            identity_context_source: Arc::new(EmptyIdentityContextSource),
            user_profile_source: Arc::new(EmptyUserProfileSource),
            model_policy_guard: None,
            model_budget_accountant: None,
            safety_context: None,
            hook_dispatcher_builder_factory: None,
            communication_context_provider: None,
            hook_security_audit_sink: None,
            turn_event_sink: None,
            attachment_read_port: None,
            scheduler_wake_wiring: None,
        })?;

        // --- product workflow over the coordinator --------------------------
        let binding_service: Arc<dyn ConversationBindingService> =
            Arc::new(product_harness.binding_service()?);
        let inbound: Arc<dyn InboundTurnService> = Arc::new(DefaultInboundTurnService::new(
            Arc::clone(&binding_service),
            thread_harness.service_instance()?,
            composition.coordinator.clone(),
        ));
        let ledger: Arc<dyn IdempotencyLedger> = Arc::new(product_harness.idempotency_ledger());
        let workflow = DefaultProductWorkflow::new(inbound, ledger, binding_service);

        Ok(RebornIntegrationHarness {
            ingress,
            workflow,
            conversation_id: self.conversation_id,
            binding,
            turn_scope,
            turn_store,
            thread_harness,
            scheduler_handle: Some(composition.scheduler_handle),
            event_seq: AtomicU64::new(1),
            capability_recorder,
            _product_harness: product_harness,
            _turn_root: turn_root,
            libsql_db_path,
        })
    }
}

/// Full-stack Reborn integration harness with a scripted raw provider beneath
/// the real decorator chain. See module docs.
pub struct RebornIntegrationHarness {
    ingress: RebornTestIngress,
    workflow: DefaultProductWorkflow,
    conversation_id: String,
    binding: ResolvedBinding,
    turn_scope: TurnScope,
    turn_store: Arc<FilesystemTurnStateStore<HarnessTurnBackend>>,
    thread_harness: RebornThreadHarness<CompositeRootFilesystem>,
    scheduler_handle: Option<ironclaw_host_runtime::TurnRunSchedulerHandle>,
    event_seq: AtomicU64,
    capability_recorder: HarnessCapabilityRecorder,
    _product_harness: super::product_workflow::RebornProductWorkflowHarness,
    /// Keeps the per-`build()` TempDir alive for the harness's lifetime.
    /// This directory is the durable root for the whole composite (thread
    /// history + turn state). For `StorageMode::LibSql`, the SQLite file lives
    /// here; for `StorageMode::InMemory`, only the LLM session cache does.
    _turn_root: Arc<tempfile::TempDir>,
    /// Path to the on-disk SQLite file when `StorageMode::LibSql` was selected.
    /// `None` for `StorageMode::InMemory` (no file on disk). Used by
    /// `assert_reply_persists_after_reopen` to open a genuinely fresh database
    /// connection so only data committed to disk is visible — the live
    /// `CompositeRootFilesystem` Arc is deliberately NOT reused.
    libsql_db_path: Option<PathBuf>,
}

impl RebornIntegrationHarness {
    /// Default harness: InMemory storage, hermetic env, real decorator chain.
    pub fn test_default() -> RebornIntegrationHarnessBuilder {
        Self::builder("conv-itest")
    }

    /// Builder for a specific conversation id.
    pub fn builder(conversation_id: impl Into<String>) -> RebornIntegrationHarnessBuilder {
        RebornIntegrationHarnessBuilder {
            conversation_id: conversation_id.into(),
            replies: Vec::new(),
            capability: RebornCapabilityBackend::Echo,
            keyed_http_responses: Vec::new(),
            storage: StorageMode::default(),
            live_shell: false,
        }
    }

    /// Submit a user turn and wait for it to complete.
    pub async fn submit_turn(&self, text: &str) -> HarnessResult<TurnRunId> {
        let event_id = format!("evt-{}", self.event_seq.fetch_add(1, Ordering::Relaxed));
        let envelope = self.ingress.verified_text_envelope_with_trigger(
            &event_id,
            HARNESS_ACTOR_ID,
            &self.conversation_id,
            text,
            ProductTriggerReason::DirectChat,
        )?;
        let ack = self.workflow.accept_inbound(envelope).await?;
        let run_id = match ack {
            ProductInboundAck::Accepted {
                submitted_run_id, ..
            } => submitted_run_id,
            other => return Err(format!("expected accepted inbound ack, got {other:?}").into()),
        };
        self.wait_for_completion(run_id).await?;
        Ok(run_id)
    }

    /// Assert the finalized assistant reply in thread history contains `text`.
    ///
    /// (Co-located with the harness fields it reads. When the `assert_*` family
    /// grows — `assert_capability_denied`/`assert_capability_order`, design §3.3 —
    /// it can move to a dedicated `assertions.rs` with deliberate field accessors.)
    pub async fn assert_reply_contains(&self, text: &str) -> HarnessResult<()> {
        self.thread_harness
            .assert_final_reply(self.binding.thread_id.clone(), text)
            .await
            .map_err(Into::into)
    }

    /// Assert the finalized reply survives a close-and-reopen of the thread
    /// service (design §3.8 durability guardrail).
    ///
    /// For `StorageMode::LibSql`: opens a **genuinely fresh** `libsql::Database`
    /// connection to the on-disk `.db` file — the live `CompositeRootFilesystem`
    /// Arc is deliberately NOT reused. Only data that was actually serialized and
    /// committed to disk is visible through the new handle, so this assertion
    /// proves real on-disk durability, not an in-process cache.
    ///
    /// For `StorageMode::InMemory`: re-instantiates the
    /// `FilesystemSessionThreadService` over the same in-process handle (no disk
    /// involved). This asserts service re-instantiation but cannot prove durability
    /// — there is nothing on disk to read back. Use `StorageMode::LibSql` for the
    /// durability guarantee.
    pub async fn assert_reply_persists_after_reopen(&self, text: &str) -> HarnessResult<()> {
        if let Some(db_path) = &self.libsql_db_path {
            // Open a fresh libsql connection — independent of the live composite.
            // `libsql::Builder::new_local` opens (or creates) the file at `db_path`;
            // under the M1 mutation (LibSql → InMemory) the file does not exist and
            // the fresh db is empty, so `list_thread_history` returns no messages and
            // `assert_final_reply` returns `Err(MissingFinalReply)`.
            let db = Arc::new(
                libsql::Builder::new_local(db_path)
                    .build()
                    .await
                    .map_err(|e| format!("failed to open fresh libsql for reopen: {e}"))?,
            );
            let fresh_fs = Arc::new(LibSqlRootFilesystem::new(db));
            // Migrations are idempotent — the schema already exists from `build()`.
            fresh_fs
                .run_migrations()
                .await
                .map_err(|e| format!("migrations on fresh libsql reopen: {e}"))?;
            let mut fresh_composite = CompositeRootFilesystem::new();
            ironclaw_reborn_composition::test_support::mount_local_dev_database_roots_for_test(
                &mut fresh_composite,
                fresh_fs,
            )?;
            let fresh_composite = Arc::new(fresh_composite);
            let fresh_harness = RebornThreadHarness::filesystem_shared_composite(
                self.thread_harness.scope.clone(),
                fresh_composite,
                Arc::clone(&self._turn_root),
            )?;
            fresh_harness
                .assert_final_reply(self.binding.thread_id.clone(), text)
                .await
                .map_err(Into::into)
        } else {
            // InMemory: re-instantiate the service over the same in-process handle.
            let reopened = self.thread_harness.reopened()?;
            reopened
                .assert_final_reply(self.binding.thread_id.clone(), text)
                .await
                .map_err(Into::into)
        }
    }

    /// Assert the named capability was invoked through the real capability path
    /// (proves the scripted tool call actually ran the tool).
    pub async fn assert_tool_invoked(&self, capability_id: &str) -> HarnessResult<()> {
        let invocations = self.capability_recorder.invocations();
        if invocations
            .iter()
            .any(|invocation| invocation.capability_id.as_str() == capability_id)
        {
            return Ok(());
        }
        let seen: Vec<&str> = invocations
            .iter()
            .map(|invocation| invocation.capability_id.as_str())
            .collect();
        Err(format!("capability {capability_id:?} was not invoked; saw {seen:?}").into())
    }

    /// Assert a tool HTTP egress request was captured (Tier-2) whose URL contains
    /// `url_substr` — the proof that the tool crossed `RuntimeHttpEgress`.
    pub async fn assert_egress_request_matching(&self, url_substr: &str) -> HarnessResult<()> {
        let requests = self.capability_recorder.runtime_http_requests();
        if requests
            .iter()
            .any(|request| request.url.contains(url_substr))
        {
            return Ok(());
        }
        let seen: Vec<&str> = requests
            .iter()
            .map(|request| request.url.as_str())
            .collect();
        Err(format!(
            "no captured runtime HTTP egress request matching {url_substr:?}; saw {seen:?}"
        )
        .into())
    }

    /// Snapshot of the captured Tier-2 runtime HTTP egress requests, in call
    /// order. Read by the egress assertions in `assertions.rs` (the canonical
    /// egress-assertion API — method/URL/body/count/order).
    pub(super) fn captured_egress_requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.capability_recorder.runtime_http_requests()
    }

    /// Assert that a `builtin.shell` command was recorded by the inert process
    /// port and that the recorded command string contains `substr`. This proves
    /// the shell tool call was dispatched through the process port without
    /// spawning a real OS process (slice 5 safety invariant).
    pub async fn assert_shell_command_recorded(&self, substr: &str) -> HarnessResult<()> {
        let commands = self.capability_recorder.recorded_process_commands();
        if commands.iter().any(|cmd| cmd.contains(substr)) {
            return Ok(());
        }
        let seen: Vec<&str> = commands.iter().map(|s| s.as_str()).collect();
        Err(format!("no recorded shell command containing {substr:?}; saw {seen:?}").into())
    }

    /// Asserts ≥1 shell command was dispatched through the inert recording
    /// process port, proving no real OS process was spawned. Passes when
    /// `recorded_process_commands()` is non-empty (the harness used the
    /// recording path, not the live-shell opt-in).
    pub async fn assert_shell_ran_through_inert_port(&self) -> HarnessResult<()> {
        let commands = self.capability_recorder.recorded_process_commands();
        if !commands.is_empty() {
            return Ok(());
        }
        Err(
            "no shell commands were recorded by the inert process port; either no \
             builtin.shell turn ran or the harness is using the live-shell path"
                .into(),
        )
    }

    /// Assert that the MCP tool named `tool_name` (the name on the mock server,
    /// e.g. `"search"`) was invoked via the real MCP runtime (slice 6).
    ///
    /// Internally maps `tool_name` → capability id `"mock-mcp.{tool_name}"` and
    /// delegates to `assert_tool_invoked`. The `"mock-mcp"` prefix matches the
    /// fixed provider id set by `with_mock_mcp`.
    pub async fn assert_mcp_tool_called(&self, tool_name: &str) -> HarnessResult<()> {
        self.assert_tool_invoked(&format!("{MOCK_MCP_PROVIDER_ID}.{tool_name}"))
            .await
    }

    /// Snapshot of the recorded capability results (tool outputs), in execution
    /// order. Read by `assert_tool_result_contains` in `assertions.rs`.
    pub(super) fn captured_capability_results(&self) -> Vec<RecordedCapabilityResult> {
        self.capability_recorder.capability_results()
    }

    async fn wait_for_completion(&self, run_id: TurnRunId) -> HarnessResult<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let state = self
                .turn_store
                .get_run_state(GetRunStateRequest {
                    scope: self.turn_scope.clone(),
                    run_id,
                })
                .await?;
            if state.status == TurnStatus::Completed {
                return Ok(());
            }
            if state.status.is_terminal() {
                return Err(format!(
                    "run reached terminal status {:?} before Completed; failure={:?}",
                    state.status, state.failure
                )
                .into());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for Completed; last status={:?} failure={:?}",
                    state.status, state.failure
                )
                .into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

impl Drop for RebornIntegrationHarness {
    fn drop(&mut self) {
        // Scheduler shutdown is async and cannot run from Drop; dropping the
        // handle closes the command channel and the supervisor task exits.
        let _ = self.scheduler_handle.take();
    }
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

/// Build the one `CompositeRootFilesystem` for a harness, selecting the durable
/// backend by `mode`. The `dir` argument is used only for `LibSql` (the SQLite
/// file is created there by the production `build_default_local_dev_database_roots`
/// sequence); `InMemory` ignores it.
///
/// Returns the composite alongside the path to the on-disk SQLite file for
/// `LibSql` (`None` for `InMemory`). The path is stored on
/// `RebornIntegrationHarness` so `assert_reply_persists_after_reopen` can open
/// a genuinely fresh database connection — independent of the live
/// `CompositeRootFilesystem` Arc — and confirm real on-disk durability.
async fn build_storage_composite(
    mode: StorageMode,
    dir: &Path,
) -> HarnessResult<(Arc<CompositeRootFilesystem>, Option<PathBuf>)> {
    let mut composite = CompositeRootFilesystem::new();
    let db_path = match mode {
        StorageMode::InMemory => {
            ironclaw_reborn_composition::test_support::mount_local_dev_database_roots_for_test(
                &mut composite,
                Arc::new(InMemoryBackend::new()),
            )?;
            None
        }
        StorageMode::LibSql => {
            ironclaw_reborn_composition::test_support::build_default_local_dev_database_roots_for_test(
                dir,
                &mut composite,
            )
            .await?;
            // The canonical filename is the production constant — one source of truth.
            Some(dir.join(ironclaw_reborn_composition::test_support::LOCAL_DEV_DB_FILENAME))
        }
    };
    Ok((Arc::new(composite), db_path))
}

/// Build a `ScopedFilesystem` that maps `/turns` → the turn-state path for
/// `binding` inside the production composite.
///
/// Uses the production path prefix `""` (no `/engine` prefix) so turn state
/// lands under `/tenants/...` inside the composite, where the database backend
/// is mounted. The 4-arm match lives in `filesystem::turns_scope_path`; the
/// binary-E2E tier reuses it via `scoped_turns_fs` in `harness.rs` with the
/// `/engine` prefix.
pub(super) fn scoped_turns_fs_composite(
    composite: Arc<CompositeRootFilesystem>,
    binding: &ResolvedBinding,
) -> HarnessResult<Arc<ScopedFilesystem<CompositeRootFilesystem>>> {
    let target = super::filesystem::turns_scope_path("", binding);
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").expect("valid turns alias"),
        VirtualPath::new(target).expect("valid turns target"),
        MountPermissions::read_write_list_delete(),
    )])?;
    Ok(Arc::new(ScopedFilesystem::with_fixed_view(
        composite, mounts,
    )))
}

// ---------------------------------------------------------------------------
// Hermetic env and private helpers
// ---------------------------------------------------------------------------

/// Hermetic env baked unconditionally so every test form inherits it and a
/// developer `.env` can never reach a vendor (design §2/§4.1). The chain itself
/// reads the explicit passthrough `LlmConfig`, so the LLM env vars are belt-and-
/// suspenders; keychain disable + UTC are genuinely load-bearing for hermeticity.
///
/// Applied exactly once per process via [`OnceLock`]: the values are constant,
/// and `cargo test` runs `#[tokio::test]`s in parallel threads within one binary
/// — a per-call `set_var`/`remove_var` would be a data race (and is `unsafe`
/// under edition 2024). Once-init runs before any concurrent `build()` mutates
/// or reads the environment.
fn apply_hermetic_env() {
    static HERMETIC_ENV: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    HERMETIC_ENV.get_or_init(|| {
        // Serialize against all other env-mutating tests in this binary.
        let _env_guard = ironclaw_common::env_helpers::lock_env();
        // SAFETY: Edition 2024 requires `unsafe` for `std::env::set_var` /
        // `remove_var`. The `lock_env()` guard serializes against all other
        // env-mutating tests in this binary; values are constants set once.
        unsafe {
            std::env::set_var("IRONCLAW_DISABLE_OS_KEYCHAIN", "1");
            std::env::set_var("TZ", "UTC");
            std::env::set_var("LLM_MAX_RETRIES", "0");
            std::env::remove_var("NEARAI_CHEAP_MODEL");
            std::env::remove_var("NEARAI_FALLBACK_MODEL");
            std::env::remove_var("LLM_CHEAP_MODEL");
            std::env::remove_var("LLM_CIRCUIT_BREAKER_THRESHOLD");
            std::env::remove_var("CIRCUIT_BREAKER_THRESHOLD");
            std::env::remove_var("LLM_RESPONSE_CACHE_ENABLED");
            std::env::remove_var("RESPONSE_CACHE_ENABLED");
            std::env::remove_var("NEARAI_SESSION_TOKEN");
        }
    });
}

/// Assemble a `ResolveBindingRequest` from a verified inbound envelope. Slice 1
/// is DirectChat-only, so the route kind is `Direct`.
fn binding_request(
    envelope: &ironclaw_product_adapters::ProductInboundEnvelope,
) -> ResolveBindingRequest {
    ResolveBindingRequest {
        adapter_id: envelope.adapter_id().clone(),
        installation_id: envelope.installation_id().clone(),
        external_actor_ref: envelope.external_actor_ref().clone(),
        external_conversation_ref: envelope.external_conversation_ref().clone(),
        external_event_id: envelope.external_event_id().clone(),
        route_kind: ProductConversationRouteKind::Direct,
        auth_claim: envelope.auth_claim().clone(),
    }
}

fn thread_scope_from_binding(binding: &ResolvedBinding) -> HarnessResult<ThreadScope> {
    Ok(ThreadScope {
        tenant_id: binding.tenant_id.clone(),
        agent_id: binding
            .agent_id
            .clone()
            .ok_or("resolved binding missing agent id")?,
        project_id: binding.project_id.clone(),
        owner_user_id: binding.subject_user_id.clone(),
        mission_id: None,
    })
}
