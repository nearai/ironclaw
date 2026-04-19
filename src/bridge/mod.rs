//! Engine v2 bridge — connects `ironclaw_engine` to existing infrastructure.
//!
//! Strategy C: parallel deployment. When `ENGINE_V2=true`, user messages
//! route through the engine instead of the existing agentic loop. All
//! existing behavior is unchanged when the flag is off.

pub mod auth_manager;
mod cost_guard_gate;
mod effect_adapter;
mod llm_adapter;
mod router;
pub mod sandbox;
pub mod skill_migration;
mod store_adapter;
mod workspace_reader;

pub use cost_guard_gate::CostGuardBudgetGate;
pub use workspace_reader::WorkspaceReaderAdapter;

pub use effect_adapter::EffectBridgeAdapter;
pub use router::{
    // DTO types
    AttentionItem,
    AuthCallbackContinuation,
    // Typed outcome from v2 bridge handlers
    BridgeOutcome,
    EngineMissionDetail,
    EngineMissionInfo,
    EngineProjectInfo,
    EngineStepInfo,
    EngineThreadDetail,
    EngineThreadInfo,
    ProjectOverviewEntry,
    ProjectsOverviewResponse,
    clear_engine_pending_auth,
    discard_engine_pending_auth_request,
    // Query functions
    fire_engine_mission,
    get_engine_mission,
    get_engine_pending_gate,
    get_engine_project,
    get_engine_projects_overview,
    get_engine_thread,
    // Action handlers
    handle_approval,
    handle_auth_gate_resolution,
    handle_clear,
    handle_exec_approval,
    handle_expected,
    handle_external_callback,
    handle_interrupt,
    handle_new_thread,
    handle_with_engine,
    has_any_pending_gate,
    has_pending_auth,
    // Initialization
    init_engine,
    is_engine_v2_enabled,
    list_engine_missions,
    list_engine_projects,
    list_engine_thread_events,
    list_engine_thread_steps,
    list_engine_threads,
    pause_engine_mission,
    resolve_engine_auth_callback,
    resolve_gate,
    resume_engine_mission,
    transition_engine_pending_auth_request_to_pairing,
};

#[cfg(feature = "libsql")]
pub use router::reset_engine_state;

// `engine_retrospectives_for_test` is a test-only reachability surface —
// integration tests live in a separate crate, so `#[cfg(test)]` wouldn't
// expose it. `#[doc(hidden)]` keeps it out of public docs and signals
// that it is not a supported API.
#[cfg(feature = "libsql")]
#[doc(hidden)]
pub use router::engine_retrospectives_for_test;

// Exposed for caller-level testing of the cross-user thread_id guard
#[cfg(test)]
pub(crate) use router::handle_mission_notification;
