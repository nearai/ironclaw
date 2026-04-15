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
pub mod skill_migration;
mod store_adapter;
mod workspace_reader;

pub use cost_guard_gate::CostGuardBudgetGate;
pub use workspace_reader::WorkspaceReaderAdapter;

pub use effect_adapter::EffectBridgeAdapter;
pub use router::{
    AuthCallbackContinuation,
    // DTO types
    EngineMissionDetail,
    EngineMissionInfo,
    EngineProjectInfo,
    EngineStepInfo,
    EngineThreadDetail,
    EngineThreadInfo,
    clear_engine_pending_auth,
    // Query functions
    fire_engine_mission,
    get_engine_mission,
    get_engine_pending_auth,
    get_engine_pending_gate,
    get_engine_project,
    get_engine_thread,
    // Action handlers
    handle_approval,
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
};

#[cfg(feature = "libsql")]
pub use router::reset_engine_state;

// Exposed for caller-level testing of the cross-user thread_id guard
#[cfg(test)]
pub(crate) use router::handle_mission_notification;
