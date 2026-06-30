//! Shared-persistence group infrastructure for Reborn integration tests.
//!
//! A **group** owns shared storage (composite filesystem, product workflow
//! harness, capability backend) exactly once, and each
//! [`RebornIntegrationGroup::thread`] call builds a per-thread turn runtime
//! over those shared pieces. Within one group, state written by thread A is
//! visible to thread B — the key e2e persistence contract.
//!
//! Separate groups are separate test binaries and run in parallel, fully
//! isolated. A single-shot [`RebornIntegrationHarness::test_default()`] is a
//! degenerate one-thread group (its own storage, baseline = 0), so all
//! existing tests are byte-identical after this refactor.
//!
//! ## Group test binary layout
//!
//! ```text
//! tests/reborn_group_approvals/
//!     main.rs                         // one #[tokio::test], drives scenarios in order
//!     scenario_gate_then_resolve.rs   // pub async fn run(g:&RebornIntegrationGroup)->HarnessResult<()>
//!     scenario_approve_always_persists.rs
//! ```
//!
//! ### Why one sequential `#[tokio::test]`, not N separate `#[test]` fns
//!
//! Cargo does not guarantee order or share an instance between multiple
//! `#[test]` fns in one binary, and `serial_test` + global statics are
//! fragile.  One orchestrating fn is the only design that gives deterministic
//! ordering over a shared group instance without fragile machinery.
//!
//! ### Scenario shape
//!
//! ```rust,no_run
//! // scenario_approve_always_persists.rs
//! use crate::reborn_support::group::HarnessResult;
//! pub async fn run(g: &super::reborn_support::group::RebornIntegrationGroup)
//!     -> HarnessResult<()>
//! {
//!     // ... build thread, submit turn, assert ...
//!     Ok(())
//! }
//! ```
//!
//! Use `?` for *dependent* scenarios (failure stops the driver) and
//! `report.record(name, scenario::run(&g).await)` for *independent* ones
//! (failure recorded, others continue).
//!
//! ### Subdir module paths
//!
//! Each group `main.rs` MUST declare BOTH `#[path]` overrides, each with
//! `#[allow(dead_code)]`:
//!
//! ```rust,no_run
//! #[allow(dead_code)] #[path = "../support/reborn/mod.rs"] mod reborn_support;
//! #[allow(dead_code)] #[path = "../support/mod.rs"] mod support;
//! ```
//!
//! Bare `mod support;` resolves to `tests/reborn_group_*/support.rs` (which
//! does not exist) and fails to compile.
//!
//! ### Two composites — use the right one
//!
//! - [`RebornIntegrationGroup::turn_composite`]: thread/turn history read-back.
//! - [`RebornIntegrationGroup::capability_harness`]: capability stores
//!   (memory, projects, extensions, secrets, approval/auto-approve).
//!
//! Do NOT read memory or approval state from `turn_composite()` — the
//! host-runtime capability stores live in a **separate** filesystem inside
//! the `HostRuntimeCapabilityHarness`, not in the integration composite.

// Shared by all group test binaries; symbols read as dead when a binary
// does not exercise every variant.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use ironclaw_filesystem::CompositeRootFilesystem;
use ironclaw_host_api::ResourceScope;
use ironclaw_llm::testing::provider_chain_over;
use ironclaw_llm::{LlmProvider, SessionConfig, create_session_manager};
use ironclaw_loop_support::{
    EmptyUserProfileSource, HostManagedModelGateway, JsonSpawnSubagentInputCodec,
    SubagentSpawnLimits,
};
use ironclaw_product_adapters::ProductTriggerReason;
use ironclaw_product_workflow::{
    ConversationBindingService, DefaultInboundTurnService, DefaultProductWorkflow,
    IdempotencyLedger, InboundTurnService,
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
use ironclaw_threads::SessionThreadService;
use ironclaw_turns::run_profile::{InMemoryLoopHostMilestoneSink, ModelProfileId};
use ironclaw_turns::{
    FilesystemTurnStateStore, InMemoryCheckpointStateStore, LoopCheckpointStore, TurnScope,
    TurnStateStore,
};

use super::builder::{
    HARNESS_ACTOR_ID, INTERACTIVE_MODEL_PROFILE, RebornIntegrationHarness, StorageMode,
    apply_hermetic_env, binding_request, build_storage_composite, resolve_canonical_subject_user,
    scoped_turns_fs_composite, thread_scope_from_binding,
};
use super::harness::{
    EmptyIdentityContextSource, HarnessCapabilityMode, HarnessTurnBackend,
    HostRuntimeCapabilityHarness, RecordingTestCapabilityPort, test_product_scope,
};
use super::product_workflow::RebornProductWorkflowHarness;
use super::reply::RebornScriptedReply;
use super::scripted_provider::{SCRIPTED_MODEL_NAME, scripted_trace_llm};
use super::session_thread::RebornThreadHarness;
use super::test_adapter::{RebornTestIngress, RebornTestProductAdapter};

/// Convenience alias matching `builder.rs` and `harness.rs`.
pub type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

// ---------------------------------------------------------------------------
// GroupSharedStorage
// ---------------------------------------------------------------------------

/// All resources shared across every thread in one `RebornIntegrationGroup`.
///
/// Owned by `Arc<GroupSharedStorage>` so harnesses can outlive the group's
/// stack frame (R6: `RebornIntegrationHarness` is `'static`).
pub(crate) struct GroupSharedStorage {
    /// Thread history + turn state composite, shared across all threads.
    pub(crate) composite: Arc<CompositeRootFilesystem>,
    /// Path to the on-disk SQLite file for `StorageMode::LibSql`; `None` for
    /// `StorageMode::InMemory`. Used by `assert_reply_persists_after_reopen`.
    pub(crate) libsql_db_path: Option<PathBuf>,
    /// Durable root TempDir: keeps the composite's on-disk files alive for
    /// the group's lifetime. `Drop` deletes the directory (req 3).
    pub(crate) turn_root: Arc<tempfile::TempDir>,
    /// Product-workflow harness (binding service + idempotency ledger).
    /// Shared so all threads resolve bindings within the same product context.
    /// `product_harness.scope` is the single-source `ResourceScope` (R5).
    pub(crate) product_harness: RebornProductWorkflowHarness,
    /// Capability backend. Groups use `HostRuntime`; the degenerate single-shot
    /// path may use `Recording`.
    pub(crate) capability: GroupCapability,
}

impl GroupSharedStorage {
    /// The `(tenant, user)` scope the dispatch-time auto-approve check is keyed
    /// on for this group's capability backend: the run tenant (from the product
    /// harness scope) combined with the user the capability harness executes its
    /// first-party tools under (NOT the binding owner — see
    /// `HostRuntimeCapabilityHarness::user_id`). Used to disable auto-approve so
    /// gates fire, and to re-enable it for the no-gate / approve-always arm.
    /// `None` for the Echo backend (no approval stores).
    pub(crate) fn auto_approve_scope(&self) -> Option<ResourceScope> {
        match &self.capability {
            GroupCapability::HostRuntime(arc) => {
                let mut scope = self.product_harness.scope.clone();
                scope.user_id = arc.user_id().clone();
                Some(scope)
            }
            GroupCapability::Recording => None,
        }
    }
}

// ---------------------------------------------------------------------------
// GroupCapability
// ---------------------------------------------------------------------------

/// Shared capability backend for a group. Groups always use `HostRuntime`
/// (sharing the approval/memory/credential stores across threads). `Recording`
/// is the single-shot echo path for text-only turns.
pub(crate) enum GroupCapability {
    /// Echo recorder — records invocations, executes nothing. Default for a
    /// text-only single-shot harness; no stores to share.
    Recording,
    /// Real first-party or MCP host runtime, shared across all threads.
    /// All approval/auto-approve/credential/memory state is common because the
    /// `Arc` is cloned per thread.
    HostRuntime(Arc<HostRuntimeCapabilityHarness>),
}

impl GroupCapability {
    /// Return a fresh `HarnessCapabilityMode` for one thread.
    ///
    /// `Recording` creates a fresh echo port each call (ports are consumed by
    /// `into_parts`). `HostRuntime` clones the `Arc` — N threads share the
    /// same underlying harness and all its stores.
    pub(crate) fn mode(&self) -> HarnessCapabilityMode {
        match self {
            Self::Recording => {
                HarnessCapabilityMode::Recording(RecordingTestCapabilityPort::echo())
            }
            Self::HostRuntime(arc) => HarnessCapabilityMode::HostRuntime(Arc::clone(arc)),
        }
    }
}

// ---------------------------------------------------------------------------
// RebornIntegrationGroup
// ---------------------------------------------------------------------------

/// Shared-storage group for cross-thread persistence tests.
///
/// Owns one `Arc<GroupSharedStorage>` covering the composite filesystem,
/// product workflow, and capability backend. Each call to
/// [`thread`](Self::thread) builds a fresh per-thread turn runtime over
/// those shared pieces so state written by thread A is visible to thread B.
///
/// Construct with [`live_approvals`](Self::live_approvals),
/// [`builtin_tools`](Self::builtin_tools), or
/// [`extension_lifecycle`](Self::extension_lifecycle), or via
/// [`builder`](Self::builder) for custom storage mode.
pub struct RebornIntegrationGroup {
    pub(crate) shared: Arc<GroupSharedStorage>,
}

impl RebornIntegrationGroup {
    /// Group with real file-tool approval stores (write_file/read_file at
    /// `PermissionMode::Ask`). Auto-approve is disabled for the group scope at
    /// construction so gated tool calls raise real `BlockedApproval` gates.
    /// Resolve with `approve_gate`/`deny_gate` per thread; re-enable with
    /// `enable_auto_approve` for the no-gate arm.
    pub async fn live_approvals() -> HarnessResult<Self> {
        Self::builder().live_approvals().await
    }

    /// Group with core built-in tools (memory/http/echo/time/json/shell).
    /// Auto-approve is enabled for all capability ids in the group scope.
    pub async fn builtin_tools() -> HarnessResult<Self> {
        Self::builder().builtin_tools().await
    }

    /// Group with extension-lifecycle tools
    /// (extension_search/install/activate/remove). Auto-approve is enabled;
    /// registry credentials are seeded.
    pub async fn extension_lifecycle() -> HarnessResult<Self> {
        Self::builder().extension_lifecycle().await
    }

    /// Builder for advanced configuration (e.g. `StorageMode::LibSql`).
    /// Defaults to `StorageMode::InMemory`.
    pub fn builder() -> RebornIntegrationGroupBuilder {
        RebornIntegrationGroupBuilder {
            storage: StorageMode::InMemory,
        }
    }

    /// Create a per-thread runtime builder for `conversation_id`.
    ///
    /// Each call gets a distinct binding/thread_id/turn_scope over the
    /// **shared** composite and capability backend. Build with
    /// `.script([...]).build().await`.
    pub fn thread(&self, conversation_id: impl Into<String>) -> RebornThreadBuilder<'_> {
        RebornThreadBuilder {
            group: self,
            conversation_id: conversation_id.into(),
            replies: Vec::new(),
        }
    }

    /// The thread/turn `CompositeRootFilesystem` shared across all threads.
    ///
    /// Use this (not `capability_harness()`) for thread-history and turn-state
    /// read-back — the host-runtime capability stores (memory, extensions,
    /// approval) live in a **separate** filesystem inside
    /// `Arc<HostRuntimeCapabilityHarness>`.
    pub fn turn_composite(&self) -> &Arc<CompositeRootFilesystem> {
        &self.shared.composite
    }

    /// The shared `HostRuntimeCapabilityHarness` for this group, if the group
    /// uses a host-runtime capability backend. Returns `None` for the Echo
    /// (text-only, single-shot) backend.
    ///
    /// Use this (not `turn_composite()`) to access capability stores: memory,
    /// projects, extensions, secrets, approval/auto-approve.
    pub fn capability_harness(&self) -> Option<&Arc<HostRuntimeCapabilityHarness>> {
        match &self.shared.capability {
            GroupCapability::HostRuntime(arc) => Some(arc),
            GroupCapability::Recording => None,
        }
    }
}

// ---------------------------------------------------------------------------
// RebornIntegrationGroupBuilder
// ---------------------------------------------------------------------------

/// Shared base data produced by [`RebornIntegrationGroupBuilder::build_base`].
///
/// Replaces the 4-tuple `(RebornProductWorkflowHarness, Arc<CompositeRootFilesystem>,
/// Option<PathBuf>, Arc<TempDir>)` so each constructor can name fields rather than
/// position-destructure a tuple.
struct GroupBaseData {
    product_harness: RebornProductWorkflowHarness,
    composite: Arc<CompositeRootFilesystem>,
    libsql_db_path: Option<PathBuf>,
    turn_root: Arc<tempfile::TempDir>,
}

/// Builder for `RebornIntegrationGroup` with optional storage mode selection.
/// Obtain via [`RebornIntegrationGroup::builder`]; defaults to
/// `StorageMode::InMemory`.
pub struct RebornIntegrationGroupBuilder {
    storage: StorageMode,
}

impl RebornIntegrationGroupBuilder {
    /// Select the durable storage backend (default: `StorageMode::InMemory`).
    /// Use `StorageMode::LibSql` to exercise on-disk durability across
    /// `assert_reply_persists_after_reopen`.
    pub fn storage(mut self, mode: StorageMode) -> Self {
        self.storage = mode;
        self
    }

    /// Shared setup for every group constructor: hermetic env, the product
    /// workflow harness over the fixed itest scope, the per-group `TempDir`, and
    /// the thread/turn composite. Returns [`GroupBaseData`] so each constructor
    /// names the fields it needs — the fixed test-scope strings live HERE only.
    async fn build_base(&self) -> HarnessResult<GroupBaseData> {
        apply_hermetic_env();
        let scope = test_product_scope(
            "tenant-itest",
            "host-user",
            "agent-itest",
            Some("project-itest"),
        );
        let product_harness = RebornProductWorkflowHarness::filesystem_temp(scope)?;
        let turn_root = Arc::new(tempfile::tempdir()?);
        let (composite, libsql_db_path) =
            build_storage_composite(self.storage, turn_root.path()).await?;
        Ok(GroupBaseData {
            product_harness,
            composite,
            libsql_db_path,
            turn_root,
        })
    }

    fn into_group(
        self,
        base: GroupBaseData,
        capability: GroupCapability,
    ) -> RebornIntegrationGroup {
        RebornIntegrationGroup {
            shared: Arc::new(GroupSharedStorage {
                composite: base.composite,
                libsql_db_path: base.libsql_db_path,
                turn_root: base.turn_root,
                product_harness: base.product_harness,
                capability,
            }),
        }
    }

    /// Build a live-approvals group. See [`RebornIntegrationGroup::live_approvals`].
    pub async fn live_approvals(self) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        // Execute first-party tools under the run's CANONICAL binding subject
        // user (the hashed `UserId` the actor `host-user` resolves to), not the
        // constructor's fixed test user, so capability dispatch, approval
        // persistence, auto-approve keying, and gate-evidence lookup all share the
        // run's `(tenant, user)` — matching production.
        let subject_user = resolve_canonical_subject_user(&base.product_harness).await?;
        let host_runtime = HostRuntimeCapabilityHarness::file_tools_requiring_approval()
            .await?
            .with_user_id(subject_user);
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        let group = self.into_group(base, capability);
        // Disable auto-approve once at build time so every thread in this group
        // faces real approval gates. The dispatch-time check is keyed on the
        // capability harness's executor user (NOT the binding owner), so target
        // `auto_approve_scope()` — `(run tenant, capability user)`.
        // `live_approvals` always constructs `GroupCapability::HostRuntime`, so
        // both `auto_approve_scope()` and `capability_harness()` are guaranteed
        // `Some` — use `expect` rather than a redundant `if let`.
        let scope = group
            .shared
            .auto_approve_scope()
            .expect("live_approvals always uses HostRuntime; scope is always Some");
        let arc = group
            .capability_harness()
            .expect("live_approvals always uses HostRuntime");
        arc.disable_global_auto_approve(scope).await?;
        Ok(group)
    }

    /// Build a core built-in tools group. See [`RebornIntegrationGroup::builtin_tools`].
    pub async fn builtin_tools(self) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        let host_runtime = HostRuntimeCapabilityHarness::core_builtin_tools().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        Ok(self.into_group(base, capability))
    }

    /// Build an extension-lifecycle group. See [`RebornIntegrationGroup::extension_lifecycle`].
    pub async fn extension_lifecycle(self) -> HarnessResult<RebornIntegrationGroup> {
        let base = self.build_base().await?;
        let host_runtime = HostRuntimeCapabilityHarness::extension_lifecycle_tools().await?;
        let capability = GroupCapability::HostRuntime(Arc::new(host_runtime));
        Ok(self.into_group(base, capability))
    }
}

// ---------------------------------------------------------------------------
// RebornThreadBuilder
// ---------------------------------------------------------------------------

/// Per-thread runtime builder for a `RebornIntegrationGroup`.
///
/// The builder borrows the group for its own lifetime (R6). Calling `build()`
/// Arc-clones all shared fields from `GroupSharedStorage` into the returned
/// `RebornIntegrationHarness`, which is `'static` and independent of the
/// group's stack frame. Two harnesses may therefore coexist, though sequential
/// drop is the intended usage pattern (each prior harness drops before the
/// next thread builds, satisfying turn-scheduler exclusivity per thread scope).
pub struct RebornThreadBuilder<'g> {
    group: &'g RebornIntegrationGroup,
    conversation_id: String,
    replies: Vec<RebornScriptedReply>,
}

impl<'g> RebornThreadBuilder<'g> {
    /// Set the scripted model replies for this thread (consumed in order at the
    /// raw-provider seam, one per model turn).
    pub fn script(mut self, replies: impl IntoIterator<Item = RebornScriptedReply>) -> Self {
        self.replies = replies.into_iter().collect();
        self
    }

    /// Build the per-thread `RebornIntegrationHarness` over the group's shared
    /// storage.
    ///
    /// Arc-clones every shared field from `GroupSharedStorage` so the returned
    /// harness is `'static` (does not borrow `'g`). Delegates to
    /// `assemble_thread_runtime` (defined below in this module).
    pub async fn build(self) -> HarnessResult<RebornIntegrationHarness> {
        let capability_mode = self.group.shared.capability.mode();
        assemble_thread_runtime(
            Arc::clone(&self.group.shared),
            &self.conversation_id,
            self.replies,
            capability_mode,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// ScenarioReport
// ---------------------------------------------------------------------------

/// Collects independent scenario outcomes for a `RebornIntegrationGroup`
/// driver.
///
/// Intentionally minimal — for richer per-scenario data, enrich the scenario
/// fn's return type. Lives in `group.rs` (R7).
///
/// ```rust,no_run
/// let mut report = ScenarioReport::new();
/// report.record("gate_then_resolve", scenario_gate_then_resolve::run(&g).await);
/// report.record("approve_always_persists", scenario_approve_always_persists::run(&g).await);
/// report.assert_all_passed();
/// ```
pub struct ScenarioReport(Vec<(String, HarnessResult<()>)>);

impl ScenarioReport {
    /// Create an empty report.
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Record a scenario result without stopping the driver. Use `?` for
    /// dependent scenarios that must pass before subsequent ones run.
    pub fn record(&mut self, name: &str, result: HarnessResult<()>) {
        self.0.push((name.to_owned(), result));
    }

    /// Assert every recorded scenario passed; panics listing all failures.
    pub fn assert_all_passed(self) {
        let failures: Vec<String> = self
            .0
            .into_iter()
            .filter_map(|(name, result)| result.err().map(|e| format!("  {name}: {e}")))
            .collect();
        if !failures.is_empty() {
            panic!(
                "{} scenario(s) failed:\n{}",
                failures.len(),
                failures.join("\n")
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Per-thread runtime assembly (shared by `build()` and `RebornThreadBuilder`)
// ---------------------------------------------------------------------------

/// Assemble one `RebornIntegrationHarness` thread over an existing
/// `GroupSharedStorage`.
///
/// Lives in `group.rs` (not `builder.rs`) because `GroupSharedStorage` and the
/// capability-mode types (`GroupCapability`, `HarnessCapabilityMode`) are owned
/// by this module. `RebornIntegrationHarness` fields are `pub(crate)` so the
/// function can set them from here. Both the single-shot
/// `RebornIntegrationHarnessBuilder::build()` and `RebornThreadBuilder::build()`
/// delegate here — the only difference is whether `shared` is the sole owner or
/// is shared with a `RebornIntegrationGroup` and sibling threads.
///
/// Sequential-use assumption. Callers are responsible for capability construction
/// and auto-approve disable/enable **before** calling this function (those
/// operations require the `Arc<HostRuntimeCapabilityHarness>` inside
/// `shared.capability`, which is accessible to callers but consumed by
/// `capability_mode.into_parts` once this function begins).
pub(crate) async fn assemble_thread_runtime(
    shared: Arc<GroupSharedStorage>,
    conversation_id: &str,
    replies: Vec<RebornScriptedReply>,
    capability_mode: HarnessCapabilityMode,
) -> HarnessResult<RebornIntegrationHarness> {
    // --- product workflow + binding ----------------------------------------
    // A fresh adapter + ingress each time (cheap, stateless).  The binding
    // service is backed by `shared.product_harness` which is shared; the
    // idempotency ledger is also shared (per-binding idempotency).
    let adapter = RebornTestProductAdapter::new("reborn-itest", "itest-install")?;
    let ingress = RebornTestIngress::new(adapter);

    // Probe: synthesise a minimal inbound envelope to trigger binding
    // resolution (same conversation_id = same thread). The probe event id is
    // hardcoded — per-thread uniqueness comes from the per-thread
    // `conversation_id`, which routes each probe to a distinct binding.
    let probe = ingress.verified_text_envelope_with_trigger(
        "binding-probe",
        HARNESS_ACTOR_ID,
        conversation_id,
        "hi",
        ProductTriggerReason::DirectChat,
    )?;
    let binding = shared
        .product_harness
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

    // --- one composite for threads + turns (slice 3) -----------------------
    // `shared.composite` and `shared.turn_root` live for the group lifetime;
    // this thread borrows them via Arc-clone.
    let thread_harness = RebornThreadHarness::filesystem_shared_composite(
        thread_scope.clone(),
        Arc::clone(&shared.composite),
        Arc::clone(&shared.turn_root),
    )?;
    let turn_store: Arc<FilesystemTurnStateStore<HarnessTurnBackend>> =
        Arc::new(FilesystemTurnStateStore::new(scoped_turns_fs_composite(
            Arc::clone(&shared.composite),
            &binding,
        )?));
    let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
    let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> = turn_store.clone();
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());

    // --- real model gateway over the scripted raw provider -----------------
    // Session path is per-conversation so group threads do not clobber each
    // other's LLM session cache under the same `turn_root`.
    let raw: Arc<dyn LlmProvider> = Arc::new(scripted_trace_llm(replies));
    let session = create_session_manager(SessionConfig {
        session_path: shared
            .turn_root
            .path()
            .join(format!("{conversation_id}.session.json")),
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

    // --- capability surface ------------------------------------------------
    let (
        capability_factory,
        capability_surface_resolver,
        capability_input_resolver,
        capability_result_writer,
        capability_recorder,
    ) = capability_mode.into_parts(milestone_sink.clone())?;

    // Baselines: the recorder may already contain entries from prior threads in
    // the same group. Record the counts now so assertions only see the delta
    // produced by *this* thread's turns (R2).
    let baseline_invocation_count = capability_recorder.invocations().len();
    let baseline_egress_count = capability_recorder.runtime_http_requests().len();
    let baseline_result_count = capability_recorder.capability_results().len();
    let baseline_process_count = capability_recorder.recorded_process_commands().len();

    // --- loop-exit evidence ------------------------------------------------
    // When the capability backend wires the real local-dev approval stores
    // (`RebornIntegrationGroup::live_approvals`), attach the approval-gate evidence store so a
    // `BlockedApproval` run is verified against the persisted `Pending` request
    // at loop exit and genuinely pauses — mirrors production
    // `ironclaw_reborn_composition::runtime` (`with_approval_gate_evidence`,
    // runtime.rs:2799). Without it the blocked run is rejected as unverified and
    // goes terminal `Failed` (driver_protocol_violation), so the real gate flow
    // (`submit_turn_until_blocked` → `approve_gate`/`deny_gate`) cannot fire.
    let turn_state_for_evidence: Arc<dyn TurnStateStore> = turn_store.clone();
    let mut evidence = ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
        thread_harness.service.clone(),
        turn_state_for_evidence,
        Arc::clone(&loop_checkpoint_store),
        thread_scope.clone(),
    );
    if let Some(approval_requests) = capability_recorder.approval_requests_store() {
        evidence = evidence.with_approval_gate_evidence(
            ironclaw_reborn_composition::test_support::build_local_dev_approval_gate_evidence_for_test(
                approval_requests,
            ),
        );
    }
    let loop_exit_evidence: Arc<dyn LoopExitEvidencePort> = Arc::new(evidence);

    // --- planned runtime composition ---------------------------------------
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

    // --- product workflow over the coordinator -----------------------------
    let binding_service: Arc<dyn ConversationBindingService> =
        Arc::new(shared.product_harness.binding_service()?);
    let inbound: Arc<dyn InboundTurnService> = Arc::new(DefaultInboundTurnService::new(
        Arc::clone(&binding_service),
        thread_harness.service_instance()?,
        composition.coordinator.clone(),
    ));
    let ledger: Arc<dyn IdempotencyLedger> = Arc::new(shared.product_harness.idempotency_ledger());
    let workflow = DefaultProductWorkflow::new(inbound, ledger, binding_service);

    Ok(RebornIntegrationHarness {
        ingress,
        workflow,
        conversation_id: conversation_id.to_owned(),
        binding,
        turn_scope,
        turn_store,
        thread_harness,
        coordinator: composition.coordinator.clone(),
        scheduler_handle: Some(composition.scheduler_handle),
        event_seq: AtomicU64::new(1),
        capability_recorder,
        _shared: shared,
        baseline_invocation_count,
        baseline_egress_count,
        baseline_result_count,
        baseline_process_count,
    })
}
