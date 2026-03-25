//! Engine v2 bridge — connects `ironclaw_engine` to existing infrastructure.
//!
//! Strategy C: parallel deployment. When `ENGINE_V2=true`, user messages
//! route through the engine instead of the existing agentic loop. All
//! existing behavior is unchanged when the flag is off.

mod effect_adapter;
mod llm_adapter;
mod router;
mod store_adapter;

pub use router::{
    handle_approval, handle_with_engine, is_engine_v2_enabled, pending_approval_for_user_thread,
};
