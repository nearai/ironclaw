//! Process lifecycle contracts for IronClaw Reborn.
//!
//! `ironclaw_processes` stores and manages host-tracked background capability
//! processes. It owns lifecycle mechanics, not capability authorization or
//! runtime dispatch policy.
//!
//! # Module map
//!
//! - [`types`] — public data types, errors, and core traits
//!   ([`ProcessStore`], [`ProcessResultStore`], [`ProcessExecutor`],
//!   [`ProcessManager`])
//! - [`cancellation`] — cooperative cancellation tokens + per-process registry
//! - [`host`] — read/poll/await/cancel surface ([`ProcessHost`],
//!   [`ProcessSubscription`])
//! - [`filesystem_store`] — the process `ProcessStore` / `ProcessResultStore`
//!   (durable over libSQL/Postgres; in-memory-backed over `InMemoryBackend` in
//!   tests, via the `test-support` helpers — arch-simplification §4.3)
//! - [`wrappers`] — composable decorators ([`EventingProcessStore`],
//!   [`ResourceManagedProcessStore`])
//! - [`services`] — composition root ([`ProcessServices`]) and the
//!   production [`BackgroundProcessManager`]

mod cancellation;
mod filesystem_store;
mod host;
mod services;
#[cfg(any(test, feature = "test-support"))]
mod test_support;
mod types;
mod wrappers;

pub use cancellation::{ProcessCancellationRegistry, ProcessCancellationToken};
pub use filesystem_store::{FilesystemProcessResultStore, FilesystemProcessStore};
pub use host::{ProcessHost, ProcessSubscription};
pub use services::{
    BackgroundErrorHandler, BackgroundFailure, BackgroundFailureStage, BackgroundProcessManager,
    ProcessServices,
};
#[cfg(any(test, feature = "test-support"))]
pub use test_support::{
    in_memory_backed_process_result_store, in_memory_backed_process_services,
    in_memory_backed_process_store, in_memory_backed_processes_filesystem,
};
pub use types::{
    ProcessError, ProcessExecutionError, ProcessExecutionRequest, ProcessExecutionResult,
    ProcessExecutor, ProcessExit, ProcessManager, ProcessRecord, ProcessResultRecord,
    ProcessResultStore, ProcessStart, ProcessStatus, ProcessStore,
};
pub use wrappers::{EventingProcessStore, ResourceManagedProcessStore};
