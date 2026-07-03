//! Shared-persistence group infrastructure for Reborn integration tests.
//!
//! A **group** owns shared storage (composite filesystem, product workflow
//! harness, capability backend) AND one shared turn runtime (coordinator +
//! scheduler) exactly once; each [`RebornIntegrationGroup::thread`] call builds
//! a per-thread workflow (binding + inbound service + scripted-gateway
//! registration) over that one shared runtime. Within one group, state written
//! by thread A is visible to thread B — the key e2e persistence contract.
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
use ironclaw_host_api::{ResourceScope, UserId};
use ironclaw_host_runtime::TurnRunSchedulerHandle;
use ironclaw_llm::testing::provider_chain_over;
use ironclaw_llm::{LlmProvider, SessionConfig, create_session_manager};
use ironclaw_loop_support::{
    HostManagedModelGateway, HostUserProfileSource, JsonSpawnSubagentInputCodec,
    SubagentSpawnLimits,
};
use ironclaw_product_adapters::ProductTriggerReason;
use ironclaw_product_workflow::{
    ConversationBindingService, DefaultInboundTurnService, DefaultProductWorkflow,
    IdempotencyLedger, InboundTurnService, ResolvedBinding,
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
use ironclaw_turns::run_profile::{
    InMemoryLoopHostMilestoneSink, InstructionSafetyContext, ModelProfileId,
};
use ironclaw_turns::{
    FilesystemTurnStateStore, InMemoryCheckpointStateStore, InMemoryTurnEventSink,
    LoopCheckpointStore, TurnCoordinator, TurnEventSink, TurnScope, TurnStateStore,
};

use super::builder::{
    HARNESS_ACTOR_ID, INTERACTIVE_MODEL_PROFILE, RebornIntegrationHarness, StorageMode,
    apply_hermetic_env, binding_request, build_storage_composite, scoped_turns_fs_composite,
    thread_scope_from_binding,
};
use super::harness::{
    EmptyIdentityContextSource, HarnessCapabilityMode, HarnessCapabilityRecorder,
    HarnessTurnBackend, HostRuntimeCapabilityHarness, RecordingTestCapabilityPort,
    test_product_scope,
};
use super::product_workflow::RebornProductWorkflowHarness;
use super::reply::RebornScriptedReply;
use super::scope_gateway::ScopeRegistryGateway;
use super::scripted_provider::{
    ErrLlm, ParkingModelGate, SCRIPTED_MODEL_NAME, parking_trace_llm, scripted_trace_llm,
};
use super::session_thread::RebornThreadHarness;
use super::test_adapter::{RebornTestIngress, RebornTestProductAdapter};
use crate::support::trace_llm::TraceLlm;

/// Per-capability preset constructors layered on `build_base`/`into_group`
/// below. A private child module (not `pub mod` from `mod.rs`) so its only
/// caller — the constructor catalog — can reach `GroupBaseData` and the
/// assembly methods via plain module-private visibility instead of widening
/// them to `pub(crate)` for the whole test-support crate.
#[path = "group_constructors.rs"]
mod group_constructors;

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
    /// The group's single shared `TurnCoordinator`, over the ONE planned
    /// runtime built once at group construction (Option P: one
    /// scheduler/coordinator/executor over the shared turn-run queue, exactly
    /// prod's shape). Every thread's `DefaultInboundTurnService` is built over
    /// `Arc::clone` of this same coordinator.
    pub(crate) coordinator: Arc<dyn TurnCoordinator>,
    /// Owns the group's single `TurnRunScheduler` background worker.
    /// `TurnRunSchedulerHandle` is not `Clone`; it lives here (not on any
    /// per-thread `RebornIntegrationHarness`) and is kept alive by `_shared`.
    /// Its `Drop` impl synchronously cancels the scheduler loop when the last
    /// `Arc<GroupSharedStorage>` is dropped.
    pub(crate) scheduler_handle: TurnRunSchedulerHandle,
    /// Scope-keyed model-gateway registry. Every thread registers its scripted
    /// gateway here (`.thread(conv).script([...]).build()`) before submitting
    /// any turn; the loop-driver host resolves the per-scope gateway at host
    /// construction (`HostManagedModelGateway::resolve_for_scope`), off the
    /// model hot path.
    pub(crate) scope_gateway: Arc<ScopeRegistryGateway>,
    /// The group's single shared turn-state store. All threads share one
    /// `FilesystemTurnStateStore` (isolation is by `run_id`, not by path —
    /// see `turns_scope_path`, which has no `thread_id` component).
    pub(crate) turn_store: Arc<FilesystemTurnStateStore<HarnessTurnBackend>>,
    /// The group's single capability recorder, shared by `Arc` with the real
    /// capability factory wired into the one planned runtime. Every thread
    /// clones this (cheap — `HarnessCapabilityRecorder` is `Clone` over
    /// `Arc`-wrapped inner state) and slices `[baseline_*..]` so assertions
    /// only see that thread's own deltas (R2).
    pub(crate) capability_recorder: HarnessCapabilityRecorder,
    /// The exact `HostUserProfileSource` wired into the group's ONE planned
    /// runtime (E-PROFILE seam; built once in `into_group` from
    /// `capability_recorder.profile_filesystem()`). Kept so a profile-round-trip
    /// test can call `resolve_user_profile` on the SAME instance the running
    /// loop reads from, rather than re-deriving an equivalent one — a mutation
    /// that breaks the `into_group` wiring (not just `build_user_profile_source_for_test`
    /// itself) is caught.
    pub(crate) user_profile_source: Arc<dyn HostUserProfileSource>,
    /// In-memory turn-lifecycle event sink wired into the group's ONE planned
    /// runtime when `.with_turn_event_sink()` opted in (C-TRACECAP seam).
    /// `None` for every group that did not opt in (`turn_event_sink` stays
    /// `None` in `DefaultPlannedRuntimeParts`, matching prior behavior).
    /// Kept as the concrete `InMemoryTurnEventSink` (not `Arc<dyn TurnEventSink>`)
    /// so a test can read `.events()` back directly.
    pub(crate) turn_event_sink: Option<Arc<InMemoryTurnEventSink>>,
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
/// product workflow, capability backend, and the group's single shared turn
/// runtime (coordinator + scheduler). Each call to
/// [`thread`](Self::thread) builds a per-thread workflow over that one shared
/// runtime so state written by thread A is visible to thread B.
///
/// Construct with [`live_approvals`](Self::live_approvals),
/// [`builtin_tools`](Self::builtin_tools),
/// [`extension_lifecycle`](Self::extension_lifecycle), or
/// [`triggers`](Self::triggers), or via
/// [`builder`](Self::builder) for custom storage mode.
///
/// The per-capability preset constructors themselves (`live_approvals`,
/// `builtin_tools`, `extension_lifecycle`, `live_auth_gate`,
/// `project_lifecycle`, `profile_tools`, `triggers`, `skill_activation_tools`,
/// `skill_management_tools`, `attachment_tools`, and their `RebornIntegrationGroupBuilder`
/// counterparts) live in the child `group_constructors` module (file:
/// `group_constructors.rs`, kept private to `group` — see the `mod
/// group_constructors` declaration below) — a thin catalog of "which
/// capability" selections layered over the one-shared-runtime assembly
/// mechanics (`build_base`/`into_group`) this file owns. Mirrors the
/// `harness_mcp.rs` split.
pub struct RebornIntegrationGroup {
    pub(crate) shared: Arc<GroupSharedStorage>,
}

impl RebornIntegrationGroup {
    /// Builder for advanced configuration (e.g. `StorageMode::LibSql`).
    /// Defaults to `StorageMode::InMemory`.
    pub fn builder() -> RebornIntegrationGroupBuilder {
        RebornIntegrationGroupBuilder {
            storage: StorageMode::InMemory,
            safety_context: None,
            turn_event_sink: None,
        }
    }

    /// Create a per-thread *workflow* builder for `conversation_id`, over the
    /// group's ONE shared runtime (coordinator + scheduler) — this does NOT
    /// build a new runtime per thread.
    ///
    /// Each call gets a distinct binding/thread_id/turn_scope over the
    /// **shared** composite and capability backend. Build with
    /// `.script([...]).build().await`.
    pub fn thread(&self, conversation_id: impl Into<String>) -> RebornThreadBuilder<'_> {
        RebornThreadBuilder {
            group: self,
            conversation_id: conversation_id.into(),
            replies: Vec::new(),
            actor_id: None,
            model_mode: ThreadModelMode::Normal,
            model_override: None,
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

    /// The exact `HostUserProfileSource` wired into this group's ONE planned
    /// runtime (E-PROFILE seam). Lets a test read back a `profile_set` write
    /// through the SAME production adapter the running loop resolves user
    /// profiles from, rather than reconstructing an equivalent one — see the
    /// field docs on `GroupSharedStorage::user_profile_source`.
    pub(crate) fn user_profile_source_for_test(&self) -> &Arc<dyn HostUserProfileSource> {
        &self.shared.user_profile_source
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
///
/// Private (not `pub(crate)`): `group_constructors.rs` is a private child
/// module of `group` (see the `mod group_constructors` declaration above), so
/// module-private visibility already reaches it without widening these
/// internals to the whole test-support crate. The per-capability preset
/// constructors there take/return this type as the opaque handoff between
/// `build_base` and `into_group` — they never read its fields except
/// `canonical_binding`.
struct GroupBaseData {
    product_harness: RebornProductWorkflowHarness,
    composite: Arc<CompositeRootFilesystem>,
    libsql_db_path: Option<PathBuf>,
    turn_root: Arc<tempfile::TempDir>,
    /// A throwaway probe binding resolved once at group construction, used
    /// ONLY to derive the group-level shared turn store path and the
    /// group-level `ThreadScope` for the single `ThreadCheckpointLoopExitEvidencePort`.
    /// Every thread in a group shares `(tenant, agent, project)` — only
    /// `thread_id` varies per conversation, and `ThreadScope` (unlike
    /// `TurnScope`) has no `thread_id` field — so this canonical binding is a
    /// valid stand-in for the whole group (see `ensure_thread_scope_matches_turn_scope`,
    /// which checks only tenant/agent/project, never thread_id).
    ///
    /// `group_constructors.rs`'s `live_approvals`/`skill_activation_tools`
    /// read the resolved tenant/subject user off this field directly (see
    /// field docs above); reachable at module-private visibility since that
    /// file is a child module of `group`.
    canonical_binding: ResolvedBinding,
}

impl GroupBaseData {
    /// The canonical binding's resolved subject user id — the hashed `UserId`
    /// the actor `host-user` resolves to. `live_approvals` and `profile_tools`
    /// both pin their capability harness's executor user to this so capability
    /// dispatch shares the run's `(tenant, user)` with the turn-store /
    /// evidence scope resolved from the SAME `canonical_binding` (see the
    /// `canonical_binding` field docs above).
    fn canonical_subject_user(&self) -> HarnessResult<UserId> {
        Ok(self
            .canonical_binding
            .subject_user_id
            .clone()
            .ok_or("canonical binding missing subject user id")?)
    }
}

/// Builder for `RebornIntegrationGroup` with optional storage mode selection.
/// Obtain via [`RebornIntegrationGroup::builder`]; defaults to
/// `StorageMode::InMemory`.
pub struct RebornIntegrationGroupBuilder {
    storage: StorageMode,
    safety_context: Option<InstructionSafetyContext>,
    /// C-TRACECAP seam: `Some` once `.with_turn_event_sink()` has been called.
    turn_event_sink: Option<Arc<InMemoryTurnEventSink>>,
}

impl RebornIntegrationGroupBuilder {
    /// Select the durable storage backend (default: `StorageMode::InMemory`).
    /// Use `StorageMode::LibSql` to exercise on-disk durability across
    /// `assert_reply_persists_after_reopen`.
    pub fn storage(mut self, mode: StorageMode) -> Self {
        self.storage = mode;
        self
    }

    /// Wire a model-visible instruction-safety banner into the group's ONE
    /// shared planned runtime (`DefaultPlannedRuntimeParts::safety_context`).
    /// Rendered verbatim as a `system`-role prompt message ahead of any
    /// per-turn instructions (`push_safety_context`); the only model-visible
    /// artifact of instruction-safety scanning on this tier (T0-SYSPROMPT /
    /// C-SAFETY). Defaults to `None` (no banner, matching today's behavior).
    pub fn safety_context(mut self, ctx: InstructionSafetyContext) -> Self {
        self.safety_context = Some(ctx);
        self
    }

    /// Install an in-memory `TurnEventSink` (`ironclaw_turns::InMemoryTurnEventSink`,
    /// a real, already-shipped production type with zero callers today — this is the
    /// seam production wires via `subscribe_best_effort` in `build_default_planned_runtime_inner`,
    /// `crates/ironclaw_reborn/src/runtime.rs:613-619`) into the group's ONE planned
    /// runtime (C-TRACECAP). Read the recorded events back with
    /// [`RebornIntegrationHarness::recorded_turn_events`] — the ONLY read path;
    /// it slices `[baseline_turn_event_count..]` so a group thread never sees a
    /// sibling thread's events. Deliberately no raw group-level sink accessor:
    /// one would bypass that slicing and reintroduce cross-thread bleed.
    pub fn with_turn_event_sink(mut self) -> Self {
        self.turn_event_sink = Some(Arc::new(InMemoryTurnEventSink::default()));
        self
    }

    /// Shared setup for every group constructor: hermetic env, the product
    /// workflow harness over the fixed itest scope, the per-group `TempDir`, and
    /// the thread/turn composite. Returns [`GroupBaseData`] so each constructor
    /// names the fields it needs — the fixed test-scope strings live HERE only.
    ///
    /// Module-private: called by the per-capability preset constructors in
    /// the child `group_constructors` module.
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

        // Resolve the group-canonical binding ONCE here so `into_group` can
        // build the single shared turn store and evidence-port `ThreadScope`
        // before any per-thread binding exists. This is the SINGLE canonical
        // resolution for the group: `live_approvals` reuses
        // `canonical_binding.subject_user_id` for its capability user rather than
        // probing a second time, so turn-store scope and approval user can't
        // drift. The probe persists one deterministic, inert binding for
        // `conv-canonical-probe` (no thread submits turns against it); group
        // tests assert on cross-thread persistence, not binding counts.
        let adapter = RebornTestProductAdapter::new("reborn-itest", "itest-install")?;
        let ingress = RebornTestIngress::new(adapter);
        let probe = ingress.verified_text_envelope_with_trigger(
            "group-canonical-probe",
            HARNESS_ACTOR_ID,
            "conv-canonical-probe",
            "hi",
            ProductTriggerReason::DirectChat,
        )?;
        let canonical_binding = product_harness
            .binding_service()?
            .resolve_binding(binding_request(&probe))
            .await?;

        Ok(GroupBaseData {
            product_harness,
            composite,
            libsql_db_path,
            turn_root,
            canonical_binding,
        })
    }

    /// Assemble the group's ONE shared planned runtime (Option P: one
    /// scheduler/coordinator/executor over the shared turn-run queue) and the
    /// rest of `GroupSharedStorage`.
    ///
    /// Builds the capability parts exactly once (`capability.mode().into_parts`)
    /// so the stored `capability_recorder` is the SAME `Arc`-backed instance the
    /// real capability factory writes through — not a second, divergent
    /// recorder. Wires `.with_checkpoint_state_store` on the group-level
    /// `ThreadCheckpointLoopExitEvidencePort` (the de-mask fix, design §4) and
    /// `.with_approval_gate_evidence` when the capability backend exposes a
    /// local-dev approval store.
    ///
    /// Module-private: called by the per-capability preset constructors in
    /// the child `group_constructors` module.
    async fn into_group(
        self,
        base: GroupBaseData,
        capability: GroupCapability,
    ) -> HarnessResult<RebornIntegrationGroup> {
        let scope_gateway = Arc::new(ScopeRegistryGateway::new());

        let turn_store: Arc<FilesystemTurnStateStore<HarnessTurnBackend>> =
            Arc::new(FilesystemTurnStateStore::new(scoped_turns_fs_composite(
                Arc::clone(&base.composite),
                &base.canonical_binding,
            )?));
        let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> = turn_store.clone();
        let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());

        let group_thread_scope = thread_scope_from_binding(&base.canonical_binding)?;
        let group_thread_harness = RebornThreadHarness::filesystem_shared_composite(
            group_thread_scope.clone(),
            Arc::clone(&base.composite),
            Arc::clone(&base.turn_root),
        )?;

        let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());

        let (
            capability_factory,
            capability_surface_resolver,
            capability_input_resolver,
            capability_result_writer,
            capability_recorder,
        ) = capability.mode().into_parts(milestone_sink.clone())?;

        // --- loop-exit evidence (group-level, built once) -----------------
        // `.with_checkpoint_state_store` is the de-mask fix: without it a
        // genuinely-`Failed` run is reported as the masking
        // `driver_protocol_violation` instead of its true failure category.
        let turn_state_for_evidence: Arc<dyn TurnStateStore> = turn_store.clone();
        let mut evidence = ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
            group_thread_harness.service.clone(),
            turn_state_for_evidence,
            Arc::clone(&loop_checkpoint_store),
            group_thread_scope.clone(),
        )
        .with_checkpoint_state_store(checkpoint_state_store.clone());
        if let Some(approval_requests) = capability_recorder.approval_requests_store() {
            evidence = evidence.with_approval_gate_evidence(
                ironclaw_reborn_composition::test_support::build_local_dev_approval_gate_evidence_for_test(
                    approval_requests,
                ),
            );
        }
        let loop_exit_evidence: Arc<dyn LoopExitEvidencePort> = Arc::new(evidence);

        // --- the group's ONE planned runtime -------------------------------
        let turn_state_for_runtime: Arc<dyn RuntimeTurnStateStore> = turn_store.clone();
        let model_gateway: Arc<dyn HostManagedModelGateway> =
            Arc::clone(&scope_gateway) as Arc<dyn HostManagedModelGateway>;
        let user_profile_source: Arc<dyn HostUserProfileSource> =
            ironclaw_reborn_composition::test_support::build_user_profile_source_for_test(
                capability_recorder.profile_filesystem(),
            );
        let composition = build_default_planned_runtime(DefaultPlannedRuntimeParts {
            turn_state: turn_state_for_runtime,
            thread_service: group_thread_harness.service.clone() as Arc<dyn SessionThreadService>,
            thread_scope: group_thread_scope,
            model_gateway,
            checkpoint_state_store: checkpoint_state_store.clone(),
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
            // E-GATEWAY: the parking scripted gateway (`park_model`) plus
            // `cancel_run` are the covered seam. This optional `cancellation_factory`
            // is intentionally left `None`: it does NOT gate whether a run reaches
            // `Cancelled`. `RebornLoopDriverHostFactory` always builds its own
            // default `TurnStateRunCancellationFactory` internally
            // (`ironclaw_reborn::loop_driver_host`), whose cancel poll loop
            // (`DEFAULT_CANCEL_POLL_INTERVAL`, 25ms) observes the durable
            // `CancelRequested` and drives the parked run to `Cancelled` on resume —
            // which is what `reborn_integration_cancel` asserts (verified 12/12).
            // Supplying a factory here would only add the coordinator's
            // `CompositeTurnRunWakeNotifier` fan-out for product-live retained-run-handle
            // observation (`ironclaw_reborn::runtime` `wake_notifier`), a path this
            // test does not exercise — so wiring one would be dead, untested code.
            cancellation_factory: None,
            // E-SKILL: wire the local-dev skill context source so an activated
            // skill's instructions inject into the model request. `Some` only for
            // `skill_activation_tools()` harnesses; `None` for every other backend,
            // so all existing group tests are behavior-identical (production wires
            // this in `build_reborn_runtime`, runtime.rs ~2875).
            skill_context_source: capability_recorder.skill_context_source(),
            input_queue: None,
            identity_context_source: Arc::new(EmptyIdentityContextSource),
            // E-PROFILE: in HostRuntime mode, back the profile source with the
            // local-dev memory filesystem so `profile_set` writes can be read back;
            // non-HostRuntime backends (and HostRuntime harnesses without a profile
            // filesystem) fall back to `EmptyUserProfileSource`. `resolve_user_profile`
            // returns `None` when no `context/profile.json` exists, so existing
            // HostRuntime group tests are behavior-identical. Built once here (not
            // per-thread) because the group's ONE planned runtime is assembled once.
            // Kept in a local (rather than built inline) so the SAME `Arc` can
            // also be stashed on `GroupSharedStorage` for a profile-round-trip
            // test to read from directly (see `user_profile_source` field docs).
            user_profile_source: Arc::clone(&user_profile_source),
            model_policy_guard: None,
            model_budget_accountant: None,
            safety_context: self.safety_context,
            hook_dispatcher_builder_factory: None,
            communication_context_provider: None,
            hook_security_audit_sink: None,
            turn_event_sink: self
                .turn_event_sink
                .clone()
                .map(|sink| sink as Arc<dyn TurnEventSink>),
            attachment_read_port: capability_recorder
                .attachment_test_support()
                .map(|support| support.read_port),
            scheduler_wake_wiring: None,
        })?;

        Ok(RebornIntegrationGroup {
            shared: Arc::new(GroupSharedStorage {
                composite: base.composite,
                libsql_db_path: base.libsql_db_path,
                turn_root: base.turn_root,
                product_harness: base.product_harness,
                capability,
                coordinator: composition.coordinator,
                scheduler_handle: composition.scheduler_handle,
                scope_gateway,
                turn_store,
                capability_recorder,
                user_profile_source,
                turn_event_sink: self.turn_event_sink,
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// RebornThreadBuilder
// ---------------------------------------------------------------------------

/// Per-thread *workflow* builder for a `RebornIntegrationGroup`.
///
/// Builds a per-thread workflow (binding + inbound service + scripted-gateway
/// registration) over the group's ONE shared runtime — it does NOT build a
/// per-thread scheduler/coordinator. The builder borrows the group for its own
/// lifetime (R6). Calling `build()` Arc-clones all shared fields from
/// `GroupSharedStorage` into the returned `RebornIntegrationHarness`, which is
/// `'static` and independent of the group's stack frame. Multiple harnesses
/// may coexist — the shared coordinator dispatches by `run_id`, so siblings
/// can be parked on gates at the same time (the `concurrent_dual_gate_resume`
/// scenario relies on exactly this).
pub struct RebornThreadBuilder<'g> {
    group: &'g RebornIntegrationGroup,
    conversation_id: String,
    replies: Vec<RebornScriptedReply>,
    actor_id: Option<String>,
    model_mode: ThreadModelMode,
    /// C-ATTACH seam: overrides `LlmModelProfileRoute.model_override` (the same
    /// production model-pin field, `model_gateway.rs:160-162`). `None` keeps the
    /// prior behavior (scripted model id, not a vision pattern, so image parts
    /// are dropped); `Some` routes through a vision-capable id so `convert_messages`
    /// builds `ContentPart::ImageUrl` parts.
    model_override: Option<String>,
}

/// A thread's model-call behavior: exactly one of normal scripted playback,
/// parked-until-released, or unconditional failure. One enum instead of an
/// `Option<ParkingModelGate>` + `bool` pair (mirrors `ShellMode` in
/// `builder.rs`) so the three modes are mutually exclusive BY CONSTRUCTION —
/// no tuple-priority rule needed at the dispatch site, and no state can
/// silently ask for "parked AND failing" at once.
#[derive(Default)]
enum ThreadModelMode {
    /// Normal scripted playback (the default).
    #[default]
    Normal,
    /// This thread's model call parks until the gate is released (E-GATEWAY
    /// seam), enabling a mid-turn cancel test.
    Parked(ParkingModelGate),
    /// This thread's model call always fails with a fixed non-retryable
    /// `LlmError` (E-GATEWAY seam, C-ERRORS) instead of playing back
    /// `replies`. See [`super::scripted_provider::ErrLlm`].
    Failing,
}

impl<'g> RebornThreadBuilder<'g> {
    /// Set the scripted model replies for this thread (consumed in order at the
    /// raw-provider seam, one per model turn).
    pub fn script(mut self, replies: impl IntoIterator<Item = RebornScriptedReply>) -> Self {
        self.replies = replies.into_iter().collect();
        self
    }

    /// Park this thread's model call until `gate` is released (E-GATEWAY seam).
    /// The parking provider sits at the same vendor-SDK seam as the scripted
    /// provider, so the real decorator chain still runs on top.
    pub fn park_model(self, gate: ParkingModelGate) -> Self {
        self.park_model_opt(Some(gate))
    }

    /// Internal: set the optional park gate (used by the flat builder to thread
    /// its own park gate through the degenerate one-thread group). A `Some`
    /// gate always wins, matching the old tuple-priority contract, even if
    /// `fail_model_opt` is called first.
    pub(crate) fn park_model_opt(mut self, gate: Option<ParkingModelGate>) -> Self {
        if let Some(gate) = gate {
            self.model_mode = ThreadModelMode::Parked(gate);
        }
        self
    }

    /// Resolve this thread's binding under a DISTINCT actor instead of the
    /// group's default `HARNESS_ACTOR_ID` (E-MULTIUSER seam). The resulting
    /// binding's `subject_user_id`/`actor_user_id` differ from every other
    /// thread's, so the run's `TurnActor` and per-turn owner-scope resolution
    /// (`ThreadScopeResolver::resolve_for_turn`, the same mechanism production
    /// uses for multi-user WebChat) isolate this thread's reads/writes under
    /// their own subtree. The owner axis of that subtree is the resolved
    /// canonical `UserId` (`binding.subject_user_id` / `ThreadScope.owner_user_id`)
    /// — NOT the external `actor_id` string passed here — the binding probe
    /// maps this `actor_id` to that canonical user id once at build time, and
    /// every subsequent op resolves its mount from the binding, not the raw
    /// string. Unset (the default) keeps the existing `HARNESS_ACTOR_ID`
    /// behavior byte-identical.
    pub fn with_actor_id(mut self, actor_id: impl Into<String>) -> Self {
        self.actor_id = Some(actor_id.into());
        self
    }

    /// Fail this thread's model call unconditionally with a fixed, non-retryable
    /// `LlmError` (E-GATEWAY seam, C-ERRORS — provider-`Err` failure category).
    /// Sits at the same vendor-SDK seam as `park_model`/scripted playback.
    pub fn fail_model(self) -> Self {
        self.fail_model_opt(true)
    }

    /// Internal: set the fail-model flag (used by the flat builder to thread
    /// its own knob through the degenerate one-thread group). Never downgrades
    /// an already-`Parked` mode, matching the old tuple-priority contract
    /// (`park_model` always wins over `fail_model`).
    pub(crate) fn fail_model_opt(mut self, fail: bool) -> Self {
        if fail && !matches!(self.model_mode, ThreadModelMode::Parked(_)) {
            self.model_mode = ThreadModelMode::Failing;
        }
        self
    }

    /// Route this thread at a specific provider model id (see
    /// `ironclaw_llm::vision_models::VISION_PATTERNS` for vision-capable ids) —
    /// C-ATTACH seam.
    pub fn with_model_override(mut self, model: impl Into<String>) -> Self {
        self.model_override = Some(model.into());
        self
    }

    /// Build the per-thread `RebornIntegrationHarness` over the group's shared
    /// storage and ONE shared planned runtime.
    ///
    /// Builds the per-thread scripted `LlmProviderModelGateway`, resolves the
    /// per-thread binding + `TurnScope`, and builds a per-thread workflow over
    /// the group's SHARED coordinator (no new runtime, no new scheduler). The
    /// gateway is **registered** on the group's `scope_gateway` only after all
    /// of that fallible (`?`) setup has succeeded, immediately before the
    /// harness is constructed — so a failed `build()` never leaves a scope
    /// registered for a harness that doesn't exist, while still guaranteeing
    /// registration happens before this fn returns (and thus before
    /// `submit_turn` can be called for this thread's scope). Arc-clones every
    /// shared field from `GroupSharedStorage` so the returned harness is
    /// `'static` (does not borrow `'g`).
    pub async fn build(self) -> HarnessResult<RebornIntegrationHarness> {
        let shared = Arc::clone(&self.group.shared);

        // --- product workflow + per-thread binding -----------------------------
        // A fresh adapter + ingress each time (cheap, stateless). The binding
        // service is backed by `shared.product_harness`, which is shared; the
        // idempotency ledger is also shared (per-binding idempotency).
        let actor_id = self.actor_id.as_deref().unwrap_or(HARNESS_ACTOR_ID);
        let adapter = RebornTestProductAdapter::new("reborn-itest", "itest-install")?;
        let ingress = RebornTestIngress::new(adapter);
        let probe = ingress.verified_text_envelope_with_trigger(
            "binding-probe",
            actor_id,
            &self.conversation_id,
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

        // --- per-thread scripted gateway, registered before any submit ---------
        // Session path is per-conversation so group threads do not clobber each
        // other's LLM session cache under the same `turn_root`.
        // Retain the concrete `TraceLlm` before the `dyn LlmProvider` upcast so
        // tests can inspect the model-visible requests via `captured_requests()`:
        // the system prompt (`assert_system_prompt_contains`, T0-SYSPROMPT) and
        // host-injected context such as activated-skill instructions
        // (`assert_model_request_contains`, E-SKILL half B).
        //
        // E-GATEWAY: when a park gate is set, swap the scripted provider for a
        // parking one at the SAME vendor-SDK seam (the decorator chain still runs
        // on top). Parking mode is only a wrapper around the same scripted
        // provider, so the `TraceLlm` is built unconditionally first and the
        // parking wrapper holds/clones that same `Arc` — the trace is retained
        // either way, so tests can inspect captured requests
        // (`assert_system_prompt_contains`, `assert_model_request_contains`)
        // regardless of whether this thread is parked.
        let scripted_llm: Arc<TraceLlm> = Arc::new(scripted_trace_llm(self.replies));
        // C-ERRORS: `Failing` swaps in `ErrLlm` at the same vendor-SDK seam;
        // `Parked` swaps in the parking wrapper. `ThreadModelMode` makes the
        // three modes mutually exclusive by construction — no priority rule
        // needed here.
        let raw: Arc<dyn LlmProvider> = match self.model_mode {
            ThreadModelMode::Parked(gate) => {
                Arc::new(parking_trace_llm(gate, scripted_llm.clone()))
            }
            ThreadModelMode::Failing => Arc::new(ErrLlm),
            ThreadModelMode::Normal => scripted_llm.clone(),
        };
        let session = create_session_manager(SessionConfig {
            session_path: shared
                .turn_root
                .path()
                .join(format!("{}.session.json", self.conversation_id)),
            ..SessionConfig::default()
        })
        .await;
        let llm_config = ironclaw_llm::testing::nearai_test_config(SCRIPTED_MODEL_NAME);
        let provider = provider_chain_over(raw, &llm_config, session).await?;
        let model_profile_id = ModelProfileId::new(INTERACTIVE_MODEL_PROFILE)
            .map_err(|reason| format!("invalid model profile id: {reason}"))?;
        let policy = LlmModelProfilePolicy::new()
            .allow_model_profile(model_profile_id, self.model_override.clone());
        let thread_gateway: Arc<dyn HostManagedModelGateway> =
            Arc::new(LlmProviderModelGateway::new(provider, policy));

        // --- per-thread thread_harness (shared composite) -----------------------
        let thread_harness = RebornThreadHarness::filesystem_shared_composite(
            thread_scope.clone(),
            Arc::clone(&shared.composite),
            Arc::clone(&shared.turn_root),
        )?;

        // --- capability recorder + baselines ------------------------------------
        // Baselines: the recorder may already contain entries from prior threads
        // in the same group. Record the counts now so assertions only see the
        // delta produced by *this* thread's turns (R2).
        let capability_recorder = shared.capability_recorder.clone();
        let baseline_invocation_count = capability_recorder.invocations().len();
        let baseline_egress_count = capability_recorder.runtime_http_requests().len();
        let baseline_result_count = capability_recorder.capability_results().len();
        let baseline_process_count = capability_recorder.recorded_process_commands().len();
        let baseline_network_count = capability_recorder.network_http_requests().len();
        let baseline_turn_event_count = shared
            .turn_event_sink
            .as_ref()
            .map(|sink| sink.events().len())
            .unwrap_or(0);

        // --- per-thread workflow over the SHARED coordinator --------------------
        let binding_service: Arc<dyn ConversationBindingService> =
            Arc::new(shared.product_harness.binding_service()?);
        let mut inbound_service = DefaultInboundTurnService::new(
            Arc::clone(&binding_service),
            thread_harness.service_instance()?,
            Arc::clone(&shared.coordinator),
        );
        // C-ATTACH: wire the real lander when the backend has one (`attachment_tools()`)
        // so `submit_inbound_with_attachments` lands through it instead of
        // failing closed. `None` for every other group (unchanged behavior).
        if let Some(support) = capability_recorder.attachment_test_support() {
            inbound_service = inbound_service.with_inbound_attachments(support.lander);
        }
        let inbound: Arc<dyn InboundTurnService> = Arc::new(inbound_service);
        let ledger: Arc<dyn IdempotencyLedger> =
            Arc::new(shared.product_harness.idempotency_ledger());
        let workflow = DefaultProductWorkflow::new(inbound, ledger, binding_service);

        // Register the gateway only now that every fallible (`?`) step above has
        // succeeded — `register` is infallible and interior-mutable, so deferring
        // it here costs nothing, but registering any earlier risks leaving the
        // scope registered for a harness that never finished building: a later
        // `?` bailing out of `build()` would leave `turn_scope` registered, and
        // retrying the same conversation would then hit the duplicate-
        // registration panic. The loop-driver host only resolves
        // `resolve_for_scope` at host construction (off the model hot path), and
        // that happens strictly after this fn returns, so registering
        // immediately before the harness is constructed still satisfies
        // "registered before any submit for this scope".
        shared
            .scope_gateway
            .register(turn_scope.clone(), thread_gateway);

        Ok(RebornIntegrationHarness {
            ingress,
            workflow,
            conversation_id: self.conversation_id,
            actor_id: actor_id.to_owned(),
            binding,
            turn_scope,
            turn_store: Arc::clone(&shared.turn_store),
            thread_harness,
            coordinator: Arc::clone(&shared.coordinator),
            event_seq: AtomicU64::new(1),
            capability_recorder,
            scripted_llm,
            _shared: Arc::clone(&shared),
            baseline_invocation_count,
            baseline_egress_count,
            baseline_result_count,
            baseline_process_count,
            baseline_network_count,
            baseline_turn_event_count,
        })
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
