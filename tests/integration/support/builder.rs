//! `RebornIntegrationHarness` — the integration test tier that runs the full
//! internal Reborn stack and intercepts the model at the vendor-SDK seam.
//!
//! Unlike `RebornBinaryE2EHarness` (swaps the whole `HostManagedModelGateway`),
//! this tier wires the REAL `LlmProviderModelGateway` over the REAL
//! `ironclaw_llm` decorator chain and only scripts the raw provider underneath
//! via `TraceLlm` — a turn exercises model-profile resolution, request/tool-def
//! assembly, and the retry/routing/circuit/cache decorators for real.
//!
//! `StorageMode { InMemory, LibSql }` — defaults to `InMemory`;
//! `.storage(StorageMode::LibSql)` selects a real SQLite file in a
//! per-`build()` `TempDir`. Both ride **one** `CompositeRootFilesystem` at
//! `/tenants/...` so thread history and turn state share the same backend.

// Shared integration-test support: not every binary that mounts the
// `reborn_support` tree consumes this module — `support_unit_tests.rs` mounts
// the tree to run the support unit tests but exercises none of the slice-1/2
// integration harness, so its symbols read as dead there under `-D warnings`.
// Module-level allow matches `assertions.rs`/`test_channel.rs`/`live_mission_helpers.rs`.
#![allow(dead_code)]

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ironclaw_filesystem::{
    CompositeRootFilesystem, InMemoryBackend, LibSqlRootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope,
    RuntimeHttpEgressRequest, UserId, VirtualPath,
};
use ironclaw_llm::Role;
use ironclaw_network::{NetworkHttpRequest, NetworkTransportRequest};
use ironclaw_product_adapters::{ProductInboundAck, ProductTriggerReason, ProductWorkflow};
use ironclaw_product_workflow::{
    DefaultProductWorkflow, ProductConversationRouteKind, ResolveBindingRequest, ResolvedBinding,
};
use ironclaw_runner::loop_driver_host::HookDispatcherBuilderFactory;
use ironclaw_runner::runtime::ToolDisclosureMode;
use ironclaw_threads::ThreadScope;
use ironclaw_turns::run_profile::{
    CommunicationContextProvider, InstructionSafetyContext, LoopHostMilestone,
};
use ironclaw_turns::{
    CancelRunRequest, CancelRunResponse, FilesystemTurnStateStore, GateRef, GateResumeDisposition,
    GetRunStateRequest, IdempotencyKey, ReplyTargetBindingRef, ResumeTurnPrecondition,
    ResumeTurnRequest, SanitizedCancelReason, SourceBindingRef, TurnActor, TurnCoordinator,
    TurnRunId, TurnRunState, TurnScope, TurnStateStore, TurnStatus,
};

use super::capability_backend::{
    CapabilityScriptingInputs, MOCK_MCP_PROVIDER_ID, RebornCapabilityBackend, ShellMode,
};
use super::doubles::ParkingCapabilityGate;
use super::group::{GroupCapability, GroupSharedStorage, RebornIntegrationGroup};
use super::harness::{HarnessCapabilityRecorder, HarnessTurnBackend, RecordedCapabilityResult};
use super::http_matcher::ScriptedHttpResponse;
use super::planned_runtime_parts_shape::DefaultPlannedRuntimePartsShape;
use super::process::ScriptedProcessResult;
use super::reply::RebornScriptedReply;
use super::scripted_provider::ParkingModelGate;
use super::session_thread::RebornThreadHarness;
use super::test_adapter::RebornTestIngress;
use crate::support::trace_llm::TraceLlm;

type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The actor/user that submits turns. Reused at binding-probe time and submit
/// time so both resolve to the same conversation binding (and thread).
pub(crate) const HARNESS_ACTOR_ID: &str = "host-user";
/// Model profile the planned runtime requests; the gateway policy permits it.
pub(crate) const INTERACTIVE_MODEL_PROFILE: &str = "interactive_model";

/// Selects the durable storage backend mounted into the integration harness's
/// `CompositeRootFilesystem`. Both modes ride **one** composite at the
/// production path layout `/tenants/<tenant>/users/<user>/...` — the only
/// difference is which `RootFilesystem` is mounted under `/tenants`,
/// `/memory`, and `/events`.
///
/// `InMemory` (default): fast, no filesystem, covers all cases that don't
/// need on-disk durability. `LibSql`: real SQLite in a per-`build()`
/// `TempDir`, full migrations, enables `assert_reply_persists_after_reopen`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StorageMode {
    /// In-memory backend: fast, no filesystem I/O, default.
    #[default]
    InMemory,
    /// Real SQLite on a per-test `TempDir`: full SQL + migrations + CAS.
    /// Enables `assert_reply_persists_after_reopen`.
    LibSql,
}

/// Builder for [`RebornIntegrationHarness`]. The script is fixed at build time
/// (no post-build mutation), matching the existing harness's construction-time
/// queue.
pub struct RebornIntegrationHarnessBuilder {
    conversation_id: String,
    replies: Vec<RebornScriptedReply>,
    capability: RebornCapabilityBackend,
    keyed_http_responses: Vec<ScriptedHttpResponse>,
    web_access_response_bodies: Vec<Vec<u8>>,
    /// W4-AUTHGATE-WIRE: FIFO scripted statuses for the `GithubIssueTools`
    /// backend's **network**-egress lane (see `with_github_network_status`).
    github_network_statuses: Vec<u16>,
    /// S1 seam: FIFO scripted response bodies for the real-egress-pipeline
    /// backend's wire-level transport recorder (see
    /// `with_real_egress_response_bodies`).
    real_egress_response_bodies: Vec<Vec<u8>>,
    storage: StorageMode,
    safety_context: Option<InstructionSafetyContext>,
    /// How the `BuiltinHttpTools` backend wires `builtin.shell`. One enum instead
    /// of a `bool` + `Option` so the modes are mutually exclusive by
    /// construction — the last shell-selecting builder method wins, and a live
    /// runtime can never carry a stale scripted result.
    shell_mode: ShellMode,
    /// E-GATEWAY: when set, the model call parks until released, enabling a
    /// mid-turn cancel test. Threaded into the degenerate one-thread group.
    park_gate: Option<ParkingModelGate>,
    /// E-GATEWAY (C-ERRORS): when `true`, the model call always fails with a
    /// fixed non-retryable `LlmError`. Threaded into the degenerate one-thread
    /// group. See [`RebornThreadBuilder::fail_model`].
    fail_model: bool,
    /// C-TRACECAP seam: install an in-memory `TurnEventSink` when `true`.
    turn_event_sink: bool,
    /// Force `ToolDisclosureMode::Bridged` into the underlying group's ONE
    /// planned runtime, bypassing `REBORN_TOOL_DISCLOSURE`/`from_env()`
    /// (test-only knob; see `RebornIntegrationGroupBuilder::tool_disclosure`).
    /// `None` (default) resolves via `ToolDisclosureMode::from_env()`, matching
    /// today's behavior byte-for-byte.
    tool_disclosure: Option<ToolDisclosureMode>,
    /// C-BUDGET: when `true`, wire the production budget accountant into the
    /// degenerate one-thread group (see `RebornIntegrationGroupBuilder::budget_accounting`).
    budget_accounting: bool,
    /// C-COMMCTX: optional communication-context provider threaded into the
    /// degenerate one-thread group (see
    /// `RebornIntegrationGroupBuilder::communication_context_provider`).
    communication_context_provider: Option<Arc<dyn CommunicationContextProvider>>,
    /// C-HOOKS / E-HOOK-INFRA: optional per-run hook dispatcher builder factory
    /// threaded into the degenerate one-thread group (see
    /// `RebornIntegrationGroupBuilder::hook_dispatcher_builder_factory`).
    hook_dispatcher_builder_factory: Option<HookDispatcherBuilderFactory>,
    /// E-GATEWAY tool-path analog of `park_gate`: when set, this harness's
    /// `BuiltinHttpTools` capability dispatch parks until released (issue
    /// #5476 lease-wedge coverage). Threaded into `RebornCapabilityBackend::install`.
    park_tool_gate: Option<ParkingCapabilityGate>,
    /// Shortens the underlying group's turn-state store lease TTL (default
    /// 90s) for lease-expiry-under-a-wedged-tool coverage. Threaded into
    /// `RebornIntegrationGroupBuilder::with_runner_lease_ttl_for_test`.
    runner_lease_ttl: Option<chrono::Duration>,
    /// Shortens the underlying group's scheduler lease-recovery sweep
    /// interval (default 10s) for lease-expiry-under-a-wedged-tool coverage.
    /// Threaded into
    /// `RebornIntegrationGroupBuilder::with_lease_recovery_interval_for_test`.
    lease_recovery_interval: Option<Duration>,
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

    /// Wire a model-visible instruction-safety banner (`InstructionSafetyContext`)
    /// into the harness's underlying group. Rendered verbatim as a `system`-role
    /// prompt message ahead of any per-turn instructions; read back via
    /// `assert_system_prompt_contains`. Defaults to `None` (no banner, matching
    /// today's behavior) — see `tests/integration/support/group.rs`'s
    /// `RebornIntegrationGroupBuilder::safety_context` for the underlying wiring.
    pub fn with_safety_context(mut self, ctx: InstructionSafetyContext) -> Self {
        self.safety_context = Some(ctx);
        self
    }

    /// Wire the production budget accountant into this harness's underlying group
    /// (C-BUDGET). On the turn's first model call the accountant seeds the run
    /// owner's daily USD cap into an in-memory governor; read it back with
    /// `assert_budget_user_cap_seeded`. Defaults off. See
    /// `RebornIntegrationGroupBuilder::budget_accounting` for the wiring.
    pub fn with_budget_accounting(mut self) -> Self {
        self.budget_accounting = true;
        self
    }

    /// Wire a `CommunicationContextProvider` into this harness's underlying group
    /// (C-COMMCTX), so the delivery-preference / connected-channel slice it
    /// resolves renders into the model request (assert via
    /// `assert_model_request_contains`). Defaults `None`. See
    /// `RebornIntegrationGroupBuilder::communication_context_provider`.
    pub fn with_communication_context_provider(
        mut self,
        provider: Arc<dyn CommunicationContextProvider>,
    ) -> Self {
        self.communication_context_provider = Some(provider);
        self
    }

    /// Wire a per-run `HookDispatcherBuilderFactory` into this harness's
    /// underlying group (C-HOOKS / E-HOOK-INFRA), so hooks fire at their
    /// lifecycle points on a coordinator-path turn. Defaults `None`. See
    /// `RebornIntegrationGroupBuilder::hook_dispatcher_builder_factory`.
    pub fn with_hook_factory(mut self, factory: HookDispatcherBuilderFactory) -> Self {
        self.hook_dispatcher_builder_factory = Some(factory);
        self
    }

    /// Park this harness's model call until `gate` is released (E-GATEWAY seam),
    /// so a test can cancel the run mid-turn. See
    /// [`RebornThreadBuilder::park_model`].
    pub fn park_model(mut self, gate: ParkingModelGate) -> Self {
        self.park_gate = Some(gate);
        self
    }

    /// Fail this harness's model call unconditionally with a fixed, non-retryable
    /// `LlmError` (E-GATEWAY seam, C-ERRORS). See
    /// [`RebornThreadBuilder::fail_model`](super::group::RebornThreadBuilder::fail_model).
    pub fn fail_model(mut self) -> Self {
        self.fail_model = true;
        self
    }

    /// Park this harness's tool/capability dispatch until released
    /// (tool-path analog of `park_model`, issue #5476 lease-wedge coverage).
    /// Only the `BuiltinHttpTools` backend wires this today. See
    /// `ParkingCapabilityGate`.
    pub fn park_tool_dispatch(mut self, gate: ParkingCapabilityGate) -> Self {
        self.park_tool_gate = Some(gate);
        self
    }

    /// Shorten the underlying group's turn-state store lease TTL (default 90s)
    /// for lease-expiry-under-a-wedged-tool coverage. `None` (default) leaves
    /// today's behavior byte-identical.
    pub fn with_runner_lease_ttl_for_test(mut self, ttl: chrono::Duration) -> Self {
        self.runner_lease_ttl = Some(ttl);
        self
    }

    /// Shorten the underlying group's scheduler lease-recovery sweep interval
    /// (default 10s) so a wedged run is reaped without waiting on the
    /// production tick. `None` (default) leaves today's behavior
    /// byte-identical. See
    /// `RebornIntegrationGroupBuilder::with_lease_recovery_interval_for_test`.
    pub fn with_lease_recovery_interval_for_test(mut self, interval: Duration) -> Self {
        self.lease_recovery_interval = Some(interval);
        self
    }

    /// Install an in-memory `TurnEventSink` into the underlying group's planned
    /// runtime (C-TRACECAP). Read the recorded events back with
    /// [`RebornIntegrationHarness::recorded_turn_events`].
    pub fn with_turn_event_sink(mut self) -> Self {
        self.turn_event_sink = true;
        self
    }

    /// Force `ToolDisclosureMode::Bridged` for this harness's underlying group
    /// (enabler (b), `REBORN_TOOL_DISCLOSURE=Bridged`), so the bridged decorator
    /// (`ToolDisclosureCapabilityDecorator`) replaces the flat per-capability
    /// tool list with the bridge meta tools in the `tools` argument shipped
    /// to the model. Only `tool_search` is ever ADVERTISED to the model;
    /// `tool_describe`/`tool_call` are retained internally for describe-first
    /// routing and never appear on the model-visible tool surface (see
    /// `tool_disclosure.rs`'s `bridged_mode_defers_wide_catalog_to_bridge_meta_tools`).
    /// Deferral is ALSO threshold-gated (`select_active_set`,
    /// `DisclosureCaps::default().max_tools = 32`): backends under the cap
    /// (e.g. `BuiltinHttpTools`, 13 tools) stay flat even in Bridged mode —
    /// pair with `.with_github_issue_tools()` (48 tools) to observe deferral.
    /// Read back with `assert_model_tools_contains`/
    /// `assert_model_tools_excludes`. Never mutates the process env — avoids
    /// the `#[tokio::test]` concurrent-test race a raw env var would hit (see
    /// `ToolDisclosureMode::from_env`, `apply_hermetic_env`).
    pub fn with_tool_disclosure_bridged(mut self) -> Self {
        self.tool_disclosure = Some(ToolDisclosureMode::Bridged);
        self
    }

    /// Force `ToolDisclosureMode::Off` for this harness's underlying group,
    /// bypassing `REBORN_TOOL_DISCLOSURE`/`from_env()`. Use this to pin a
    /// negative-control test's mode explicitly rather than relying on the
    /// ambient env default — see
    /// `RebornIntegrationGroupBuilder::with_tool_disclosure_off` for why the
    /// env-resolution path alone is not control-safe.
    pub fn with_tool_disclosure_off(mut self) -> Self {
        self.tool_disclosure = Some(ToolDisclosureMode::Off);
        self
    }

    /// Use the real first-party tool runtime so scripted tool calls execute through
    /// `RuntimeHttpEgress`, captured at the recording egress (no network). Required
    /// for tool-calling tests; a text-only turn needs only the default echo backend.
    pub fn with_builtin_http_tools(mut self) -> Self {
        self.capability = RebornCapabilityBackend::BuiltinHttpTools;
        self
    }

    /// `write_file`/`read_file` tools (same set as `file_tools()`), backed by
    /// the REAL `LocalDevCapabilityIo` (durable tool-result projection seam,
    /// issue #5838) instead of the ephemeral `ProductLiveCapabilityIo` test
    /// double, so a large `read_file` output is persisted durably and
    /// `result_read` can page through it.
    pub fn with_durable_capability_io_file_tools(mut self) -> Self {
        self.capability = RebornCapabilityBackend::FileToolsDurableIo;
        self
    }

    /// Harness-port-seam Change 4: same as `.with_builtin_http_tools()` plus a
    /// confirmed `/host` mount grant, so `wrap_local_dev_surface_disclosure`'s
    /// scoped-roots note is observable on `read_file`'s captured tool
    /// definition (the layer is disabled without a confirmed host-home mount).
    pub fn with_confirmed_host_mount(mut self) -> Self {
        self.capability = RebornCapabilityBackend::BuiltinHttpToolsConfirmedHostMount;
        self
    }

    /// Opt-in to real shell execution for this harness. By default the
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
        self.shell_mode = ShellMode::Live;
        self
    }

    /// Script the inert recording process port so `builtin.shell` returns a
    /// non-zero exit code (error-path coverage). The tool still surfaces a
    /// *Completed* result carrying `exit_code`/`success: false`. Implies
    /// [`with_builtin_http_tools`](Self::with_builtin_http_tools).
    pub fn with_shell_exit_code(mut self, exit_code: i64) -> Self {
        self.capability = RebornCapabilityBackend::BuiltinHttpTools;
        self.shell_mode = ShellMode::Scripted(ScriptedProcessResult::ExitCode(exit_code));
        self
    }

    /// Script the inert recording process port so `builtin.shell` returns a
    /// timeout error (`RuntimeProcessError::Timeout`), which the tool maps to a
    /// recoverable model-visible `Failed{Resource}` capability error. Implies
    /// [`with_builtin_http_tools`](Self::with_builtin_http_tools).
    pub fn with_shell_timeout(mut self) -> Self {
        self.capability = RebornCapabilityBackend::BuiltinHttpTools;
        self.shell_mode = ShellMode::Scripted(ScriptedProcessResult::Timeout);
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

    /// Wire the GitHub first-party WASM capabilities behind a
    /// `GithubHarnessAuthorizer`, which allows every dispatch with an
    /// `InjectCredentialAccountOnce` obligation. A scripted `github.*` tool call
    /// then executes the real WASM module, whose outbound HTTP request has a
    /// synthetic `Authorization: Bearer <token>` credential injected by the host
    /// egress pipeline before it reaches the recording network egress. Proves
    /// credential injection reaches the wire (T0-SECRET-INJECT).
    ///
    /// Script the model with
    /// `RebornScriptedReply::tool_call("github.get_repo", json!({"owner": ..., "repo": ...}))`
    /// followed by a `RebornScriptedReply::text(..)` turn, then assert with
    /// [`assert_network_egress_header_contains`](RebornIntegrationHarness::assert_network_egress_header_contains).
    pub fn with_github_issue_tools(mut self) -> Self {
        self.capability = RebornCapabilityBackend::GithubIssueTools;
        self
    }

    /// W4-AUTHGATE-WIRE: script the GitHub WASM capability's real HTTP call to
    /// come back with `status` instead of the default `200` (FIFO, one call
    /// consumed per queued status). The `GithubIssueTools` backend's real call
    /// flows through the **network** egress lane, not the runtime-egress lane
    /// `with_keyed_http_responses` scripts (`try_with_host_http_egress`
    /// overwrites the runtime port), so a runtime-401 (credential-injected-but-401,
    /// distinct from `github_issue_tools_auth_required`'s credential-missing
    /// path) must be scripted here. Implies [`with_github_issue_tools`](Self::with_github_issue_tools).
    pub fn with_github_network_status(mut self, status: u16) -> Self {
        self.capability = RebornCapabilityBackend::GithubIssueTools;
        self.github_network_statuses.push(status);
        self
    }

    /// Wire the real first-party `web-access.search` / `web-access.get_content`
    /// capabilities (C-WEBACCESS). `response_bodies` scripts the three-leg Exa
    /// MCP handshake (`initialize` → `notifications/initialized` → `tools/call`)
    /// in call order — all three legs target the same URL/method/capability, so
    /// they cannot be told apart by the keyed HTTP matcher and are instead
    /// installed onto the recording egress's FIFO queue at build time. Script
    /// the model with
    /// `RebornScriptedReply::tool_call("web-access.search", json!({"query": ...}))`
    /// followed by a trailing text turn.
    pub fn with_web_access_tools(
        mut self,
        response_bodies: impl IntoIterator<Item = Vec<u8>>,
    ) -> Self {
        self.capability = RebornCapabilityBackend::WebAccessTools;
        self.web_access_response_bodies = response_bodies.into_iter().collect();
        self
    }

    /// S1 seam: wire the real first-party tool runtime over the REAL
    /// production egress pipeline — `PolicyNetworkHttpEgress` (network-policy
    /// enforcement + DNS/private-IP checks) and `HostHttpEgressService` (leak
    /// scan) both run for real; only the wire-level transport is a recorder.
    /// Distinct from [`with_builtin_http_tools`](Self::with_builtin_http_tools),
    /// whose `RecordingRuntimeHttpEgress` bypasses both security layers.
    pub fn with_real_egress_pipeline(mut self) -> Self {
        self.capability = RebornCapabilityBackend::BuiltinHttpToolsRealEgress;
        self
    }

    /// Like [`with_real_egress_pipeline`](Self::with_real_egress_pipeline),
    /// but also installs FIFO scripted response bodies onto the wire-level
    /// transport recorder — for scripting a response the real leak-scan
    /// pipeline should react to.
    pub fn with_real_egress_response_bodies(
        mut self,
        bodies: impl IntoIterator<Item = Vec<u8>>,
    ) -> Self {
        self.capability = RebornCapabilityBackend::BuiltinHttpToolsRealEgress;
        self.real_egress_response_bodies = bodies.into_iter().collect();
        self
    }

    /// Wire the real MCP runtime backed by a loopback mock MCP server.
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
    ///
    /// Routes through an internal, degenerate one-thread `RebornIntegrationGroup`
    /// so there is exactly ONE assembly path for both groups and single-shot
    /// harnesses — no de-facto fork.
    pub async fn build(self) -> HarnessResult<RebornIntegrationHarness> {
        apply_hermetic_env();

        // --- capability backend → GroupCapability --------------------------
        // Echo by default (records, executes nothing — a text reply invokes no
        // tool). Builtin/MCP swap in the real first-party runtime. (Live approval
        // stores are a group-only backend; see `RebornIntegrationGroup::live_approvals`.)
        let group_capability = self
            .capability
            .install(
                self.shell_mode,
                CapabilityScriptingInputs {
                    keyed_http_responses: self.keyed_http_responses,
                    web_access_response_bodies: self.web_access_response_bodies,
                    github_network_statuses: self.github_network_statuses,
                    real_egress_response_bodies: self.real_egress_response_bodies,
                },
                self.park_tool_gate,
            )
            .await?;

        // Routed through the group/thread builder (one assembly path for both
        // groups and single-shot harnesses). A single-shot harness is a
        // degenerate one-thread group and submits as the default
        // `HARNESS_ACTOR_ID`.
        let mut group_builder = RebornIntegrationGroup::builder().storage(self.storage);
        if let Some(ctx) = self.safety_context {
            group_builder = group_builder.safety_context(ctx);
        }
        if self.turn_event_sink {
            group_builder = group_builder.with_turn_event_sink();
        }
        match self.tool_disclosure {
            Some(ToolDisclosureMode::Bridged) => {
                group_builder = group_builder.with_tool_disclosure_bridged();
            }
            Some(ToolDisclosureMode::Off) => {
                group_builder = group_builder.with_tool_disclosure_off();
            }
            None => {}
        }
        if self.budget_accounting {
            group_builder = group_builder.budget_accounting();
        }
        if let Some(provider) = self.communication_context_provider {
            group_builder = group_builder.communication_context_provider(provider);
        }
        if let Some(factory) = self.hook_dispatcher_builder_factory {
            group_builder = group_builder.hook_dispatcher_builder_factory(factory);
        }
        if let Some(ttl) = self.runner_lease_ttl {
            group_builder = group_builder.with_runner_lease_ttl_for_test(ttl);
        }
        if let Some(interval) = self.lease_recovery_interval {
            group_builder = group_builder.with_lease_recovery_interval_for_test(interval);
        }
        let group: RebornIntegrationGroup = group_builder
            .build_with_capability(group_capability)
            .await?;
        group
            .thread(self.conversation_id)
            .script(self.replies)
            .park_model_opt(self.park_gate)
            .fail_model_opt(self.fail_model)
            .build()
            .await
    }
}

/// Full-stack Reborn integration harness with a scripted raw provider beneath
/// the real decorator chain. See module docs.
pub struct RebornIntegrationHarness {
    pub(crate) ingress: RebornTestIngress,
    pub(crate) workflow: DefaultProductWorkflow,
    pub(crate) conversation_id: String,
    /// External (raw, pre-resolution) actor id every submit for this thread is
    /// made under. Defaults to `HARNESS_ACTOR_ID`; a group thread built with
    /// `with_actor_id` (E-MULTIUSER seam) carries its distinct actor here so
    /// submit-time envelopes resolve the SAME binding as the build-time probe.
    ///
    /// NOT redundant with `binding.actor_user_id` (a one-way hashed opaque
    /// `UserId`): `verified_text_envelope_with_trigger` needs the raw,
    /// pre-hash string to compute the SAME `binding_path` hash the probe
    /// persisted under — substituting the hashed field here would silently
    /// resolve a different binding on every submit. `binding.actor_user_id`
    /// remains the right source for `resume_run`'s `TurnActor` (no envelope
    /// round-trip there).
    pub(crate) actor_id: String,
    pub(crate) binding: ResolvedBinding,
    pub(crate) turn_scope: TurnScope,
    pub(crate) turn_store: Arc<FilesystemTurnStateStore<HarnessTurnBackend>>,
    pub(crate) thread_harness: RebornThreadHarness<CompositeRootFilesystem>,
    /// Turn coordinator, used to resume a `BlockedApproval`/`BlockedAuth` run
    /// after `approve_gate`/`deny_gate` resolves the gate. Mirrors the binary-E2E
    /// harness's `resume_with_gate` path.
    pub(crate) coordinator: Arc<dyn TurnCoordinator>,
    pub(crate) event_seq: AtomicU64,
    pub(crate) capability_recorder: HarnessCapabilityRecorder,
    /// The concrete scripted `TraceLlm` retained before it was upcast to
    /// `dyn LlmProvider`. Its `captured_requests()` lets assertions inspect the
    /// exact model-visible requests (system prompt, host-injected context —
    /// `assert_system_prompt_contains`/`assert_model_request_contains`).
    /// Retained even when parked (`park_model`, E-GATEWAY): `ParkingLlm` only
    /// wraps this SAME `TraceLlm`.
    pub(crate) scripted_llm: Arc<TraceLlm>,
    /// Shared storage bundle keeping the composite, TempDir, product harness, and
    /// capability alive for this harness's lifetime. For a single-shot harness the
    /// Arc is the sole owner; for a group thread it is shared with the group and
    /// any sibling harnesses (R6: sequential-drop is a usage convention, not a
    /// lifetime bound).
    pub(crate) _shared: Arc<GroupSharedStorage>,
    /// Invocation count at harness construction (before any turn). Assertions
    /// slice `[baseline..]` so a group thread only sees its own entries even
    /// when sharing a recorder with prior threads (R2).
    pub(crate) baseline_invocation_count: usize,
    /// Egress-request count at harness construction. See `baseline_invocation_count`.
    pub(crate) baseline_egress_count: usize,
    /// Capability-result count at harness construction. See `baseline_invocation_count`.
    pub(crate) baseline_result_count: usize,
    /// Recorded-process-command count at harness construction. See `baseline_invocation_count`.
    pub(crate) baseline_process_count: usize,
    /// Network-egress-request count at harness construction. See `baseline_invocation_count`.
    pub(crate) baseline_network_count: usize,
    /// Security-audit-event count at harness construction. See
    /// `baseline_invocation_count`.
    pub(crate) baseline_security_audit_count: usize,
    /// Turn-lifecycle-event count on the group-shared `InMemoryTurnEventSink` at
    /// harness construction, if `.with_turn_event_sink()` opted in. The sink has
    /// no per-thread channel, so without this baseline a group thread's
    /// `assert_turn_event_recorded` could pass on an earlier thread's event.
    /// `recorded_turn_events` slices `[baseline_turn_event_count..]` like the
    /// other `baseline_*_count` fields (R2).
    pub(crate) baseline_turn_event_count: usize,
    /// Loop milestone count at harness construction. Milestones share one group
    /// sink, so assertions slice from this baseline just like capability and
    /// turn-event recordings.
    pub(crate) baseline_milestone_count: usize,
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
            web_access_response_bodies: Vec::new(),
            github_network_statuses: Vec::new(),
            real_egress_response_bodies: Vec::new(),
            storage: StorageMode::default(),
            safety_context: None,
            shell_mode: ShellMode::default(),
            park_gate: None,
            fail_model: false,
            turn_event_sink: false,
            tool_disclosure: None,
            budget_accounting: false,
            communication_context_provider: None,
            hook_dispatcher_builder_factory: None,
            park_tool_gate: None,
            runner_lease_ttl: None,
            lease_recovery_interval: None,
        }
    }

    /// W5-WIRING-PARITY: the Some/None shape of the `DefaultPlannedRuntimeParts`
    /// literal this (degenerate one-thread group's) planned runtime was
    /// actually built from, captured at `into_group` construction time. See
    /// `tests/integration/wiring_parity.rs`.
    pub fn planned_runtime_parts_shape(&self) -> DefaultPlannedRuntimePartsShape {
        self._shared.planned_runtime_parts_shape
    }

    /// Loop host milestones emitted after this harness was built. Scoped to
    /// `support` (not `pub`) — tests must read milestones through a named
    /// `assert_*` helper in `assertions.rs`, not by pattern-matching raw
    /// `LoopHostMilestoneKind` variants at the call site.
    pub(super) fn loop_milestones(&self) -> Vec<LoopHostMilestone> {
        self._shared
            .milestone_sink
            .milestones()
            .into_iter()
            .skip(self.baseline_milestone_count)
            .collect()
    }

    /// Number of loop milestones recorded for this harness right now (i.e.
    /// `[baseline_milestone_count..]` so far). Capture at the START of a turn
    /// on a multi-turn harness and pass to `assert_compaction_failed_since` so
    /// a prior turn's milestone can't satisfy the assertion — the
    /// milestone analogue of `history_len`.
    pub async fn milestone_len(&self) -> HarnessResult<usize> {
        Ok(self.loop_milestones().len())
    }

    /// Submit a user turn and wait for it to complete.
    pub async fn submit_turn(&self, text: &str) -> HarnessResult<TurnRunId> {
        let run_id = self.submit_turn_async(text).await?;
        self.wait_for_status(run_id, TurnStatus::Completed).await?;
        Ok(run_id)
    }

    /// Enqueue additional scripted replies AFTER the harness is built — for a
    /// second turn whose tool-call arguments depend on a server-minted value
    /// (e.g. a durable `result_ref`) only known once an earlier turn has
    /// completed and its result has been read back from persisted state. The
    /// fixed-at-build-time script (`.script(..)`) remains the norm; reach for
    /// this only when the dependent value genuinely cannot be known ahead of
    /// time.
    pub fn push_script(&self, replies: impl IntoIterator<Item = RebornScriptedReply>) {
        for reply in replies {
            self.scripted_llm.push_step(reply.into_step());
        }
    }

    /// Submit a user turn and return its run id **without** waiting for any status
    /// — the caller drives the wait (`wait_for_status`). Used by approval/auth flows
    /// where the turn blocks on a gate rather than completing.
    pub async fn submit_turn_async(&self, text: &str) -> HarnessResult<TurnRunId> {
        Self::run_id_from_ack(self.submit_turn_ack(text).await?)
    }

    /// Submit a user turn carrying one inline image attachment, and wait for it
    /// to complete (C-ATTACH). Lands `bytes` through the harness's real
    /// `InboundAttachmentLander` (production `ProjectScopedAttachmentLander` over
    /// the local-dev workspace filesystem — wired only by `.attachment_tools()`
    /// groups) via `DefaultProductWorkflow::submit_inbound_with_attachments`, the
    /// same production entry point a synchronous host surface (e.g. the
    /// OpenAI-compatible API) uses for inline image bytes. Errors clearly if the
    /// harness has no lander wired.
    pub async fn submit_turn_with_image_attachment(
        &self,
        text: &str,
        filename: &str,
        mime_type: &str,
        bytes: Vec<u8>,
    ) -> HarnessResult<TurnRunId> {
        if self.capability_recorder.attachment_test_support().is_none() {
            return Err(
                "no attachment lander wired — build the harness via RebornIntegrationGroup::attachment_tools()"
                    .into(),
            );
        }
        let (event_id, envelope) = self.build_user_envelope(text)?;
        let attachment = ironclaw_attachments::InboundAttachment {
            id: format!("{event_id}-att-0"),
            mime_type: mime_type.to_string(),
            filename: Some(filename.to_string()),
            bytes,
        };
        let ack = self
            .workflow
            .submit_inbound_with_attachments(envelope, vec![attachment])
            .await?;
        let run_id = Self::run_id_from_ack(ack)?;
        self.wait_for_status(run_id, TurnStatus::Completed).await?;
        Ok(run_id)
    }

    /// Submit a user turn carrying N inline attachments of any mime type, and
    /// wait for it to complete (W4-ATTACH-VARIANTS). Generalizes
    /// `submit_turn_with_image_attachment` to multiple attachments and
    /// non-image kinds (e.g. `text/plain`, classified `Document` — extracted
    /// to text and rendered into the `<attachments>` block rather than read
    /// back as a multimodal part). Same entry point and lander requirement as
    /// `submit_turn_with_image_attachment`.
    pub async fn submit_turn_with_attachments(
        &self,
        text: &str,
        attachments: Vec<(&str, &str, Vec<u8>)>,
    ) -> HarnessResult<TurnRunId> {
        if self.capability_recorder.attachment_test_support().is_none() {
            return Err(
                "no attachment lander wired — build the harness via RebornIntegrationGroup::attachment_tools()"
                    .into(),
            );
        }
        let (event_id, envelope) = self.build_user_envelope(text)?;
        let inbound = attachments
            .into_iter()
            .enumerate()
            .map(
                |(index, (filename, mime_type, bytes))| ironclaw_attachments::InboundAttachment {
                    id: format!("{event_id}-att-{index}"),
                    mime_type: mime_type.to_string(),
                    filename: Some(filename.to_string()),
                    bytes,
                },
            )
            .collect();
        let ack = self
            .workflow
            .submit_inbound_with_attachments(envelope, inbound)
            .await?;
        let run_id = Self::run_id_from_ack(ack)?;
        self.wait_for_status(run_id, TurnStatus::Completed).await?;
        Ok(run_id)
    }

    /// Build the synthetic inbound envelope `submit_turn_ack` and
    /// `submit_turn_with_image_attachment` both submit, plus the `event_id` it
    /// was minted from (attachment ids derive from it).
    fn build_user_envelope(
        &self,
        text: &str,
    ) -> HarnessResult<(String, ironclaw_product_adapters::ProductInboundEnvelope)> {
        let event_id = format!("evt-{}", self.event_seq.fetch_add(1, Ordering::Relaxed));
        let envelope = self.ingress.verified_text_envelope_with_trigger(
            &event_id,
            &self.actor_id,
            &self.conversation_id,
            text,
            ProductTriggerReason::DirectChat,
        )?;
        Ok((event_id, envelope))
    }

    /// Extract the submitted run id from an `Accepted` ack, or a descriptive
    /// error for any other outcome. Shared by every `submit_turn*` entry point.
    fn run_id_from_ack(ack: ProductInboundAck) -> HarnessResult<TurnRunId> {
        match ack {
            ProductInboundAck::Accepted {
                submitted_run_id, ..
            } => Ok(submitted_run_id),
            other => Err(format!("expected accepted inbound ack, got {other:?}").into()),
        }
    }

    /// Submit a user turn and return the raw `ProductInboundAck` — `Accepted` OR
    /// `RejectedBusy` (thread already has an active run). Most callers want
    /// `submit_turn_async`, which narrows to `Accepted` and errors on any other
    /// ack; this is the seam for C-ERRORS' busy-reject test, which needs to
    /// observe `RejectedBusy` without that narrowing turning it into an `Err`.
    pub async fn submit_turn_ack(&self, text: &str) -> HarnessResult<ProductInboundAck> {
        let event_id = format!("evt-{}", self.event_seq.fetch_add(1, Ordering::Relaxed));
        let envelope = self.ingress.verified_text_envelope_with_trigger(
            &event_id,
            &self.actor_id,
            &self.conversation_id,
            text,
            ProductTriggerReason::DirectChat,
        )?;
        Ok(self.workflow.accept_inbound(envelope).await?)
    }

    /// Submit a user turn and wait until it blocks on an approval gate, returning
    /// the run id and the raised `GateRef`. The named C1 fixture: a scripted
    /// destructive tool call in a `RebornIntegrationGroup::live_approvals` thread
    /// blocks here; the test then calls `approve_gate`/`deny_gate` and
    /// `wait_for_status(Completed)`.
    pub async fn submit_turn_until_blocked(
        &self,
        text: &str,
    ) -> HarnessResult<(TurnRunId, GateRef)> {
        let run_id = self.submit_turn_async(text).await?;
        let state = self
            .wait_for_status(run_id, TurnStatus::BlockedApproval)
            .await?;
        let gate_ref = state
            .gate_ref
            .ok_or("blocked approval run missing gate ref")?;
        if !gate_ref.as_str().starts_with("gate:approval-") {
            return Err(format!("expected a local-dev approval gate, got {gate_ref:?}").into());
        }
        Ok((run_id, gate_ref))
    }

    /// Submit a user turn and wait until it blocks on an **auth** gate, returning
    /// the run id and the raised `GateRef`. Mirror of `submit_turn_until_blocked`
    /// for the `RebornIntegrationGroup::live_auth_gate` fixture: a scripted
    /// capability whose credential account resolves to `AuthRequired` blocks here
    /// at `TurnStatus::BlockedAuth` (E-AUTHGATE seam).
    pub async fn submit_turn_until_auth_blocked(
        &self,
        text: &str,
    ) -> HarnessResult<(TurnRunId, GateRef)> {
        let run_id = self.submit_turn_async(text).await?;
        let state = self
            .wait_for_status(run_id, TurnStatus::BlockedAuth)
            .await?;
        let gate_ref = state.gate_ref.ok_or("blocked auth run missing gate ref")?;
        if !gate_ref.as_str().starts_with("gate:auth-") {
            return Err(format!("expected an auth gate ref, got {gate_ref:?}").into());
        }
        Ok((run_id, gate_ref))
    }

    /// Resolve a blocked approval gate via a REAL `submit_inbound(ApprovalResolution)`
    /// — the dispatch arm a real adapter's "approve"/"deny" reply hits
    /// (`ApprovalInteractionService::resolve`), unlike `approve_gate`/`deny_gate`
    /// (which resume the coordinator directly, bypassing the interaction
    /// service entirely). Only reaches a real resolution when the group was
    /// built with `.with_real_gate_dispatch_services()` — otherwise the
    /// workflow's default `RejectingApprovalInteractionService` rejects the
    /// payload outright.
    pub async fn submit_approval_resolution(
        &self,
        gate_ref: &GateRef,
        decision: ironclaw_product_adapters::ApprovalDecision,
    ) -> HarnessResult<ProductInboundAck> {
        let event_id = format!("evt-{}", self.event_seq.fetch_add(1, Ordering::Relaxed));
        let envelope = self.ingress.verified_approval_resolution_envelope(
            &event_id,
            &self.actor_id,
            &self.conversation_id,
            gate_ref.as_str(),
            decision,
        )?;
        Ok(self.workflow.submit_inbound(envelope).await?)
    }

    /// Auth-side counterpart of [`submit_approval_resolution`](Self::submit_approval_resolution):
    /// a REAL `submit_inbound(AuthResolution)`, dispatching through
    /// `AuthInteractionService::resolve` instead of `resolve_auth_gate`/
    /// `deny_auth_gate`'s direct coordinator resume.
    pub async fn submit_auth_resolution(
        &self,
        gate_ref: &GateRef,
        result: ironclaw_product_adapters::AuthResolutionResult,
    ) -> HarnessResult<ProductInboundAck> {
        let event_id = format!("evt-{}", self.event_seq.fetch_add(1, Ordering::Relaxed));
        let envelope = self.ingress.verified_auth_resolution_envelope(
            &event_id,
            &self.actor_id,
            &self.conversation_id,
            gate_ref.as_str(),
            result,
        )?;
        Ok(self.workflow.submit_inbound(envelope).await?)
    }

    /// Assert the finalized assistant reply in thread history contains `text`.
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
    /// connection to the on-disk file (the live composite `Arc` is deliberately
    /// NOT reused), so this proves real on-disk durability. For `InMemory`:
    /// re-instantiates the service over the same in-process handle — asserts
    /// re-instantiation only, not durability (nothing on disk to read back).
    pub async fn assert_reply_persists_after_reopen(&self, text: &str) -> HarnessResult<()> {
        if let Some(db_path) = &self._shared.libsql_db_path {
            // Open a fresh composite — independent of the live one.
            // `libsql::Builder::new_local` opens (or creates) the file at `db_path`;
            // under the M1 mutation (LibSql → InMemory) the file does not exist and
            // the fresh db is empty, so `list_thread_history` returns no messages and
            // `assert_final_reply` returns `Err(MissingFinalReply)`.
            let fresh_composite = reopen_fresh_libsql_composite(db_path).await?;
            let fresh_harness = RebornThreadHarness::filesystem_shared_composite(
                self.thread_harness.scope.clone(),
                fresh_composite,
                Arc::clone(&self._shared.turn_root),
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

    /// S2 seam: assert `run_id` is parked on `expected_gate_ref` in a
    /// **genuinely fresh** turn-state store connection to the on-disk LibSql
    /// file (mirrors [`assert_reply_persists_after_reopen`]'s reopen idiom,
    /// but reads run/gate state instead of thread history). Requires
    /// `StorageMode::LibSql` — errors otherwise, since there is no on-disk
    /// file for an `InMemory` group to independently reopen.
    pub async fn assert_gate_survives_reopen(
        &self,
        run_id: TurnRunId,
        expected_gate_ref: &GateRef,
    ) -> HarnessResult<()> {
        let db_path = self
            ._shared
            .libsql_db_path
            .as_ref()
            .ok_or("assert_gate_survives_reopen requires StorageMode::LibSql")?;
        let fresh_composite = reopen_fresh_libsql_composite(db_path).await?;
        let fresh_turn_store = FilesystemTurnStateStore::new(scoped_turns_fs_composite(
            fresh_composite,
            &self._shared.canonical_binding,
        )?);
        let state = fresh_turn_store
            .get_run_state(GetRunStateRequest {
                scope: self.turn_scope.clone(),
                run_id,
            })
            .await?;
        if state.status != TurnStatus::BlockedApproval {
            return Err(format!(
                "expected BlockedApproval after reopen, got {:?}",
                state.status
            )
            .into());
        }
        match state.gate_ref.as_ref().map(GateRef::as_str) {
            Some(seen) if seen == expected_gate_ref.as_str() => Ok(()),
            other => Err(format!(
                "gate ref after reopen was {other:?}, expected {:?}",
                expected_gate_ref.as_str()
            )
            .into()),
        }
    }

    /// E-DURABLE: assert an installed extension survives an independent reopen
    /// of the capability composite. Opens a FRESH `ExtensionInstallationStore`
    /// at the capability harness's on-disk `storage_root` (a handle independent
    /// of the live `Arc`) and asserts `extension_id` is present — proving the
    /// install persisted to disk, not just to in-memory state. Parallels
    /// `assert_reply_persists_after_reopen` for capability-produced state.
    pub async fn assert_extension_install_persists_after_reopen(
        &self,
        extension_id: &str,
    ) -> HarnessResult<()> {
        self._shared
            .capability
            .assert_extension_install_persists_after_reopen(extension_id)
            .await
    }

    /// Assert the named capability was invoked through the real capability path
    /// (proves the scripted tool call actually ran the tool).
    ///
    /// Checks only the `[baseline_invocation_count..]` delta so a group thread
    /// never spuriously passes on a prior thread's entry (R2).
    pub async fn assert_tool_invoked(&self, capability_id: &str) -> HarnessResult<()> {
        let all = self.capability_recorder.invocations();
        let delta = &all[self.baseline_invocation_count..];
        if delta
            .iter()
            .any(|invocation| invocation.capability_id.as_str() == capability_id)
        {
            return Ok(());
        }
        let seen: Vec<&str> = delta
            .iter()
            .map(|invocation| invocation.capability_id.as_str())
            .collect();
        Err(format!("capability {capability_id:?} was not invoked; saw {seen:?}").into())
    }

    /// Assert the named capability was NOT invoked through the real
    /// capability path (proves a visibility/gating filter held). Same
    /// delta-scoping as `assert_tool_invoked` (R2), but the diagnostic
    /// `seen` list is captured on the failure branch that matters here —
    /// when the capability unexpectedly WAS dispatched.
    pub async fn assert_tool_not_invoked(&self, capability_id: &str) -> HarnessResult<()> {
        let all = self.capability_recorder.invocations();
        let delta = &all[self.baseline_invocation_count..];
        if !delta
            .iter()
            .any(|invocation| invocation.capability_id.as_str() == capability_id)
        {
            return Ok(());
        }
        let seen: Vec<&str> = delta
            .iter()
            .map(|invocation| invocation.capability_id.as_str())
            .collect();
        Err(format!("capability {capability_id:?} was invoked; saw {seen:?}").into())
    }

    /// S2 seam: assert the named capability produced EXACTLY `expected`
    /// recorded RESULTS (`captured_capability_results`) — the proof that a
    /// gate resume dispatched the gated capability's real execution once,
    /// not zero (lost gate) or twice (double-execution on resume). Reads the
    /// result-write recorder, NOT `invocations()`: a gated call is recorded
    /// as an invocation attempt before the gate parks the run (no result is
    /// written yet), so `invocations()` legitimately counts 2 for any
    /// gate-then-resume flow — that is not a double-execution signal.
    pub async fn assert_capability_result_count(
        &self,
        capability_id: &str,
        expected: usize,
    ) -> HarnessResult<()> {
        let results = self.captured_capability_results();
        let actual = results
            .iter()
            .filter(|result| result.capability_id.as_str() == capability_id)
            .count();
        if actual == expected {
            return Ok(());
        }
        Err(format!(
            "expected capability {capability_id:?} to produce {expected} recorded result(s), saw {actual}"
        )
        .into())
    }

    /// Assert a tool HTTP egress request was captured (Tier-2) whose URL contains
    /// `url_substr` — the proof that the tool crossed `RuntimeHttpEgress`.
    ///
    /// Checks only the `[baseline_egress_count..]` delta (R2).
    pub async fn assert_egress_request_matching(&self, url_substr: &str) -> HarnessResult<()> {
        let requests = self.captured_egress_requests();
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

    /// Snapshot of the captured Tier-2 runtime HTTP egress requests for this
    /// thread only (`[baseline_egress_count..]` delta), in call order. Read by
    /// the egress assertions in `assertions.rs` (canonical egress-assertion API
    /// — method/URL/body/count/order).
    pub(super) fn captured_egress_requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        let all = self.capability_recorder.runtime_http_requests();
        all[self.baseline_egress_count..].to_vec()
    }

    /// Every `System`-role prompt the model saw across the captured requests, in
    /// call order. Reads the scripted `TraceLlm` retained before the
    /// `dyn LlmProvider` upcast (`scripted_llm`). Empty until the first turn is
    /// submitted. Read by `assert_system_prompt_contains` in `assertions.rs`.
    ///
    /// No `[baseline..]` slice (unlike `captured_egress_requests`): `scripted_llm`
    /// is a fresh per-thread `Arc<TraceLlm>` built in `RebornThreadBuilder::build`,
    /// not a group-shared recorder, so it only ever holds this thread's requests.
    pub(super) fn captured_system_prompts(&self) -> Vec<String> {
        self.scripted_llm
            .captured_requests()
            .into_iter()
            .flatten()
            .filter(|message| matches!(message.role, Role::System))
            .map(|message| message.content)
            .collect()
    }

    /// Turn-lifecycle events recorded by the in-memory `TurnEventSink`
    /// installed via `.with_turn_event_sink()` (C-TRACECAP), for this thread
    /// only. Empty when the harness did not opt in. Reads the group-shared
    /// sink but slices `[baseline_turn_event_count..]` (R2) so a group thread
    /// never sees an earlier sibling thread's events.
    pub(super) fn recorded_turn_events(&self) -> Vec<ironclaw_turns::TurnLifecycleEvent> {
        self._shared
            .turn_event_sink
            .as_ref()
            .map(|sink| {
                let all = sink.events();
                all[self.baseline_turn_event_count.min(all.len())..].to_vec()
            })
            .unwrap_or_default()
    }

    /// Security-audit events recorded by the always-wired harness recorder, for
    /// this thread only. Reads the group-shared sink but slices
    /// `[baseline_security_audit_count..]` so a group thread never sees an
    /// earlier sibling thread's events.
    pub(super) fn recorded_security_audit_events(
        &self,
    ) -> Vec<ironclaw_events::SecurityAuditEvent> {
        let all = self._shared.security_audit_sink.events();
        all[self.baseline_security_audit_count.min(all.len())..].to_vec()
    }

    /// Every `data:` URL from a `ContentPart::ImageUrl` part across all captured
    /// model requests (C-ATTACH). Empty when no multimodal image part reached
    /// the model — either no image was attached, the model id wasn't
    /// vision-capable (see `RebornThreadBuilder::with_model_override`), or the
    /// attachment read port failed to land/read the bytes.
    pub(super) fn captured_image_data_urls(&self) -> Vec<String> {
        self.scripted_llm
            .captured_requests()
            .into_iter()
            .flatten()
            .flat_map(|message| message.content_parts)
            .filter_map(|part| match part {
                ironclaw_llm::ContentPart::ImageUrl { image_url } => Some(image_url.url),
                ironclaw_llm::ContentPart::Text { .. } => None,
            })
            .collect()
    }

    /// Snapshot of the captured **network** egress requests for this thread only
    /// (`[baseline_network_count..]` delta), in call order. Read by
    /// `assert_network_egress_header_contains` (assertions.rs) — the T0-SECRET-INJECT
    /// credential-injection assertion, which observes a different recorder lane
    /// than `captured_egress_requests` (see that assertion's docs for why).
    pub(super) fn captured_network_requests(&self) -> Vec<NetworkHttpRequest> {
        let mut all = self.capability_recorder.network_http_requests();
        all.split_off(self.baseline_network_count)
    }

    /// S1 seam: every request that reached the real-egress-pipeline's
    /// wire-level transport recorder (`.with_real_egress_pipeline()`), in call
    /// order. Empty (not baseline-sliced — this backend is single-shot, never
    /// group-shared) both when the harness didn't opt in and when real
    /// network-policy enforcement denied every call before the transport.
    pub(super) fn real_egress_transport_requests(&self) -> Vec<NetworkTransportRequest> {
        self.capability_recorder.real_egress_transport_requests()
    }

    /// Assert that a `builtin.shell` command was recorded by the inert process
    /// port and that the recorded command string contains `substr`. This proves
    /// the shell tool call was dispatched through the process port without
    /// spawning a real OS process (a safety invariant).
    ///
    /// Checks only the `[baseline_process_count..]` delta so a group thread
    /// never spuriously passes on a prior thread's entry (R2).
    pub async fn assert_shell_command_recorded(&self, substr: &str) -> HarnessResult<()> {
        let all = self.capability_recorder.recorded_process_commands();
        let commands = &all[self.baseline_process_count..];
        if commands.iter().any(|cmd| cmd.contains(substr)) {
            return Ok(());
        }
        let seen: Vec<&str> = commands.iter().map(|s| s.as_str()).collect();
        Err(format!("no recorded shell command containing {substr:?}; saw {seen:?}").into())
    }

    /// Asserts ≥1 shell command was dispatched through the inert recording
    /// process port, proving no real OS process was spawned. Passes when
    /// the `[baseline_process_count..]` delta is non-empty (the harness used
    /// the recording path, not the live-shell opt-in).
    ///
    /// Checks only the `[baseline_process_count..]` delta so a group thread
    /// never spuriously passes on a prior thread's entry (R2).
    pub async fn assert_shell_ran_through_inert_port(&self) -> HarnessResult<()> {
        let all = self.capability_recorder.recorded_process_commands();
        let commands = &all[self.baseline_process_count..];
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
    /// e.g. `"search"`) was invoked via the real MCP runtime.
    ///
    /// Internally maps `tool_name` → capability id `"mock-mcp.{tool_name}"` and
    /// delegates to `assert_tool_invoked`. The `"mock-mcp"` prefix matches the
    /// fixed provider id set by `with_mock_mcp`.
    pub async fn assert_mcp_tool_called(&self, tool_name: &str) -> HarnessResult<()> {
        self.assert_tool_invoked(&format!("{MOCK_MCP_PROVIDER_ID}.{tool_name}"))
            .await
    }

    /// Assert that the workspace file at `relative` (a path under the
    /// `/workspace` mount, e.g. `"approved.txt"`) exists on disk and its
    /// contents contain `expected`. Reads the REAL persisted file the gated
    /// capability wrote after approval — the genuine side effect — not a
    /// recorded result (a `builtin.write_file` result does not echo the written
    /// content). Only available on a host-runtime capability harness; returns
    /// `Err` for the Echo backend.
    pub async fn assert_workspace_file_contains(
        &self,
        relative: &str,
        expected: &str,
    ) -> HarnessResult<()> {
        let path = self
            .capability_recorder
            .workspace_file_path(relative)
            .ok_or("harness is not using host-runtime capabilities")?;
        let contents = std::fs::read_to_string(&path).map_err(|error| {
            format!("workspace file {relative:?} not readable at {path:?}: {error}")
        })?;
        if contents.contains(expected) {
            return Ok(());
        }
        Err(format!(
            "workspace file {relative:?} did not contain {expected:?} (actual length {} bytes)",
            contents.len()
        )
        .into())
    }

    /// Assert that no workspace file exists at `relative` — i.e. a gated capability
    /// that was denied never performed its write. The faithful negative
    /// side-effect check (the file on disk, not the absence of a recorded result).
    pub async fn assert_workspace_file_absent(&self, relative: &str) -> HarnessResult<()> {
        let path = self
            .capability_recorder
            .workspace_file_path(relative)
            .ok_or("harness is not using host-runtime capabilities")?;
        if path.exists() {
            return Err(format!(
                "workspace file {relative:?} exists at {path:?} but should not (write was denied)"
            )
            .into());
        }
        Ok(())
    }

    /// Snapshot of the recorded capability results (tool outputs) for this thread
    /// only (`[baseline_result_count..]` delta), in execution order. Read by
    /// `assert_tool_result_contains` in `assertions.rs`.
    pub(super) fn captured_capability_results(&self) -> Vec<RecordedCapabilityResult> {
        let all = self.capability_recorder.capability_results();
        all[self.baseline_result_count..].to_vec()
    }

    /// Shared poll loop for `wait_for_status`/`wait_for_terminal`; `decide`
    /// picks the stop condition so the deadline/interval have one home.
    /// `timeout_context` keeps each caller's timeout message distinct (e.g.
    /// `wait_for_status`'s byte-identical `"timed out waiting for {expected:?}"`).
    async fn poll_run_state_until(
        &self,
        run_id: TurnRunId,
        mut decide: impl FnMut(&TurnRunState) -> ControlFlow<HarnessResult<TurnRunState>>,
        timeout_context: &str,
    ) -> HarnessResult<TurnRunState> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let state = self
                .turn_store
                .get_run_state(GetRunStateRequest {
                    scope: self.turn_scope.clone(),
                    run_id,
                })
                .await?;
            if let ControlFlow::Break(outcome) = decide(&state) {
                return outcome;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for {timeout_context}; last status={:?} failure={:?}",
                    state.status, state.failure
                )
                .into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Poll the turn-state store until the run reaches `expected`, returning the
    /// matching `TurnRunState`. Fails fast if the run reaches a *different*
    /// terminal status first (terminal states are never left, so it can never
    /// reach `expected`). One loop, three callers: `submit_turn` waits on
    /// `Completed`, `submit_turn_until_blocked` on `BlockedApproval`, and the auth
    /// slice on `BlockedAuth`. Mirrors the binary-E2E harness's `wait_for_status`.
    pub async fn wait_for_status(
        &self,
        run_id: TurnRunId,
        expected: TurnStatus,
    ) -> HarnessResult<TurnRunState> {
        self.poll_run_state_until(
            run_id,
            |state| {
                if state.status == expected {
                    ControlFlow::Break(Ok(state.clone()))
                } else if state.status.is_terminal() {
                    ControlFlow::Break(Err(format!(
                        "expected {expected:?} but run reached terminal status {:?}; failure={:?}",
                        state.status, state.failure
                    )
                    .into()))
                } else {
                    ControlFlow::Continue(())
                }
            },
            &format!("{expected:?}"),
        )
        .await
    }

    /// Poll until ANY terminal status (#5466): unlike `wait_for_status`, does
    /// NOT fail fast on an unexpected terminal — caller branches on the result.
    pub async fn wait_for_terminal(&self, run_id: TurnRunId) -> HarnessResult<TurnRunState> {
        self.poll_run_state_until(
            run_id,
            |state| {
                if state.status.is_terminal() {
                    ControlFlow::Break(Ok(state.clone()))
                } else {
                    ControlFlow::Continue(())
                }
            },
            "terminal condition",
        )
        .await
    }

    /// Approve a blocked approval gate and resume the run (the user-approves path).
    /// Resolves the persisted approval request to an issued lease, then resumes the
    /// run so the originally-gated capability re-dispatches and the turn completes.
    ///
    /// Resumes with `ResumeTurnPrecondition::BlockedApprovalGate` — the same
    /// precondition `ApprovalInteractionService` uses for its production
    /// approval-resume path. That precondition is enforced server-side
    /// (`resume_turn_once` requires `record.status == BlockedApproval`), so a
    /// stale or wrong (non-approval) gate ref fails the resume with
    /// `TurnError::InvalidTransition` instead of silently resuming whatever
    /// gate class happens to be blocked.
    pub async fn approve_gate(&self, run_id: TurnRunId, gate_ref: &GateRef) -> HarnessResult<()> {
        self.capability_recorder
            .approve_local_dev_gate(gate_ref)
            .await?;
        self.resume_run(
            run_id,
            gate_ref.clone(),
            None,
            ResumeTurnPrecondition::BlockedApprovalGate,
        )
        .await
    }

    /// Test-support variant of [`approve_gate`](Self::approve_gate) for the
    /// stale-gate-ref-resume regression guard (C-DENYEDGE row 7): resolves the
    /// LOCAL-DEV approval using `real_gate_ref` (so the approval-store lookup
    /// succeeds) but issues the COORDINATOR resume with a DIFFERENT
    /// `stale_gate_ref`, reaching `resume_turn_once`'s gate-ref-mismatch check
    /// (`InvalidRequest`) — a path `approve_gate` itself can never reach.
    pub async fn approve_gate_with_stale_resume_ref(
        &self,
        run_id: TurnRunId,
        real_gate_ref: &GateRef,
        stale_gate_ref: &GateRef,
    ) -> HarnessResult<()> {
        self.capability_recorder
            .approve_local_dev_gate(real_gate_ref)
            .await?;
        self.resume_run(
            run_id,
            stale_gate_ref.clone(),
            None,
            ResumeTurnPrecondition::BlockedApprovalGate,
        )
        .await
    }

    /// Resume-only companion to
    /// [`approve_gate_with_stale_resume_ref`](Self::approve_gate_with_stale_resume_ref):
    /// issues the coordinator resume WITHOUT re-running the local-dev approval
    /// resolve step. Needed for a non-vacuity follow-up after a failed
    /// stale-ref resume: the record is already `Approved`, so re-calling
    /// `approve_gate` would hit a double-resolve `NotPending` error instead of
    /// completing the still-blocked run.
    pub async fn resume_gate(&self, run_id: TurnRunId, gate_ref: &GateRef) -> HarnessResult<()> {
        self.resume_run(
            run_id,
            gate_ref.clone(),
            None,
            ResumeTurnPrecondition::BlockedApprovalGate,
        )
        .await
    }

    /// Deny a blocked approval gate and resume the run (the user-declines path).
    /// Resolves the persisted request to `Denied` (no lease) and resumes with
    /// `GateResumeDisposition::Denied`, so the executor surfaces a non-retryable
    /// authorization failure to the model rather than re-dispatching the gate.
    ///
    /// See [`approve_gate`](Self::approve_gate) for why this resumes with
    /// `ResumeTurnPrecondition::BlockedApprovalGate`.
    pub async fn deny_gate(&self, run_id: TurnRunId, gate_ref: &GateRef) -> HarnessResult<()> {
        self.capability_recorder
            .deny_local_dev_gate(gate_ref)
            .await?;
        self.resume_run(
            run_id,
            gate_ref.clone(),
            Some(GateResumeDisposition::Denied),
            ResumeTurnPrecondition::BlockedApprovalGate,
        )
        .await
    }

    /// Deny a blocked AUTH gate and resume the run (user-declines path). Unlike
    /// [`deny_gate`](Self::deny_gate) (approval gates resolve a persisted
    /// request), auth gates have no such store entry, so this resumes directly
    /// with `GateResumeDisposition::Denied` — `short_circuit_denied_resume`
    /// then surfaces a model-visible gate-declined failure instead of
    /// re-dispatching (which would re-block on the missing credential forever).
    ///
    /// Resumes with the gate-class-specific `ResumeTurnPrecondition::BlockedAuthGate`
    /// (server-enforced: `resume_turn_once` requires `status == BlockedAuth`),
    /// same shape as `deny_gate`'s `BlockedApprovalGate`. A client-side
    /// `gate:auth-` prefix check adds cheap defense-in-depth on top.
    pub async fn deny_auth_gate(&self, run_id: TurnRunId, gate_ref: &GateRef) -> HarnessResult<()> {
        if !gate_ref.as_str().starts_with("gate:auth-") {
            return Err(format!("expected an auth gate ref, got {gate_ref:?}").into());
        }
        self.resume_run(
            run_id,
            gate_ref.clone(),
            Some(GateResumeDisposition::Denied),
            ResumeTurnPrecondition::BlockedAuthGate,
        )
        .await
    }

    /// Resolve a blocked AUTH gate the "user submitted credentials" way
    /// (C-JOURNEY convergence seam): seed a real GitHub credential account
    /// (`seed_github_credential_account`) so the parked capability's next
    /// credential-resolver lookup resolves, then resume with NO deny
    /// disposition so the parked `github.*` capability re-dispatches and the
    /// run completes.
    ///
    /// Only valid on a harness built via
    /// `HostRuntimeCapabilityHarness::file_and_github_auth_tools` — the
    /// credential-seeding path needs `build_reborn_services` product-auth
    /// wiring that `deny_auth_gate`'s sibling fixture (`live_auth_gate`, a
    /// lower-level build with a hardcoded resolver) does not have.
    pub async fn resolve_auth_gate(
        &self,
        run_id: TurnRunId,
        gate_ref: &GateRef,
    ) -> HarnessResult<()> {
        if !gate_ref.as_str().starts_with("gate:auth-") {
            return Err(format!("expected an auth gate ref, got {gate_ref:?}").into());
        }
        let harness = match &self._shared.capability {
            GroupCapability::HostRuntime(arc) => arc,
            GroupCapability::Recording => {
                return Err(
                    "no host-runtime capability backend to seed a github credential account".into(),
                );
            }
        };
        // Seed under THIS run's actual (tenant, user, agent, project) — the
        // resolver's `account_visible_from_runtime_scope` check matches all
        // four, so a differently-scoped seed would leave the run stuck at
        // `BlockedAuth` forever.
        let scope = self.run_resource_scope_for_user(self.binding.actor_user_id.clone());
        harness.seed_github_credential_account(&scope).await?;
        self.resume_run(
            run_id,
            gate_ref.clone(),
            None,
            ResumeTurnPrecondition::BlockedAuthGate,
        )
        .await
    }

    /// Seed a Configured credential account WITH real secret material for
    /// `provider` through the production manual-token flow, scoped so this
    /// group's CAPABILITY dispatch finds it: account selection matches all of
    /// `(tenant, user, agent, project)`, and the user must be the capability
    /// harness's dispatch user — which, on groups that do not align it to the
    /// binding subject, differs from this thread's binding actor.
    pub async fn seed_capability_credential_account(
        &self,
        provider: &str,
        label: &str,
        provider_scopes: &[&str],
    ) -> HarnessResult<()> {
        let harness = match &self._shared.capability {
            GroupCapability::HostRuntime(arc) => arc,
            GroupCapability::Recording => {
                return Err(
                    "no host-runtime capability backend to seed a credential account".into(),
                );
            }
        };
        let scope = self.run_resource_scope_for_user(harness.capability_user_id().clone());
        harness
            .seed_credential_account_with_material(&scope, provider, label, provider_scopes)
            .await
    }

    /// Flip every non-revoked credential account for `provider` under this
    /// group's capability dispatch scope to `Revoked` — modeling an external
    /// revocation of the user's grant (#5878 shape). Same scoping as
    /// [`Self::seed_capability_credential_account`]; errors if nothing was
    /// revoked so a vacuous revoke can't silently pass a re-auth assertion.
    pub async fn revoke_capability_credential_accounts(&self, provider: &str) -> HarnessResult<()> {
        let harness = match &self._shared.capability {
            GroupCapability::HostRuntime(arc) => arc,
            GroupCapability::Recording => {
                return Err(
                    "no host-runtime capability backend to revoke credential accounts".into(),
                );
            }
        };
        let scope = self.run_resource_scope_for_user(harness.capability_user_id().clone());
        let revoked = harness
            .revoke_credential_accounts_for_provider(&scope, provider)
            .await?;
        if revoked == 0 {
            return Err(format!(
                "no non-revoked credential account for provider {provider:?} was found to revoke"
            )
            .into());
        }
        Ok(())
    }

    /// This thread's run `(tenant, agent, project)` scope with `user_id` as
    /// the owner — the exact four fields dispatch-time credential-account
    /// selection matches. Which user is correct depends on the caller: the
    /// binding actor for user-aligned groups, the capability dispatch user
    /// otherwise.
    fn run_resource_scope_for_user(&self, user_id: UserId) -> ResourceScope {
        ResourceScope {
            tenant_id: self.turn_scope.tenant_id.clone(),
            user_id,
            agent_id: self.turn_scope.agent_id.clone(),
            project_id: self.turn_scope.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    /// Flip the per-`(tenant, user)` auto-approve toggle back ON for the run's
    /// capability scope via the real CAS-persisted `AutoApproveSettingStore` (the
    /// no-gate / approve-always arm: with auto-approve on, the same capability
    /// completes without a gate, and the flip persists across threads in the
    /// group because the store is shared).
    ///
    /// Scope = `_shared.auto_approve_scope()` — `(run tenant, capability user)`,
    /// the exact `(tenant, user)` the dispatch-time auto-approve check is keyed on
    /// (NOT the binding owner).
    pub async fn enable_auto_approve(&self) -> HarnessResult<()> {
        let scope = self
            ._shared
            .auto_approve_scope()
            .ok_or("group has no host-runtime capability backend for auto-approve")?;
        self.capability_recorder
            .enable_auto_approve_for(scope)
            .await
    }

    /// Flip the per-`(tenant, user)` auto-approve toggle OFF for the run's
    /// capability scope (the gate arm: with auto-approve disabled, an
    /// `Ask`-mode capability raises a real `BlockedApproval` gate instead of
    /// dispatching). Inverse of [`enable_auto_approve`](Self::enable_auto_approve);
    /// same `_shared.auto_approve_scope()` = `(run tenant, capability user)`.
    pub async fn disable_auto_approve(&self) -> HarnessResult<()> {
        let scope = self
            ._shared
            .auto_approve_scope()
            .ok_or("group has no host-runtime capability backend for auto-approve")?;
        self.capability_recorder
            .disable_auto_approve_for(scope)
            .await
    }

    async fn resume_run(
        &self,
        run_id: TurnRunId,
        gate_ref: GateRef,
        resume_disposition: Option<GateResumeDisposition>,
        precondition: ResumeTurnPrecondition,
    ) -> HarnessResult<()> {
        self.resume_run_in_scope_impl(
            self.turn_scope.clone(),
            run_id,
            gate_ref,
            resume_disposition,
            precondition,
        )
        .await
    }

    /// Shared implementation behind both `resume_run` (resumes in
    /// `self.turn_scope`) and `triggered_submit.rs`'s `resume_run_in_scope`
    /// (resumes in an explicit, possibly different, `TurnScope` for
    /// triggered/non-thread runs).
    pub(crate) async fn resume_run_in_scope_impl(
        &self,
        scope: TurnScope,
        run_id: TurnRunId,
        gate_ref: GateRef,
        resume_disposition: Option<GateResumeDisposition>,
        precondition: ResumeTurnPrecondition,
    ) -> HarnessResult<()> {
        // Key on `(run_id, gate_ref)`, NOT `run_id` alone: a C-JOURNEY chained
        // turn can resume the SAME run twice in a row (e.g. an approval gate
        // resolved, immediately followed by the auth gate the re-dispatched
        // capability then raises) — a `run_id`-only key collides with the
        // FIRST resume's idempotency key, so the coordinator treats the
        // second resume as a replay and returns the cached `Queued` response
        // without actually re-queuing any work. The gate_ref changes per gate
        // resolution (a fresh gate raised after a prior resume gets a new
        // ref), so including it keeps each DISTINCT gate resolution unique
        // while still deduping a genuine retry of the SAME resume call.
        let idempotency_key =
            IdempotencyKey::new(format!("resume-{run_id}-{}", gate_ref.as_str()))?;
        let response = self
            .coordinator
            .resume_turn(ResumeTurnRequest {
                scope,
                actor: TurnActor::new(self.binding.actor_user_id.clone()),
                run_id,
                gate_resolution_ref: gate_ref,
                precondition,
                source_binding_ref: SourceBindingRef::new("src:resume")?,
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:resume")?,
                idempotency_key,
                resume_disposition,
            })
            .await?;
        if response.status != TurnStatus::Queued {
            return Err(format!("expected resumed run to queue, got {:?}", response.status).into());
        }
        Ok(())
    }

    /// Request cancellation of an in-flight run (E-GATEWAY seam). Mirrors
    /// `resume_run`'s coordinator-call shape; drives the mid-turn cancel path so
    /// a parked model call can be cancelled and the run reaches `Cancelled`.
    pub async fn cancel_run(&self, run_id: TurnRunId) -> HarnessResult<CancelRunResponse> {
        self.coordinator
            .cancel_run(CancelRunRequest {
                scope: self.turn_scope.clone(),
                actor: TurnActor::new(self.binding.actor_user_id.clone()),
                run_id,
                reason: SanitizedCancelReason::UserRequested,
                idempotency_key: IdempotencyKey::new(format!("cancel-{run_id}"))?,
            })
            .await
            .map_err(Into::into)
    }
}

// Scheduler shutdown: the group's `TurnRunSchedulerHandle` lives on
// `GroupSharedStorage` (not on any per-thread `RebornIntegrationHarness`), so
// no `Drop` impl is needed here. `TurnRunSchedulerHandle::drop` synchronously
// cancels the scheduler loop when the last `Arc<GroupSharedStorage>` (held by
// every harness's `_shared` field) goes away.

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

/// Open a **genuinely fresh** `CompositeRootFilesystem` connection to the
/// on-disk LibSql file at `db_path`, independent of any live composite over
/// the same file. Shared by every "survives an independent reopen" assertion
/// (`assert_reply_persists_after_reopen`, `assert_gate_survives_reopen`) —
/// each builds its own higher-level store (thread service, turn-state store)
/// over the fresh composite this returns.
async fn reopen_fresh_libsql_composite(
    db_path: &Path,
) -> HarnessResult<Arc<CompositeRootFilesystem>> {
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
    Ok(Arc::new(fresh_composite))
}

/// Build the one `CompositeRootFilesystem` for a harness, selecting the durable
/// backend by `mode`. `dir` is used only for `LibSql` (the SQLite file is
/// created there); `InMemory` ignores it.
///
/// Returns the composite alongside the on-disk SQLite path for `LibSql`
/// (`None` for `InMemory`) — stored on the harness so
/// `assert_reply_persists_after_reopen` can open a genuinely fresh connection.
pub(crate) async fn build_storage_composite(
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
pub(crate) fn scoped_turns_fs_composite(
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
pub(crate) fn apply_hermetic_env() {
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
            // Loop-level counterpart of LLM_MAX_RETRIES=0: production rides
            // out provider outages for minutes (deep availability retries with
            // long backoff), which would stall any scenario that deliberately
            // scripts a model failure. One attempt keeps failure paths fast.
            std::env::set_var("IRONCLAW_REBORN_MODEL_AVAILABILITY_RETRY_ATTEMPTS", "1");
            std::env::remove_var("NEARAI_CHEAP_MODEL");
            std::env::remove_var("NEARAI_FALLBACK_MODEL");
            std::env::remove_var("LLM_CHEAP_MODEL");
            std::env::remove_var("LLM_CIRCUIT_BREAKER_THRESHOLD");
            std::env::remove_var("CIRCUIT_BREAKER_THRESHOLD");
            std::env::remove_var("LLM_RESPONSE_CACHE_ENABLED");
            std::env::remove_var("RESPONSE_CACHE_ENABLED");
            std::env::remove_var("NEARAI_SESSION_TOKEN");
            // No integration test should inherit the ambient tool-disclosure
            // knob: `ToolDisclosureMode::from_env()` resolution is opt-in per
            // test via `.with_tool_disclosure_bridged()`/`.with_tool_disclosure_off()`,
            // never ambient (see `tool_disclosure.rs`'s negative control).
            std::env::remove_var(ironclaw_runner::runtime::REBORN_TOOL_DISCLOSURE_ENV);
        }
    });
}

/// Assemble a `ResolveBindingRequest` from a verified inbound envelope. This
/// harness only submits DirectChat turns, so the route kind is `Direct`.
pub(crate) fn binding_request(
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

pub(crate) fn thread_scope_from_binding(binding: &ResolvedBinding) -> HarnessResult<ThreadScope> {
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

// The shared planned-runtime assembly (`RebornIntegrationGroupBuilder::into_group`)
// and per-thread harness assembly (`RebornThreadBuilder::build`) live in
// `group.rs` (imported above) — that module owns `GroupSharedStorage` and the
// capability mode types.
