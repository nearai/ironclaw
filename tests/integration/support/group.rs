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

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use ironclaw_assistant::{
    DefaultInboundTurnService, DefaultProductSurface, IdempotencyLedger, InboundTurnService,
};
use ironclaw_composition::RebornTrajectoryObserver;
use ironclaw_composition::build_default_budget_accountant;
use ironclaw_composition::test_support::ChannelConnectionTestBundle;
use ironclaw_config::BudgetDefaults;
use ironclaw_event_log::{DurableEventLog, NonBlockingEventSink};
use ironclaw_event_store::{CoalescingEventSink, EventBatchConfig};
use ironclaw_extension_contracts::channel_adapter::ProductTriggerReason;
use ironclaw_extension_registry::ExtensionInstallationStorePort;
use ironclaw_filesystem::CompositeRootFilesystem;
use ironclaw_host_api::{
    capability_surface::CapabilitySurfacePolicy, ids::UserId, resource::ResourceScope,
};
use ironclaw_llm::testing::{provider_chain_over, provider_chain_over_with_fallback};
use ironclaw_llm::{LlmProvider, SessionConfig, create_session_manager};
use ironclaw_loop_contracts::{
    CommunicationContextProvider, InMemoryLoopHostMilestoneSink, InstructionSafetyContext,
    LoopHostMilestone, LoopHostMilestoneSink, ModelProfileId,
};
use ironclaw_loop_host::ToolDisclosureMode;
use ironclaw_loop_host::{
    CapabilitySurfaceProfileResolver, HostManagedModelGateway, HostUserProfileSource,
    JsonSpawnSubagentInputCodec, ModelCostTable, SubagentSpawnLimits, ZeroCostTable,
};
use ironclaw_loop_host::{LlmModelProfilePolicy, LlmProviderModelGateway};
use ironclaw_product_contracts::binding::ProductBindingResolver;
use ironclaw_product_contracts::binding::ResolvedBinding;
use ironclaw_resources::test_support::in_memory_backed_budget_gate_store;
use ironclaw_resources::{
    BudgetEventSink, BudgetGateStorePort, InMemoryBudgetEventSink, InMemoryResourceGovernor,
    ResourceAccount, ResourceGovernor,
};
use ironclaw_threads::SessionThreadService;
use ironclaw_turn_runner::loop_driver_host::HookDispatcherBuilderFactory;
use ironclaw_turn_runner::loop_exit_applier::ThreadCheckpointLoopExitEvidencePort;
use ironclaw_turn_runner::milestone_events::{
    DurableLoopHostMilestoneScope, DurableLoopHostMilestoneSink,
};
use ironclaw_turn_runner::runtime::{
    DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts, ProcessRuntimeSystem,
    build_default_planned_runtime,
};
use ironclaw_turn_runner::subagent::{
    await_edge::{
        boot_recovery::ScopeRecoveryDriver, resolver::AwaitEdgeResolver, store::AwaitEdgeStore,
    },
    flavors::StaticSubagentDefinitionResolver,
};
use ironclaw_turn_runner::turn_scheduler::TurnRunSchedulerHandle;
use ironclaw_turns::loop_exit::LoopExitEvidencePort;
use ironclaw_turns::{
    AgentTurnProcessRuntime, AgentTurnRuntimePort, InMemoryTurnEventSink, LoopCheckpointStore,
    ProcessLoopCheckpointStore, TurnCoordinator, TurnEventSink, TurnScope,
};

use super::builder::{
    HARNESS_ACTOR_ID, INTERACTIVE_MODEL_PROFILE, RebornIntegrationHarness, StorageMode,
    apply_hermetic_env, binding_request, build_storage_composite, scoped_processes_fs_composite,
    thread_scope_from_binding,
};
use super::doubles::{FailingTranscriptWriteThreadService, RecordingSecurityAuditSink};
use super::harness::{
    EmptyIdentityContextSource, HarnessCapabilityMode, HarnessCapabilityRecorder,
    HostRuntimeCapabilityHarness, RecordingTestCapabilityPort,
    StaticCapabilitySurfaceProfileResolver, test_product_scope,
};
use super::planned_runtime_parts_shape::{
    DefaultPlannedRuntimePartsShape, harness_planned_runtime_parts_shape,
};
use super::product_surface::RebornProductSurfaceHarness;
use super::reply::RebornScriptedReply;
use super::scope_gateway::ScopeRegistryGateway;
use super::scripted_provider::{
    ErrLlm, ErrLlmKind, FallbackProviderCallProbe, ModelProviderCallProbe, ParkingModelGate,
    RecoverableModelFailureScript, SCRIPTED_FALLBACK_MODEL_NAME, SCRIPTED_MODEL_NAME,
    delayed_trace_llm, parking_trace_llm, recording_llm, recoverable_failure_trace_llm,
    scripted_fallback_vendor_pair, scripted_trace_llm,
};
use super::session_thread::RebornThreadHarness;
use super::test_adapter::RebornTestIngress;
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
/// `with_narrowed_capability_surface_policy_for_bridged_test`, `budget_accounting`,
/// `communication_context_provider`,
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
    /// Exact runtime-wiring recipe consumed by `restart_planned_runtime`.
    ///
    /// Retaining the configured builder prevents the restart harness from
    /// silently falling back to defaults and thereby testing a different
    /// runtime from the one that admitted the run.
    pub(crate) restart_builder: RebornIntegrationGroupBuilder,
    /// Thread history + turn state composite, shared across all threads.
    pub(crate) composite: Arc<CompositeRootFilesystem>,
    /// Fresh-connection reopen handle per storage mode (SQLite file path /
    /// Postgres URL + live container). Used by
    /// `assert_reply_persists_after_reopen`.
    pub(crate) storage_reopen: super::builder::StorageReopen,
    /// Durable root TempDir: keeps the composite's on-disk files alive for
    /// the group's lifetime. `Drop` deletes the directory (req 3).
    pub(crate) turn_root: Arc<tempfile::TempDir>,
    /// Product-workflow harness (binding service + idempotency ledger).
    /// Shared so all threads resolve bindings within the same product context.
    /// `product_harness.scope` is the single-source `ResourceScope` (R5).
    pub(crate) product_harness: RebornProductSurfaceHarness,
    /// Capability backend. Groups use `HostRuntime`; the degenerate single-shot
    /// path may use `Recording`.
    pub(crate) capability: GroupCapability,
    /// C-SLACK-LIFECYCLE (issue #6105): the REAL generic channel-connection
    /// service + OAuth-callback-shaped connect handles, built over the
    /// capability harness's own `RebornServices` (same durable stores, same
    /// late-bound cleanup slot `extension_remove` dispatches to).
    /// `Some` only for `extension_lifecycle()` groups.
    pub(crate) channel_connection: Option<Arc<ChannelConnectionTestBundle>>,
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
    /// The group's single authoritative process runtime.
    pub(crate) process_system: ProcessRuntimeSystem,
    /// Agent-turn query/projection facade over `process_system`.
    pub(crate) turn_runtime: Arc<AgentTurnProcessRuntime>,
    /// S2 seam: the SAME canonical binding process journal is scoped to.
    /// Retained so a reopen can
    /// rebuild the identical scoped path independently, instead of
    /// re-deriving it from a second binding resolution.
    pub(crate) canonical_binding: ResolvedBinding,
    /// The group's single capability recorder, shared by `Arc` with the real
    /// capability factory wired into the one planned runtime. Every thread
    /// clones this (cheap — `HarnessCapabilityRecorder` is `Clone` over
    /// `Arc`-wrapped inner state) and slices `[baseline_*..]` so assertions
    /// only see that thread's own deltas (R2).
    pub(crate) capability_recorder: HarnessCapabilityRecorder,
    /// The steering/follow-up input queue wired into the group's ONE planned
    /// runtime (`parts.input_queue`). Every thread's `DefaultInboundTurnService`
    /// enqueues busy-thread messages through this SAME instance, mirroring
    /// production composition.
    pub(crate) input_enqueue: Arc<dyn ironclaw_loop_host::HostInputEnqueuePort>,
    /// The exact `HostUserProfileSource` wired into the group's ONE planned
    /// runtime (E-PROFILE seam). Kept so a profile-round-trip test reads from
    /// the SAME instance the running loop uses, not a re-derived equivalent —
    /// catches wiring mutations, not just the builder itself.
    pub(crate) user_profile_source: Arc<dyn HostUserProfileSource>,
    /// In-memory turn-lifecycle event sink wired in when `.with_turn_event_sink()`
    /// opted in (C-TRACECAP seam); `None` otherwise. Concrete type (not `Arc<dyn
    /// TurnEventSink>`) so a test can read `.events()` back directly.
    pub(crate) turn_event_sink: Option<Arc<InMemoryTurnEventSink>>,
    /// Production RootFilesystem-backed event log used by the durable loop
    /// milestone sink when the measured workload opts in.
    pub(crate) durable_event_log: Option<Arc<dyn DurableEventLog>>,
    /// Production-shaped non-blocking writer for `durable_event_log`. Retained
    /// so read-back assertions can flush accepted events before replay.
    pub(crate) durable_event_sink: Option<Arc<dyn NonBlockingEventSink>>,
    /// W5-WIRING-PARITY: production local-dev always wires a security-audit
    /// sink; the harness mirrors that shape with a recording sink so tests can
    /// assert events emitted through real caller paths.
    pub(crate) security_audit_sink: Arc<RecordingSecurityAuditSink>,
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
            GroupCapability::Recording
            | GroupCapability::RecordingNoProgress
            | GroupCapability::RecordingRecoverablePortError => None,
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
            GroupCapability::Recording
            | GroupCapability::RecordingNoProgress
            | GroupCapability::RecordingRecoverablePortError => None,
        }
    }
}

// ---------------------------------------------------------------------------
// GroupCapability
// ---------------------------------------------------------------------------

/// Shared capability backend for a group. Groups always use `HostRuntime`
/// (sharing the approval/memory/credential stores across threads). The
/// recording variants are single-shot echo paths for text-only turns.
pub(crate) enum GroupCapability {
    /// Echo recorder — records invocations, executes nothing. Default for a
    /// text-only single-shot harness; no stores to share.
    Recording,
    /// Recording echo whose results deliberately report `NoChange`.
    RecordingNoProgress,
    /// Recording echo whose port returns a caller-shaped `InvalidInvocation`
    /// error instead of a resolution, projecting to `FailureKind::InputEncode`
    /// (#6284 capability-stage contract).
    RecordingRecoverablePortError,
    /// Real first-party or MCP host runtime, shared across all threads.
    /// All approval/auto-approve/credential/memory state is common because the
    /// `Arc` is cloned per thread.
    HostRuntime(Arc<HostRuntimeCapabilityHarness>),
}

impl GroupCapability {
    /// Return a fresh `HarnessCapabilityMode` for one thread.
    ///
    /// Recording variants create a fresh echo port each call (ports are
    /// consumed by `into_parts`). `HostRuntime` clones the `Arc` — N threads
    /// share the same underlying harness and all its stores.
    pub(crate) fn mode(&self) -> HarnessCapabilityMode {
        match self {
            Self::Recording => {
                HarnessCapabilityMode::Recording(RecordingTestCapabilityPort::echo())
            }
            Self::RecordingNoProgress => {
                HarnessCapabilityMode::Recording(RecordingTestCapabilityPort::no_progress())
            }
            Self::RecordingRecoverablePortError => HarnessCapabilityMode::Recording(
                RecordingTestCapabilityPort::recoverable_port_error(),
            ),
            Self::HostRuntime(arc) => HarnessCapabilityMode::HostRuntime(Arc::clone(arc)),
        }
    }

    /// The durable gate-record store this backend's capability port persists
    /// `GateRecord::Auth` into (§5.2.9) — the SAME `Arc` the turn executor must
    /// re-read an auth block's `credential_requirements` from. Recording
    /// backends return `None`; the host-runtime backend always resolves a store
    /// (`HostRuntimeCapabilityHarness::gate_record_store` returns `Some`).
    pub(crate) fn gate_record_store(
        &self,
    ) -> Option<Arc<dyn ironclaw_approvals::GateRecordStorePort>> {
        match self {
            Self::HostRuntime(harness) => harness.gate_record_store(),
            Self::Recording | Self::RecordingNoProgress | Self::RecordingRecoverablePortError => {
                None
            }
        }
    }

    /// Return the same reply-attachment intent port used by a
    /// production-composed built-in handler. The planned runtime finalizer
    /// must seal that exact store; lightweight backends without composed
    /// Reborn services retain the prior isolated in-memory test store.
    pub(crate) fn reply_attachment_intent_port(
        &self,
    ) -> Arc<dyn ironclaw_outbound::ReplyAttachmentIntentPort> {
        let fresh_store = || {
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store())
                as Arc<dyn ironclaw_outbound::ReplyAttachmentIntentPort>
        };
        match self {
            Self::HostRuntime(harness) => harness
                .reborn_services_for_test()
                .and_then(|runtime| runtime.outbound_delivery_stores_for_test())
                .map(|(_, _, _, reply_attachment_intents, _)| reply_attachment_intents)
                .unwrap_or_else(fresh_store),
            Self::Recording | Self::RecordingNoProgress | Self::RecordingRecoverablePortError => {
                fresh_store()
            }
        }
    }

    /// E-DURABLE core: assert `extension_id` is present in a FRESHLY reopened
    /// `ExtensionInstallationStorePort` at this backend's on-disk `storage_root`
    /// (a handle independent of the live `Arc`) — proving the install
    /// persisted to disk, not just to in-memory state. One implementation
    /// behind both the harness- and group-level
    /// `assert_extension_install_persists_after_reopen` so the reopen shape
    /// and the `seen` diagnostics cannot drift.
    pub(crate) async fn assert_extension_install_persists_after_reopen(
        &self,
        extension_id: &str,
    ) -> HarnessResult<()> {
        let harness = match self {
            Self::HostRuntime(arc) => arc,
            Self::Recording | Self::RecordingNoProgress | Self::RecordingRecoverablePortError => {
                return Err("no host-runtime capability backend for durable reopen".into());
            }
        };
        let store =
            ironclaw_composition::test_support::open_standalone_extension_installation_store_for_test(
                &harness.storage_root_for_test(),
            )
            .await?;
        let installations = store.list_installations().await?;
        if installations
            .iter()
            .any(|installation| installation.extension_id().as_str() == extension_id)
        {
            return Ok(());
        }
        let seen: Vec<&str> = installations
            .iter()
            .map(|installation| installation.extension_id().as_str())
            .collect();
        Err(
            format!("extension {extension_id:?} not found after independent reopen; saw {seen:?}")
                .into(),
        )
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
            durable_milestone_event_store: false,
            // General integration groups stay hermetic across production
            // default changes. Disclosure-specific tests opt into Bridged.
            tool_disclosure: ToolDisclosureMode::Off,
            narrowed_bridged_policy: None,
            budget: false,
            communication_context_provider: None,
            hook_dispatcher_builder_factory: None,
            trajectory_observer: None,
            runner_lease_ttl_override: None,
            lease_recovery_interval_override: None,
            planned_default_iteration_limit: None,
            runner_heartbeat_interval_override: None,
            fail_append_finalized_assistant_message: false,
            fail_append_tool_result_reference: false,
            real_gate_dispatch_services: false,
            channel_connection: None,
            bound_memory: None,
        }
    }

    /// Gracefully stop and rebuild the group's complete planned runtime over a
    /// genuinely fresh LibSQL connection to the same durable process rows.
    ///
    /// This consumes the group and requires every thread harness built from it
    /// to have been dropped first. That requirement is intentional: a surviving
    /// harness owns the old coordinator and would make a "restart" assertion
    /// dishonest. The capability backend is retained because its durable gate
    /// and approval stores model the external host state a restarted runner
    /// reconnects to; the scheduler, coordinator, executor, scope gateway,
    /// checkpoint adapters, and process journal are all reconstructed.
    ///
    /// Only LibSQL is supported because it is the hermetic integration backend
    /// with an independent reopen recipe. Other storage modes fail loudly.
    pub async fn restart_planned_runtime(self) -> HarnessResult<Self> {
        let shared_count = Arc::strong_count(&self.shared);
        let shared = Arc::try_unwrap(self.shared).map_err(|_| {
            format!(
                "restart_planned_runtime requires every thread harness to be dropped; \
                 group shared state still has {shared_count} owners"
            )
        })?;
        let GroupSharedStorage {
            restart_builder,
            storage_reopen,
            turn_root,
            product_harness,
            capability,
            canonical_binding,
            scheduler_handle,
            ..
        } = shared;

        // This awaits cancellation, aborts in-flight executor tasks, and
        // relinquishes claimed runs before a replacement scheduler can claim
        // them. Dropping the handle would only signal cancellation.
        scheduler_handle.shutdown().await;

        let composite = match &storage_reopen {
            super::builder::StorageReopen::LibSql { db_path } => {
                super::builder::reopen_fresh_libsql_composite(db_path).await?
            }
            super::builder::StorageReopen::None => {
                return Err("restart_planned_runtime requires StorageMode::LibSql; \
                     in-memory storage cannot survive a runtime restart"
                    .into());
            }
            super::builder::StorageReopen::Postgres { .. } => {
                return Err(
                    "restart_planned_runtime does not yet have a fresh Postgres \
                     composite reopen recipe"
                        .into(),
                );
            }
        };
        let base = GroupBaseData {
            product_harness,
            composite,
            storage_reopen,
            turn_root,
            canonical_binding,
        };
        restart_builder.into_group(base, capability).await
    }

    /// Enabler (c): the trace scope key the production trace-capture sink was
    /// seeded with; `Some` only after `.with_trace_capture()`. Pair with
    /// `ironclaw_trace_commons::contribution::queued_trace_envelope_paths_for_scope`
    /// to assert an enrolled turn queued a contribution envelope.
    pub fn trace_capture_scope(&self) -> Option<&str> {
        self.shared.trace_capture_scope.as_deref()
    }

    /// C-SLACK-LIFECYCLE (issue #6105): the real generic channel-connection
    /// bundle for this group. `Some` only for [`Self::extension_lifecycle`]
    /// groups.
    pub fn channel_connection(&self) -> Option<Arc<ChannelConnectionTestBundle>> {
        self.shared.channel_connection.clone()
    }

    /// The group-canonical binding's ACTOR user id — the identity capability
    /// dispatch stamps as `authenticated_actor_user_id` on execution contexts
    /// (loop-host capability port reads `run_context.actor()`), and therefore
    /// the caller identity extension-removal channel cleanup disconnects.
    pub fn canonical_actor_user(&self) -> UserId {
        self.shared.canonical_binding.actor_user_id.clone()
    }

    /// Register a hermetic external delivery target on the exact local
    /// outbound registry `builtin.trigger_create` consults. Scenarios pass the
    /// host-sealed reply binding read from an already-submitted source run;
    /// no model-authored id participates in this setup.
    pub fn register_source_delivery_target_for_test(
        &self,
        provider_key: &str,
        target_id: &str,
        reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef,
    ) -> HarnessResult<()> {
        let GroupCapability::HostRuntime(harness) = &self.shared.capability else {
            return Err("source delivery target requires a host-runtime capability backend".into());
        };
        let runtime = harness
            .reborn_services_for_test()
            .ok_or("source delivery target requires composed Reborn runtime")?;
        let target_id = ironclaw_assistant::RebornOutboundDeliveryTargetId::new(target_id)?;
        let display_name = target_id.as_str().to_string();
        // Registry-key the registration by the TARGET id (always unique per
        // call), never by `provider_key`/channel: the registry's
        // `providers.insert` silently REPLACES whatever already sits at a
        // given key (`OutboundDeliveryTargetRegistrationOutcome::Replaced`),
        // so two targets on the SAME channel (e.g. two Slack DMs) registered
        // under the shared channel-named key would leave only the
        // last-registered one resolvable — the first would look
        // unregistered to the model that named it.
        let registry_key = target_id.as_str().to_string();
        runtime.register_static_outbound_delivery_target_for_test(
            registry_key,
            target_id,
            provider_key,
            display_name.as_str(),
            None,
            reply_target_binding_ref,
        )?;
        Ok(())
    }

    /// Register the same real scripted provider chain used by ordinary group
    /// threads for a caller scope materialized from a channel adapter. This
    /// lets channel-origin whole-path tests execute model tool calls on the
    /// exact admitted run instead of pre-writing the side effect under test.
    pub async fn register_scope_script_for_test(
        &self,
        scope: TurnScope,
        session_label: &str,
        replies: impl IntoIterator<Item = RebornScriptedReply>,
    ) -> HarnessResult<Arc<TraceLlm>> {
        let scripted_llm = Arc::new(scripted_trace_llm(replies));
        let raw: Arc<dyn LlmProvider> = scripted_llm.clone();
        let session = create_session_manager(SessionConfig {
            session_path: self
                .shared
                .turn_root
                .path()
                .join(format!("{session_label}.session.json")),
            ..SessionConfig::default()
        })
        .await;
        let llm_config = ironclaw_llm::testing::nearai_test_config(SCRIPTED_MODEL_NAME);
        let provider = provider_chain_over(raw, &llm_config, session).await?;
        let model_profile_id = ModelProfileId::new(INTERACTIVE_MODEL_PROFILE)
            .map_err(|reason| format!("invalid model profile id: {reason}"))?;
        let policy = LlmModelProfilePolicy::new().allow_model_profile(model_profile_id, None);
        let gateway: Arc<dyn HostManagedModelGateway> =
            Arc::new(LlmProviderModelGateway::new(provider, policy));
        self.shared.scope_gateway.register(scope, gateway);
        Ok(scripted_llm)
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
            record_model_calls: false,
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
            GroupCapability::Recording
            | GroupCapability::RecordingNoProgress
            | GroupCapability::RecordingRecoverablePortError => None,
        }
    }

    /// Group-level twin of the harness's
    /// `assert_extension_install_persists_after_reopen`, for scenarios that
    /// assert durable state without building a thread (E-DURABLE / T5).
    pub async fn assert_extension_install_persists_after_reopen(
        &self,
        extension_id: &str,
    ) -> HarnessResult<()> {
        self.shared
            .capability
            .assert_extension_install_persists_after_reopen(extension_id)
            .await
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
    /// `AutoApproveSettingStorePort`. In a `multiuser_approvals` group (built with
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
    /// `AutoApproveSettingStorePort`. Auto-approve defaults ON when a user has no
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
/// Replaces the 4-tuple `(RebornProductSurfaceHarness, Arc<CompositeRootFilesystem>,
/// Option<PathBuf>, Arc<TempDir>)` so each constructor can name fields rather than
/// position-destructure a tuple.
///
/// Plain module-private visibility: `group_constructors.rs` reaches this at
/// plain module-private visibility as a descendant of `group` (see the `mod
/// group_constructors` declaration above), so the fields stay private and the
/// per-capability preset constructors there — including their own
/// `build_group_capability_with_base` helper, which calls
/// `canonical_actor_user()` — take/return this type as the opaque handoff
/// between `build_base` and `into_group`; `build_base`/`into_group` themselves
/// stay module-private too.
struct GroupBaseData {
    product_harness: RebornProductSurfaceHarness,
    composite: Arc<CompositeRootFilesystem>,
    storage_reopen: super::builder::StorageReopen,
    turn_root: Arc<tempfile::TempDir>,
    /// A throwaway probe binding resolved once at group construction, used
    /// ONLY to derive the group-level shared turn store path and the
    /// group-level `ThreadScope`. Every thread in a group shares `(tenant,
    /// agent, project)` — only `thread_id` varies, and `ThreadScope` has no
    /// `thread_id` field — so this binding is a valid stand-in for the whole
    /// group. `group_constructors.rs` reads tenant/actor user off this
    /// field directly (module-private; it's a child module of `group`).
    canonical_binding: ResolvedBinding,
}

impl GroupBaseData {
    /// The canonical binding's actor user id — the hashed `UserId` the actor
    /// `host-user` resolves to (a run acts as the user who invoked it).
    /// `live_approvals` and `profile_tools` both pin their capability
    /// harness's executor user to this so capability dispatch shares the
    /// run's `(tenant, user)` with the turn-store / evidence scope resolved
    /// from the SAME `canonical_binding` (see the `canonical_binding` field
    /// docs above).
    fn canonical_actor_user(&self) -> HarnessResult<UserId> {
        Ok(self.canonical_binding.actor_user_id.clone())
    }
}

/// Builder for `RebornIntegrationGroup` with optional storage mode selection.
/// Obtain via [`RebornIntegrationGroup::builder`]; defaults to
/// `StorageMode::InMemory`.
#[derive(Clone)]
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
    /// Wire the production durable loop-milestone adapter over this group's
    /// selected RootFilesystem.
    durable_milestone_event_store: bool,
    /// Enabler (b): pinned to `Off` for general hermetic tests and changed to
    /// `Bridged` only by `.with_tool_disclosure_bridged()`.
    tool_disclosure: ToolDisclosureMode,
    /// #5647 RED-pin seam: opt-in override of the forced `CapabilitySurfacePolicy::allow_all()`
    /// for Bridged-mode groups. `None` preserves today's behavior; only
    /// consumed when `tool_disclosure == Bridged` (`into_group` fails fast otherwise).
    narrowed_bridged_policy: Option<CapabilitySurfacePolicy>,
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
    /// C-TRAJECTORY: optional observer wired into the group's ONE production
    /// capability-port factory. Default `None`.
    trajectory_observer: Option<Arc<dyn RebornTrajectoryObserver>>,
    /// Lease-wedge coverage: overrides the turn-state store's
    /// `runner_lease_ttl` (default 90s) when set. Builder method lives in
    /// `group_options.rs`. Default `None` (today's behavior, byte-identical).
    runner_lease_ttl_override: Option<chrono::Duration>,
    /// Lease-wedge coverage: overrides the scheduler's
    /// `lease_recovery_interval` (default 10s) when set. Builder method lives
    /// in `group_options.rs`. Default `None` (today's behavior, byte-identical).
    lease_recovery_interval_override: Option<Duration>,
    /// Test-only scheduler heartbeat interval override for measured workloads.
    runner_heartbeat_interval_override: Option<Duration>,
    /// Test-only override for the canonical loop's default iteration limit.
    planned_default_iteration_limit: Option<std::num::NonZeroU32>,
    /// Test-only runtime seam that rejects final assistant transcript writes.
    fail_append_finalized_assistant_message: bool,
    /// Test-only runtime seam that rejects tool-result transcript writes.
    fail_append_tool_result_reference: bool,
    /// When `true`, wire the REAL approval/auth interaction services into
    /// every thread's `DefaultProductSurface` (see
    /// `with_real_gate_dispatch_services`). Default `false` (every workflow
    /// keeps the `Rejecting*InteractionService` stubs, matching today's
    /// behavior byte-for-byte).
    real_gate_dispatch_services: bool,
    /// C-SLACK-LIFECYCLE (issue #6105): the real generic channel-connection
    /// bundle built over the capability harness's own `RebornServices`.
    /// Set by `extension_lifecycle()` before `into_group`; `None` for every
    /// other constructor.
    channel_connection: Option<Arc<ChannelConnectionTestBundle>>,
    /// E-MEMORY: a bound memory provider + the lifecycle set its manifest
    /// declares. When set, `into_group` derives the three memory consumers
    /// (prompt-context service, after-turn writer, profile source) through
    /// the PRODUCTION decision helper
    /// (`ironclaw_composition::memory_lifecycle_consumers`), so the
    /// integration tier drives the same lifecycle gating runtime assembly
    /// wires. Default `None` (no memory consumers, today's behavior).
    bound_memory: Option<(
        Arc<dyn ironclaw_memory::MemoryService>,
        ironclaw_extension_contracts::memory::MemoryDescriptor,
    )>,
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
        let product_harness = RebornProductSurfaceHarness::filesystem_temp(scope)?;
        let turn_root = Arc::new(tempfile::tempdir()?);
        let (composite, storage_reopen) =
            build_storage_composite(self.storage, turn_root.path()).await?;

        // Resolve the group-canonical binding ONCE here so `into_group` can
        // build the single shared turn store and evidence-port `ThreadScope`
        // before any per-thread binding exists. This is the SINGLE canonical
        // resolution for the group: `live_approvals` reuses
        // `canonical_binding.actor_user_id` for its capability user rather than
        // probing a second time, so turn-store scope and approval user can't
        // drift. The probe persists one deterministic, inert binding for
        // `conv-canonical-probe` (no thread submits turns against it); group
        // tests assert on cross-thread persistence, not binding counts.
        let ingress = RebornTestIngress::new("reborn-itest", "itest-install")?;
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
            storage_reopen,
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
    /// recorder. Wires checkpoint evidence through the process journal and
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
        let restart_builder = self.clone();
        // Harness-seam misuse guard (§7): fail fast instead of a silent no-op
        // if the override is set without Bridged mode also selected.
        if self.narrowed_bridged_policy.is_some()
            && self.tool_disclosure != ToolDisclosureMode::Bridged
        {
            return Err(
                "with_narrowed_capability_surface_policy_for_bridged_test() was set but \
                 tool_disclosure is not Bridged — the override only applies to \
                 bridged-disclosure groups; call .with_tool_disclosure_bridged() too"
                    .into(),
            );
        }

        let scope_gateway = Arc::new(ScopeRegistryGateway::new());

        let processes_scoped_fs =
            scoped_processes_fs_composite(Arc::clone(&base.composite), &base.canonical_binding)?;
        let mut process_store =
            ironclaw_processes::ProcessJournalStore::new(Arc::clone(&processes_scoped_fs));
        if let Some(ttl) = self.runner_lease_ttl_override {
            process_store = process_store.with_lease_duration(
                ttl.to_std()
                    .map_err(|error| format!("invalid runner lease TTL: {error}"))?,
            );
        }
        let process_system =
            ProcessRuntimeSystem::from_process_journal_store(Arc::new(process_store));
        let turn_runtime = Arc::new(process_system.agent_turn_runtime());
        let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> = Arc::new(
            ProcessLoopCheckpointStore::new(process_system.checkpoints()),
        );
        let group_thread_scope = thread_scope_from_binding(&base.canonical_binding)?;
        let group_thread_harness = RebornThreadHarness::filesystem_shared_composite(
            group_thread_scope.clone(),
            Arc::clone(&base.composite),
            Arc::clone(&base.turn_root),
        )?;

        let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
        let (durable_event_log, durable_event_sink) = if self.durable_milestone_event_store {
            let event_log = ironclaw_event_store::build_reborn_event_stores_from_root_filesystem(
                Arc::clone(&base.composite),
            )?
            .events;
            let event_sink: Arc<dyn NonBlockingEventSink> = Arc::new(CoalescingEventSink::new(
                Arc::clone(&event_log),
                EventBatchConfig::default(),
            ));
            (Some(event_log), Some(event_sink))
        } else {
            (None, None)
        };
        let runtime_milestone_sink: Arc<dyn LoopHostMilestoneSink> =
            if let Some(event_sink) = &durable_event_sink {
                let durable_scope =
                    DurableLoopHostMilestoneScope::from_thread_scope(&group_thread_scope)?;
                Arc::new(FanOutLoopHostMilestoneSink(vec![
                    milestone_sink.clone() as Arc<dyn LoopHostMilestoneSink>,
                    Arc::new(DurableLoopHostMilestoneSink::new(
                        Arc::clone(event_sink),
                        durable_scope,
                    )),
                ]))
            } else {
                milestone_sink.clone()
            };
        let (
            capability_factory,
            capability_surface_resolver,
            capability_input_resolver,
            capability_result_writer,
            capability_recorder,
        ) = capability.mode().into_parts(
            Arc::clone(&runtime_milestone_sink),
            group_thread_harness.service.clone() as Arc<dyn SessionThreadService>,
            process_system.clone(),
            self.trajectory_observer.clone(),
        )?;

        // Enabler (b): production resolves `CapabilitySurfacePolicy::allow_all()` for a
        // top-level user turn; mirror that for bridged groups (narrowed
        // override = the #5647 seam). Bridge ids now survive narrowing via
        // disclosure's synthetic bridge surface, so this is production parity,
        // not a bug dodge.
        let capability_surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver> =
            if self.tool_disclosure.is_enabled() {
                Arc::new(StaticCapabilitySurfaceProfileResolver {
                    policy: self
                        .narrowed_bridged_policy
                        .unwrap_or(CapabilitySurfacePolicy::allow_all()),
                })
            } else {
                capability_surface_resolver
            };

        // --- loop-exit evidence (group-level, built once) -----------------
        let await_edge_store = Arc::new(AwaitEdgeStore::new(process_system.dependencies()));
        let await_edge_resolver = Arc::new(AwaitEdgeResolver::new_unbound(
            Arc::clone(&await_edge_store),
            turn_runtime.clone() as Arc<dyn ironclaw_turns::AgentTurnSpawnTreeRuntimePort>,
            capability_result_writer.clone(),
            group_thread_harness.service.clone(),
        ));
        let await_edge_driver = Arc::new(ScopeRecoveryDriver::new(
            Arc::clone(&await_edge_resolver),
            Arc::clone(&await_edge_store),
        ));
        let turn_state_for_evidence: Arc<dyn AgentTurnRuntimePort> = turn_runtime.clone();
        let mut evidence = ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
            group_thread_harness.service.clone(),
            turn_state_for_evidence,
            Arc::clone(&loop_checkpoint_store),
            Arc::clone(&await_edge_store)
                as Arc<dyn ironclaw_turn_runner::loop_exit_applier::AwaitDependentRunEvidenceStore>,
            group_thread_scope.clone(),
        );
        if let Some(approval_requests) = capability_recorder.approval_requests_store() {
            evidence = evidence.with_approval_gate_evidence(
                ironclaw_composition::test_support::build_approval_gate_evidence_for_test(
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
            let actor_user = base.canonical_actor_user()?;
            let (sink, scope) =
                ironclaw_composition::test_support::trace_capture_turn_event_sink_for_test(
                    group_thread_harness.service.clone() as Arc<dyn SessionThreadService>,
                    base.canonical_binding.tenant_id.as_str(),
                    actor_user.as_str(),
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
        let model_gateway: Arc<dyn HostManagedModelGateway> =
            Arc::clone(&scope_gateway) as Arc<dyn HostManagedModelGateway>;
        let user_profile_source: Arc<dyn HostUserProfileSource> =
            ironclaw_composition::test_support::build_user_profile_source_for_test(
                capability_recorder.profile_filesystem(),
            );
        let mut runtime_thread_service =
            group_thread_harness.service.clone() as Arc<dyn SessionThreadService>;
        if self.fail_append_finalized_assistant_message {
            runtime_thread_service = Arc::new(
                FailingTranscriptWriteThreadService::append_finalized_assistant_message(
                    runtime_thread_service,
                ),
            );
        }
        if self.fail_append_tool_result_reference {
            runtime_thread_service = Arc::new(
                FailingTranscriptWriteThreadService::append_tool_result_reference(
                    runtime_thread_service,
                ),
            );
        }

        // --- steering/follow-up input queue (production-parity) ----------------
        // Production always wires the durable `FilesystemHostInputQueue` over
        // the composed filesystem, so a message hitting a busy thread is
        // queued as steering input for the active run instead of rejected —
        // and the queue survives a planned-runtime restart over the same
        // durable store (`restart_planned_runtime` with `StorageMode::LibSql`).
        // The harness mirrors that shape over the group's composite: the SAME
        // queue instance is both the loop's drain reader (`parts.input_queue`)
        // and every thread's inbound enqueue port.
        let host_input_queue = Arc::new(ironclaw_loop_host::FilesystemHostInputQueue::new(
            Arc::clone(&processes_scoped_fs),
            ironclaw_turns::TurnScope::new_with_owner(
                base.canonical_binding.tenant_id.clone(),
                base.canonical_binding.agent_id.clone(),
                base.canonical_binding.project_id.clone(),
                base.canonical_binding.thread_id.clone(),
                Some(base.canonical_binding.actor_user_id.clone()),
            )
            .to_resource_scope(),
            Arc::clone(&runtime_thread_service),
        ));
        let host_input_queue_for_cancel_reconcile: Arc<
            dyn ironclaw_loop_host::HostInputQueueReconcile,
        > = host_input_queue.clone();

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
                Arc::new(in_memory_backed_budget_gate_store()) as Arc<dyn BudgetGateStorePort>,
                Arc::new(InMemoryBudgetEventSink::new()) as Arc<dyn BudgetEventSink>,
                &BudgetDefaults::compiled_defaults(),
            );
            let account = ResourceAccount::user(
                base.canonical_binding.tenant_id.clone(),
                base.canonical_actor_user()?,
            );
            (Some(accountant), Some(governor), Some(account))
        } else {
            (None, None, None)
        };
        let security_audit_sink: Arc<RecordingSecurityAuditSink> =
            Arc::new(RecordingSecurityAuditSink::default());
        let hook_security_audit_sink: Arc<dyn ironclaw_event_log::SecurityAuditSink> =
            security_audit_sink.clone();

        // W5-WIRING-PARITY: bind the literal to a local before consuming it so
        // `harness_planned_runtime_parts_shape` can read the REAL Some/None
        // shape this group's runtime is built from — the only place this
        // struct value exists before `build_default_planned_runtime` takes it
        // by value.
        let milestone_sink_for_assertions = Arc::clone(&milestone_sink);
        // E-MEMORY: derive the memory consumers through the production
        // decision helper so an undeclared lifecycle hook is never wired here
        // either — the same gate `build_reborn_runtime` applies.
        let memory_consumers = self.bound_memory.as_ref().map(|(provider, lifecycle)| {
            ironclaw_composition::memory_lifecycle_consumers(Some(Arc::clone(provider)), lifecycle)
        });
        // E-PROFILE / E-MEMORY: resolve ONE effective profile source and wire
        // the SAME `Arc` into the runtime parts and `GroupSharedStorage` (so
        // `user_profile_source_for_test()` reads what the runtime uses). A
        // bound provider's declaration is authoritative, mirroring production
        // (`runtime.rs`): ProfileRead → the provider-backed adapter; bound
        // WITHOUT ProfileRead → `EmptyUserProfileSource` — never the group's
        // local-dev filesystem source, which would fabricate profile reads
        // production skips. No bound provider → the group default (HostRuntime
        // mode: local-dev memory filesystem so `profile_set` writes read back;
        // other backends: Empty).
        let effective_user_profile_source: Arc<dyn HostUserProfileSource> =
            match memory_consumers.as_ref() {
                Some(consumers) => consumers.user_profile_source.clone().unwrap_or_else(|| {
                    Arc::new(ironclaw_loop_host::EmptyUserProfileSource)
                        as Arc<dyn HostUserProfileSource>
                }),
                None => Arc::clone(&user_profile_source),
            };
        let reply_attachment_intent_port = capability.reply_attachment_intent_port();
        let parts = DefaultPlannedRuntimeParts {
            process_system: process_system.clone(),
            thread_service: runtime_thread_service,
            thread_scope: group_thread_scope,
            model_gateway,
            loop_checkpoint_store,
            milestone_sink: runtime_milestone_sink,
            capability_factory,
            capability_surface_resolver,
            capability_result_writer,
            subagent_await_edge_writer: await_edge_driver
                as Arc<dyn ironclaw_loop_host::AwaitEdgeWriter>,
            subagent_await_edge_settler: await_edge_resolver
                as Arc<dyn ironclaw_loop_host::AwaitEdgeSettler>,
            subagent_await_edge_evidence: await_edge_store
                as Arc<dyn ironclaw_turn_runner::loop_exit_applier::AwaitDependentRunEvidenceStore>,
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
                heartbeat_interval: self
                    .runner_heartbeat_interval_override
                    .unwrap_or(DefaultPlannedRuntimeConfig::default().heartbeat_interval),
                // Enabler (b): test groups are hermetically pinned and never
                // resolve this production mode from the process environment.
                tool_disclosure: self.tool_disclosure,
                tool_disclosure_profile_pins: std::collections::HashMap::from([(
                    ironclaw_loop_contracts::CapabilitySurfaceProfileId::new("interactive_tools")
                        .expect("valid integration capability profile id"),
                    vec![
                        ironclaw_host_api::ids::CapabilityId::new("github.search_code")
                            .expect("valid integration profile pin"),
                    ],
                )]),
                // Loop-level counterpart of hermetic `LLM_MAX_RETRIES=0`:
                // production rides out provider outages for minutes (deep
                // availability retries with long backoff), which would stall
                // scenarios that deliberately script a model failure (e.g.
                // `failure_category_demasked`). One attempt keeps deliberate
                // failure paths fast while still exercising retry-then-abort.
                planned_model_availability_retry_attempts: Some(
                    std::num::NonZeroU32::new(1).expect("nonzero"),
                ),
                planned_default_iteration_limit: self.planned_default_iteration_limit,
                ..DefaultPlannedRuntimeConfig::default()
            },
            model_route_resolver: None,
            // E-GATEWAY: left `None` — it does not gate whether a run reaches
            // `Cancelled`. `RebornLoopDriverHostFactory` always builds its own
            // default `AgentTurnRunCancellationFactory`, whose cancel poll loop
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
            input_queue: Some(
                host_input_queue.clone() as Arc<dyn ironclaw_loop_host::HostInputQueue>
            ),
            input_queue_reconcile: Some(
                host_input_queue.clone() as Arc<dyn ironclaw_loop_host::HostInputQueueReconcile>
            ),
            identity_context_source: Arc::new(EmptyIdentityContextSource),
            // E-PROFILE / E-MEMORY: the ONE effective profile source (also
            // stashed on `GroupSharedStorage`, so
            // `user_profile_source_for_test()` reads exactly what the runtime
            // wires).
            user_profile_source: Arc::clone(&effective_user_profile_source),
            // E-MEMORY: derived through the PRODUCTION lifecycle-gating helper
            // when a bound memory provider is opted in
            // (`with_bound_memory_provider`); `None` for every other group, so
            // existing tests are behavior-identical. wiring_parity.rs carries
            // the explicit divergence for the un-opted default.
            memory_context_service: memory_consumers
                .as_ref()
                .and_then(|consumers| consumers.memory_context_service.clone()),
            after_turn_memory_writer: memory_consumers
                .as_ref()
                .and_then(|consumers| consumers.after_turn_memory_writer.clone()),
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
            // W5-WIRING-PARITY: production local-dev always wires
            // TracingSecurityAuditSink; the harness mirrors the shape with a
            // recorder so integration tests can assert emitted events.
            hook_security_audit_sink: Some(hook_security_audit_sink),
            turn_event_sink: composed_turn_event_sink,
            attachment_read_port: capability_recorder
                .attachment_test_support()
                .map(|support| support.read_port),
            prompt_diagnostic_sink: Some(Arc::new(
                ironclaw_assistant::inspector_store::InMemoryDiagnosticStore::default(),
            )),
            reply_attachment_intent_port: Some(reply_attachment_intent_port),
            // §5.2.9 render-from-record: the SAME durable gate-record store this
            // group's capability port persists `GateRecord::Auth` into, so the
            // turn executor re-reads an auth block's `credential_requirements`
            // from the exact record the port saved (mirrors production
            // `runtime.rs`'s `local_runtime.gate_record_store`).
            gate_record_store: capability.gate_record_store(),
            scheduler_wake_wiring: None,
        };
        let planned_runtime_parts_shape = harness_planned_runtime_parts_shape(&parts);
        let composition = build_default_planned_runtime(parts)?;

        Ok(RebornIntegrationGroup {
            shared: Arc::new(GroupSharedStorage {
                restart_builder,
                composite: base.composite,
                storage_reopen: base.storage_reopen,
                turn_root: base.turn_root,
                product_harness: base.product_harness,
                capability,
                // Production parity: the composed coordinator is decorated with
                // the cancel-time steering-queue reconciler, exactly like
                // `build_reborn_runtime` wires it.
                coordinator: Arc::new(
                    ironclaw_turn_runner::steering_reconcile::CancelReconcilingTurnCoordinator::new(
                        composition.coordinator,
                        host_input_queue_for_cancel_reconcile,
                    ),
                ),
                scheduler_handle: composition.scheduler_handle,
                scope_gateway,
                process_system,
                turn_runtime,
                canonical_binding: base.canonical_binding,
                capability_recorder,
                input_enqueue: host_input_queue,
                user_profile_source: effective_user_profile_source,
                turn_event_sink: self.turn_event_sink,
                durable_event_log,
                durable_event_sink,
                security_audit_sink,
                milestone_sink: milestone_sink_for_assertions,
                trace_capture_scope: trace_capture.map(|(_, scope)| scope),
                budget_governor,
                budget_account,
                planned_runtime_parts_shape,
                real_gate_dispatch_services: self.real_gate_dispatch_services,
                channel_connection: self.channel_connection,
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

/// Mirrors production's single durable milestone sink while retaining the
/// integration recorder for focused assertions.
struct FanOutLoopHostMilestoneSink(Vec<Arc<dyn LoopHostMilestoneSink>>);

#[async_trait::async_trait]
impl LoopHostMilestoneSink for FanOutLoopHostMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: LoopHostMilestone,
    ) -> Result<(), ironclaw_loop_contracts::AgentLoopHostError> {
        let mut first_error = None;
        for sink in &self.0 {
            if let Err(error) = sink.publish_loop_milestone(milestone.clone()).await {
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
    /// Additive raw-provider call recording for this thread.
    record_model_calls: bool,
    /// C-ATTACH seam: overrides `LlmModelProfileRoute.model_override` (the same
    /// production model-pin field, `model_gateway.rs:160-162`). `None` keeps the
    /// prior behavior (scripted model id, not a vision pattern, so image parts
    /// are dropped); `Some` routes through a vision-capable id so `convert_messages`
    /// builds `ContentPart::ImageUrl` parts.
    model_override: Option<String>,
}

/// A thread's model-call behavior: exactly one of normal scripted playback,
/// parked-until-released, bounded recoverable failure, or unconditional
/// failure. One enum instead of an `Option<ParkingModelGate>` + `bool` pair
/// (mirrors `ShellMode` in `builder.rs`) so the four modes are mutually exclusive BY CONSTRUCTION —
/// no tuple-priority rule needed at the dispatch site, and no state can
/// silently ask for "parked AND failing" at once.
#[derive(Default)]
pub(crate) enum ThreadModelMode {
    /// Normal scripted playback (the default).
    #[default]
    Normal,
    /// This thread's model call parks until the gate is released (E-GATEWAY
    /// seam), enabling a mid-turn cancel test.
    Parked(ParkingModelGate),
    /// Delays each scripted vendor call while keeping the real decorator chain.
    Delayed(Duration),
    /// Reports a recoverable provider failure a bounded number of times, then
    /// resumes normal scripted playback.
    Recoverable(RecoverableModelFailureScript),
    /// Primary vendor failure followed by ordered fallback success through the
    /// production retry/failover/circuit-breaker/decorator chain.
    FallbackAdvance,
    /// This thread's model call always fails with a fixed non-retryable
    /// `LlmError` (E-GATEWAY seam, C-ERRORS) instead of playing back
    /// `replies`. See [`super::scripted_provider::ErrLlm`].
    Failing(ErrLlmKind),
}

type ThreadModelProviderParts = (
    Arc<dyn LlmProvider>,
    Option<ModelProviderCallProbe>,
    Option<Arc<dyn LlmProvider>>,
    Option<FallbackProviderCallProbe>,
);

impl<'g> RebornThreadBuilder<'g> {
    /// Set the scripted model replies for this thread (consumed in order at the
    /// raw-provider seam, one per model turn).
    pub fn script(mut self, replies: impl IntoIterator<Item = RebornScriptedReply>) -> Self {
        self.replies = replies.into_iter().collect();
        self
    }

    pub(crate) fn model_mode(mut self, mode: ThreadModelMode) -> Self {
        self.model_mode = mode;
        self
    }

    pub(crate) fn record_model_calls_for_test(mut self, record: bool) -> Self {
        self.record_model_calls = record;
        self
    }

    /// Park this thread's model call until `gate` is released (E-GATEWAY seam).
    /// The parking provider sits at the same vendor-SDK seam as the scripted
    /// provider, so the real decorator chain still runs on top.
    pub fn park_model(mut self, gate: ParkingModelGate) -> Self {
        self.model_mode = ThreadModelMode::Parked(gate);
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
    pub fn fail_model(mut self) -> Self {
        self.model_mode = ThreadModelMode::Failing(ErrLlmKind::ContextLength);
        self
    }

    /// Credentials arm of [`Self::fail_model`]: the model call always fails
    /// with non-retryable `LlmError::AuthFailed`, driving the pinned
    /// `model_credentials_unavailable` failure category through the real
    /// provider-error mapping.
    pub fn fail_model_auth(mut self) -> Self {
        self.model_mode = ThreadModelMode::Failing(ErrLlmKind::AuthFailed);
        self
    }

    /// Fail the primary vendor route as unavailable and let loop recovery
    /// advance to the scripted fallback provider.
    pub fn advance_fallback_after_unavailable(mut self) -> Self {
        self.model_mode = ThreadModelMode::FallbackAdvance;
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
        if self.actor_id.is_some() && shared.durable_event_log.is_some() {
            return Err(
                "custom-actor group threads cannot use the canonical-actor durable milestone sink"
                    .into(),
            );
        }

        // --- product workflow + per-thread binding -----------------------------
        // A fresh adapter + ingress each time (cheap, stateless). The binding
        // service is backed by `shared.product_harness`, which is shared; the
        // idempotency ledger is also shared (per-binding idempotency).
        let actor_id = self.actor_id.as_deref().unwrap_or(HARNESS_ACTOR_ID);
        let ingress = RebornTestIngress::new("reborn-itest", "itest-install")?;
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
        // The run is scoped to the acting user (the pinger); owner == actor
        // under ephemeral-per-ping. Mirrors production scope derivation.
        let turn_scope = TurnScope::new_with_owner(
            binding.tenant_id.clone(),
            binding.agent_id.clone(),
            binding.project_id.clone(),
            binding.thread_id.clone(),
            Some(binding.actor_user_id.clone()),
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
        // `Parked` swaps in the parking wrapper. `ThreadModelMode` keeps all
        // provider modes mutually exclusive by construction — no priority
        // rule is needed here.
        let (raw, model_provider_call_probe, fallback_raw, fallback_provider_call_probe):
            ThreadModelProviderParts = match self.model_mode {
            ThreadModelMode::Parked(gate) => (
                Arc::new(parking_trace_llm(gate, scripted_llm.clone())),
                None,
                None,
                None,
            ),
            ThreadModelMode::Delayed(delay) => (
                Arc::new(delayed_trace_llm(delay, scripted_llm.clone())),
                None,
                None,
                None,
            ),
            ThreadModelMode::Recoverable(script) => {
                let (provider, probe) = recoverable_failure_trace_llm(
                    script.failure,
                    script.successful_calls_before_failures,
                    script.failures,
                    scripted_llm.clone(),
                );
                (Arc::new(provider), Some(probe), None, None)
            }
            ThreadModelMode::FallbackAdvance => {
                let (primary, fallback, probe) =
                    scripted_fallback_vendor_pair(scripted_llm.clone());
                (
                    Arc::new(primary),
                    None,
                    Some(Arc::new(fallback)),
                    Some(probe),
                )
            }
            ThreadModelMode::Failing(kind) => {
                let (provider, probe) = ErrLlm::new(kind);
                (Arc::new(provider), Some(probe), None, None)
            }
            ThreadModelMode::Normal => (scripted_llm.clone(), None, None, None),

        };
        let (raw, model_provider_call_probe) =
            if self.record_model_calls && model_provider_call_probe.is_none() {
                let (provider, probe) = recording_llm(raw);
                (Arc::new(provider) as Arc<dyn LlmProvider>, Some(probe))
            } else {
                (raw, model_provider_call_probe)
            };
        let session = create_session_manager(SessionConfig {
            session_path: shared
                .turn_root
                .path()
                .join(format!("{}.session.json", self.conversation_id)),
            ..SessionConfig::default()
        })
        .await;
        let mut llm_config = ironclaw_llm::testing::nearai_test_config(SCRIPTED_MODEL_NAME);
        let provider = if let Some(fallback) = fallback_raw {
            llm_config.max_retries = 1;
            llm_config.circuit_breaker_threshold = Some(2);
            llm_config.response_cache_enabled = true;
            llm_config.nearai.fallback_model = Some(SCRIPTED_FALLBACK_MODEL_NAME.to_string());
            provider_chain_over_with_fallback(raw, fallback, &llm_config, session).await?
        } else {
            provider_chain_over(raw, &llm_config, session).await?
        };
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
        let baseline_security_audit_count = shared.security_audit_sink.events().len();
        let baseline_turn_event_count = shared
            .turn_event_sink
            .as_ref()
            .map(|sink| sink.events().len())
            .unwrap_or(0);
        let baseline_milestone_count = shared.milestone_sink.milestones().len();

        // --- per-thread workflow over the SHARED coordinator --------------------
        let binding_service: Arc<dyn ProductBindingResolver> =
            Arc::new(shared.product_harness.binding_service()?);
        let mut inbound_service = DefaultInboundTurnService::new(
            Arc::clone(&binding_service),
            thread_harness.service_instance()?,
            Arc::clone(&shared.coordinator),
            Arc::clone(&shared.input_enqueue),
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
        let mut workflow = DefaultProductSurface::new(inbound, ledger, binding_service);

        // Real gate-dispatch seam: wire the harness's own local-dev interaction
        // services, but over the GROUP's shared `turn_store` (not the harness's
        // own disjoint `local_runtime.turn_state`) — otherwise their turn-run
        // locator can never see this group's real runs. Only when the builder
        // opted in (`with_real_gate_dispatch_services`); every other group's
        // workflow keeps the default Rejecting stubs.
        if shared.real_gate_dispatch_services {
            let harness = match &shared.capability {
                GroupCapability::HostRuntime(arc) => arc,
                GroupCapability::Recording
                | GroupCapability::RecordingNoProgress
                | GroupCapability::RecordingRecoverablePortError => {
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
                .standalone_approval_interaction_service_with_turn_state_for_test(
                    Arc::clone(&shared.coordinator),
                    shared.process_system.gates(),
                )?
                .ok_or(
                    "local-dev approval interaction service unavailable (harness has no local runtime)",
                )?;
            let auth_interaction_service = reborn_services
                .standalone_auth_interaction_service_with_turn_state_for_test(
                    Arc::clone(&shared.coordinator),
                    shared.process_system.gates(),
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
            workflow: Arc::new(workflow),
            conversation_id: self.conversation_id,
            actor_id: actor_id.to_owned(),
            binding,
            turn_scope,
            turn_runtime: Arc::clone(&shared.turn_runtime),
            thread_harness,
            coordinator: Arc::clone(&shared.coordinator),
            event_seq: AtomicU64::new(1),
            capability_recorder,
            scripted_llm,
            model_provider_call_probe,
            fallback_provider_call_probe,
            _shared: Arc::clone(&shared),
            baseline_invocation_count,
            baseline_egress_count,
            baseline_result_count,
            baseline_process_count,
            baseline_network_count,
            baseline_security_audit_count,
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
