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
//! - [`memory_store`] — in-memory `ProcessStore` / `ProcessResultStore`
//! - [`filesystem_store`] — durable filesystem-backed implementations
//! - [`wrappers`] — composable decorators ([`EventingProcessStore`],
//!   [`ResourceManagedProcessStore`])
//! - [`services`] — composition root ([`ProcessServices`]) and the
//!   production [`BackgroundProcessManager`]

pub mod cancellation;
pub mod filesystem_store;
pub mod host;
pub mod memory_store;
pub mod services;
pub mod types;
pub mod wrappers;

pub use cancellation::{ProcessCancellationRegistry, ProcessCancellationToken};
pub use filesystem_store::{FilesystemProcessResultStore, FilesystemProcessStore};
pub use host::{ProcessHost, ProcessSubscription};
pub use memory_store::{InMemoryProcessResultStore, InMemoryProcessStore};
pub use services::{
    BackgroundErrorHandler, BackgroundFailure, BackgroundFailureStage, BackgroundProcessManager,
    ProcessServices,
};
pub use types::{
    ProcessError, ProcessExecutionError, ProcessExecutionRequest, ProcessExecutionResult,
    ProcessExecutor, ProcessExit, ProcessManager, ProcessRecord, ProcessResultRecord,
    ProcessResultStore, ProcessStart, ProcessStatus, ProcessStore,
};
pub use wrappers::{EventingProcessStore, ResourceManagedProcessStore};
