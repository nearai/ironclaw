//! Private same-build wire adapter for running the canonical loop out of process.
//!
//! This is deliberately not a stable protocol. Host and worker are built from
//! one commit and the worker validates the resolved driver identity before it
//! executes. Raw model-visible content stays in this implementation crate rather
//! than becoming contracts-tier wire vocabulary.

mod client;
mod protocol;
mod server;
#[cfg(test)]
mod tests;

pub use client::{RemoteAgentLoopDriverHost, read_worker_bootstrap, remote_host_from_stdio};
pub use protocol::{
    LOOP_WORKER_MAX_FRAME_BYTES, LOOP_WORKER_WIRE_VERSION, LoopWorkerBootstrap, LoopWorkerFailure,
    LoopWorkerInvocation, LoopWorkerOutcome, LoopWorkerSettings,
};
pub use server::serve_loop_worker;
