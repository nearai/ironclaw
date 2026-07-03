use std::path::PathBuf;

pub mod assertions;
pub mod cleanup;
#[cfg(feature = "libsql")]
pub mod gateway_workflow_harness;
pub mod instrumented_llm;
#[cfg(feature = "libsql")]
pub mod live_harness;
#[cfg(feature = "libsql")]
pub mod live_mission_helpers;
pub mod metrics;
pub mod mock_mcp_server;
pub mod mock_openai_server;
pub mod replay_outcome;
pub mod test_channel;
pub mod test_rig;
pub mod trace_llm;
#[cfg(feature = "libsql")]
pub mod trace_runner;

pub fn repo_root() -> PathBuf {
    std::env::var_os("GITHUB_WORKSPACE")
        .or_else(|| std::env::var_os("CARGO_MANIFEST_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}
