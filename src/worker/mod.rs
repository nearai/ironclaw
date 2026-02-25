//! Worker mode for running inside Docker containers.
//!
//! When `clawyer worker` is invoked, the binary starts in worker mode:
//! - Connects to the orchestrator over HTTP
//! - Uses a `ProxyLlmProvider` that routes LLM calls through the orchestrator
//! - Runs container-safe tools (shell, file ops, patch)
//! - Reports status and completion back to the orchestrator
//!
//! ```text
//! ┌────────────────────────────────┐
//! │        Docker Container         │
//! │                                 │
//! │  clawyer worker                │
//! │    ├─ ProxyLlmProvider ─────────┼──▶ Orchestrator /worker/{id}/llm/complete
//! │    ├─ SafetyLayer               │
//! │    ├─ ToolRegistry              │
//! │    │   ├─ shell                 │
//! │    │   ├─ read_file             │
//! │    │   ├─ write_file            │
//! │    │   ├─ list_dir              │
//! │    │   └─ apply_patch           │
//! │    └─ WorkerHttpClient ─────────┼──▶ Orchestrator /worker/{id}/status
//! │                                 │
//! └────────────────────────────────┘
//! ```

pub mod api;
pub mod claude_bridge;
pub mod proxy_llm;
pub mod runtime;

pub use api::WorkerHttpClient;
pub use claude_bridge::ClaudeBridgeRuntime;
pub use proxy_llm::ProxyLlmProvider;
pub use runtime::WorkerRuntime;
