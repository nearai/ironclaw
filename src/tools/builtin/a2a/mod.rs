//! A2A (Agent-to-Agent) protocol bridge tool.
//!
//! Connects to remote agents via the Google A2A protocol (JSON-RPC 2.0 + SSE
//! streaming). The tool sends a query, reads the first SSE event to determine
//! if the result is immediate or async, and spawns a background consumer for
//! long-running tasks that pushes results back via `inject_tx`.

mod bridge;
pub(crate) mod protocol;

pub use bridge::A2aBridgeTool;
