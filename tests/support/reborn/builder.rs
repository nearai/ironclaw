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
    MountAlias, MountGrant, MountPermissions, MountView, RuntimeHttpEgressRequest, UserId,
    VirtualPath,
};
use ironclaw_product_adapters::{ProductInboundAck, ProductTriggerReason, ProductWorkflow};
use ironclaw_product_workflow::{
    ConversationBindingService, DefaultProductWorkflow, ProductConversationRouteKind,
    ResolveBindingRequest, ResolvedBinding,
};
use ironclaw_threads::ThreadScope;
use ironclaw_turns::{
    FilesystemTurnStateStore, GateRef, GateResumeDisposition, GetRunStateRequest, IdempotencyKey,
    ReplyTargetBindingRef, ResumeTurnPrecondition, ResumeTurnRequest, SourceBindingRef, TurnActor,
    TurnCoordinator, TurnRunId, TurnRunState, TurnScope, TurnStateStore, TurnStatus,
};

use super::group::{GroupCapability, GroupSharedStorage, assemble_thread_runtime};
use super::harness::{
    HarnessCapabilityRecorder, HarnessTurnBackend, HostRuntimeCapabilityHarness,
    RecordedCapabilityResult, test_product_scope,
};
use super::http_matcher::ScriptedHttpResponse;
use super::reply::RebornScriptedReply;
use super::session_thread::RebornThreadHarness;
use super::test_adapter::{RebornTestIngress, RebornTestProductAdapter};

type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The actor/user that submits turns. Reused at binding-probe time and submit
/// time so both resolve to the same conversation binding (and thread).
pub(crate) const HARNESS_ACTOR_ID: &str = "host-user";
/// Model profile the planned runtime requests; the gateway policy permits it.
pub(crate) const INTERACTIVE_MODEL_PROFILE: &str = "interactive_model";

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
    ///
    /// Constructs a one-thread `GroupSharedStorage` (matching this builder's
    /// capability/storage/live_shell/keyed_http_responses selections) and
    /// delegates to `assemble_thread_runtime`. Behavior is byte-identical to
    /// the old inline build — existing tests are unaffected (R1/R5).
    pub async fn build(self) -> HarnessResult<RebornIntegrationHarness> {
        apply_hermetic_env();

        // --- capability backend → GroupCapability --------------------------
        // Echo by default (records, executes nothing — a text reply invokes no
        // tool). Builtin/MCP swap in the real first-party runtime. (Live approval
        // stores are a group-only backend; see `RebornIntegrationGroup::live_approvals`.)
        let group_capability = match self.capability {
            RebornCapabilityBackend::Echo => GroupCapability::Recording,
            RebornCapabilityBackend::BuiltinHttpTools => {
                // Slice 5: `.with_live_shell()` opts into the real LocalHostProcessPort;
                // the default recording path uses the inert RecordingProcessPort.
                let host_runtime = if self.live_shell {
                    HostRuntimeCapabilityHarness::core_builtin_tools_with_live_shell().await?
                } else {
                    HostRuntimeCapabilityHarness::core_builtin_tools().await?
                };
                host_runtime.install_http_responses(self.keyed_http_responses)?;
                GroupCapability::HostRuntime(Arc::new(host_runtime))
            }
            RebornCapabilityBackend::MockMcp { mcp_url } => {
                // Slice 6: real MCP runtime backed by the loopback mock server.
                let host_runtime = HostRuntimeCapabilityHarness::mock_mcp_tools(
                    &mcp_url,
                    MOCK_MCP_PROVIDER_ID,
                    &format!("{MOCK_MCP_PROVIDER_ID}.search"),
                )
                .await?;
                GroupCapability::HostRuntime(Arc::new(host_runtime))
            }
        };

        // --- product workflow + storage ------------------------------------
        let scope = test_product_scope(
            "tenant-itest",
            "host-user",
            "agent-itest",
            Some("project-itest"),
        );
        let product_harness =
            super::product_workflow::RebornProductWorkflowHarness::filesystem_temp(scope)?;
        let turn_root = Arc::new(tempfile::tempdir()?);
        let (composite, libsql_db_path) =
            build_storage_composite(self.storage, turn_root.path()).await?;

        let shared = Arc::new(GroupSharedStorage {
            composite,
            libsql_db_path,
            turn_root,
            product_harness,
            capability: group_capability,
        });
        let capability_mode = shared.capability.mode();

        assemble_thread_runtime(shared, &self.conversation_id, self.replies, capability_mode).await
    }
}

/// Full-stack Reborn integration harness with a scripted raw provider beneath
/// the real decorator chain. See module docs.
pub struct RebornIntegrationHarness {
    pub(crate) ingress: RebornTestIngress,
    pub(crate) workflow: DefaultProductWorkflow,
    pub(crate) conversation_id: String,
    pub(crate) binding: ResolvedBinding,
    pub(crate) turn_scope: TurnScope,
    pub(crate) turn_store: Arc<FilesystemTurnStateStore<HarnessTurnBackend>>,
    pub(crate) thread_harness: RebornThreadHarness<CompositeRootFilesystem>,
    /// Turn coordinator, used to resume a `BlockedApproval`/`BlockedAuth` run
    /// after `approve_gate`/`deny_gate` resolves the gate. Mirrors the binary-E2E
    /// harness's `resume_with_gate` path.
    pub(crate) coordinator: Arc<dyn TurnCoordinator>,
    pub(crate) scheduler_handle: Option<ironclaw_host_runtime::TurnRunSchedulerHandle>,
    pub(crate) event_seq: AtomicU64,
    pub(crate) capability_recorder: HarnessCapabilityRecorder,
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
        let run_id = self.submit_turn_async(text).await?;
        self.wait_for_status(run_id, TurnStatus::Completed).await?;
        Ok(run_id)
    }

    /// Submit a user turn and return its run id **without** waiting for any status
    /// — the caller drives the wait (`wait_for_status`). Used by approval/auth flows
    /// where the turn blocks on a gate rather than completing.
    pub async fn submit_turn_async(&self, text: &str) -> HarnessResult<TurnRunId> {
        let event_id = format!("evt-{}", self.event_seq.fetch_add(1, Ordering::Relaxed));
        let envelope = self.ingress.verified_text_envelope_with_trigger(
            &event_id,
            HARNESS_ACTOR_ID,
            &self.conversation_id,
            text,
            ProductTriggerReason::DirectChat,
        )?;
        let ack = self.workflow.accept_inbound(envelope).await?;
        match ack {
            ProductInboundAck::Accepted {
                submitted_run_id, ..
            } => Ok(submitted_run_id),
            other => Err(format!("expected accepted inbound ack, got {other:?}").into()),
        }
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
        if let Some(db_path) = &self._shared.libsql_db_path {
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

    /// Assert that a `builtin.shell` command was recorded by the inert process
    /// port and that the recorded command string contains `substr`. This proves
    /// the shell tool call was dispatched through the process port without
    /// spawning a real OS process (slice 5 safety invariant).
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
    /// e.g. `"search"`) was invoked via the real MCP runtime (slice 6).
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
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let state = self
                .turn_store
                .get_run_state(GetRunStateRequest {
                    scope: self.turn_scope.clone(),
                    run_id,
                })
                .await?;
            if state.status == expected {
                return Ok(state);
            }
            if state.status.is_terminal() {
                return Err(format!(
                    "expected {expected:?} but run reached terminal status {:?}; failure={:?}",
                    state.status, state.failure
                )
                .into());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for {expected:?}; last status={:?} failure={:?}",
                    state.status, state.failure
                )
                .into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Approve a blocked approval gate and resume the run (the user-approves path).
    /// Resolves the persisted approval request to an issued lease, then resumes the
    /// run so the originally-gated capability re-dispatches and the turn completes.
    pub async fn approve_gate(&self, run_id: TurnRunId, gate_ref: &GateRef) -> HarnessResult<()> {
        self.capability_recorder
            .approve_local_dev_gate(gate_ref)
            .await?;
        self.resume_run(run_id, gate_ref.clone(), None).await
    }

    /// Deny a blocked approval gate and resume the run (the user-declines path).
    /// Resolves the persisted request to `Denied` (no lease) and resumes with
    /// `GateResumeDisposition::Denied`, so the executor surfaces a non-retryable
    /// authorization failure to the model rather than re-dispatching the gate.
    pub async fn deny_gate(&self, run_id: TurnRunId, gate_ref: &GateRef) -> HarnessResult<()> {
        self.capability_recorder
            .deny_local_dev_gate(gate_ref)
            .await?;
        self.resume_run(
            run_id,
            gate_ref.clone(),
            Some(GateResumeDisposition::Denied),
        )
        .await
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

    async fn resume_run(
        &self,
        run_id: TurnRunId,
        gate_ref: GateRef,
        resume_disposition: Option<GateResumeDisposition>,
    ) -> HarnessResult<()> {
        let response = self
            .coordinator
            .resume_turn(ResumeTurnRequest {
                scope: self.turn_scope.clone(),
                actor: TurnActor::new(self.binding.actor_user_id.clone()),
                run_id,
                gate_resolution_ref: gate_ref,
                precondition: ResumeTurnPrecondition::AnyBlockedGate,
                source_binding_ref: SourceBindingRef::new("src:resume")?,
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:resume")?,
                idempotency_key: IdempotencyKey::new(format!("resume-{run_id}"))?,
                resume_disposition,
            })
            .await?;
        if response.status != TurnStatus::Queued {
            return Err(format!("expected resumed run to queue, got {:?}", response.status).into());
        }
        Ok(())
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

/// Resolve the canonical subject `UserId` that turns submitted under
/// [`HARNESS_ACTOR_ID`] run as. Product binding resolution maps the external
/// actor (`"host-user"`) to a hashed canonical `UserId`; the turn scope owner,
/// the capability dispatch scope, the auto-approve key, and the approval-gate
/// evidence lookup must all agree on THAT user. The group sets its capability
/// harness to execute under this user (via `with_user_id`) so a real approval
/// gate is persisted and verified under one consistent `(tenant, user)` —
/// matching production, where the run owner is the capability user. The actor →
/// canonical mapping is deterministic, so the throwaway probe binding here yields
/// the same user every thread will resolve to.
pub(crate) async fn resolve_canonical_subject_user(
    product_harness: &super::product_workflow::RebornProductWorkflowHarness,
) -> HarnessResult<UserId> {
    let adapter = RebornTestProductAdapter::new("reborn-itest", "itest-install")?;
    let ingress = RebornTestIngress::new(adapter);
    let probe = ingress.verified_text_envelope_with_trigger(
        "subject-user-probe",
        HARNESS_ACTOR_ID,
        "conv-subject-user-probe",
        "hi",
        ProductTriggerReason::DirectChat,
    )?;
    let binding = product_harness
        .binding_service()?
        .resolve_binding(binding_request(&probe))
        .await?;
    binding
        .subject_user_id
        .ok_or_else(|| "resolved binding missing subject user id".into())
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

// `assemble_thread_runtime` lives in `group.rs` (imported above) — that
// module owns `GroupSharedStorage` and the capability mode types.
