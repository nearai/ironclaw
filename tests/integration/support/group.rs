//! Shared-persistence group infrastructure for Reborn integration tests.
//!
//! A **group** owns shared storage (composite filesystem, product workflow
//! harness, capability backend) AND one shared turn runtime (coordinator +
//! scheduler) exactly once; each [`RebornIntegrationGroup::thread`] call builds
//! a per-thread workflow (binding + inbound service + scripted-gateway
//! registration) over that one shared runtime. Within one group, state written
//! by thread A is visible to thread B — the key e2e persistence contract.
//! Separate groups are separate test binaries, fully isolated. A single-shot
//! [`RebornIntegrationHarness::test_default()`] is a degenerate one-thread
//! group (its own storage, baseline = 0).
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
//! One sequential `#[tokio::test]` drives all scenarios (Cargo doesn't
//! guarantee order or share state across `#[test]` fns in one binary). Use `?`
//! for *dependent* scenarios (failure stops the driver) and
//! `report.record(name, scenario::run(&g).await)` for *independent* ones
//! (failure recorded, others continue).
//!
//! ### Subdir module paths (required)
//!
//! Each group `main.rs` MUST declare BOTH `#[path]` overrides, each with
//! `#[allow(dead_code)]` — bare `mod support;` resolves relative to the
//! group's own subdir and fails to compile:
//!
//! ```rust,no_run
//! #[allow(dead_code)] #[path = "../support/mod.rs"] mod reborn_support;
//! #[allow(dead_code)] #[path = "../../support/mod.rs"] mod support;
//! ```
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
use ironclaw_llm::testing::provider_chain_over;
use ironclaw_llm::{LlmProvider, SessionConfig, create_session_manager};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilitySurfaceProfileResolver, HostManagedModelGateway,
    HostUserProfileSource, JsonSpawnSubagentInputCodec, ModelCostTable, SubagentSpawnLimits,
    ZeroCostTable,
};
use ironclaw_product_adapters::ProductTriggerReason;
use ironclaw_product_workflow::{
    ConversationBindingService, DefaultInboundTurnService, DefaultProductWorkflow,
    IdempotencyLedger, InboundTurnService, ResolvedBinding,
};
use ironclaw_reborn_composition::build_default_budget_accountant;
use ironclaw_reborn_config::BudgetDefaults;
use ironclaw_resources::{
    BudgetEventSink, BudgetGateStore, InMemoryBudgetEventSink, InMemoryBudgetGateStore,
    InMemoryResourceGovernor, ResourceAccount, ResourceGovernor,
};
use ironclaw_runner::loop_driver_host::HookDispatcherBuilderFactory;
use ironclaw_runner::loop_exit_applier::{
    LoopExitEvidencePort, ThreadCheckpointLoopExitEvidencePort,
};
use ironclaw_runner::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
use ironclaw_runner::runtime::{
    DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts, RuntimeTurnStateStore,
    ToolDisclosureMode, build_default_planned_runtime,
};
use ironclaw_runner::subagent::{
    await_edge::{
        boot_recovery::ScopeRecoveryDriver, resolver::AwaitEdgeResolver,
        store::FilesystemAwaitEdgeStore,
    },
    flavors::StaticSubagentDefinitionResolver,
    goal_store::InMemoryBoundedSubagentGoalStore,
};
use ironclaw_runner::turn_scheduler::TurnRunSchedulerHandle;
use ironclaw_threads::SessionThreadService;
use ironclaw_turns::run_profile::{
    CommunicationContextProvider, InMemoryLoopHostMilestoneSink, InstructionSafetyContext,
    ModelProfileId,
};
use ironclaw_turns::{
    FilesystemTurnStateStore, InMemoryCheckpointStateStore, InMemoryTurnEventSink,
    InMemoryTurnStateStoreLimits, LoopCheckpointStore, TurnCoordinator, TurnEventSink, TurnScope,
    TurnStateStore,
};

use super::builder::{
    HARNESS_ACTOR_ID, INTERACTIVE_MODEL_PROFILE, RebornIntegrationHarness, StorageMode,
    apply_hermetic_env, binding_request, build_storage_composite, scoped_turns_fs_composite,
    thread_scope_from_binding,
};
use super::harness::{
    EmptyIdentityContextSource, HarnessCapabilityMode, HarnessCapabilityRecorder,
    HarnessTurnBackend, HostRuntimeCapabilityHarness, RecordingTestCapabilityPort,
    StaticCapabilitySurfaceProfileResolver, test_product_scope,
};
use super::planned_runtime_parts_shape::{
    DefaultPlannedRuntimePartsShape, harness_planned_runtime_parts_shape,
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

/// Optional-runtime-wiring setters (`storage`, `safety_context`,
/// `with_turn_event_sink`, `with_trace_capture`, `with_tool_disclosure_bridged`,
/// `budget_accounting`, `communication_context_provider`,
/// `hook_dispatcher_builder_factory`) on
/// [`RebornIntegrationGroupBuilder`]. A private child module (not `pub mod`
/// from `mod.rs`), same precedent as `group_constructors` above — it reaches
/// the builder's private fields at plain module-private visibility instead
/// of widening them to `pub(crate)` for the whole test-support crate.
#[path = "group_options.rs"]
mod group_options;

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
    /// S2 seam: the SAME canonical binding `turn_store`'s `/turns` mount is
    /// scoped to (`scoped_turns_fs_composite`). Retained so a reopen can
    /// rebuild the identical scoped path independently, instead of
    /// re-deriving it from a second binding resolution.
    pub(crate) canonical_binding: ResolvedBinding,
    /// The group's single capability recorder, shared by `Arc` with the real
    /// capability factory wired into the one planned runtime. Every thread
    /// clones this (cheap — `HarnessCapabilityRecorder` is `Clone` over
    /// `Arc`-wrapped inner state) and slices `[baseline_*..]` so assertions
    /// only see that thread's own deltas (R2).
    pub(crate) capability_recorder: HarnessCapabilityRecorder,
    /// The exact `HostUserProfileSource` wired into the group's ONE planned
    /// runtime (E-PROFILE seam). Kept so a profile-round-trip test reads from
    /// the SAME instance the running loop uses, not a re-derived equivalent —
    /// catches wiring mutations, not just the builder itself.
    pub(crate) user_profile_source: Arc<dyn HostUserProfileSource>,
    /// In-memory turn-lifecycle event sink wired in when `.with_turn_event_sink()`
    /// opted in (C-TRACECAP seam); `None` otherwise. Concrete type (not `Arc<dyn
    /// TurnEventSink>`) so a test can read `.events()` back directly.
    pub(crate) turn_event_sink: Option<Arc<InMemoryTurnEventSink>>,
    /// The exact loop milestone sink wired into the group's ONE planned runtime.
    /// Retained so integration tests can assert production loop milestones
    /// without adding event-specific hooks to the runtime path.
    pub(crate) milestone_sink: Arc<InMemoryLoopHostMilestoneSink>,
    /// Enabler (c): the `trace_scope_key(tenant, owner)` the production
    /// trace-capture sink was seeded with when `.with_trace_capture()` opted
    /// in; `None` otherwise. Recorded at wiring time so a test asserts against
    /// EXACTLY the scope the sink observes, not a re-derived equivalent.
    pub(crate) trace_capture_scope: Option<String>,
    /// C-BUDGET: the in-memory `ResourceGovernor` behind the group's
    /// `model_budget_accountant`. Retained so a test can read back the account
    /// the accountant seeds on a turn's first model call — proof the
    /// accountant is wired and fires. `None` unless budget accounting is wired.
    pub(crate) budget_governor: Option<Arc<InMemoryResourceGovernor>>,
    /// C-BUDGET: the `(tenant, run-owner-user)` account the group's turns
    /// reserve against — computed once from the canonical binding so a test
    /// reads the SAME account the loop's accountant seeds. `None` unless
    /// budget accounting is wired.
    pub(crate) budget_account: Option<ResourceAccount>,
    /// W5-WIRING-PARITY: the Some/None shape of the `DefaultPlannedRuntimeParts`
    /// literal this group's ONE planned runtime was actually built from,
    /// captured at construction (before `build_default_planned_runtime`
    /// consumes the struct by value) so a parity test can read back the
    /// harness's REAL wiring shape, not a re-derived approximation.
    pub(crate) planned_runtime_parts_shape: DefaultPlannedRuntimePartsShape,
    /// See `RebornIntegrationGroupBuilder::with_real_gate_dispatch_services`.
    /// Read by `RebornThreadBuilder::build()` to decide whether to wire the
    /// real approval/auth interaction services into the thread's workflow.
    pub(crate) real_gate_dispatch_services: bool,
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

    /// C-MULTIUSER: the auto-approve `(tenant, user)` scope for a SPECIFIC run
    /// owner. Uses the group's real run tenant (`product_harness.scope`, e.g.
    /// `tenant-itest`) with `owner`'s user id — the exact key the dispatch-time
    /// auto-approve check reads for a run OWNED by `owner` once the capability
    /// backend is built with `with_run_owner_scoped_capability_dispatch`. Unlike
    /// [`auto_approve_scope`] (which keys on the fixed capability user, shared by
    /// all actors), this keys per actor, so a grant seeded here applies to that
    /// owner's runs only. `None` for the Echo backend (no approval stores).
    pub(crate) fn auto_approve_scope_for_owner(&self, owner: &UserId) -> Option<ResourceScope> {
        match &self.capability {
            GroupCapability::HostRuntime(_) => {
                let mut scope = self.product_harness.scope.clone();
                scope.user_id = owner.clone();
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
/// The per-capability preset constructors (`live_approvals`, `builtin_tools`,
/// `extension_lifecycle`, etc., and their `RebornIntegrationGroupBuilder`
/// counterparts) live in the private child module `group_constructors` — a
/// thin catalog of "which capability" selections layered over the
/// one-shared-runtime assembly mechanics (`build_base`/`into_group`) this
/// file owns.
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
            trace_capture: false,
            tool_disclosure: None,
            budget: false,
            communication_context_provider: None,
            hook_dispatcher_builder_factory: None,
            runner_lease_ttl_override: None,
            lease_recovery_interval_override: None,
            real_gate_dispatch_services: false,
        }
    }

    /// Enabler (c): the trace scope key the production trace-capture sink was
    /// seeded with; `Some` only after `.with_trace_capture()`. Pair with
    /// `ironclaw_reborn_traces::contribution::queued_trace_envelope_paths_for_scope`
    /// to assert an enrolled turn queued a contribution envelope.
    pub fn trace_capture_scope(&self) -> Option<&str> {
        self.shared.trace_capture_scope.as_deref()
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

    /// W5-WIRING-PARITY: the Some/None shape of the `DefaultPlannedRuntimeParts`
    /// literal this group's ONE planned runtime was actually built from
    /// (`into_group`), captured at construction time before the struct was
    /// consumed. See `tests/integration/wiring_parity.rs`.
    pub fn planned_runtime_parts_shape(&self) -> DefaultPlannedRuntimePartsShape {
        self.shared.planned_runtime_parts_shape
    }

    /// C-MULTIUSER: grant global always-allow (auto-approve) for a SPECIFIC run
    /// owner's `(tenant, user)` scope over the shared CAS-persisted
    /// `AutoApproveSettingStore`. In a `multiuser_approvals` group (built with
    /// `with_run_owner_scoped_capability_dispatch`), a turn OWNED by `owner`
    /// then dispatches its capability without raising an approval gate, while
    /// any OTHER owner's identical call still gates — the per-actor isolation
    /// proof. Errors for the Echo backend (no approval stores).
    pub async fn enable_auto_approve_for_owner(&self, owner: &UserId) -> HarnessResult<()> {
        let scope = self
            .shared
            .auto_approve_scope_for_owner(owner)
            .ok_or("group has no host-runtime capability backend for auto-approve")?;
        self.shared
            .capability_recorder
            .enable_auto_approve_for(scope)
            .await
    }

    /// C-MULTIUSER: set a SPECIFIC run owner's always-allow OFF over the shared
    /// `AutoApproveSettingStore`. Auto-approve defaults ON when a user has no
    /// record (`AUTO_APPROVE_DEFAULT_ENABLED = true`, production), so a per-actor
    /// isolation test that needs owner B to still GATE must give B its own
    /// explicit OFF setting — exactly as `live_approvals` disables its dispatch
    /// scope to make gates fire. Errors for the Echo backend.
    pub async fn disable_auto_approve_for_owner(&self, owner: &UserId) -> HarnessResult<()> {
        let scope = self
            .shared
            .auto_approve_scope_for_owner(owner)
            .ok_or("group has no host-runtime capability backend for auto-approve")?;
        self.shared
            .capability_recorder
            .disable_auto_approve_for(scope)
            .await
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
/// Plain module-private visibility: `group_constructors.rs` reaches this at
/// plain module-private visibility as a descendant of `group` (see the `mod
/// group_constructors` declaration above), so the fields stay private and the
/// per-capability preset constructors there — including their own
/// `build_group_capability_with_base` helper, which calls
/// `canonical_subject_user()` — take/return this type as the opaque handoff
/// between `build_base` and `into_group`; `build_base`/`into_group` themselves
/// stay module-private too.
struct GroupBaseData {
    product_harness: RebornProductWorkflowHarness,
    composite: Arc<CompositeRootFilesystem>,
    libsql_db_path: Option<PathBuf>,
    turn_root: Arc<tempfile::TempDir>,
    /// A throwaway probe binding resolved once at group construction, used
    /// ONLY to derive the group-level shared turn store path and the
    /// group-level `ThreadScope`. Every thread in a group shares `(tenant,
    /// agent, project)` — only `thread_id` varies, and `ThreadScope` has no
    /// `thread_id` field — so this binding is a valid stand-in for the whole
    /// group. `group_constructors.rs` reads tenant/subject user off this
    /// field directly (module-private; it's a child module of `group`).
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
    /// Enabler (c): `true` once `.with_trace_capture()` has been called —
    /// `into_group` wires the PRODUCTION `TraceCaptureTurnEventSink` (via
    /// composition's `trace_capture_turn_event_sink_for_test`) into the
    /// group's one planned runtime, fan-out-composed with the in-memory sink
    /// when both are opted in.
    trace_capture: bool,
    /// Enabler (b): `Some(ToolDisclosureMode::Bridged)` once
    /// `.with_tool_disclosure_bridged()` has been called; `None` resolves via
    /// `ToolDisclosureMode::from_env()` in `into_group` (today's behavior).
    tool_disclosure: Option<ToolDisclosureMode>,
    /// C-BUDGET: when `true`, `into_group` wires the production
    /// `build_default_budget_accountant` (in-memory governor + gate store +
    /// zero-cost table + compiled-default seeding) into the group's ONE planned
    /// runtime and retains the governor for read-back. Default `false` (no
    /// accountant — byte-identical to today's behavior).
    budget: bool,
    /// C-COMMCTX: an optional `CommunicationContextProvider` wired into the
    /// group's ONE planned runtime, so the delivery-preference / connected-channel
    /// slice it resolves lands in the model request. Default `None` (no comm
    /// section, matching today's behavior).
    communication_context_provider: Option<Arc<dyn CommunicationContextProvider>>,
    /// C-HOOKS / E-HOOK-INFRA: an optional per-run hook dispatcher builder
    /// factory wired into the group's ONE planned runtime, so hooks fire at the
    /// lifecycle points on a coordinator-path turn. Default `None` (hook
    /// framework dormant, matching today's behavior).
    hook_dispatcher_builder_factory: Option<HookDispatcherBuilderFactory>,
    /// Lease-wedge coverage: overrides the turn-state store's
    /// `runner_lease_ttl` (default 90s) when set. Builder method lives in
    /// `group_options.rs`. Default `None` (today's behavior, byte-identical).
    runner_lease_ttl_override: Option<chrono::Duration>,
    /// Lease-wedge coverage: overrides the scheduler's
    /// `lease_recovery_interval` (default 10s) when set. Builder method lives
    /// in `group_options.rs`. Default `None` (today's behavior, byte-identical).
    lease_recovery_interval_override: Option<Duration>,
    /// When `true`, wire the REAL approval/auth interaction services into
    /// every thread's `DefaultProductWorkflow` (see
    /// `with_real_gate_dispatch_services`). Default `false` (every workflow
    /// keeps the `Rejecting*InteractionService` stubs, matching today's
    /// behavior byte-for-byte).
    real_gate_dispatch_services: bool,
}

impl RebornIntegrationGroupBuilder {
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

        // Issue #5476 lease-wedge coverage: `.with_limits` is the store's own
        // public builder method (`ironclaw_turns::filesystem_store`); this only
        // calls it a second time with a shortened `runner_lease_ttl` when a test
        // opts in via `with_runner_lease_ttl_for_test`. `None` (default) leaves
        // `InMemoryTurnStateStoreLimits::default()` untouched, byte-identical to
        // today's behavior.
        let mut turn_state_limits = InMemoryTurnStateStoreLimits::default();
        if let Some(ttl) = self.runner_lease_ttl_override {
            turn_state_limits.runner_lease_ttl = ttl;
        }
        let turns_scoped_fs =
            scoped_turns_fs_composite(Arc::clone(&base.composite), &base.canonical_binding)?;
        let turn_store: Arc<FilesystemTurnStateStore<HarnessTurnBackend>> = Arc::new(
            FilesystemTurnStateStore::new(Arc::clone(&turns_scoped_fs))
                .with_limits(turn_state_limits),
        );
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
        ) = capability.mode().into_parts(
            milestone_sink.clone(),
            group_thread_harness.service.clone() as Arc<dyn SessionThreadService>,
        )?;

        // Enabler (b): production resolves `CapabilityAllowSet::All` for a
        // top-level user turn, making `CapabilitySurfaceProfileFilter` a no-op
        // — so the disclosure decorator's synthetic bridge ids
        // (`ironclaw.tool_search` etc., never in any granted set) survive to
        // the model. The harness default (allowlist of exactly the granted
        // capability ids) is NARROWER than production there and would strip
        // the deferred bridge surface down to zero tools. Mirror production
        // for bridged groups only; every non-bridged group keeps the strict
        // allowlist.
        let capability_surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver> =
            if self.tool_disclosure == Some(ToolDisclosureMode::Bridged) {
                Arc::new(StaticCapabilitySurfaceProfileResolver {
                    allow_set: CapabilityAllowSet::All,
                })
            } else {
                capability_surface_resolver
            };

        // --- loop-exit evidence (group-level, built once) -----------------
        // `.with_checkpoint_state_store` is the de-mask fix: without it a
        // genuinely-`Failed` run is reported as the masking
        // `driver_protocol_violation` instead of its true failure category.
        // Same shared `ScopedFilesystem` handle the turn store uses (`/turns`
        // mount) — the await-edge tree lives at
        // `/turns/subagent-await-edges/...`, a sibling prefix, per §4.5a's
        // "one shared handle, never a per-store fixed view" rule.
        let await_edge_store =
            Arc::new(FilesystemAwaitEdgeStore::new(Arc::clone(&turns_scoped_fs)));
        let await_edge_goal_store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
        let await_edge_resolver = Arc::new(AwaitEdgeResolver::new_unbound(
            Arc::clone(&await_edge_store),
            await_edge_goal_store.clone() as Arc<dyn ironclaw_loop_support::SubagentSpawnGoalStore>,
            turn_store.clone() as Arc<dyn ironclaw_turns::TurnSpawnTreeStateStore>,
            capability_result_writer.clone(),
            group_thread_harness.service.clone(),
        ));
        let await_edge_driver = Arc::new(ScopeRecoveryDriver::new(
            Arc::clone(&await_edge_resolver),
            Arc::clone(&await_edge_store),
        ));
        let turn_state_for_evidence: Arc<dyn TurnStateStore> = turn_store.clone();
        let mut evidence = ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
            group_thread_harness.service.clone(),
            turn_state_for_evidence,
            Arc::clone(&loop_checkpoint_store),
            Arc::clone(&await_edge_store)
                as Arc<dyn ironclaw_runner::loop_exit_applier::AwaitDependentRunEvidenceStore>,
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

        // --- trace capture (enabler (c), C-TRACECAP) ------------------------
        // The PRODUCTION TraceCaptureTurnEventSink over the group's thread
        // service, seeded with the runtime owner's trace scope — the same
        // recipe `build_reborn_runtime` uses. Policy-gated per scope, so it
        // is inert until the test enrolls the scope. The factory returns the
        // scope it seeded the sink with directly — this is the ONE source of
        // truth for that scope; do not recompute `trace_scope_key` here too
        // (a second, independent computation could silently drift from what
        // the sink actually observes if either recipe changes).
        let trace_capture = if self.trace_capture {
            let subject_user = base.canonical_subject_user()?;
            let (sink, scope) =
                ironclaw_reborn_composition::test_support::trace_capture_turn_event_sink_for_test(
                    group_thread_harness.service.clone() as Arc<dyn SessionThreadService>,
                    base.canonical_binding.tenant_id.as_str(),
                    subject_user.as_str(),
                );
            Some((sink, scope))
        } else {
            None
        };
        // The planned runtime has ONE turn-event-sink slot; compose the two
        // opt-in sinks through the fan-out only when both are present so
        // single-sink groups keep today's wiring byte-for-byte.
        let mut turn_event_sinks: Vec<Arc<dyn TurnEventSink>> = Vec::new();
        if let Some(sink) = self.turn_event_sink.clone() {
            turn_event_sinks.push(sink as Arc<dyn TurnEventSink>);
        }
        if let Some((sink, _)) = &trace_capture {
            turn_event_sinks.push(Arc::clone(sink));
        }
        let composed_turn_event_sink: Option<Arc<dyn TurnEventSink>> = match turn_event_sinks.len()
        {
            0 | 1 => turn_event_sinks.pop(),
            _ => Some(Arc::new(FanOutTurnEventSink(turn_event_sinks))),
        };

        // --- the group's ONE planned runtime -------------------------------
        let turn_state_for_runtime: Arc<dyn RuntimeTurnStateStore> = turn_store.clone();
        let model_gateway: Arc<dyn HostManagedModelGateway> =
            Arc::clone(&scope_gateway) as Arc<dyn HostManagedModelGateway>;
        let user_profile_source: Arc<dyn HostUserProfileSource> =
            ironclaw_reborn_composition::test_support::build_user_profile_source_for_test(
                capability_recorder.profile_filesystem(),
            );

        // --- C-BUDGET: production budget accountant (wiring-liveness only) -----
        // Build the SAME `GovernorBackedAccountant` production composes, via the
        // shared `build_default_budget_accountant` helper, over in-memory leaf
        // ports + compiled-default seeding. Retain the governor + the run-owner
        // account so `assert_budget_user_cap_seeded` can read back the daily cap
        // the accountant seeds on the turn's first model call. Built here (not
        // per-thread) because the group's ONE planned runtime is assembled once.
        // The governor/account are stashed independent of the struct field so a
        // mutation that drops `model_budget_accountant` (setting it `None`) still
        // has a governor to read — surfacing "never seeded" (RED), not a panic.
        let (budget_accountant, budget_governor, budget_account) = if self.budget {
            let governor: Arc<InMemoryResourceGovernor> = Arc::new(InMemoryResourceGovernor::new());
            let accountant = build_default_budget_accountant(
                Arc::clone(&governor) as Arc<dyn ResourceGovernor>,
                Arc::new(ZeroCostTable) as Arc<dyn ModelCostTable>,
                Arc::new(InMemoryBudgetGateStore::new()) as Arc<dyn BudgetGateStore>,
                Arc::new(InMemoryBudgetEventSink::new()) as Arc<dyn BudgetEventSink>,
                &BudgetDefaults::compiled_defaults(),
            );
            let account = ResourceAccount::user(
                base.canonical_binding.tenant_id.clone(),
                base.canonical_subject_user()?,
            );
            (Some(accountant), Some(governor), Some(account))
        } else {
            (None, None, None)
        };

        // W5-WIRING-PARITY: bind the literal to a local before consuming it so
        // `harness_planned_runtime_parts_shape` can read the REAL Some/None
        // shape this group's runtime is built from — the only place this
        // struct value exists before `build_default_planned_runtime` takes it
        // by value.
        let milestone_sink_for_assertions = Arc::clone(&milestone_sink);
        let parts = DefaultPlannedRuntimeParts {
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
            subagent_goal_store: await_edge_goal_store,
            subagent_await_edge_writer: await_edge_driver
                as Arc<dyn ironclaw_loop_support::AwaitEdgeWriter>,
            subagent_await_edge_settler: await_edge_resolver
                as Arc<dyn ironclaw_loop_support::AwaitEdgeSettler>,
            subagent_await_edge_evidence: await_edge_store
                as Arc<dyn ironclaw_runner::loop_exit_applier::AwaitDependentRunEvidenceStore>,
            subagent_definition_resolver: Arc::new(StaticSubagentDefinitionResolver),
            subagent_spawn_input_codec: Arc::new(JsonSpawnSubagentInputCodec::new(
                capability_input_resolver,
            )),
            subagent_spawn_limits: SubagentSpawnLimits::default(),
            loop_exit_evidence,
            config: DefaultPlannedRuntimeConfig {
                poll_interval: Duration::from_millis(10),
                lease_recovery_interval: self
                    .lease_recovery_interval_override
                    .unwrap_or(DefaultPlannedRuntimeConfig::default().lease_recovery_interval),
                // Enabler (b): explicit builder opt-in wins; otherwise resolve
                // via `from_env()` exactly like `DefaultPlannedRuntimeConfig`'s
                // own `Default` impl — never mutate the process env from a
                // test (see `ToolDisclosureMode::from_env` doc, `apply_hermetic_env`).
                tool_disclosure: self
                    .tool_disclosure
                    .unwrap_or_else(ToolDisclosureMode::from_env),
                ..DefaultPlannedRuntimeConfig::default()
            },
            model_route_resolver: None,
            // E-GATEWAY: left `None` — it does not gate whether a run reaches
            // `Cancelled`. `RebornLoopDriverHostFactory` always builds its own
            // default `TurnStateRunCancellationFactory`, whose cancel poll loop
            // drives a parked run to `Cancelled` on resume regardless (verified
            // by `reborn_integration_cancel`). Supplying one here would only add
            // the product-live wake-notifier fan-out, unexercised by this test.
            cancellation_factory: None,
            // E-SKILL: wire the local-dev skill context source so an activated
            // skill's instructions inject into the model request. `Some` only for
            // `skill_activation_tools()` harnesses; `None` for every other backend,
            // so all existing group tests are behavior-identical (production wires
            // this in `build_reborn_runtime`, runtime.rs ~2875).
            skill_context_source: capability_recorder.skill_context_source(),
            input_queue: None,
            identity_context_source: Arc::new(EmptyIdentityContextSource),
            // E-PROFILE: HostRuntime mode backs this with the local-dev memory
            // filesystem so `profile_set` writes read back; other backends fall
            // back to `EmptyUserProfileSource`. Built as a local (not inline) so
            // the SAME `Arc` is also stashed on `GroupSharedStorage` for a
            // profile-round-trip test to read directly.
            user_profile_source: Arc::clone(&user_profile_source),
            model_policy_guard: None,
            // C-BUDGET: production `build_default_budget_accountant` (Some only
            // for `budget_accounting()` groups; `None` otherwise, so all existing
            // group/flat tests are behavior-identical).
            model_budget_accountant: budget_accountant,
            safety_context: self.safety_context,
            // C-HOOKS / E-HOOK-INFRA: per-run hook dispatcher builder factory
            // (Some only when `hook_dispatcher_builder_factory()` was set).
            hook_dispatcher_builder_factory: self.hook_dispatcher_builder_factory,
            // C-COMMCTX: delivery-preference / connected-channel provider (Some
            // only when `communication_context_provider()` was set).
            communication_context_provider: self.communication_context_provider,
            // No RecordingSecurityAuditSink double exists yet (nearai/ironclaw#5640);
            // wiring_parity.rs's ALLOWED_DIVERGENCES tracks this field by name, not line.
            hook_security_audit_sink: None,
            turn_event_sink: composed_turn_event_sink,
            attachment_read_port: capability_recorder
                .attachment_test_support()
                .map(|support| support.read_port),
            scheduler_wake_wiring: None,
        };
        let planned_runtime_parts_shape = harness_planned_runtime_parts_shape(&parts);
        let composition = build_default_planned_runtime(parts)?;

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
                canonical_binding: base.canonical_binding,
                capability_recorder,
                user_profile_source,
                turn_event_sink: self.turn_event_sink,
                milestone_sink: milestone_sink_for_assertions,
                trace_capture_scope: trace_capture.map(|(_, scope)| scope),
                budget_governor,
                budget_account,
                planned_runtime_parts_shape,
                real_gate_dispatch_services: self.real_gate_dispatch_services,
            }),
        })
    }
}

/// Fan-out `TurnEventSink`: the planned runtime exposes ONE sink slot
/// (`DefaultPlannedRuntimeParts.turn_event_sink`), so `.with_turn_event_sink()`
/// (in-memory recorder) and `.with_trace_capture()` (production trace sink)
/// compose through this when both are opted in. Test-local because
/// production's equivalent (`CompositeTurnEventSink`) is `pub(crate)` inside
/// composition.
struct FanOutTurnEventSink(Vec<Arc<dyn TurnEventSink>>);

#[async_trait::async_trait]
impl TurnEventSink for FanOutTurnEventSink {
    /// Publishes to every sink unconditionally — a failing sink must not
    /// short-circuit the others (e.g. the in-memory recorder must still see
    /// the event even if the trace-capture sink errors, and vice versa).
    /// Returns the FIRST error only after every sink has been attempted.
    async fn publish(
        &self,
        event: ironclaw_turns::TurnLifecycleEvent,
    ) -> Result<(), ironclaw_turns::TurnError> {
        let mut first_error = None;
        for sink in &self.0 {
            if let Err(error) = sink.publish(event.clone()).await {
                first_error.get_or_insert(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
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
    /// group's default `HARNESS_ACTOR_ID` (E-MULTIUSER seam), so per-turn
    /// owner-scope resolution isolates this thread's reads/writes under their
    /// own subtree (keyed on the resolved canonical `UserId`, not the raw
    /// `actor_id` string). Unset keeps the default `HARNESS_ACTOR_ID` behavior.
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
        // other's LLM session cache under the same `turn_root`. Retain the
        // concrete `TraceLlm` before the `dyn LlmProvider` upcast so tests can
        // inspect captured requests via `captured_requests()`.
        //
        // E-GATEWAY: the `TraceLlm` is built unconditionally first; a park gate
        // wraps it in a parking provider at the SAME vendor-SDK seam (decorator
        // chain still runs on top), so captured requests stay inspectable either
        // way.
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
        let baseline_milestone_count = shared.milestone_sink.milestones().len();

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
        let mut workflow = DefaultProductWorkflow::new(inbound, ledger, binding_service);

        // Real gate-dispatch seam: wire the harness's own local-dev interaction
        // services, but over the GROUP's shared `turn_store` (not the harness's
        // own disjoint `local_runtime.turn_state`) — otherwise their turn-run
        // locator can never see this group's real runs. Only when the builder
        // opted in (`with_real_gate_dispatch_services`); every other group's
        // workflow keeps the default Rejecting stubs.
        if shared.real_gate_dispatch_services {
            let harness = match &shared.capability {
                GroupCapability::HostRuntime(arc) => arc,
                GroupCapability::Recording => {
                    return Err(
                        "with_real_gate_dispatch_services requires a HostRuntime capability backend"
                            .into(),
                    );
                }
            };
            let reborn_services = harness.reborn_services_for_test().ok_or(
                "with_real_gate_dispatch_services requires a harness built via new_with_options",
            )?;
            let approval_interaction_service = reborn_services
                .local_dev_approval_interaction_service_with_turn_state_for_test(
                    Arc::clone(&shared.coordinator),
                    Arc::clone(&shared.turn_store),
                )?
                .ok_or(
                    "local-dev approval interaction service unavailable (harness has no local runtime)",
                )?;
            let auth_interaction_service = reborn_services
                .local_dev_auth_interaction_service_with_turn_state_for_test(
                    Arc::clone(&shared.coordinator),
                    Arc::clone(&shared.turn_store),
                )
                .ok_or(
                    "local-dev auth interaction service unavailable (harness has no local runtime)",
                )?;
            workflow = workflow
                .with_approval_interaction_service(approval_interaction_service)
                .with_auth_interaction_service(auth_interaction_service);
        }

        // Register the gateway only now that every fallible (`?`) step above has
        // succeeded — registering earlier risks leaving the scope registered
        // for a harness that never finished building (a later `?` bailing out
        // would make a retry hit the duplicate-registration panic).
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
            baseline_milestone_count,
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
